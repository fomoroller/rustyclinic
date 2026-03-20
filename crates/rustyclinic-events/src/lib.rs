//! Event primitives: audit log entries, outbox events, idempotency records.
//!
//! Every state-changing transaction persists domain rows + audit + outbox + op-log
//! together in one commit.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Append-only audit log entry. Hash-chained for tamper detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: Uuid,
    pub facility_id: Uuid,
    pub actor_id: Uuid,
    pub device_id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub action: String,
    pub aggregate_type: String,
    pub aggregate_id: Uuid,
    pub payload: serde_json::Value,
    pub prev_hash: Vec<u8>,
    pub entry_hash: Vec<u8>,
}

/// Outbox event for async processing (projections, notifications, sync relay).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboxEvent {
    pub id: Uuid,
    pub facility_id: Uuid,
    pub aggregate_type: String,
    pub aggregate_id: Uuid,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub published: bool,
}

/// Idempotency record to ensure mutations are replayable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdempotencyRecord {
    pub key: String,
    pub facility_id: Uuid,
    pub response: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Operation-log entry for sync replication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpLogEntry {
    pub id: Uuid,
    pub sequence: u64,
    pub facility_id: Uuid,
    pub device_id: Uuid,
    pub actor_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub aggregate_type: String,
    pub aggregate_id: Uuid,
    pub payload: serde_json::Value,
    pub prev_hash: Vec<u8>,
    pub entry_hash: Vec<u8>,
}
