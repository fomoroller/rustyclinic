//! SQLite implementation of ProgramEnrollmentRepo.

use rusqlite::Connection;
use rustyclinic_clinical::program::{EnrollmentStatus, ProgramEnrollment, ProgramEnrollmentRepo};
use rustyclinic_core::error::{AppError, AppResult};
use uuid::Uuid;

pub struct SqliteProgramEnrollmentRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteProgramEnrollmentRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

impl ProgramEnrollmentRepo for SqliteProgramEnrollmentRepo<'_> {
    fn create(&self, enrollment: &ProgramEnrollment) -> AppResult<()> {
        self.conn
            .execute(
                "INSERT INTO program_enrollments (id, patient_id, facility_id, program_code, program_name, status, enrolled_by, enrolled_at, activated_at, paused_at, completed_at, withdrawn_at, notes, created_at, version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                rusqlite::params![
                    enrollment.id.to_string(),
                    enrollment.patient_id.to_string(),
                    enrollment.facility_id.to_string(),
                    enrollment.program_code,
                    enrollment.program_name,
                    enrollment.status.to_string(),
                    enrollment.enrolled_by.to_string(),
                    enrollment.enrolled_at.map(|t| t.to_rfc3339()),
                    enrollment.activated_at.map(|t| t.to_rfc3339()),
                    enrollment.paused_at.map(|t| t.to_rfc3339()),
                    enrollment.completed_at.map(|t| t.to_rfc3339()),
                    enrollment.withdrawn_at.map(|t| t.to_rfc3339()),
                    enrollment.notes,
                    enrollment.created_at.to_rfc3339(),
                    enrollment.version,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn find_by_id(&self, id: Uuid) -> AppResult<Option<ProgramEnrollment>> {
        let result = self.conn.query_row(
            "SELECT id, patient_id, facility_id, program_code, program_name, status, enrolled_by, enrolled_at, activated_at, paused_at, completed_at, withdrawn_at, notes, created_at, version
             FROM program_enrollments WHERE id = ?1",
            rusqlite::params![id.to_string()],
            |row| Ok(row_to_enrollment(row)),
        );

        match result {
            Ok(e) => Ok(Some(e.map_err(|e| AppError::Database(e.to_string()))?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(AppError::Database(e.to_string())),
        }
    }

    fn find_by_patient(&self, patient_id: Uuid) -> AppResult<Vec<ProgramEnrollment>> {
        query_enrollments(
            self.conn,
            "SELECT id, patient_id, facility_id, program_code, program_name, status, enrolled_by, enrolled_at, activated_at, paused_at, completed_at, withdrawn_at, notes, created_at, version
             FROM program_enrollments WHERE patient_id = ?1 ORDER BY created_at DESC",
            rusqlite::params![patient_id.to_string()],
        )
    }

    fn find_active_by_facility(&self, facility_id: Uuid) -> AppResult<Vec<ProgramEnrollment>> {
        query_enrollments(
            self.conn,
            "SELECT id, patient_id, facility_id, program_code, program_name, status, enrolled_by, enrolled_at, activated_at, paused_at, completed_at, withdrawn_at, notes, created_at, version
             FROM program_enrollments WHERE facility_id = ?1 AND status NOT IN ('completed', 'withdrawn')
             ORDER BY created_at ASC",
            rusqlite::params![facility_id.to_string()],
        )
    }

    fn update(&self, enrollment: &ProgramEnrollment) -> AppResult<()> {
        let affected = self.conn
            .execute(
                "UPDATE program_enrollments SET status=?1, enrolled_at=?2, activated_at=?3, paused_at=?4, completed_at=?5, withdrawn_at=?6, notes=?7, version=?8
                 WHERE id=?9 AND version=?10",
                rusqlite::params![
                    enrollment.status.to_string(),
                    enrollment.enrolled_at.map(|t| t.to_rfc3339()),
                    enrollment.activated_at.map(|t| t.to_rfc3339()),
                    enrollment.paused_at.map(|t| t.to_rfc3339()),
                    enrollment.completed_at.map(|t| t.to_rfc3339()),
                    enrollment.withdrawn_at.map(|t| t.to_rfc3339()),
                    enrollment.notes,
                    enrollment.version,
                    enrollment.id.to_string(),
                    enrollment.version - 1,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        if affected == 0 {
            return Err(AppError::Conflict {
                message: "program enrollment was modified concurrently".to_string(),
            });
        }
        Ok(())
    }
}

fn query_enrollments(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> AppResult<Vec<ProgramEnrollment>> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| AppError::Database(e.to_string()))?;
    let rows = stmt
        .query_map(params, |row| Ok(row_to_enrollment(row)))
        .map_err(|e| AppError::Database(e.to_string()))?;

    let mut results = Vec::new();
    for row in rows {
        let e = row
            .map_err(|e| AppError::Database(e.to_string()))?
            .map_err(|e| AppError::Database(e.to_string()))?;
        results.push(e);
    }
    Ok(results)
}

fn row_to_enrollment(row: &rusqlite::Row) -> Result<ProgramEnrollment, rusqlite::Error> {
    let id_str: String = row.get(0)?;
    let patient_str: String = row.get(1)?;
    let facility_str: String = row.get(2)?;
    let program_code: String = row.get(3)?;
    let program_name: String = row.get(4)?;
    let status_str: String = row.get(5)?;
    let enrolled_by_str: String = row.get(6)?;
    let enrolled_at_str: Option<String> = row.get(7)?;
    let activated_at_str: Option<String> = row.get(8)?;
    let paused_at_str: Option<String> = row.get(9)?;
    let completed_at_str: Option<String> = row.get(10)?;
    let withdrawn_at_str: Option<String> = row.get(11)?;
    let notes: Option<String> = row.get(12)?;
    let created_str: String = row.get(13)?;
    let version: u32 = row.get(14)?;

    let parse_dt = |s: &str| {
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now())
    };

    Ok(ProgramEnrollment {
        id: Uuid::parse_str(&id_str).unwrap_or_default(),
        patient_id: Uuid::parse_str(&patient_str).unwrap_or_default(),
        facility_id: Uuid::parse_str(&facility_str).unwrap_or_default(),
        program_code,
        program_name,
        status: EnrollmentStatus::from_str_safe(&status_str),
        enrolled_by: Uuid::parse_str(&enrolled_by_str).unwrap_or_default(),
        enrolled_at: enrolled_at_str.as_deref().map(parse_dt),
        activated_at: activated_at_str.as_deref().map(parse_dt),
        paused_at: paused_at_str.as_deref().map(parse_dt),
        completed_at: completed_at_str.as_deref().map(parse_dt),
        withdrawn_at: withdrawn_at_str.as_deref().map(parse_dt),
        notes,
        created_at: parse_dt(&created_str),
        version,
    })
}
