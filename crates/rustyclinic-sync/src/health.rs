//! Sync health metrics projection.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use rustyclinic_db::sqlite::sync_repo::SqliteSyncRepo;
use rustyclinic_db::sync_repo::{OpLogSyncRepo, SyncConflictRepo, SyncCursorRepo};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Aggregated sync health metrics for a device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncHealth {
    pub pending_ops_count: u64,
    pub last_push_at: Option<DateTime<Utc>>,
    pub last_pull_at: Option<DateTime<Utc>>,
    pub conflict_queue_depth: u32,
    pub cursor_lag: u64,
}

/// Compute sync health for the given facility on this database.
pub fn compute_sync_health(
    conn: &Connection,
    facility_id: Uuid,
    device_id: Uuid,
    server_sequence: u64,
) -> Result<SyncHealth> {
    let repo = SqliteSyncRepo::new(conn);

    // Count pending ops
    let pending_ops_count: u64 = repo
        .count_pending(facility_id)
        .context("count pending ops")?;

    // Count unresolved conflicts
    let conflict_queue_depth = SyncConflictRepo::list_pending(&repo, facility_id)
        .map(|conflicts| conflicts.len() as u32)
        .unwrap_or(0);

    // Read cursor for last push/pull timestamps and cursor lag
    let (last_pulled_seq, last_timestamp) = match repo.get(device_id, facility_id)? {
        Some(cursor) => (cursor.last_pulled_sequence, Some(cursor.updated_at)),
        None => (0, None),
    };

    let cursor_lag = server_sequence.saturating_sub(last_pulled_seq);

    Ok(SyncHealth {
        pending_ops_count,
        last_push_at: last_timestamp,
        last_pull_at: last_timestamp,
        conflict_queue_depth,
        cursor_lag,
    })
}
