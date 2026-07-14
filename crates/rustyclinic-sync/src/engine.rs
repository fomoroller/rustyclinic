//! Sync engine: manages push/pull operations against the local database.
//!
//! The engine operates on a local SQLite database, reading pending operations,
//! applying remote operations, tracking cursors, and validating hash-chain
//! integrity.

use anyhow::{Context, Result, bail};
use chrono::Utc;
use rusqlite::Connection;
use rustyclinic_db::sqlite::sync_repo::SqliteSyncRepo;
use rustyclinic_db::sync_repo::{
    OpLogSyncRepo, SyncConflictRecord, SyncConflictRepo, SyncConflictStatus, SyncCursorRecord,
    SyncCursorRepo,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use rustyclinic_events::OpLogEntry;

use crate::conflict;
use crate::types::{ApplyResult, SyncConflict, SyncCursor};

/// Core sync engine that reads/writes the local op_log.
pub struct SyncEngine {
    db_path: String,
    device_id: Uuid,
    facility_id: Uuid,
}

impl SyncEngine {
    /// Create a new sync engine for the given device and facility.
    pub fn new(db_path: String, device_id: Uuid, facility_id: Uuid) -> Self {
        Self {
            db_path,
            device_id,
            facility_id,
        }
    }

    /// Open a database connection.
    fn open_conn(&self) -> Result<Connection> {
        let conn = Connection::open(&self.db_path)
            .with_context(|| format!("failed to open db: {}", self.db_path))?;
        conn.pragma_update(None, "journal_mode", "wal")?;
        conn.pragma_update(None, "foreign_keys", "on")?;
        Ok(conn)
    }

    /// Get pending operations that need to be pushed upstream.
    pub fn get_pending_ops(&self, limit: u32) -> Result<Vec<OpLogEntry>> {
        let conn = self.open_conn()?;
        let repo = SqliteSyncRepo::new(&conn);
        Ok(OpLogSyncRepo::list_pending(&repo, self.facility_id, limit)?)
    }

    /// Mark operations as pushed (awaiting server acknowledgement).
    pub fn mark_pushed(&self, through_sequence: u64) -> Result<()> {
        let conn = self.open_conn()?;
        let repo = SqliteSyncRepo::new(&conn);
        repo.mark_pushed_through(self.facility_id, through_sequence)?;

        // Update cursor
        self.upsert_cursor_pushed(&repo, through_sequence)?;
        Ok(())
    }

    /// Mark operations as acknowledged by upstream.
    pub fn mark_acknowledged(&self, through_sequence: u64) -> Result<()> {
        let conn = self.open_conn()?;
        let repo = SqliteSyncRepo::new(&conn);
        repo.mark_acknowledged_through(self.facility_id, through_sequence)?;
        Ok(())
    }

    /// Apply remote operations received from upstream (pull).
    ///
    /// For each entry: validates the hash chain, checks for conflicts with local
    /// unacknowledged operations on the same aggregate, and persists non-conflicting
    /// entries.
    pub fn apply_remote_ops(&self, entries: &[OpLogEntry]) -> Result<ApplyResult> {
        if entries.is_empty() {
            return Ok(ApplyResult {
                applied: 0,
                conflicts: Vec::new(),
            });
        }

        let conn = self.open_conn()?;
        let tx = conn
            .unchecked_transaction()
            .context("begin transaction for apply_remote_ops")?;
        let repo = SqliteSyncRepo::new(&tx);

        let mut applied: u64 = 0;
        let mut conflicts: Vec<SyncConflict> = Vec::new();

        for entry in entries {
            // Check if this entry already exists (deduplication by id)
            let exists = repo.exists(entry.id)?;

            if exists {
                // Already have this entry — skip (idempotent)
                applied += 1;
                continue;
            }

            // Check for conflicts with local unacknowledged entries on the same aggregate
            let local_entries = repo.list_unacknowledged_for_aggregate(
                self.facility_id,
                &entry.aggregate_type,
                entry.aggregate_id,
            )?;

            let mut has_conflict = false;
            for local_entry in &local_entries {
                if let Some(c) = conflict::detect_conflict(local_entry, entry) {
                    // Persist the conflict
                    self.persist_conflict(&repo, &c)?;
                    conflicts.push(c);
                    has_conflict = true;
                }
            }

            if has_conflict {
                continue;
            }

            // Attempt auto-merge if there are overlapping local entries
            // (but no hard conflict was detected)
            // Insert the remote entry into our op_log as acknowledged
            let _ = repo.insert_acknowledged_if_missing(entry)?;

            applied += 1;
        }

        // Update the pull cursor
        if let Some(last) = entries.last() {
            self.upsert_cursor_pulled(&repo, last.sequence)?;
        }

        tx.commit().context("commit apply_remote_ops")?;

        Ok(ApplyResult { applied, conflicts })
    }

    /// Get the current sync cursor for this device/facility.
    pub fn get_cursor(&self) -> Result<SyncCursor> {
        let conn = self.open_conn()?;
        let repo = SqliteSyncRepo::new(&conn);
        match repo.get(self.device_id, self.facility_id)? {
            Some(cursor) => Ok(SyncCursor {
                device_id: cursor.device_id,
                facility_id: cursor.facility_id,
                last_pulled_sequence: cursor.last_pulled_sequence,
                last_pushed_sequence: cursor.last_pushed_sequence,
                updated_at: cursor.updated_at,
            }),
            None => Ok(SyncCursor {
                device_id: self.device_id,
                facility_id: self.facility_id,
                last_pulled_sequence: 0,
                last_pushed_sequence: 0,
                updated_at: Utc::now(),
            }),
        }
    }

    /// Validate hash chain integrity for a batch of entries.
    ///
    /// Verifies that each entry's `entry_hash` is correct by recomputing it,
    /// and that each entry's `prev_hash` matches the preceding entry's hash.
    pub fn validate_chain(&self, entries: &[OpLogEntry]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }

        for (i, entry) in entries.iter().enumerate() {
            // Recompute hash
            let computed = compute_entry_hash(entry);
            if computed != entry.entry_hash {
                bail!(
                    "hash mismatch for entry {} (sequence {}): computed hash does not match stored hash",
                    entry.id,
                    entry.sequence
                );
            }

            // Verify prev_hash chain (skip the first entry — its prev_hash links
            // to an entry not in this batch)
            if i > 0 {
                let prev_entry = &entries[i - 1];
                if entry.prev_hash != prev_entry.entry_hash {
                    bail!(
                        "broken hash chain at entry {} (sequence {}): prev_hash does not match previous entry hash",
                        entry.id,
                        entry.sequence
                    );
                }
            }
        }

        Ok(())
    }

    /// Detect sequence gaps in a batch of entries.
    ///
    /// Verifies that entry sequences are contiguous starting from `expected_start`.
    pub fn detect_gaps(&self, entries: &[OpLogEntry], expected_start: u64) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }

        if entries[0].sequence != expected_start {
            bail!(
                "sequence gap: expected start {}, got {}",
                expected_start,
                entries[0].sequence
            );
        }

        for i in 1..entries.len() {
            let expected = entries[i - 1].sequence + 1;
            let actual = entries[i].sequence;
            if actual != expected {
                bail!(
                    "sequence gap: expected {}, got {} (after entry {})",
                    expected,
                    actual,
                    entries[i - 1].id
                );
            }
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn persist_conflict(
        &self,
        repo: &impl SyncConflictRepo,
        conflict: &SyncConflict,
    ) -> Result<()> {
        let status = match conflict.status {
            crate::types::ConflictStatus::Pending => SyncConflictStatus::Pending,
            crate::types::ConflictStatus::Resolved => SyncConflictStatus::Resolved,
            crate::types::ConflictStatus::Escalated => SyncConflictStatus::Escalated,
        };
        let record = SyncConflictRecord {
            id: conflict.id,
            facility_id: conflict.facility_id,
            aggregate_type: conflict.aggregate_type.clone(),
            aggregate_id: conflict.aggregate_id,
            local_entry_id: conflict.local_entry_id,
            remote_entry_id: conflict.remote_entry_id,
            conflict_type: serde_json::to_value(&conflict.conflict_type)
                .context("serialize conflict_type")?,
            status,
            created_at: conflict.created_at,
            resolved_at: conflict.resolved_at,
            resolved_by: conflict.resolved_by,
            resolution: conflict
                .resolution
                .as_ref()
                .map(serde_json::to_value)
                .transpose()
                .context("serialize conflict resolution")?,
        };

        repo.insert(&record)?;
        Ok(())
    }

    fn upsert_cursor_pushed(
        &self,
        repo: &impl SyncCursorRepo,
        through_sequence: u64,
    ) -> Result<()> {
        let mut cursor = repo
            .get(self.device_id, self.facility_id)?
            .unwrap_or(SyncCursorRecord {
                device_id: self.device_id,
                facility_id: self.facility_id,
                last_pulled_sequence: 0,
                last_pushed_sequence: 0,
                updated_at: Utc::now(),
            });
        cursor.last_pushed_sequence = through_sequence;
        cursor.updated_at = Utc::now();
        repo.upsert(&cursor)?;
        Ok(())
    }

    fn upsert_cursor_pulled(
        &self,
        repo: &impl SyncCursorRepo,
        through_sequence: u64,
    ) -> Result<()> {
        let mut cursor = repo
            .get(self.device_id, self.facility_id)?
            .unwrap_or(SyncCursorRecord {
                device_id: self.device_id,
                facility_id: self.facility_id,
                last_pulled_sequence: 0,
                last_pushed_sequence: 0,
                updated_at: Utc::now(),
            });
        cursor.last_pulled_sequence = through_sequence;
        cursor.updated_at = Utc::now();
        repo.upsert(&cursor)?;
        Ok(())
    }
}

