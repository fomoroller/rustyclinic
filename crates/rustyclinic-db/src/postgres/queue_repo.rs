//! PostgreSQL implementation of QueueEntryRepo.

use rustyclinic_clinical::queue::{QueueEntry, QueueEntryRepo, QueueStatus};
use rustyclinic_core::error::{AppError, AppResult};
use tokio_postgres::Client;
use uuid::Uuid;

pub struct PgQueueRepo<'a> {
    client: &'a Client,
}

impl<'a> PgQueueRepo<'a> {
    pub fn new(client: &'a Client) -> Self {
        Self { client }
    }

    fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f))
    }
}

impl QueueEntryRepo for PgQueueRepo<'_> {
    fn create(&self, entry: &QueueEntry) -> AppResult<()> {
        self.block_on(async {
            self.client
                .execute(
                    "INSERT INTO queue_entries (id, facility_id, patient_id, service_type, department, encounter_id, status, assigned_to, position, arrived_at, called_at, service_started_at, completed_at, created_at, version)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
                    &[
                        &entry.id,
                        &entry.facility_id,
                        &entry.patient_id,
                        &entry.service_type,
                        &entry.department,
                        &entry.encounter_id,
                        &entry.status.to_string(),
                        &entry.assigned_to,
                        &(entry.position as i32),
                        &entry.arrived_at,
                        &entry.called_at,
                        &entry.service_started_at,
                        &entry.completed_at,
                        &entry.created_at,
                        &(entry.version as i32),
                    ],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;
            Ok(())
        })
    }

    fn find_by_id(&self, id: Uuid) -> AppResult<Option<QueueEntry>> {
        self.block_on(async {
            let row = self.client
                .query_opt(
                    "SELECT id, facility_id, patient_id, service_type, status, assigned_to, position, arrived_at, called_at, service_started_at, completed_at, created_at, version, department, encounter_id
                     FROM queue_entries WHERE id = $1",
                    &[&id],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            match row {
                Some(row) => Ok(Some(row_to_queue_entry(&row)?)),
                None => Ok(None),
            }
        })
    }

    fn find_active_by_facility(&self, facility_id: Uuid) -> AppResult<Vec<QueueEntry>> {
        self.block_on(async {
            let rows = self.client
                .query(
                    "SELECT id, facility_id, patient_id, service_type, status, assigned_to, position, arrived_at, called_at, service_started_at, completed_at, created_at, version, department, encounter_id
                     FROM queue_entries
                     WHERE facility_id = $1 AND status NOT IN ('completed', 'cancelled', 'no_show')
                     ORDER BY position ASC",
                    &[&facility_id],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            let mut entries = Vec::new();
            for row in &rows {
                entries.push(row_to_queue_entry(row)?);
            }
            Ok(entries)
        })
    }

    fn find_active_by_facility_and_department(
        &self,
        facility_id: Uuid,
        department: &str,
    ) -> AppResult<Vec<QueueEntry>> {
        self.block_on(async {
            let rows = self
                .client
                .query(
                    "SELECT id, facility_id, patient_id, service_type, status, assigned_to, position, arrived_at, called_at, service_started_at, completed_at, created_at, version, department, encounter_id
                     FROM queue_entries
                     WHERE facility_id = $1 AND department = $2 AND status NOT IN ('completed', 'cancelled', 'no_show')
                     ORDER BY position ASC",
                    &[&facility_id, &department],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            let mut entries = Vec::new();
            for row in &rows {
                entries.push(row_to_queue_entry(row)?);
            }
            Ok(entries)
        })
    }

    fn find_by_encounter(&self, encounter_id: Uuid) -> AppResult<Vec<QueueEntry>> {
        self.block_on(async {
            let rows = self
                .client
                .query(
                    "SELECT id, facility_id, patient_id, service_type, status, assigned_to, position, arrived_at, called_at, service_started_at, completed_at, created_at, version, department, encounter_id
                     FROM queue_entries
                     WHERE encounter_id = $1
                     ORDER BY position ASC",
                    &[&encounter_id],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            let mut entries = Vec::new();
            for row in &rows {
                entries.push(row_to_queue_entry(row)?);
            }
            Ok(entries)
        })
    }

    fn update(&self, entry: &QueueEntry) -> AppResult<()> {
        self.block_on(async {
            let affected = self.client
                .execute(
                    "UPDATE queue_entries SET status=$1, assigned_to=$2, called_at=$3, service_started_at=$4, completed_at=$5, version=$6, department=$7, encounter_id=$8
                     WHERE id=$9 AND version=$10",
                    &[
                        &entry.status.to_string(),
                        &entry.assigned_to,
                        &entry.called_at,
                        &entry.service_started_at,
                        &entry.completed_at,
                        &(entry.version as i32),
                        &entry.department,
                        &entry.encounter_id,
                        &entry.id,
                        &((entry.version - 1) as i32),
                    ],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            if affected == 0 {
                return Err(AppError::Conflict {
                    message: "queue entry was modified concurrently".to_string(),
                });
            }
            Ok(())
        })
    }

    fn next_position(&self, facility_id: Uuid) -> AppResult<u32> {
        self.block_on(async {
            let row = self.client
                .query_one(
                    "SELECT COALESCE(MAX(position), 0) FROM queue_entries WHERE facility_id = $1 AND arrived_at::date = CURRENT_DATE",
                    &[&facility_id],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            let max: i32 = row.try_get(0).map_err(|e| AppError::Database(e.to_string()))?;
            Ok((max as u32) + 1)
        })
    }
}

fn row_to_queue_entry(row: &tokio_postgres::Row) -> AppResult<QueueEntry> {
    let status_str: String = row
        .try_get(4)
        .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(QueueEntry {
        id: row
            .try_get(0)
            .map_err(|e| AppError::Database(e.to_string()))?,
        facility_id: row
            .try_get(1)
            .map_err(|e| AppError::Database(e.to_string()))?,
        patient_id: row
            .try_get(2)
            .map_err(|e| AppError::Database(e.to_string()))?,
        service_type: row
            .try_get(3)
            .map_err(|e| AppError::Database(e.to_string()))?,
        department: row
            .try_get(13)
            .map_err(|e| AppError::Database(e.to_string()))?,
        encounter_id: row
            .try_get(14)
            .map_err(|e| AppError::Database(e.to_string()))?,
        status: match status_str.as_str() {
            "created" => QueueStatus::Created,
            "waiting" => QueueStatus::Waiting,
            "called" => QueueStatus::Called,
            "in_service" => QueueStatus::InService,
            "transferred" => QueueStatus::Transferred,
            "completed" => QueueStatus::Completed,
            "no_show" => QueueStatus::NoShow,
            "cancelled" => QueueStatus::Cancelled,
            _ => QueueStatus::Created,
        },
        assigned_to: row
            .try_get(5)
            .map_err(|e| AppError::Database(e.to_string()))?,
        position: row
            .try_get::<_, i32>(6)
            .map_err(|e| AppError::Database(e.to_string()))? as u32,
        arrived_at: row
            .try_get(7)
            .map_err(|e| AppError::Database(e.to_string()))?,
        called_at: row
            .try_get(8)
            .map_err(|e| AppError::Database(e.to_string()))?,
        service_started_at: row
            .try_get(9)
            .map_err(|e| AppError::Database(e.to_string()))?,
        completed_at: row
            .try_get(10)
            .map_err(|e| AppError::Database(e.to_string()))?,
        created_at: row
            .try_get(11)
            .map_err(|e| AppError::Database(e.to_string()))?,
        version: row
            .try_get::<_, i32>(12)
            .map_err(|e| AppError::Database(e.to_string()))? as u32,
    })
}
