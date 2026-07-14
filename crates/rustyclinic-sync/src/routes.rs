//! Sync HTTP endpoints.
//!
//! Served under `/sync/` by `ServeRole::Sync` or `ServeRole::All`.
//!
//! All endpoints open their own database connection (same pattern as the API
//! and web crates).

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::engine::{SyncEngine, compute_entry_hash};
use crate::health;
use crate::types::{PushResult, RejectedEntry};
use rustyclinic_events::OpLogEntry;

/// Shared state for sync handlers.
#[derive(Clone)]
pub struct SyncState {
    inner: Arc<SyncStateInner>,
}

pub struct SyncStateInner {
    pub db_path: String,
}

impl SyncState {
    pub fn new(db_path: String) -> Self {
        Self {
            inner: Arc::new(SyncStateInner { db_path }),
        }
    }
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct PushRequest {
    pub facility_id: Uuid,
    pub device_id: Uuid,
    pub entries: Vec<OpLogEntry>,
}

#[derive(Deserialize)]
pub struct PullParams {
    pub facility_id: Uuid,
    pub device_id: Uuid,
    pub since: u64,
    pub limit: Option<u32>,
}

#[derive(Deserialize)]
pub struct AckRequest {
    pub facility_id: Uuid,
    pub device_id: Uuid,
    pub acked_through: u64,
}

#[derive(Deserialize)]
pub struct CursorParams {
    pub facility_id: Uuid,
    pub device_id: Uuid,
}

#[derive(Deserialize)]
pub struct HealthParams {
    pub facility_id: Uuid,
    pub device_id: Uuid,
}

#[derive(Serialize)]
pub struct PullResponse {
    pub entries: Vec<OpLogEntry>,
    pub total_remaining: u64,
    pub server_sequence: u64,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /sync/push — receive pushed ops from a device.
pub async fn push(
    State(state): State<SyncState>,
    Json(req): Json<PushRequest>,
) -> impl IntoResponse {
    let engine = SyncEngine::new(state.inner.db_path.clone(), req.device_id, req.facility_id);

    // Validate hash chain of incoming entries
    if let Err(e) = engine.validate_chain(&req.entries) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("hash chain validation failed: {e}") })),
        );
    }

    // Process each entry: deduplicate, detect conflicts, persist
    let conn = match rusqlite::Connection::open(&state.inner.db_path) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("db error: {e}") })),
            );
        }
    };

    let tx = match conn.unchecked_transaction() {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("transaction error: {e}") })),
            );
        }
    };

    let mut accepted: u64 = 0;
    let mut rejected: Vec<RejectedEntry> = Vec::new();

    // Get current max sequence to assign server-side sequences
    let max_seq: u64 = tx
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) FROM op_log WHERE facility_id = ?1",
            rusqlite::params![req.facility_id.to_string()],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let mut server_seq = max_seq;

    for entry in &req.entries {
        // Deduplicate by entry ID
        let exists: bool = tx
            .query_row(
                "SELECT COUNT(*) FROM op_log WHERE id = ?1",
                rusqlite::params![entry.id.to_string()],
                |row| {
                    let count: u64 = row.get(0)?;
                    Ok(count > 0)
                },
            )
            .unwrap_or(false);

        if exists {
            accepted += 1;
            continue;
        }

        // Verify individual entry hash
        let computed = compute_entry_hash(entry);
        if computed != entry.entry_hash {
            rejected.push(RejectedEntry {
                entry_id: entry.id,
                reason: "entry hash mismatch".to_string(),
            });
            continue;
        }

        // Assign server sequence and persist
        server_seq += 1;

        let insert_result = tx.execute(
            "INSERT INTO op_log (id, sequence, facility_id, device_id, actor_id, created_at,
                aggregate_type, aggregate_id, payload, prev_hash, entry_hash, sync_state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'acknowledged')",
            rusqlite::params![
                entry.id.to_string(),
                server_seq,
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
        );

        match insert_result {
            Ok(_) => accepted += 1,
            Err(e) => {
                rejected.push(RejectedEntry {
                    entry_id: entry.id,
                    reason: format!("insert failed: {e}"),
                });
            }
        }
    }

    if let Err(e) = tx.commit() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("commit failed: {e}") })),
        );
    }

    let result = PushResult {
        accepted,
        rejected,
        conflicts: Vec::new(), // Server-side conflict detection deferred to apply_remote_ops
        server_sequence: server_seq,
    };

    (
        StatusCode::OK,
        Json(serde_json::to_value(result).unwrap_or_default()),
    )
}

