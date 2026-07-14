use chrono::{DateTime, Utc};
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_events::OpLogEntry;
use std::str::FromStr;
use tokio_postgres::Client;
use uuid::Uuid;

use crate::sync_repo::{
    OpLogSyncRepo, SyncConflictRecord, SyncConflictRepo, SyncConflictStatus, SyncCursorRecord,
    SyncCursorRepo,
};

pub struct PgSyncRepo<'a> {
    client: &'a Client,
}

impl<'a> PgSyncRepo<'a> {
    pub fn new(client: &'a Client) -> Self {
        Self { client }
    }

    fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f))
    }

    fn insert_with_state(&self, entry: &OpLogEntry, sync_state: &str) -> AppResult<bool> {
        self.block_on(async {
            let affected = self
                .client
                .execute(
                    "INSERT INTO op_log (id, sequence, facility_id, device_id, actor_id, created_at,
                        aggregate_type, aggregate_id, payload, prev_hash, entry_hash, sync_state)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                     ON CONFLICT(id) DO NOTHING",
                    &[
                        &entry.id,
                        &(entry.sequence as i64),
                        &entry.facility_id,
                        &entry.device_id,
                        &entry.actor_id,
                        &entry.created_at,
                        &entry.aggregate_type,
                        &entry.aggregate_id,
                        &entry.payload,
                        &entry.prev_hash,
                        &entry.entry_hash,
                        &sync_state,
                    ],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;
            Ok(affected > 0)
        })
    }
}

