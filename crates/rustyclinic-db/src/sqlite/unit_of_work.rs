//! Unit of work: commits domain rows + audit + outbox + op-log atomically.
//!
//! ```text
//! TRANSACTION BOUNDARY:
//!
//!   BEGIN
//!   ├── persist domain rows (patient, queue entry, etc.)
//!   ├── persist audit entry (hash-chained)
//!   ├── persist outbox event
//!   ├── persist op-log entry
//!   COMMIT
//!
//!   Power loss at any point → full rollback on restart (SQLite WAL).
//! ```

use chrono::Utc;
use rusqlite::Connection;
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_core::types::{ActorContext, new_id};
use rustyclinic_events::{AuditEntry, OpLogEntry, OutboxEvent};
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};

/// Captures side-effect records to be committed alongside domain writes.
pub struct UnitOfWork<'a> {
    tx: rusqlite::Transaction<'a>,
    audit_entries: Vec<AuditEntry>,
    outbox_events: Vec<OutboxEvent>,
    op_log_entries: Vec<OpLogEntry>,
    idempotency: Option<(uuid::Uuid, String, JsonValue)>, // (facility_id, key, response)
}

impl<'a> UnitOfWork<'a> {
    pub fn try_new(conn: &'a Connection) -> AppResult<Self> {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| AppError::Database(format!("failed to begin transaction: {e}")))?;

        Ok(Self {
            tx,
            audit_entries: Vec::new(),
            outbox_events: Vec::new(),
            op_log_entries: Vec::new(),
            idempotency: None,
        })
    }

    pub fn new(conn: &'a Connection) -> Self {
        Self::try_new(conn).expect("UnitOfWork::try_new should succeed")
    }

    /// Access the connection for domain writes within the transaction.
    pub fn conn(&self) -> &Connection {
        &self.tx
    }

    /// Record an idempotency key and its response for replay on retry.
    pub fn record_idempotency(
        &mut self,
        facility_id: uuid::Uuid,
        key: String,
        response: JsonValue,
    ) {
        self.idempotency = Some((facility_id, key, response));
    }

    /// Record an auditable action. Hash chain is computed at commit time.
    pub fn record_audit(
        &mut self,
        actor: &ActorContext,
        action: &str,
        aggregate_type: &str,
        aggregate_id: uuid::Uuid,
        payload: JsonValue,
    ) {
        self.audit_entries.push(AuditEntry {
            id: new_id(),
            facility_id: actor.facility_id,
            actor_id: actor.user_id,
            device_id: actor.device_id,
            timestamp: Utc::now(),
            action: action.to_string(),
            aggregate_type: aggregate_type.to_string(),
            aggregate_id,
            payload,
            prev_hash: Vec::new(),  // computed at commit
            entry_hash: Vec::new(), // computed at commit
        });
    }

    /// Record an outbox event for async processing.
    pub fn record_outbox(
        &mut self,
        facility_id: uuid::Uuid,
        aggregate_type: &str,
        aggregate_id: uuid::Uuid,
        event_type: &str,
        payload: JsonValue,
    ) {
        self.outbox_events.push(OutboxEvent {
            id: new_id(),
            facility_id,
            aggregate_type: aggregate_type.to_string(),
            aggregate_id,
            event_type: event_type.to_string(),
            payload,
            created_at: Utc::now(),
            published: false,
        });
    }

    /// Record an op-log entry for sync replication.
    pub fn record_op_log(
        &mut self,
        actor: &ActorContext,
        aggregate_type: &str,
        aggregate_id: uuid::Uuid,
        payload: JsonValue,
    ) {
        self.op_log_entries.push(OpLogEntry {
            id: new_id(),
            sequence: 0, // assigned at commit
            facility_id: actor.facility_id,
            device_id: actor.device_id,
            actor_id: actor.user_id,
            created_at: Utc::now(),
            aggregate_type: aggregate_type.to_string(),
            aggregate_id,
            payload,
            prev_hash: Vec::new(),  // computed at commit
            entry_hash: Vec::new(), // computed at commit
        });
    }