/// GET /sync/pull — serve ops since cursor.
pub async fn pull(
    State(state): State<SyncState>,
    Query(params): Query<PullParams>,
) -> impl IntoResponse {
    let conn = match rusqlite::Connection::open(&state.inner.db_path) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("db error: {e}") })),
            );
        }
    };

    let limit = params.limit.unwrap_or(1000).min(5000);

    // Fetch entries since the cursor, excluding entries created by the requesting device
    let mut stmt = match conn.prepare(
        "SELECT id, sequence, facility_id, device_id, actor_id, created_at,
                aggregate_type, aggregate_id, payload, prev_hash, entry_hash
         FROM op_log
         WHERE facility_id = ?1 AND sequence > ?2 AND device_id != ?3
         ORDER BY sequence ASC
         LIMIT ?4",
    ) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("prepare failed: {e}") })),
            );
        }
    };

    let entries_result: Result<Vec<OpLogEntry>, _> = stmt
        .query_map(
            rusqlite::params![
                params.facility_id.to_string(),
                params.since,
                params.device_id.to_string(),
                limit,
            ],
            |row| {
                Ok(OpLogEntry {
                    id: parse_uuid(row, 0)?,
                    sequence: row.get(1)?,
                    facility_id: parse_uuid(row, 2)?,
                    device_id: parse_uuid(row, 3)?,
                    actor_id: parse_uuid(row, 4)?,
                    created_at: parse_dt(row, 5)?,
                    aggregate_type: row.get(6)?,
                    aggregate_id: parse_uuid(row, 7)?,
                    payload: parse_json(row, 8)?,
                    prev_hash: row.get(9)?,
                    entry_hash: row.get(10)?,
                })
            },
        )
        .and_then(|rows| rows.collect());

    let entries = match entries_result {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("query failed: {e}") })),
            );
        }
    };

    // Count total remaining
    let total_count: u64 = conn
        .query_row(
            "SELECT COUNT(*) FROM op_log
             WHERE facility_id = ?1 AND sequence > ?2 AND device_id != ?3",
            rusqlite::params![
                params.facility_id.to_string(),
                params.since,
                params.device_id.to_string(),
            ],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let total_remaining = total_count.saturating_sub(entries.len() as u64);

    // Get server sequence
    let server_sequence: u64 = conn
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) FROM op_log WHERE facility_id = ?1",
            rusqlite::params![params.facility_id.to_string()],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let response = PullResponse {
        entries,
        total_remaining,
        server_sequence,
    };

    (
        StatusCode::OK,
        Json(serde_json::to_value(response).unwrap_or_default()),
    )
}

/// POST /sync/ack — acknowledge received ops.
pub async fn ack(State(state): State<SyncState>, Json(req): Json<AckRequest>) -> impl IntoResponse {
    let engine = SyncEngine::new(state.inner.db_path.clone(), req.device_id, req.facility_id);

    match engine.mark_acknowledged(req.acked_through) {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("ack failed: {e}") })),
        ),
    }
}

/// GET /sync/cursor — return current cursor positions.
pub async fn cursor(
    State(state): State<SyncState>,
    Query(params): Query<CursorParams>,
) -> impl IntoResponse {
    let engine = SyncEngine::new(
        state.inner.db_path.clone(),
        params.device_id,
        params.facility_id,
    );

    match engine.get_cursor() {
        Ok(c) => (
            StatusCode::OK,
            Json(serde_json::to_value(c).unwrap_or_default()),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("cursor fetch failed: {e}") })),
        ),
    }
}

/// GET /sync/health — sync health metrics.
pub async fn sync_health(
    State(state): State<SyncState>,
    Query(params): Query<HealthParams>,
) -> impl IntoResponse {
    let conn = match rusqlite::Connection::open(&state.inner.db_path) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("db error: {e}") })),
            );
        }
    };

    // Get server sequence for lag calculation
    let server_sequence: u64 = conn
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) FROM op_log WHERE facility_id = ?1",
            rusqlite::params![params.facility_id.to_string()],
            |row| row.get(0),
        )
        .unwrap_or(0);

    match health::compute_sync_health(&conn, params.facility_id, params.device_id, server_sequence)
    {
        Ok(h) => (
            StatusCode::OK,
            Json(serde_json::to_value(h).unwrap_or_default()),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("health check failed: {e}") })),
        ),
    }
}

/// Build the sync router with all routes.
pub fn sync_router(state: SyncState) -> axum::Router {
    use axum::routing::{get, post};

    axum::Router::new()
        .route("/sync/push", post(push))
        .route("/sync/pull", get(pull))
        .route("/sync/ack", post(ack))
        .route("/sync/cursor", get(cursor))
        .route("/sync/health", get(sync_health))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Row parsing helpers
// ---------------------------------------------------------------------------

fn parse_uuid(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<Uuid> {
    let s: String = row.get(idx)?;
    Uuid::parse_str(&s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn parse_dt(
    row: &rusqlite::Row<'_>,
    idx: usize,
) -> rusqlite::Result<chrono::DateTime<chrono::Utc>> {
    let s: String = row.get(idx)?;
    chrono::DateTime::parse_from_rfc3339(&s)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, Box::new(e))
        })
}

fn parse_json(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<serde_json::Value> {
    let s: String = row.get(idx)?;
    serde_json::from_str(&s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, Box::new(e))
    })
}