impl OpLogSyncRepo for PgSyncRepo<'_> {
    fn list_pending(&self, facility_id: Uuid, limit: u32) -> AppResult<Vec<OpLogEntry>> {
        self.block_on(async {
            let rows = self
                .client
                .query(
                    "SELECT id, sequence, facility_id, device_id, actor_id, created_at,
                            aggregate_type, aggregate_id, payload, prev_hash, entry_hash
                     FROM op_log
                     WHERE facility_id = $1 AND sync_state = 'pending'
                     ORDER BY sequence ASC
                     LIMIT $2",
                    &[&facility_id, &(limit as i64)],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            let mut out = Vec::new();
            for row in &rows {
                out.push(row_to_op_log(row)?);
            }
            Ok(out)
        })
    }

    fn count_pending(&self, facility_id: Uuid) -> AppResult<u64> {
        self.block_on(async {
            let row = self
                .client
                .query_one(
                    "SELECT COUNT(*)::bigint FROM op_log WHERE facility_id = $1 AND sync_state = 'pending'",
                    &[&facility_id],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;
            let count: i64 = row.try_get(0).map_err(|e| AppError::Database(e.to_string()))?;
            Ok(count as u64)
        })
    }

    fn list_since_excluding_device(
        &self,
        facility_id: Uuid,
        since_sequence: u64,
        excluded_device_id: Uuid,
        limit: u32,
    ) -> AppResult<Vec<OpLogEntry>> {
        self.block_on(async {
            let rows = self
                .client
                .query(
                    "SELECT id, sequence, facility_id, device_id, actor_id, created_at,
                            aggregate_type, aggregate_id, payload, prev_hash, entry_hash
                     FROM op_log
                     WHERE facility_id = $1 AND sequence > $2 AND device_id != $3
                     ORDER BY sequence ASC
                     LIMIT $4",
                    &[
                        &facility_id,
                        &(since_sequence as i64),
                        &excluded_device_id,
                        &(limit as i64),
                    ],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            let mut out = Vec::new();
            for row in &rows {
                out.push(row_to_op_log(row)?);
            }
            Ok(out)
        })
    }

    fn list_unacknowledged_for_aggregate(
        &self,
        facility_id: Uuid,
        aggregate_type: &str,
        aggregate_id: Uuid,
    ) -> AppResult<Vec<OpLogEntry>> {
        self.block_on(async {
            let rows = self
                .client
                .query(
                    "SELECT id, sequence, facility_id, device_id, actor_id, created_at,
                            aggregate_type, aggregate_id, payload, prev_hash, entry_hash
                     FROM op_log
                     WHERE facility_id = $1
                       AND aggregate_type = $2
                       AND aggregate_id = $3
                       AND sync_state != 'acknowledged'",
                    &[&facility_id, &aggregate_type, &aggregate_id],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            let mut out = Vec::new();
            for row in &rows {
                out.push(row_to_op_log(row)?);
            }
            Ok(out)
        })
    }

    fn exists(&self, id: Uuid) -> AppResult<bool> {
        self.block_on(async {
            let row = self
                .client
                .query_one("SELECT COUNT(*)::bigint FROM op_log WHERE id = $1", &[&id])
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;
            let count: i64 = row
                .try_get(0)
                .map_err(|e| AppError::Database(e.to_string()))?;
            Ok(count > 0)
        })
    }

    fn insert_pending_if_missing(&self, entry: &OpLogEntry) -> AppResult<bool> {
        self.insert_with_state(entry, "pending")
    }

    fn insert_acknowledged_if_missing(&self, entry: &OpLogEntry) -> AppResult<bool> {
        self.insert_with_state(entry, "acknowledged")
    }

    fn mark_pushed_through(&self, facility_id: Uuid, through_sequence: u64) -> AppResult<()> {
        self.block_on(async {
            self.client
                .execute(
                    "UPDATE op_log SET sync_state = 'pushed'
                     WHERE facility_id = $1 AND sequence <= $2 AND sync_state = 'pending'",
                    &[&facility_id, &(through_sequence as i64)],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;
            Ok(())
        })
    }

    fn mark_acknowledged_through(&self, facility_id: Uuid, through_sequence: u64) -> AppResult<()> {
        self.block_on(async {
            self.client
                .execute(
                    "UPDATE op_log SET sync_state = 'acknowledged'
                     WHERE facility_id = $1 AND sequence <= $2 AND sync_state IN ('pending', 'pushed')",
                    &[&facility_id, &(through_sequence as i64)],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;
            Ok(())
        })
    }
}

impl SyncCursorRepo for PgSyncRepo<'_> {
    fn get(&self, device_id: Uuid, facility_id: Uuid) -> AppResult<Option<SyncCursorRecord>> {
        self.block_on(async {
            let row = self
                .client
                .query_opt(
                    "SELECT device_id, facility_id, last_pulled_sequence, last_pushed_sequence, updated_at
                     FROM sync_cursors
                     WHERE device_id = $1 AND facility_id = $2",
                    &[&device_id.to_string(), &facility_id.to_string()],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            match row {
                Some(row) => Ok(Some(SyncCursorRecord {
                    device_id: parse_uuid_text(
                        row.try_get::<_, String>(0)
                            .map_err(|e| AppError::Database(e.to_string()))?,
                    )?,
                    facility_id: parse_uuid_text(
                        row.try_get::<_, String>(1)
                            .map_err(|e| AppError::Database(e.to_string()))?,
                    )?,
                    last_pulled_sequence: row
                        .try_get::<_, i64>(2)
                        .map_err(|e| AppError::Database(e.to_string()))?
                        as u64,
                    last_pushed_sequence: row
                        .try_get::<_, i64>(3)
                        .map_err(|e| AppError::Database(e.to_string()))?
                        as u64,
                    updated_at: row.try_get(4).map_err(|e| AppError::Database(e.to_string()))?,
                })),
                None => Ok(None),
            }
        })
    }

    fn upsert(&self, cursor: &SyncCursorRecord) -> AppResult<()> {
        self.block_on(async {
            self.client
                .execute(
                    "INSERT INTO sync_cursors (device_id, facility_id, last_pulled_sequence, last_pushed_sequence, updated_at)
                     VALUES ($1, $2, $3, $4, $5)
                     ON CONFLICT(device_id, facility_id)
                     DO UPDATE SET last_pulled_sequence = excluded.last_pulled_sequence,
                                   last_pushed_sequence = excluded.last_pushed_sequence,
                                   updated_at = excluded.updated_at",
                    &[
                        &cursor.device_id.to_string(),
                        &cursor.facility_id.to_string(),
                        &(cursor.last_pulled_sequence as i64),
                        &(cursor.last_pushed_sequence as i64),
                        &cursor.updated_at,
                    ],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;
            Ok(())
        })
    }
}

impl SyncConflictRepo for PgSyncRepo<'_> {
    fn insert(&self, conflict: &SyncConflictRecord) -> AppResult<()> {
        self.block_on(async {
            self.client
                .execute(
                    "INSERT INTO sync_conflicts (id, facility_id, aggregate_type, aggregate_id,
                        local_entry_id, remote_entry_id, conflict_type, status, created_at,
                        resolved_at, resolved_by, resolution)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
                    &[
                        &conflict.id.to_string(),
                        &conflict.facility_id.to_string(),
                        &conflict.aggregate_type,
                        &conflict.aggregate_id.to_string(),
                        &conflict.local_entry_id.to_string(),
                        &conflict.remote_entry_id.to_string(),
                        &conflict.conflict_type.to_string(),
                        &conflict.status.as_str(),
                        &conflict.created_at,
                        &conflict.resolved_at,
                        &conflict.resolved_by.map(|id| id.to_string()),
                        &conflict.resolution.as_ref().map(|v| v.to_string()),
                    ],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;
            Ok(())
        })
    }

    fn list_pending(&self, facility_id: Uuid) -> AppResult<Vec<SyncConflictRecord>> {
        self.block_on(async {
            let rows = self
                .client
                .query(
                    "SELECT id, facility_id, aggregate_type, aggregate_id, local_entry_id, remote_entry_id,
                            conflict_type, status, created_at, resolved_at, resolved_by, resolution
                     FROM sync_conflicts
                     WHERE facility_id = $1 AND status = 'pending'
                     ORDER BY created_at ASC",
                    &[&facility_id.to_string()],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            let mut out = Vec::new();
            for row in &rows {
                out.push(row_to_conflict(row)?);
            }
            Ok(out)
        })
    }

    fn mark_resolved(
        &self,
        conflict_id: Uuid,
        resolved_by: Uuid,
        resolution: serde_json::Value,
        resolved_at: DateTime<Utc>,
    ) -> AppResult<()> {
        self.block_on(async {
            self.client
                .execute(
                    "UPDATE sync_conflicts
                     SET status = 'resolved', resolved_by = $2, resolved_at = $3, resolution = $4
                     WHERE id = $1",
                    &[
                        &conflict_id.to_string(),
                        &resolved_by.to_string(),
                        &resolved_at,
                        &resolution.to_string(),
                    ],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;
            Ok(())
        })
    }
}

fn row_to_op_log(row: &tokio_postgres::Row) -> AppResult<OpLogEntry> {
    Ok(OpLogEntry {
        id: row
            .try_get(0)
            .map_err(|e| AppError::Database(e.to_string()))?,
        sequence: row
            .try_get::<_, i64>(1)
            .map_err(|e| AppError::Database(e.to_string()))? as u64,
        facility_id: row
            .try_get(2)
            .map_err(|e| AppError::Database(e.to_string()))?,
        device_id: row
            .try_get(3)
            .map_err(|e| AppError::Database(e.to_string()))?,
        actor_id: row
            .try_get(4)
            .map_err(|e| AppError::Database(e.to_string()))?,
        created_at: row
            .try_get(5)
            .map_err(|e| AppError::Database(e.to_string()))?,
        aggregate_type: row
            .try_get(6)
            .map_err(|e| AppError::Database(e.to_string()))?,
        aggregate_id: row
            .try_get(7)
            .map_err(|e| AppError::Database(e.to_string()))?,
        payload: row
            .try_get(8)
            .map_err(|e| AppError::Database(e.to_string()))?,
        prev_hash: row
            .try_get(9)
            .map_err(|e| AppError::Database(e.to_string()))?,
        entry_hash: row
            .try_get(10)
            .map_err(|e| AppError::Database(e.to_string()))?,
    })
}

fn row_to_conflict(row: &tokio_postgres::Row) -> AppResult<SyncConflictRecord> {
    let status_str: String = row
        .try_get(7)
        .map_err(|e| AppError::Database(e.to_string()))?;
    let status = SyncConflictStatus::from_str(&status_str).map_err(AppError::Database)?;
    let conflict_type_str: String = row
        .try_get(6)
        .map_err(|e| AppError::Database(e.to_string()))?;
    let resolution_str: Option<String> = row
        .try_get(11)
        .map_err(|e| AppError::Database(e.to_string()))?;

    Ok(SyncConflictRecord {
        id: parse_uuid_text(
            row.try_get::<_, String>(0)
                .map_err(|e| AppError::Database(e.to_string()))?,
        )?,
        facility_id: parse_uuid_text(
            row.try_get::<_, String>(1)
                .map_err(|e| AppError::Database(e.to_string()))?,
        )?,
        aggregate_type: row
            .try_get(2)
            .map_err(|e| AppError::Database(e.to_string()))?,
        aggregate_id: parse_uuid_text(
            row.try_get::<_, String>(3)
                .map_err(|e| AppError::Database(e.to_string()))?,
        )?,
        local_entry_id: parse_uuid_text(
            row.try_get::<_, String>(4)
                .map_err(|e| AppError::Database(e.to_string()))?,
        )?,
        remote_entry_id: parse_uuid_text(
            row.try_get::<_, String>(5)
                .map_err(|e| AppError::Database(e.to_string()))?,
        )?,
        conflict_type: serde_json::from_str(&conflict_type_str).unwrap_or(serde_json::Value::Null),
        status,
        created_at: row
            .try_get(8)
            .map_err(|e| AppError::Database(e.to_string()))?,
        resolved_at: row
            .try_get(9)
            .map_err(|e| AppError::Database(e.to_string()))?,
        resolved_by: row
            .try_get::<_, Option<String>>(10)
            .map_err(|e| AppError::Database(e.to_string()))?
            .map(parse_uuid_text)
            .transpose()?,
        resolution: resolution_str.and_then(|s| serde_json::from_str(&s).ok()),
    })
}

fn parse_uuid_text(s: String) -> AppResult<Uuid> {
    Uuid::parse_str(&s).map_err(|e| AppError::Database(e.to_string()))
}
