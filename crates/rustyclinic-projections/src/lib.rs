//! Projection builders for read-optimized views.
//!
//! Projections are rebuildable from canonical data and outbox history.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use rustyclinic_clinical::queue::QueueStatus;

/// Queue board projection — the primary view for the queue screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueBoardEntry {
    pub queue_entry_id: Uuid,
    pub patient_id: Uuid,
    pub patient_name: String,
    pub patient_mrn: Option<String>,
    pub service_type: String,
    pub status: QueueStatus,
    pub position: u32,
    pub arrived_at: DateTime<Utc>,
    pub assigned_to_name: Option<String>,
    pub wait_minutes: i64,
}

/// Queue board summary stats.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct QueueBoardStats {
    pub waiting: u32,
    pub in_service: u32,
    pub completed: u32,
    pub avg_wait_minutes: u32,
}

/// Patient summary projection for the patient header/card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatientSummary {
    pub patient_id: Uuid,
    pub given_name: String,
    pub family_name: String,
    pub sex: String,
    pub age: Option<u32>,
    pub national_id: Option<String>,
    pub last_visit: Option<DateTime<Utc>>,
    pub active_programs: Vec<String>,
}

/// Trait for building and querying queue board projections.
pub trait QueueBoardProjection {
    fn rebuild(&self, facility_id: Uuid) -> rustyclinic_core::error::AppResult<()>;
    fn get_board(&self, facility_id: Uuid) -> rustyclinic_core::error::AppResult<Vec<QueueBoardEntry>>;
    fn get_stats(&self, facility_id: Uuid) -> rustyclinic_core::error::AppResult<QueueBoardStats>;
}
