//! Projection builders for read-optimized views.
//!
//! Projections are rebuildable from canonical data and outbox history.

use chrono::{DateTime, Utc};
use rustyclinic_clinical::queue::QueueStatus;
use rustyclinic_core::error::AppResult;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Queue board projection — the primary view for the queue screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueBoardEntry {
    pub queue_entry_id: Uuid,
    pub patient_id: Uuid,
    pub patient_name: String,
    pub patient_mrn: Option<String>,
    pub service_type: String,
    pub department: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub patient_id: Uuid,
    pub source_aggregate_type: String,
    pub source_aggregate_id: Uuid,
    pub entry_type: String,
    pub title: String,
    pub detail: Option<String>,
    pub occurred_at: DateTime<Utc>,
}

/// Trait for building and querying queue board projections.
pub trait QueueBoardProjection {
    fn rebuild(&self, facility_id: Uuid) -> rustyclinic_core::error::AppResult<()>;
    fn get_board(
        &self,
        facility_id: Uuid,
    ) -> rustyclinic_core::error::AppResult<Vec<QueueBoardEntry>>;
    fn get_stats(&self, facility_id: Uuid) -> rustyclinic_core::error::AppResult<QueueBoardStats>;
}

pub trait PatientSummaryProjection {
    fn rebuild(&self, facility_id: Uuid) -> rustyclinic_core::error::AppResult<()>;
    fn get_summary(
        &self,
        facility_id: Uuid,
        patient_id: Uuid,
    ) -> rustyclinic_core::error::AppResult<Option<PatientSummary>>;
}

pub trait LongitudinalTimelineProjection {
    fn rebuild(&self, facility_id: Uuid) -> rustyclinic_core::error::AppResult<()>;
    fn get_timeline(
        &self,
        facility_id: Uuid,
        patient_id: Uuid,
        limit: usize,
    ) -> rustyclinic_core::error::AppResult<Vec<TimelineEntry>>;
}

pub mod sqlite;

pub struct SqliteProjectionApplier<'a> {
    conn: &'a rusqlite::Connection,
}

impl<'a> SqliteProjectionApplier<'a> {
    pub fn new(conn: &'a rusqlite::Connection) -> Self {
        Self { conn }
    }

    pub fn apply_outbox_event(&self, event: &rustyclinic_events::OutboxEvent) -> AppResult<()> {
        match event.aggregate_type.as_str() {
            "QueueEntry" => {
                let projection = sqlite::SqliteQueueBoardProjection::new(self.conn);
                projection.apply_queue_entry(event.facility_id, event.aggregate_id)
            }
            "Patient" => {
                let summary = sqlite::SqlitePatientSummaryProjection::new(self.conn);
                summary.apply_patient(event.facility_id, event.aggregate_id)?;
                let timeline = sqlite::SqliteLongitudinalTimelineProjection::new(self.conn);
                timeline.apply_patient(event.facility_id, event.aggregate_id)
            }
            "Encounter" => {
                let projection = sqlite::SqlitePatientSummaryProjection::new(self.conn);
                projection.apply_encounter(event.facility_id, event.aggregate_id)?;
                let timeline = sqlite::SqliteLongitudinalTimelineProjection::new(self.conn);
                timeline.apply_encounter(event.facility_id, event.aggregate_id)
            }
            "ProgramEnrollment" => {
                let projection = sqlite::SqlitePatientSummaryProjection::new(self.conn);
                projection.apply_program_enrollment(event.facility_id, event.aggregate_id)?;
                let timeline = sqlite::SqliteLongitudinalTimelineProjection::new(self.conn);
                timeline.apply_program_enrollment(event.facility_id, event.aggregate_id)
            }
            "LabOrder" => {
                let timeline = sqlite::SqliteLongitudinalTimelineProjection::new(self.conn);
                timeline.apply_lab_order(event.facility_id, event.aggregate_id)
            }
            "MedicationDispense" => {
                let timeline = sqlite::SqliteLongitudinalTimelineProjection::new(self.conn);
                timeline.apply_medication_dispense(event.facility_id, event.aggregate_id)
            }
            _ => Ok(()),
        }
    }
}