    /// Commit all pending records in a single transaction.
    /// Domain writes should already have been executed on self.conn.
    pub fn commit(mut self) -> AppResult<()> {
        let tx = &self.tx;

        // Compute audit hash chain
        let last_audit_hash = self.get_last_audit_hash(tx)?;
        let mut prev = last_audit_hash;
        for entry in &mut self.audit_entries {
            entry.prev_hash = prev.clone();
            entry.entry_hash = compute_audit_hash(entry);
            prev = entry.entry_hash.clone();

            tx.execute(
                "INSERT INTO audit_log (id, facility_id, actor_id, device_id, timestamp, action, aggregate_type, aggregate_id, payload, prev_hash, entry_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                rusqlite::params![
                    entry.id.to_string(),
                    entry.facility_id.to_string(),
                    entry.actor_id.to_string(),
                    entry.device_id.to_string(),
                    entry.timestamp.to_rfc3339(),
                    entry.action,
                    entry.aggregate_type,
                    entry.aggregate_id.to_string(),
                    entry.payload.to_string(),
                    entry.prev_hash,
                    entry.entry_hash,
                ],
            )
            .map_err(|e| AppError::Database(format!("audit insert failed: {e}")))?;
        }

        // Write outbox events
        for event in &self.outbox_events {
            tx.execute(
                "INSERT INTO outbox_events (id, facility_id, aggregate_type, aggregate_id, event_type, payload, created_at, published)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
                rusqlite::params![
                    event.id.to_string(),
                    event.facility_id.to_string(),
                    event.aggregate_type,
                    event.aggregate_id.to_string(),
                    event.event_type,
                    event.payload.to_string(),
                    event.created_at.to_rfc3339(),
                ],
            )
            .map_err(|e| AppError::Database(format!("outbox insert failed: {e}")))?;
        }

        // Write op-log entries with sequence numbers
        let last_seq = self.get_last_op_sequence(tx)?;
        let last_op_hash = self.get_last_op_hash(tx)?;
        let mut seq = last_seq;
        let mut op_prev = last_op_hash;
        for entry in &mut self.op_log_entries {
            seq += 1;
            entry.sequence = seq;
            entry.prev_hash = op_prev.clone();
            entry.entry_hash = compute_op_hash(entry);
            op_prev = entry.entry_hash.clone();

            tx.execute(
                "INSERT INTO op_log (id, sequence, facility_id, device_id, actor_id, created_at, aggregate_type, aggregate_id, payload, prev_hash, entry_hash, sync_state)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'pending')",
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
                    entry.entry_hash,
                ],
            )
            .map_err(|e| AppError::Database(format!("op_log insert failed: {e}")))?;
        }

        // Write idempotency record if present
        if let Some((facility_id, key, response)) = &self.idempotency {
            super::idempotency::store_idempotency(tx, *facility_id, key, response)?;
        }

        self.tx
            .commit()
            .map_err(|e| AppError::Database(format!("commit failed: {e}")))?;
        Ok(())
    }

    fn get_last_audit_hash(&self, conn: &Connection) -> AppResult<Vec<u8>> {
        let result: Result<Vec<u8>, _> = conn.query_row(
            "SELECT entry_hash FROM audit_log ORDER BY rowid DESC LIMIT 1",
            [],
            |row| row.get(0),
        );
        Ok(result.unwrap_or_default())
    }

    fn get_last_op_sequence(&self, conn: &Connection) -> AppResult<u64> {
        let result: Result<u64, _> =
            conn.query_row("SELECT COALESCE(MAX(sequence), 0) FROM op_log", [], |row| {
                row.get(0)
            });
        Ok(result.unwrap_or(0))
    }

    fn get_last_op_hash(&self, conn: &Connection) -> AppResult<Vec<u8>> {
        let result: Result<Vec<u8>, _> = conn.query_row(
            "SELECT entry_hash FROM op_log ORDER BY sequence DESC LIMIT 1",
            [],
            |row| row.get(0),
        );
        Ok(result.unwrap_or_default())
    }
}

fn compute_audit_hash(entry: &AuditEntry) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(entry.id.as_bytes());
    hasher.update(entry.facility_id.as_bytes());
    hasher.update(entry.actor_id.as_bytes());
    hasher.update(entry.timestamp.to_rfc3339().as_bytes());
    hasher.update(entry.action.as_bytes());
    hasher.update(entry.aggregate_type.as_bytes());
    hasher.update(entry.aggregate_id.as_bytes());
    hasher.update(entry.payload.to_string().as_bytes());
    hasher.update(&entry.prev_hash);
    hasher.finalize().to_vec()
}

fn compute_op_hash(entry: &OpLogEntry) -> Vec<u8> {
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
