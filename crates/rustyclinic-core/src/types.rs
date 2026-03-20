//! Shared types used across all crates.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Generate a new UUIDv7 (time-sortable, globally unique).
pub fn new_id() -> Uuid {
    Uuid::now_v7()
}

/// Facility scope — every tenant-owned row carries this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FacilityScope {
    pub facility_id: Uuid,
}

/// Actor context resolved at the start of every operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActorContext {
    pub user_id: Uuid,
    pub facility_id: Uuid,
    pub device_id: Uuid,
    pub roles: Vec<String>,
    pub purpose: String,
    pub session_id: Uuid,
}

/// Metadata attached to every write for audit and sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationMeta {
    pub id: Uuid,
    pub actor: ActorContext,
    pub timestamp: DateTime<Utc>,
    pub idempotency_key: Option<String>,
}

impl OperationMeta {
    pub fn new(actor: ActorContext) -> Self {
        Self {
            id: new_id(),
            actor,
            timestamp: Utc::now(),
            idempotency_key: None,
        }
    }

    pub fn with_idempotency_key(mut self, key: String) -> Self {
        self.idempotency_key = Some(key);
        self
    }
}

/// Sex as used in patient demographics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sex {
    Female,
    Male,
    Other,
    Unknown,
}
