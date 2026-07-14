use chrono::{DateTime, Utc};
use rusqlite::Connection;
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_events::OpLogEntry;
use std::str::FromStr;
use uuid::Uuid;

use crate::sync_repo::{
    OpLogSyncRepo, SyncConflictRecord, SyncConflictRepo, SyncConflictStatus, SyncCursorRecord,
    SyncCursorRepo,
};

pub struct SqliteSyncRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteSyncRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    fn insert_with_state(&self, entry: &OpLogEntry, sync_state: &str) -> AppResult<bool> {
        let affected = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO op_log (id, sequence, facility_id, device_id, actor_id, created_at, aggregate_type, aggregate_id, payload, prev_hash, entry_hash, sync_state)
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
                    entry.entry_hash,
                    sync_state,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(affected > 0)
    }
}

impl OpLogSyncRepo for SqliteSyncRepo<'_> {
    fn list_pending(&self, facility_id: Uuid, limit: u32) -> AppResult<Vec<OpLogEntry>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, sequence, facility_id, device_id, actor_id, created_at,
                        aggregate_type, aggregate_id, payload, prev_hash, entry_hash
                 FROM op_log
                 WHERE facility_id = ?1 AND sync_state = 'pending'
                 ORDER BY sequence ASC
                 LIMIT ?2",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(
                rusqlite::params![facility_id.to_string(), limit],
                row_to_op_log,
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| AppError::Database(e.to_string()))?);
        }
        Ok(out)
    }

    fn count_pending(&self, facility_id: Uuid) -> AppResult<u64> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM op_log WHERE facility_id = ?1 AND sync_state = 'pending'",
                rusqlite::params![facility_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| AppError::Database(e.to_string()))
    }

    fn list_since_excluding_device(
        &self,
        facility_id: Uuid,
        since_sequence: u64,
        excluded_device_id: Uuid,
        limit: u32,
    ) -> AppResult<Vec<OpLogEntry>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, sequence, facility_id, device_id, actor_id, created_at,
                        aggregate_type, aggregate_id, payload, prev_hash, entry_hash
                 FROM op_log
                 WHERE facility_id = ?1 AND sequence > ?2 AND device_id != ?3
                 ORDER BY sequence ASC
                 LIMIT ?4",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(
                rusqlite::params![
                    facility_id.to_string(),
                    since_sequence,
                    excluded_device_id.to_string(),
                    limit,
                ],
                row_to_op_log,
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| AppError::Database(e.to_string()))?);
        }
        Ok(out)
    }

    fn list_unacknowledged_for_aggregate(
        &self,
        facility_id: Uuid,
        aggregate_type: &str,
        aggregate_id: Uuid,
    ) -> AppResult<Vec<OpLogEntry>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, sequence, facility_id, device_id, actor_id, created_at,
                        aggregate_type, aggregate_id, payload, prev_hash, entry_hash
                 FROM op_log
                 WHERE facility_id = ?1
                   AND aggregate_type = ?2
                   AND aggregate_id = ?3
                   AND sync_state != 'acknowledged'",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(
                rusqlite::params![
                    facility_id.to_string(),
                    aggregate_type,
                    aggregate_id.to_string()
                ],
                row_to_op_log,
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| AppError::Database(e.to_string()))?);
        }
        Ok(out)
    }

    fn exists(&self, id: Uuid) -> AppResult<bool> {
        let count: u64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM op_log WHERE id = ?1",
                rusqlite::params![id.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(count > 0)
    }

    fn insert_pending_if_missing(&self, entry: &OpLogEntry) -> AppResult<bool> {
        self.insert_with_state(entry, "pending")
    }

    fn insert_acknowledged_if_missing(&self, entry: &OpLogEntry) -> AppResult<bool> {
        self.insert_with_state(entry, "acknowledged")
    }

    fn mark_pushed_through(&self, facility_id: Uuid, through_sequence: u64) -> AppResult<()> {
        self.conn
            .execute(
                "UPDATE op_log SET sync_state = 'pushed'
                 WHERE facility_id = ?1 AND sequence <= ?2 AND sync_state = 'pending'",
                rusqlite::params![facility_id.to_string(), through_sequence],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn mark_acknowledged_through(&self, facility_id: Uuid, through_sequence: u64) -> AppResult<()> {
        self.conn
            .execute(
                "UPDATE op_log SET sync_state = 'acknowledged'
                 WHERE facility_id = ?1 AND sequence <= ?2 AND sync_state IN ('pending', 'pushed')",
                rusqlite::params![facility_id.to_string(), through_sequence],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }
}

impl SyncCursorRepo for SqliteSyncRepo<'_> {
    fn get(&self, device_id: Uuid, facility_id: Uuid) -> AppResult<Option<SyncCursorRecord>> {
        let result = self.conn.query_row(
            "SELECT device_id, facility_id, last_pulled_sequence, last_pushed_sequence, updated_at
             FROM sync_cursors
             WHERE device_id = ?1 AND facility_id = ?2",
            rusqlite::params![device_id.to_string(), facility_id.to_string()],
            |row| {
                Ok(SyncCursorRecord {
                    device_id: parse_uuid_row(row, 0)?,
                    facility_id: parse_uuid_row(row, 1)?,
                    last_pulled_sequence: row.get(2)?,
                    last_pushed_sequence: row.get(3)?,
                    updated_at: parse_dt_row(row, 4)?,
                })
            },
        );

        match result {
            Ok(cursor) => Ok(Some(cursor)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(AppError::Database(e.to_string())),
        }
    }

    fn upsert(&self, cursor: &SyncCursorRecord) -> AppResult<()> {
        self.conn
            .execute(
                "INSERT INTO sync_cursors (device_id, facility_id, last_pulled_sequence, last_pushed_sequence, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(device_id, facility_id)
                 DO UPDATE SET last_pulled_sequence = excluded.last_pulled_sequence,
                               last_pushed_sequence = excluded.last_pushed_sequence,
                               updated_at = excluded.updated_at",
                rusqlite::params![
                    cursor.device_id.to_string(),
                    cursor.facility_id.to_string(),
                    cursor.last_pulled_sequence,
                    cursor.last_pushed_sequence,
                    cursor.updated_at.to_rfc3339(),
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }
}

impl SyncConflictRepo for SqliteSyncRepo<'_> {
    fn insert(&self, conflict: &SyncConflictRecord) -> AppResult<()> {
        self.conn
            .execute(
                "INSERT INTO sync_conflicts (id, facility_id, aggregate_type, aggregate_id,
                    local_entry_id, remote_entry_id, conflict_type, status, created_at,
                    resolved_at, resolved_by, resolution)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                rusqlite::params![
                    conflict.id.to_string(),
                    conflict.facility_id.to_string(),
                    conflict.aggregate_type,
                    conflict.aggregate_id.to_string(),
                    conflict.local_entry_id.to_string(),
                    conflict.remote_entry_id.to_string(),
                    conflict.conflict_type.to_string(),
                    conflict.status.as_str(),
                    conflict.created_at.to_rfc3339(),
                    conflict.resolved_at.map(|dt| dt.to_rfc3339()),
                    conflict.resolved_by.map(|id| id.to_string()),
                    conflict.resolution.as_ref().map(|v| v.to_string()),
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn list_pending(&self, facility_id: Uuid) -> AppResult<Vec<SyncConflictRecord>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, facility_id, aggregate_type, aggregate_id, local_entry_id, remote_entry_id,
                        conflict_type, status, created_at, resolved_at, resolved_by, resolution
                 FROM sync_conflicts
                 WHERE facility_id = ?1 AND status = 'pending'
                 ORDER BY created_at ASC",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(
                rusqlite::params![facility_id.to_string()],
                row_to_sync_conflict,
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| AppError::Database(e.to_string()))?);
        }
        Ok(out)
    }

    fn mark_resolved(
        &self,
        conflict_id: Uuid,
        resolved_by: Uuid,
        resolution: serde_json::Value,
        resolved_at: DateTime<Utc>,
    ) -> AppResult<()> {
        self.conn
            .execute(
                "UPDATE sync_conflicts
                 SET status = 'resolved', resolved_by = ?2, resolved_at = ?3, resolution = ?4
                 WHERE id = ?1",
                rusqlite::params![
                    conflict_id.to_string(),
                    resolved_by.to_string(),
                    resolved_at.to_rfc3339(),
                    resolution.to_string(),
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }
}

fn row_to_op_log(row: &rusqlite::Row<'_>) -> rusqlite::Result<OpLogEntry> {
    Ok(OpLogEntry {
        id: parse_uuid_row(row, 0)?,
        sequence: row.get(1)?,
        facility_id: parse_uuid_row(row, 2)?,
        device_id: parse_uuid_row(row, 3)?,
        actor_id: parse_uuid_row(row, 4)?,
        created_at: parse_dt_row(row, 5)?,
        aggregate_type: row.get(6)?,
        aggregate_id: parse_uuid_row(row, 7)?,
        payload: parse_json_row(row, 8)?,
        prev_hash: row.get(9)?,
        entry_hash: row.get(10)?,
    })
}

fn row_to_sync_conflict(row: &rusqlite::Row<'_>) -> rusqlite::Result<SyncConflictRecord> {
    let conflict_type: String = row.get(6)?;
    let status: String = row.get(7)?;
    let resolution: Option<String> = row.get(11)?;

    let parsed_status = SyncConflictStatus::from_str(status.as_str()).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            7,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(e)),
        )
    })?;

    Ok(SyncConflictRecord {
        id: parse_uuid_row(row, 0)?,
        facility_id: parse_uuid_row(row, 1)?,
        aggregate_type: row.get(2)?,
        aggregate_id: parse_uuid_row(row, 3)?,
        local_entry_id: parse_uuid_row(row, 4)?,
        remote_entry_id: parse_uuid_row(row, 5)?,
        conflict_type: serde_json::from_str(&conflict_type).unwrap_or(serde_json::Value::Null),
        status: parsed_status,
        created_at: parse_dt_row(row, 8)?,
        resolved_at: parse_opt_dt_row(row, 9)?,
        resolved_by: parse_opt_uuid_row(row, 10)?,
        resolution: resolution.and_then(|s| serde_json::from_str(&s).ok()),
    })
}

fn parse_uuid_row(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<Uuid> {
    let s: String = row.get(idx)?;
    Uuid::parse_str(&s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn parse_opt_uuid_row(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<Option<Uuid>> {
    let maybe: Option<String> = row.get(idx)?;
    maybe
        .map(|s| {
            Uuid::parse_str(&s).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    idx,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })
        })
        .transpose()
}

fn parse_dt_row(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<DateTime<Utc>> {
    let s: String = row.get(idx)?;
    chrono::DateTime::parse_from_rfc3339(&s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, Box::new(e))
        })
}

fn parse_opt_dt_row(
    row: &rusqlite::Row<'_>,
    idx: usize,
) -> rusqlite::Result<Option<DateTime<Utc>>> {
    let maybe: Option<String> = row.get(idx)?;
    maybe
        .map(|s| {
            chrono::DateTime::parse_from_rfc3339(&s)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        idx,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })
        })
        .transpose()
}

fn parse_json_row(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<serde_json::Value> {
    let s: String = row.get(idx)?;
    serde_json::from_str(&s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, Box::new(e))
    })
}
