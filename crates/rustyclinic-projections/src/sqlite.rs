use chrono::{Datelike, Utc};
use rusqlite::{Connection, OptionalExtension};
use rustyclinic_clinical::queue::QueueStatus;
use rustyclinic_core::error::{AppError, AppResult};
use uuid::Uuid;

use crate::{
    LongitudinalTimelineProjection, PatientSummary, PatientSummaryProjection, QueueBoardEntry,
    QueueBoardProjection, QueueBoardStats, TimelineEntry,
};

type QueueCanonicalRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    u32,
    String,
    Option<String>,
    Option<String>,
);
type PatientCanonicalRow = (String, String, String, Option<String>, Option<String>);
type PatientSummaryRow = (
    String,
    String,
    String,
    Option<u32>,
    Option<String>,
    Option<String>,
    String,
);
type LabOrderRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
);
type MedicationDispenseRow = (String, String, Option<String>, Option<String>, String);
type ProgramEnrollmentRow = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
);

pub struct SqliteQueueBoardProjection<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteQueueBoardProjection<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn apply_queue_entry(&self, facility_id: Uuid, queue_entry_id: Uuid) -> AppResult<()> {
        self.upsert_from_canonical(facility_id, queue_entry_id)
    }

    fn upsert_from_canonical(&self, facility_id: Uuid, queue_entry_id: Uuid) -> AppResult<()> {
        let now = Utc::now();
        let fid = facility_id.to_string();
        let qid = queue_entry_id.to_string();

        let mut stmt = self
            .conn
            .prepare(
                "SELECT q.id, q.patient_id, p.given_name, p.family_name, p.national_id,
                        q.service_type, q.department, q.status, q.position, q.arrived_at,
                        q.assigned_to, u.display_name
                 FROM queue_entries q
                 JOIN patients p ON p.id = q.patient_id
                 LEFT JOIN users u ON u.id = q.assigned_to
                 WHERE q.facility_id = ?1 AND q.id = ?2",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let row: Option<QueueCanonicalRow> = stmt
            .query_row(rusqlite::params![fid, qid], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                    r.get(11)?,
                ))
            })
            .optional()
            .map_err(|e| AppError::Database(e.to_string()))?;

        let Some((
            queue_entry_id,
            patient_id,
            given_name,
            family_name,
            national_id,
            service_type,
            department,
            status,
            position,
            arrived_at,
            assigned_to,
            assigned_to_name,
        )) = row
        else {
            self.conn
                .execute(
                    "DELETE FROM projection_queue_board_v1 WHERE facility_id = ?1 AND queue_entry_id = ?2",
                    rusqlite::params![facility_id.to_string(), queue_entry_id.to_string()],
                )
                .map_err(|e| AppError::Database(e.to_string()))?;
            return Ok(());
        };

        let patient_name = format!("{family_name}, {given_name}");

        self.conn
            .execute(
                "INSERT INTO projection_queue_board_v1
                    (facility_id, queue_entry_id, patient_id, patient_name, patient_mrn,
                     service_type, department, status, position, arrived_at,
                     assigned_to, assigned_to_name, updated_at)
                 VALUES
                    (?1, ?2, ?3, ?4, ?5,
                     ?6, ?7, ?8, ?9, ?10,
                     ?11, ?12, ?13)
                 ON CONFLICT(facility_id, queue_entry_id) DO UPDATE SET
                    patient_id = excluded.patient_id,
                    patient_name = excluded.patient_name,
                    patient_mrn = excluded.patient_mrn,
                    service_type = excluded.service_type,
                    department = excluded.department,
                    status = excluded.status,
                    position = excluded.position,
                    arrived_at = excluded.arrived_at,
                    assigned_to = excluded.assigned_to,
                    assigned_to_name = excluded.assigned_to_name,
                    updated_at = excluded.updated_at",
                rusqlite::params![
                    facility_id.to_string(),
                    queue_entry_id,
                    patient_id,
                    patient_name,
                    national_id,
                    service_type,
                    department,
                    status,
                    position,
                    arrived_at,
                    assigned_to,
                    assigned_to_name,
                    now.to_rfc3339(),
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    fn parse_status(value: &str) -> QueueStatus {
        match value {
            "created" => QueueStatus::Created,
            "waiting" => QueueStatus::Waiting,
            "called" => QueueStatus::Called,
            "in_service" => QueueStatus::InService,
            "transferred" => QueueStatus::Transferred,
            "completed" => QueueStatus::Completed,
            "no_show" => QueueStatus::NoShow,
            "cancelled" => QueueStatus::Cancelled,
            _ => QueueStatus::Created,
        }
    }
}

impl QueueBoardProjection for SqliteQueueBoardProjection<'_> {
    fn rebuild(&self, _facility_id: Uuid) -> AppResult<()> {
        let facility_id = _facility_id;
        self.conn
            .execute(
                "DELETE FROM projection_queue_board_v1 WHERE facility_id = ?1",
                rusqlite::params![facility_id.to_string()],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut stmt = self
            .conn
            .prepare(
                "SELECT id FROM queue_entries
                 WHERE facility_id = ?1 AND date(arrived_at) = date('now')",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let ids = stmt
            .query_map(rusqlite::params![facility_id.to_string()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| AppError::Database(e.to_string()))?;

        for id in ids.flatten() {
            let qid = Uuid::parse_str(&id).unwrap_or_default();
            self.upsert_from_canonical(facility_id, qid)?;
        }

        Ok(())
    }

    fn get_board(&self, facility_id: Uuid) -> AppResult<Vec<QueueBoardEntry>> {
        let now = Utc::now();
        let fid = facility_id.to_string();

        let existing: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM projection_queue_board_v1 WHERE facility_id = ?1 AND date(arrived_at) = date('now')",
                rusqlite::params![fid.clone()],
                |row| row.get(0),
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        if existing == 0 {
            self.rebuild(facility_id)?;
        }

        let mut stmt = self
            .conn
            .prepare(
                "SELECT queue_entry_id, patient_id, patient_name, patient_mrn,
                        service_type, department, status, position, arrived_at,
                        assigned_to_name
                 FROM projection_queue_board_v1
                 WHERE facility_id = ?1
                   AND status NOT IN ('completed', 'cancelled', 'no_show')
                   AND date(arrived_at) = date('now')
                 ORDER BY position ASC",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params![fid], |row| {
                let queue_entry_id: String = row.get(0)?;
                let patient_id: String = row.get(1)?;
                let patient_name: String = row.get(2)?;
                let patient_mrn: Option<String> = row.get(3)?;
                let service_type: String = row.get(4)?;
                let department: String = row.get(5)?;
                let status: String = row.get(6)?;
                let position: u32 = row.get(7)?;
                let arrived_str: String = row.get(8)?;
                let assigned_to_name: Option<String> = row.get(9)?;

                let arrived_at = chrono::DateTime::parse_from_rfc3339(&arrived_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or(now);

                let wait_minutes = now.signed_duration_since(arrived_at).num_minutes();

                Ok(QueueBoardEntry {
                    queue_entry_id: Uuid::parse_str(&queue_entry_id).unwrap_or_default(),
                    patient_id: Uuid::parse_str(&patient_id).unwrap_or_default(),
                    patient_name,
                    patient_mrn,
                    service_type,
                    department,
                    status: Self::parse_status(&status),
                    position,
                    arrived_at,
                    assigned_to_name,
                    wait_minutes,
                })
            })
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    fn get_stats(&self, facility_id: Uuid) -> AppResult<QueueBoardStats> {
        let board = self.get_board(facility_id)?;

        let mut stats = QueueBoardStats::default();
        let mut total_wait = 0i64;
        let mut wait_count = 0u32;

        for e in &board {
            match e.status {
                QueueStatus::Waiting | QueueStatus::Created => stats.waiting += 1,
                QueueStatus::Called | QueueStatus::InService | QueueStatus::Transferred => {
                    stats.in_service += 1
                }
                QueueStatus::Completed => stats.completed += 1,
                _ => {}
            }

            total_wait += e.wait_minutes;
            wait_count += 1;
        }

        if wait_count > 0 {
            stats.avg_wait_minutes = (total_wait / wait_count as i64) as u32;
        }

        Ok(stats)
    }
}

pub struct SqlitePatientSummaryProjection<'a> {
    conn: &'a Connection,
}

impl<'a> SqlitePatientSummaryProjection<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn apply_patient(&self, facility_id: Uuid, patient_id: Uuid) -> AppResult<()> {
        self.upsert_from_canonical(facility_id, patient_id)
    }

    pub fn apply_program_enrollment(
        &self,
        facility_id: Uuid,
        enrollment_id: Uuid,
    ) -> AppResult<()> {
        let patient_id: Option<String> = self
            .conn
            .query_row(
                "SELECT patient_id FROM program_enrollments WHERE facility_id = ?1 AND id = ?2",
                rusqlite::params![facility_id.to_string(), enrollment_id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| AppError::Database(e.to_string()))?;

        if let Some(patient_id) = patient_id {
            let patient_id = Uuid::parse_str(&patient_id).unwrap_or_default();
            self.upsert_from_canonical(facility_id, patient_id)?;
        }

        Ok(())
    }

    pub fn apply_encounter(&self, facility_id: Uuid, encounter_id: Uuid) -> AppResult<()> {
        let patient_id: Option<String> = self
            .conn
            .query_row(
                "SELECT patient_id FROM encounters WHERE facility_id = ?1 AND id = ?2",
                rusqlite::params![facility_id.to_string(), encounter_id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| AppError::Database(e.to_string()))?;

        if let Some(patient_id) = patient_id {
            let patient_id = Uuid::parse_str(&patient_id).unwrap_or_default();
            self.upsert_from_canonical(facility_id, patient_id)?;
        }

        Ok(())
    }

    fn upsert_from_canonical(&self, facility_id: Uuid, patient_id: Uuid) -> AppResult<()> {
        let now = Utc::now();
        let fid = facility_id.to_string();
        let pid = patient_id.to_string();

        let row: Option<PatientCanonicalRow> = self
            .conn
            .query_row(
                "SELECT given_name, family_name, sex, date_of_birth, national_id
                 FROM patients
                 WHERE facility_id = ?1 AND id = ?2",
                rusqlite::params![fid, pid],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| AppError::Database(e.to_string()))?;

        let Some((given_name, family_name, sex, date_of_birth, national_id)) = row else {
            self.conn
                .execute(
                    "DELETE FROM projection_patient_summary_v1 WHERE facility_id = ?1 AND patient_id = ?2",
                    rusqlite::params![facility_id.to_string(), patient_id.to_string()],
                )
                .map_err(|e| AppError::Database(e.to_string()))?;
            return Ok(());
        };

        let last_visit: Option<String> = self
            .conn
            .query_row(
                "SELECT MAX(COALESCE(ended_at, started_at)) FROM encounters
                 WHERE facility_id = ?1 AND patient_id = ?2",
                rusqlite::params![facility_id.to_string(), patient_id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| AppError::Database(e.to_string()))?
            .flatten();

        let age = date_of_birth
            .as_deref()
            .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            .map(|dob| {
                let today = Utc::now().date_naive();
                let mut years = today.year() - dob.year();
                if (today.month(), today.day()) < (dob.month(), dob.day()) {
                    years -= 1;
                }
                years.max(0) as u32
            });

        let active_programs: Vec<String> = self
            .conn
            .prepare(
                "SELECT program_name FROM program_enrollments
                 WHERE facility_id = ?1 AND patient_id = ?2 AND status = 'active'
                 ORDER BY activated_at DESC, created_at DESC",
            )
            .map_err(|e| AppError::Database(e.to_string()))?
            .query_map(
                rusqlite::params![facility_id.to_string(), patient_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .map_err(|e| AppError::Database(e.to_string()))?
            .filter_map(|row| row.ok())
            .collect();

        self.conn
            .execute(
                "INSERT INTO projection_patient_summary_v1
                    (facility_id, patient_id, given_name, family_name, sex, age,
                     national_id, last_visit, active_programs, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(facility_id, patient_id) DO UPDATE SET
                    given_name = excluded.given_name,
                    family_name = excluded.family_name,
                    sex = excluded.sex,
                    age = excluded.age,
                    national_id = excluded.national_id,
                    last_visit = excluded.last_visit,
                    active_programs = excluded.active_programs,
                    updated_at = excluded.updated_at",
                rusqlite::params![
                    facility_id.to_string(),
                    patient_id.to_string(),
                    given_name,
                    family_name,
                    sex,
                    age,
                    national_id,
                    last_visit,
                    serde_json::to_string(&active_programs).unwrap_or_else(|_| "[]".to_string()),
                    now.to_rfc3339(),
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }
}

impl PatientSummaryProjection for SqlitePatientSummaryProjection<'_> {
    fn rebuild(&self, facility_id: Uuid) -> AppResult<()> {
        self.conn
            .execute(
                "DELETE FROM projection_patient_summary_v1 WHERE facility_id = ?1",
                rusqlite::params![facility_id.to_string()],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut stmt = self
            .conn
            .prepare("SELECT id FROM patients WHERE facility_id = ?1")
            .map_err(|e| AppError::Database(e.to_string()))?;

        let ids = stmt
            .query_map(rusqlite::params![facility_id.to_string()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| AppError::Database(e.to_string()))?;

        for id in ids.flatten() {
            let patient_id = Uuid::parse_str(&id).unwrap_or_default();
            self.upsert_from_canonical(facility_id, patient_id)?;
        }

        Ok(())
    }

    fn get_summary(
        &self,
        facility_id: Uuid,
        patient_id: Uuid,
    ) -> AppResult<Option<PatientSummary>> {
        let row: Option<PatientSummaryRow> = self
            .conn
            .query_row(
                "SELECT given_name, family_name, sex, age, national_id, last_visit, active_programs
                 FROM projection_patient_summary_v1
                 WHERE facility_id = ?1 AND patient_id = ?2",
                rusqlite::params![facility_id.to_string(), patient_id.to_string()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| AppError::Database(e.to_string()))?;

        if row.is_none() {
            self.upsert_from_canonical(facility_id, patient_id)?;
        }

        let row: Option<PatientSummaryRow> = self
            .conn
            .query_row(
                "SELECT given_name, family_name, sex, age, national_id, last_visit, active_programs
                 FROM projection_patient_summary_v1
                 WHERE facility_id = ?1 AND patient_id = ?2",
                rusqlite::params![facility_id.to_string(), patient_id.to_string()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(row.map(
            |(given_name, family_name, sex, age, national_id, last_visit, active_programs)| {
                let last_visit = last_visit.and_then(|s| {
                    chrono::DateTime::parse_from_rfc3339(&s)
                        .ok()
                        .map(|dt| dt.with_timezone(&Utc))
                });
                let active_programs = serde_json::from_str(&active_programs).unwrap_or_default();

                PatientSummary {
                    patient_id,
                    given_name,
                    family_name,
                    sex,
                    age,
                    national_id,
                    last_visit,
                    active_programs,
                }
            },
        ))
    }
}

pub struct SqliteLongitudinalTimelineProjection<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteLongitudinalTimelineProjection<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn apply_patient(&self, facility_id: Uuid, patient_id: Uuid) -> AppResult<()> {
        let row: Option<(String, Option<String>, String)> = self
            .conn
            .query_row(
                "SELECT created_at, national_id, id FROM patients WHERE facility_id = ?1 AND id = ?2",
                rusqlite::params![facility_id.to_string(), patient_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|e| AppError::Database(e.to_string()))?;

        if let Some((created_at, national_id, aggregate_id)) = row {
            let detail = national_id.map(|id| format!("National ID: {id}"));
            self.upsert_entry(
                facility_id,
                patient_id,
                "Patient",
                &aggregate_id,
                "patient",
                "Patient registered",
                detail,
                &created_at,
            )?;
        }

        Ok(())
    }

    pub fn apply_encounter(&self, facility_id: Uuid, encounter_id: Uuid) -> AppResult<()> {
        let row: Option<(String, String, Option<String>, String, String)> = self
            .conn
            .query_row(
                "SELECT patient_id, status, ended_at, started_at, visit_notes
                 FROM encounters WHERE facility_id = ?1 AND id = ?2",
                rusqlite::params![facility_id.to_string(), encounter_id.to_string()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| AppError::Database(e.to_string()))?;

        if let Some((patient_id, status, ended_at, started_at, visit_notes)) = row {
            let patient_id = Uuid::parse_str(&patient_id).unwrap_or_default();
            let occurred_at = ended_at.unwrap_or(started_at);
            let title = if status == "completed" {
                "Encounter completed"
            } else {
                "Encounter started"
            };
            let detail = (!visit_notes.is_empty()).then_some(visit_notes);

            self.upsert_entry(
                facility_id,
                patient_id,
                "Encounter",
                &encounter_id.to_string(),
                "encounter",
                title,
                detail,
                &occurred_at,
            )?;
        }

        Ok(())
    }

    pub fn apply_lab_order(&self, facility_id: Uuid, order_id: Uuid) -> AppResult<()> {
        let row: Option<LabOrderRow> = self
            .conn
            .query_row(
                "SELECT patient_id, status, specimen_type, verified_at, resulted_at, created_at
                     FROM lab_orders WHERE facility_id = ?1 AND id = ?2",
                rusqlite::params![facility_id.to_string(), order_id.to_string()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| AppError::Database(e.to_string()))?;

        if let Some((patient_id, status, specimen_type, verified_at, resulted_at, created_at)) = row
        {
            let patient_id = Uuid::parse_str(&patient_id).unwrap_or_default();
            let occurred_at = verified_at.or(resulted_at).unwrap_or(created_at);
            let title = format!("Lab order {status}");
            let detail = specimen_type.map(|specimen| format!("Specimen: {specimen}"));

            self.upsert_entry(
                facility_id,
                patient_id,
                "LabOrder",
                &order_id.to_string(),
                "lab_order",
                &title,
                detail,
                &occurred_at,
            )?;
        }

        Ok(())
    }

    pub fn apply_medication_dispense(&self, facility_id: Uuid, dispense_id: Uuid) -> AppResult<()> {
        let row: Option<MedicationDispenseRow> = self
            .conn
            .query_row(
                "SELECT patient_id, status, dispensed_at, notes, created_at
                 FROM medication_dispenses WHERE facility_id = ?1 AND id = ?2",
                rusqlite::params![facility_id.to_string(), dispense_id.to_string()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| AppError::Database(e.to_string()))?;

        if let Some((patient_id, status, dispensed_at, notes, created_at)) = row {
            let patient_id = Uuid::parse_str(&patient_id).unwrap_or_default();
            let occurred_at = dispensed_at.unwrap_or(created_at);
            let title = format!("Medication dispense {status}");
            let detail = notes.filter(|value| !value.is_empty());

            self.upsert_entry(
                facility_id,
                patient_id,
                "MedicationDispense",
                &dispense_id.to_string(),
                "medication_dispense",
                &title,
                detail,
                &occurred_at,
            )?;
        }

        Ok(())
    }

    pub fn apply_program_enrollment(
        &self,
        facility_id: Uuid,
        enrollment_id: Uuid,
    ) -> AppResult<()> {
        let row: Option<ProgramEnrollmentRow> = self
            .conn
            .query_row(
                "SELECT patient_id, program_name, status, enrolled_at, activated_at, paused_at, completed_at, withdrawn_at, created_at
                 FROM program_enrollments WHERE facility_id = ?1 AND id = ?2",
                rusqlite::params![facility_id.to_string(), enrollment_id.to_string()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| AppError::Database(e.to_string()))?;

        if let Some((
            patient_id,
            program_name,
            status,
            enrolled_at,
            activated_at,
            paused_at,
            completed_at,
            withdrawn_at,
            created_at,
        )) = row
        {
            let patient_id = Uuid::parse_str(&patient_id).unwrap_or_default();
            let occurred_at = withdrawn_at
                .or(completed_at)
                .or(paused_at)
                .or(activated_at)
                .or(enrolled_at)
                .unwrap_or(created_at);
            let title = format!("Program enrollment {status}");

            self.upsert_entry(
                facility_id,
                patient_id,
                "ProgramEnrollment",
                &enrollment_id.to_string(),
                "program_enrollment",
                &title,
                Some(program_name),
                &occurred_at,
            )?;
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn upsert_entry(
        &self,
        facility_id: Uuid,
        patient_id: Uuid,
        source_aggregate_type: &str,
        source_aggregate_id: &str,
        entry_type: &str,
        title: &str,
        detail: Option<String>,
        occurred_at: &str,
    ) -> AppResult<()> {
        self.conn
            .execute(
                "INSERT INTO projection_longitudinal_timeline_v1
                    (facility_id, patient_id, source_aggregate_type, source_aggregate_id,
                     entry_type, title, detail, occurred_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(facility_id, source_aggregate_type, source_aggregate_id) DO UPDATE SET
                    patient_id = excluded.patient_id,
                    entry_type = excluded.entry_type,
                    title = excluded.title,
                    detail = excluded.detail,
                    occurred_at = excluded.occurred_at,
                    updated_at = excluded.updated_at",
                rusqlite::params![
                    facility_id.to_string(),
                    patient_id.to_string(),
                    source_aggregate_type,
                    source_aggregate_id,
                    entry_type,
                    title,
                    detail,
                    occurred_at,
                    Utc::now().to_rfc3339(),
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }
}

impl LongitudinalTimelineProjection for SqliteLongitudinalTimelineProjection<'_> {
    fn rebuild(&self, facility_id: Uuid) -> AppResult<()> {
        self.conn
            .execute(
                "DELETE FROM projection_longitudinal_timeline_v1 WHERE facility_id = ?1",
                rusqlite::params![facility_id.to_string()],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut patient_stmt = self
            .conn
            .prepare("SELECT id FROM patients WHERE facility_id = ?1")
            .map_err(|e| AppError::Database(e.to_string()))?;
        let patient_ids = patient_stmt
            .query_map(rusqlite::params![facility_id.to_string()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| AppError::Database(e.to_string()))?;
        for id in patient_ids.flatten() {
            self.apply_patient(facility_id, Uuid::parse_str(&id).unwrap_or_default())?;
        }

        let mut encounter_stmt = self
            .conn
            .prepare("SELECT id FROM encounters WHERE facility_id = ?1")
            .map_err(|e| AppError::Database(e.to_string()))?;
        let encounter_ids = encounter_stmt
            .query_map(rusqlite::params![facility_id.to_string()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| AppError::Database(e.to_string()))?;
        for id in encounter_ids.flatten() {
            self.apply_encounter(facility_id, Uuid::parse_str(&id).unwrap_or_default())?;
        }

        let mut lab_stmt = self
            .conn
            .prepare("SELECT id FROM lab_orders WHERE facility_id = ?1")
            .map_err(|e| AppError::Database(e.to_string()))?;
        let lab_ids = lab_stmt
            .query_map(rusqlite::params![facility_id.to_string()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| AppError::Database(e.to_string()))?;
        for id in lab_ids.flatten() {
            self.apply_lab_order(facility_id, Uuid::parse_str(&id).unwrap_or_default())?;
        }

        let mut dispense_stmt = self
            .conn
            .prepare("SELECT id FROM medication_dispenses WHERE facility_id = ?1")
            .map_err(|e| AppError::Database(e.to_string()))?;
        let dispense_ids = dispense_stmt
            .query_map(rusqlite::params![facility_id.to_string()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| AppError::Database(e.to_string()))?;
        for id in dispense_ids.flatten() {
            self.apply_medication_dispense(facility_id, Uuid::parse_str(&id).unwrap_or_default())?;
        }

        let mut enrollment_stmt = self
            .conn
            .prepare("SELECT id FROM program_enrollments WHERE facility_id = ?1")
            .map_err(|e| AppError::Database(e.to_string()))?;
        let enrollment_ids = enrollment_stmt
            .query_map(rusqlite::params![facility_id.to_string()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| AppError::Database(e.to_string()))?;
        for id in enrollment_ids.flatten() {
            self.apply_program_enrollment(facility_id, Uuid::parse_str(&id).unwrap_or_default())?;
        }

        Ok(())
    }

    fn get_timeline(
        &self,
        facility_id: Uuid,
        patient_id: Uuid,
        limit: usize,
    ) -> AppResult<Vec<TimelineEntry>> {
        let existing: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM projection_longitudinal_timeline_v1 WHERE facility_id = ?1 AND patient_id = ?2",
                rusqlite::params![facility_id.to_string(), patient_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        if existing == 0 {
            self.rebuild(facility_id)?;
        }

        let mut stmt = self
            .conn
            .prepare(
                "SELECT source_aggregate_type, source_aggregate_id, entry_type, title, detail, occurred_at
                 FROM projection_longitudinal_timeline_v1
                 WHERE facility_id = ?1 AND patient_id = ?2
                 ORDER BY occurred_at DESC
                 LIMIT ?3",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(
                rusqlite::params![
                    facility_id.to_string(),
                    patient_id.to_string(),
                    limit as i64
                ],
                |row| {
                    let source_aggregate_type: String = row.get(0)?;
                    let source_aggregate_id: String = row.get(1)?;
                    let entry_type: String = row.get(2)?;
                    let title: String = row.get(3)?;
                    let detail: Option<String> = row.get(4)?;
                    let occurred_at: String = row.get(5)?;

                    let occurred_at = chrono::DateTime::parse_from_rfc3339(&occurred_at)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now());

                    Ok(TimelineEntry {
                        patient_id,
                        source_aggregate_type,
                        source_aggregate_id: Uuid::parse_str(&source_aggregate_id)
                            .unwrap_or_default(),
                        entry_type,
                        title,
                        detail,
                        occurred_at,
                    })
                },
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(rows.filter_map(|row| row.ok()).collect())
    }
}
