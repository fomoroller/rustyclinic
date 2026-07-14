use chrono::{DateTime, Utc};
use rustyclinic_core::error::AppResult;
use rustyclinic_events::OpLogEntry;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncCursorRecord {
    pub device_id: Uuid,
    pub facility_id: Uuid,
    pub last_pulled_sequence: u64,
    pub last_pushed_sequence: u64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncConflictStatus {
    Pending,
    Resolved,
    Escalated,
}

impl SyncConflictStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Resolved => "resolved",
            Self::Escalated => "escalated",
        }
    }
}

impl std::str::FromStr for SyncConflictStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "resolved" => Ok(Self::Resolved),
            "escalated" => Ok(Self::Escalated),
            other => Err(format!("unknown sync conflict status: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConflictRecord {
    pub id: Uuid,
    pub facility_id: Uuid,
    pub aggregate_type: String,
    pub aggregate_id: Uuid,
    pub local_entry_id: Uuid,
    pub remote_entry_id: Uuid,
    pub conflict_type: serde_json::Value,
    pub status: SyncConflictStatus,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolved_by: Option<Uuid>,
    pub resolution: Option<serde_json::Value>,
}

pub trait OpLogSyncRepo {
    fn list_pending(&self, facility_id: Uuid, limit: u32) -> AppResult<Vec<OpLogEntry>>;
    fn count_pending(&self, facility_id: Uuid) -> AppResult<u64>;
    fn list_since_excluding_device(
        &self,
        facility_id: Uuid,
        since_sequence: u64,
        excluded_device_id: Uuid,
        limit: u32,
    ) -> AppResult<Vec<OpLogEntry>>;
    fn list_unacknowledged_for_aggregate(
        &self,
        facility_id: Uuid,
        aggregate_type: &str,
        aggregate_id: Uuid,
    ) -> AppResult<Vec<OpLogEntry>>;
    fn exists(&self, id: Uuid) -> AppResult<bool>;
    fn insert_pending_if_missing(&self, entry: &OpLogEntry) -> AppResult<bool>;
    fn insert_acknowledged_if_missing(&self, entry: &OpLogEntry) -> AppResult<bool>;
    fn mark_pushed_through(&self, facility_id: Uuid, through_sequence: u64) -> AppResult<()>;
    fn mark_acknowledged_through(&self, facility_id: Uuid, through_sequence: u64) -> AppResult<()>;
}

pub trait SyncCursorRepo {
    fn get(&self, device_id: Uuid, facility_id: Uuid) -> AppResult<Option<SyncCursorRecord>>;
    fn upsert(&self, cursor: &SyncCursorRecord) -> AppResult<()>;
}

pub trait SyncConflictRepo {
    fn insert(&self, conflict: &SyncConflictRecord) -> AppResult<()>;
    fn list_pending(&self, facility_id: Uuid) -> AppResult<Vec<SyncConflictRecord>>;
    fn mark_resolved(
        &self,
        conflict_id: Uuid,
        resolved_by: Uuid,
        resolution: serde_json::Value,
        resolved_at: DateTime<Utc>,
    ) -> AppResult<()>;
}
