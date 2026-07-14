//! SQLite implementation of AdmissionRepo.

use rusqlite::Connection;
use rustyclinic_clinical::admission::{Admission, AdmissionRepo, AdmissionStatus};
use rustyclinic_core::error::{AppError, AppResult};
use uuid::Uuid;

pub struct SqliteAdmissionRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteAdmissionRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

impl AdmissionRepo for SqliteAdmissionRepo<'_> {
    fn create(&self, admission: &Admission) -> AppResult<()> {
        self.conn
            .execute(
                "INSERT INTO admissions (id, encounter_id, patient_id, facility_id, status, ward, bed, admitted_by, admitted_at, transferred_to_ward, transferred_at, discharged_at, discharged_by, discharge_reason, notes, created_at, version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                rusqlite::params![
                    admission.id.to_string(),
                    admission.encounter_id.to_string(),
                    admission.patient_id.to_string(),
                    admission.facility_id.to_string(),
                    admission.status.to_string(),
                    admission.ward,
                    admission.bed,
                    admission.admitted_by.to_string(),
                    admission.admitted_at.map(|t| t.to_rfc3339()),
                    admission.transferred_to_ward,
                    admission.transferred_at.map(|t| t.to_rfc3339()),
                    admission.discharged_at.map(|t| t.to_rfc3339()),
                    admission.discharged_by.map(|u| u.to_string()),
                    admission.discharge_reason,
                    admission.notes,
                    admission.created_at.to_rfc3339(),
                    admission.version,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn find_by_id(&self, id: Uuid) -> AppResult<Option<Admission>> {
        let result = self.conn.query_row(
            "SELECT id, encounter_id, patient_id, facility_id, status, ward, bed, admitted_by, admitted_at, transferred_to_ward, transferred_at, discharged_at, discharged_by, discharge_reason, notes, created_at, version
             FROM admissions WHERE id = ?1",
            rusqlite::params![id.to_string()],
            |row| Ok(row_to_admission(row)),
        );

        match result {
            Ok(a) => Ok(Some(a.map_err(|e| AppError::Database(e.to_string()))?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(AppError::Database(e.to_string())),
        }
    }

    fn find_by_patient(&self, patient_id: Uuid) -> AppResult<Vec<Admission>> {
        query_admissions(
            self.conn,
            "SELECT id, encounter_id, patient_id, facility_id, status, ward, bed, admitted_by, admitted_at, transferred_to_ward, transferred_at, discharged_at, discharged_by, discharge_reason, notes, created_at, version
             FROM admissions WHERE patient_id = ?1 ORDER BY created_at DESC",
            rusqlite::params![patient_id.to_string()],
        )
    }

    fn find_by_encounter(&self, encounter_id: Uuid) -> AppResult<Vec<Admission>> {
        query_admissions(
            self.conn,
            "SELECT id, encounter_id, patient_id, facility_id, status, ward, bed, admitted_by, admitted_at, transferred_to_ward, transferred_at, discharged_at, discharged_by, discharge_reason, notes, created_at, version
             FROM admissions WHERE encounter_id = ?1 ORDER BY created_at ASC",
            rusqlite::params![encounter_id.to_string()],
        )
    }

    fn find_active_by_facility(&self, facility_id: Uuid) -> AppResult<Vec<Admission>> {
        query_admissions(
            self.conn,
            "SELECT id, encounter_id, patient_id, facility_id, status, ward, bed, admitted_by, admitted_at, transferred_to_ward, transferred_at, discharged_at, discharged_by, discharge_reason, notes, created_at, version
             FROM admissions WHERE facility_id = ?1 AND status NOT IN ('discharged')
             ORDER BY created_at ASC",
            rusqlite::params![facility_id.to_string()],
        )
    }

    fn update(&self, admission: &Admission) -> AppResult<()> {
        let affected = self.conn
            .execute(
                "UPDATE admissions SET status=?1, ward=?2, bed=?3, admitted_at=?4, transferred_to_ward=?5, transferred_at=?6, discharged_at=?7, discharged_by=?8, discharge_reason=?9, notes=?10, version=?11
                 WHERE id=?12 AND version=?13",
                rusqlite::params![
                    admission.status.to_string(),
                    admission.ward,
                    admission.bed,
                    admission.admitted_at.map(|t| t.to_rfc3339()),
                    admission.transferred_to_ward,
                    admission.transferred_at.map(|t| t.to_rfc3339()),
                    admission.discharged_at.map(|t| t.to_rfc3339()),
                    admission.discharged_by.map(|u| u.to_string()),
                    admission.discharge_reason,
                    admission.notes,
                    admission.version,
                    admission.id.to_string(),
                    admission.version - 1,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        if affected == 0 {
            return Err(AppError::Conflict {
                message: "admission was modified concurrently".to_string(),
            });
        }
        Ok(())
    }
}

fn query_admissions(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> AppResult<Vec<Admission>> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| AppError::Database(e.to_string()))?;
    let rows = stmt
        .query_map(params, |row| Ok(row_to_admission(row)))
        .map_err(|e| AppError::Database(e.to_string()))?;

    let mut results = Vec::new();
    for row in rows {
        let a = row
            .map_err(|e| AppError::Database(e.to_string()))?
            .map_err(|e| AppError::Database(e.to_string()))?;
        results.push(a);
    }
    Ok(results)
}

fn row_to_admission(row: &rusqlite::Row) -> Result<Admission, rusqlite::Error> {
    let id_str: String = row.get(0)?;
    let encounter_str: String = row.get(1)?;
    let patient_str: String = row.get(2)?;
    let facility_str: String = row.get(3)?;
    let status_str: String = row.get(4)?;
    let ward: String = row.get(5)?;
    let bed: Option<String> = row.get(6)?;
    let admitted_by_str: String = row.get(7)?;
    let admitted_at_str: Option<String> = row.get(8)?;
    let transferred_to_ward: Option<String> = row.get(9)?;
    let transferred_at_str: Option<String> = row.get(10)?;
    let discharged_at_str: Option<String> = row.get(11)?;
    let discharged_by_str: Option<String> = row.get(12)?;
    let discharge_reason: Option<String> = row.get(13)?;
    let notes: Option<String> = row.get(14)?;
    let created_str: String = row.get(15)?;
    let version: u32 = row.get(16)?;

    let parse_dt = |s: &str| {
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now())
    };

    Ok(Admission {
        id: Uuid::parse_str(&id_str).unwrap_or_default(),
        encounter_id: Uuid::parse_str(&encounter_str).unwrap_or_default(),
        patient_id: Uuid::parse_str(&patient_str).unwrap_or_default(),
        facility_id: Uuid::parse_str(&facility_str).unwrap_or_default(),
        status: AdmissionStatus::from_str_safe(&status_str),
        ward,
        bed,
        admitted_by: Uuid::parse_str(&admitted_by_str).unwrap_or_default(),
        admitted_at: admitted_at_str.as_deref().map(parse_dt),
        transferred_to_ward,
        transferred_at: transferred_at_str.as_deref().map(parse_dt),
        discharged_at: discharged_at_str.as_deref().map(parse_dt),
        discharged_by: discharged_by_str.and_then(|s| Uuid::parse_str(&s).ok()),
        discharge_reason,
        notes,
        created_at: parse_dt(&created_str),
        version,
    })
}