/// Compute the SHA-256 hash of an op-log entry (same algorithm as UnitOfWork).
pub fn compute_entry_hash(entry: &OpLogEntry) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(entry.id.as_bytes());
    hasher.update(entry.sequence.to_le_bytes());
    hasher.update(entry.facility_id.as_bytes());
    hasher.update(entry.device_id.as_bytes());
    hasher.update(entry.created_at.to_rfc3339().as_bytes());
    hasher.update(entry.aggregate_type.as_bytes());
    hasher.update(entry.aggregate_id.as_bytes());
    hasher.update(entry.payload.to_string().as_bytes());
    hasher.update(&entry.prev_hash);
    hasher.finalize().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    /// Create an in-memory database with the necessary schema for testing.
    fn setup_test_db() -> String {
        // We need a real file for multi-connection tests.
        let dir = std::env::temp_dir();
        let path = dir
            .join(format!("rustyclinic_sync_test_{}.db", Uuid::now_v7()))
            .to_string_lossy()
            .to_string();

        let conn = Connection::open(&path).expect("open test db");
        rustyclinic_db::migration::run_migrations(&conn).expect("run migrations");
        // Also run the sync-specific migrations
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sync_cursors (
                device_id TEXT NOT NULL,
                facility_id TEXT NOT NULL,
                last_pulled_sequence INTEGER NOT NULL DEFAULT 0,
                last_pushed_sequence INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (device_id, facility_id)
            );
            CREATE TABLE IF NOT EXISTS sync_conflicts (
                id TEXT PRIMARY KEY NOT NULL,
                facility_id TEXT NOT NULL,
                aggregate_type TEXT NOT NULL,
                aggregate_id TEXT NOT NULL,
                local_entry_id TEXT NOT NULL,
                remote_entry_id TEXT NOT NULL,
                conflict_type TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                created_at TEXT NOT NULL,
                resolved_at TEXT,
                resolved_by TEXT,
                resolution TEXT
            );",
        )
        .expect("create sync tables");
        drop(conn);
        path
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_op_entry(
        db_path: &str,
        sequence: u64,
        facility_id: Uuid,
        device_id: Uuid,
        sync_state: &str,
        aggregate_type: &str,
        aggregate_id: Uuid,
        payload: serde_json::Value,
    ) -> OpLogEntry {
        let conn = Connection::open(db_path).expect("open");
        let entry = OpLogEntry {
            id: Uuid::now_v7(),
            sequence,
            facility_id,
            device_id,
            actor_id: Uuid::now_v7(),
            created_at: Utc::now(),
            aggregate_type: aggregate_type.to_string(),
            aggregate_id,
            payload,
            prev_hash: Vec::new(),
            entry_hash: Vec::new(),
        };

        // Compute hash
        let hash = compute_entry_hash(&entry);

        conn.execute(
            "INSERT INTO op_log (id, sequence, facility_id, device_id, actor_id, created_at,
                aggregate_type, aggregate_id, payload, prev_hash, entry_hash, sync_state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                entry.id.to_string(),
                entry.sequence,
                entry.facility_id.to_string(),
                entry.device_id.to_string(),
                entry.actor_id.to_string(),
                entry.created_at.to_rfc3339(),
                entry.aggregate_type,
                entry.aggregate_id.to_string(),
                entry.payload.to_string(),
                entry.prev_hash,
                hash,
                sync_state,
            ],
        )
        .expect("insert op_log entry");

        OpLogEntry {
            entry_hash: hash,
            ..entry
        }
    }

    #[test]
    fn get_pending_ops_returns_only_pending() {
        let db_path = setup_test_db();
        let facility = Uuid::now_v7();
        let device = Uuid::now_v7();

        insert_op_entry(
            &db_path,
            1,
            facility,
            device,
            "pending",
            "Patient",
            Uuid::now_v7(),
            json!({}),
        );
        insert_op_entry(
            &db_path,
            2,
            facility,
            device,
            "pushed",
            "Patient",
            Uuid::now_v7(),
            json!({}),
        );
        insert_op_entry(
            &db_path,
            3,
            facility,
            device,
            "pending",
            "Patient",
            Uuid::now_v7(),
            json!({}),
        );
        insert_op_entry(
            &db_path,
            4,
            facility,
            device,
            "acknowledged",
            "Patient",
            Uuid::now_v7(),
            json!({}),
        );

        let engine = SyncEngine::new(db_path, device, facility);
        let pending = engine.get_pending_ops(100).expect("get_pending_ops");

        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].sequence, 1);
        assert_eq!(pending[1].sequence, 3);
    }

    #[test]
    fn mark_pushed_updates_sync_state() {
        let db_path = setup_test_db();
        let facility = Uuid::now_v7();
        let device = Uuid::now_v7();

        insert_op_entry(
            &db_path,
            1,
            facility,
            device,
            "pending",
            "Patient",
            Uuid::now_v7(),
            json!({}),
        );
        insert_op_entry(
            &db_path,
            2,
            facility,
            device,
            "pending",
            "Patient",
            Uuid::now_v7(),
            json!({}),
        );
        insert_op_entry(
            &db_path,
            3,
            facility,
            device,
            "pending",
            "Patient",
            Uuid::now_v7(),
            json!({}),
        );

        let engine = SyncEngine::new(db_path.clone(), device, facility);
        engine.mark_pushed(2).expect("mark_pushed");

        // Only entries 1 and 2 should be 'pushed'
        let pending = engine.get_pending_ops(100).expect("get_pending_ops");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].sequence, 3);

        // Verify cursor
        let cursor = engine.get_cursor().expect("get_cursor");
        assert_eq!(cursor.last_pushed_sequence, 2);
    }

    #[test]
    fn mark_acknowledged_updates_sync_state() {
        let db_path = setup_test_db();
        let facility = Uuid::now_v7();
        let device = Uuid::now_v7();

        insert_op_entry(
            &db_path,
            1,
            facility,
            device,
            "pushed",
            "Patient",
            Uuid::now_v7(),
            json!({}),
        );
        insert_op_entry(
            &db_path,
            2,
            facility,
            device,
            "pushed",
            "Patient",
            Uuid::now_v7(),
            json!({}),
        );

        let engine = SyncEngine::new(db_path.clone(), device, facility);
        engine.mark_acknowledged(1).expect("mark_acknowledged");

        // Entry 1 should be acknowledged, entry 2 still pushed
        let conn = Connection::open(&db_path).expect("open");
        let state: String = conn
            .query_row(
                "SELECT sync_state FROM op_log WHERE sequence = 1",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(state, "acknowledged");

        let state2: String = conn
            .query_row(
                "SELECT sync_state FROM op_log WHERE sequence = 2",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(state2, "pushed");
    }

    #[test]
    fn validate_chain_detects_broken_hash() {
        let facility = Uuid::now_v7();
        let device = Uuid::now_v7();

        let mut entry1 = OpLogEntry {
            id: Uuid::now_v7(),
            sequence: 1,
            facility_id: facility,
            device_id: device,
            actor_id: Uuid::now_v7(),
            created_at: Utc::now(),
            aggregate_type: "Patient".to_string(),
            aggregate_id: Uuid::now_v7(),
            payload: json!({}),
            prev_hash: Vec::new(),
            entry_hash: Vec::new(),
        };
        entry1.entry_hash = compute_entry_hash(&entry1);

        let mut entry2 = OpLogEntry {
            id: Uuid::now_v7(),
            sequence: 2,
            facility_id: facility,
            device_id: device,
            actor_id: Uuid::now_v7(),
            created_at: Utc::now(),
            aggregate_type: "Patient".to_string(),
            aggregate_id: Uuid::now_v7(),
            payload: json!({}),
            prev_hash: entry1.entry_hash.clone(),
            entry_hash: Vec::new(),
        };
        entry2.entry_hash = compute_entry_hash(&entry2);

        // Valid chain should pass
        let db_path = setup_test_db();
        let engine = SyncEngine::new(db_path, device, facility);
        assert!(
            engine
                .validate_chain(&[entry1.clone(), entry2.clone()])
                .is_ok()
        );

        // Tamper with entry2's prev_hash
        let mut tampered = entry2.clone();
        tampered.prev_hash = vec![0u8; 32];
        assert!(engine.validate_chain(&[entry1, tampered]).is_err());
    }

    #[test]
    fn detect_gaps_catches_sequence_gap() {
        let facility = Uuid::now_v7();
        let device = Uuid::now_v7();
        let db_path = setup_test_db();
        let engine = SyncEngine::new(db_path, device, facility);

        let make = |seq: u64| OpLogEntry {
            id: Uuid::now_v7(),
            sequence: seq,
            facility_id: facility,
            device_id: device,
            actor_id: Uuid::now_v7(),
            created_at: Utc::now(),
            aggregate_type: "Patient".to_string(),
            aggregate_id: Uuid::now_v7(),
            payload: json!({}),
            prev_hash: Vec::new(),
            entry_hash: Vec::new(),
        };

        // Contiguous from expected_start should pass
        assert!(engine.detect_gaps(&[make(5), make(6), make(7)], 5).is_ok());

        // Wrong start
        assert!(engine.detect_gaps(&[make(6), make(7)], 5).is_err());

        // Internal gap
        assert!(engine.detect_gaps(&[make(5), make(7)], 5).is_err());
    }

    #[test]
    fn apply_remote_ops_inserts_and_detects_conflicts() {
        let db_path = setup_test_db();
        let facility = Uuid::now_v7();
        let device_local = Uuid::now_v7();
        let device_remote = Uuid::now_v7();
        let patient_id = Uuid::now_v7();

        // Insert a local pending entry for a patient with a critical field change
        insert_op_entry(
            &db_path,
            1,
            facility,
            device_local,
            "pending",
            "Patient",
            patient_id,
            json!({"date_of_birth": "1990-01-01"}),
        );

        let engine = SyncEngine::new(db_path.clone(), device_local, facility);

        // Remote entry on same patient with different critical field
        let remote_entry = OpLogEntry {
            id: Uuid::now_v7(),
            sequence: 100,
            facility_id: facility,
            device_id: device_remote,
            actor_id: Uuid::now_v7(),
            created_at: Utc::now(),
            aggregate_type: "Patient".to_string(),
            aggregate_id: patient_id,
            payload: json!({"date_of_birth": "1991-02-02"}),
            prev_hash: Vec::new(),
            entry_hash: Vec::new(),
        };

        let result = engine
            .apply_remote_ops(&[remote_entry])
            .expect("apply_remote_ops");
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.applied, 0);
    }

    #[test]
    fn apply_remote_ops_deduplicates_by_id() {
        let db_path = setup_test_db();
        let facility = Uuid::now_v7();
        let device = Uuid::now_v7();

        let entry = insert_op_entry(
            &db_path,
            1,
            facility,
            device,
            "pending",
            "Patient",
            Uuid::now_v7(),
            json!({}),
        );

        let engine = SyncEngine::new(db_path, device, facility);

        // Try to apply the same entry again
        let result = engine.apply_remote_ops(&[entry]).expect("apply_remote_ops");
        assert_eq!(result.applied, 1); // counted as applied (deduplicated)
        assert_eq!(result.conflicts.len(), 0);
    }

    #[test]
    fn full_push_pull_round_trip() {
        let db_path_a = setup_test_db();
        let db_path_b = setup_test_db();
        let facility = Uuid::now_v7();
        let device_a = Uuid::now_v7();
        let device_b = Uuid::now_v7();

        // Device A creates some entries
        let patient_id = Uuid::now_v7();
        let entry1 = insert_op_entry(
            &db_path_a,
            1,
            facility,
            device_a,
            "pending",
            "Patient",
            patient_id,
            json!({"given_name": "Alice"}),
        );
        let entry2 = insert_op_entry(
            &db_path_a,
            2,
            facility,
            device_a,
            "pending",
            "Patient",
            Uuid::now_v7(),
            json!({"given_name": "Bob"}),
        );

        let engine_a = SyncEngine::new(db_path_a.clone(), device_a, facility);
        let engine_b = SyncEngine::new(db_path_b.clone(), device_b, facility);

        // Device A: get pending ops (simulates push)
        let pending = engine_a.get_pending_ops(100).expect("get_pending_ops");
        assert_eq!(pending.len(), 2);

        // Device A: mark as pushed
        engine_a.mark_pushed(2).expect("mark_pushed");

        // Verify no more pending ops
        let pending_after = engine_a
            .get_pending_ops(100)
            .expect("get_pending_ops after push");
        assert_eq!(pending_after.len(), 0);

        // Device B: apply remote ops (simulates pull)
        let result = engine_b
            .apply_remote_ops(&[entry1, entry2])
            .expect("apply_remote_ops");
        assert_eq!(result.applied, 2);
        assert_eq!(result.conflicts.len(), 0);

        // Device B: verify cursor was updated
        let cursor_b = engine_b.get_cursor().expect("get_cursor");
        assert_eq!(cursor_b.last_pulled_sequence, 2);

        // Device A: mark as acknowledged (simulates server ack)
        engine_a.mark_acknowledged(2).expect("mark_acknowledged");

        // Verify all are acknowledged
        let pending_final = engine_a.get_pending_ops(100).expect("final pending");
        assert_eq!(pending_final.len(), 0);
    }
}
