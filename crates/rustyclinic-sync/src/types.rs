//! Core sync protocol types.

use chrono::{DateTime, Utc};
use rustyclinic_events::OpLogEntry;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Tracks sync progress for a device within a facility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncCursor {
    pub device_id: Uuid,
    pub facility_id: Uuid,
    pub last_pulled_sequence: u64,
    pub last_pushed_sequence: u64,
    pub updated_at: DateTime<Utc>,
}

/// Envelope for sync messages exchanged between devices.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncMessage {
    /// Protocol version (starts at 1).
    pub version: u8,
    pub message_type: SyncMessageType,
    pub facility_id: Uuid,
    pub device_id: Uuid,
    pub entries: Vec<OpLogEntry>,
}

/// Discriminator for sync message direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncMessageType {
    Push,
    Pull,
    Ack,
}

/// Result of a push operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushResult {
    pub accepted: u64,
    pub rejected: Vec<RejectedEntry>,
    pub conflicts: Vec<SyncConflict>,
    pub server_sequence: u64,
}

/// An entry that was rejected during push.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectedEntry {
    pub entry_id: Uuid,
    pub reason: String,
}

/// Result of applying remote operations locally.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyResult {
    pub applied: u64,
    pub conflicts: Vec<SyncConflict>,
}

/// A conflict detected during sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConflict {
    pub id: Uuid,
    pub facility_id: Uuid,
    pub aggregate_type: String,
    pub aggregate_id: Uuid,
    pub local_entry_id: Uuid,
    pub remote_entry_id: Uuid,
    pub conflict_type: ConflictType,
    pub status: ConflictStatus,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolved_by: Option<Uuid>,
    pub resolution: Option<ConflictResolution>,
}

/// What kind of conflict was detected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConflictType {
    FieldConflict { field_name: String },
    StatusTransitionConflict,
    LeaseConflict,
    OwnershipConflict,
}

/// Current status of a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictStatus {
    Pending,
    Resolved,
    Escalated,
}

/// How a conflict was resolved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConflictResolution {
    AcceptLocal,
    AcceptRemote,
    ManualMerge { merged_payload: serde_json::Value },
}

impl std::fmt::Display for ConflictStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Resolved => write!(f, "resolved"),
            Self::Escalated => write!(f, "escalated"),
        }
    }
}

impl std::str::FromStr for ConflictStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "resolved" => Ok(Self::Resolved),
            "escalated" => Ok(Self::Escalated),
            other => Err(format!("unknown conflict status: {other}")),
        }
    }
}
