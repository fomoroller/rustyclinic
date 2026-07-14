//! SQLite implementation of PatientRepo.

use rusqlite::Connection;
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_core::types::Sex;
use rustyclinic_identity::{Patient, PatientRepo, PatientSearch};
use uuid::Uuid;

pub struct SqlitePatientRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqlitePatientRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

impl PatientRepo for SqlitePatientRepo<'_> {
    fn create(&self, patient: &Patient) -> AppResult<()> {
        self.conn
            .execute(
                "INSERT INTO patients (id, facility_id, given_name, family_name, sex, date_of_birth, phone, address, national_id, created_at, updated_at, version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                rusqlite::params![
                    patient.id.to_string(),
                    patient.facility_id.to_string(),
                    patient.given_name,
                    patient.family_name,
                    format!("{:?}", patient.sex),
                    patient.date_of_birth.map(|d| d.to_string()),
                    patient.phone,
                    patient.address,
                    patient.national_id,
                    patient.created_at.to_rfc3339(),
                    patient.updated_at.to_rfc3339(),
                    patient.version,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn find_by_id(&self, id: Uuid) -> AppResult<Option<Patient>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, facility_id, given_name, family_name, sex, date_of_birth, phone, address, national_id, created_at, updated_at, version
                 FROM patients WHERE id = ?1",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let result = stmt
            .query_row(rusqlite::params![id.to_string()], |row| {
                Ok(row_to_patient(row))
            })
            .optional()
            .map_err(|e| AppError::Database(e.to_string()))?;

        match result {
            Some(p) => Ok(Some(p.map_err(|e| AppError::Database(e.to_string()))?)),
            None => Ok(None),
        }
    }

    fn search(&self, query: &PatientSearch) -> AppResult<Vec<Patient>> {
        let mut sql = String::from(
            "SELECT id, facility_id, given_name, family_name, sex, date_of_birth, phone, address, national_id, created_at, updated_at, version FROM patients WHERE 1=1",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref name) = query.given_name {
            sql.push_str(&format!(" AND given_name LIKE ?{}", params.len() + 1));
            params.push(Box::new(format!("%{name}%")));
        }
        if let Some(ref name) = query.family_name {
            sql.push_str(&format!(" AND family_name LIKE ?{}", params.len() + 1));
            params.push(Box::new(format!("%{name}%")));
        }
        if let Some(ref nid) = query.national_id {
            sql.push_str(&format!(" AND national_id = ?{}", params.len() + 1));
            params.push(Box::new(nid.clone()));
        }

        let limit = if query.limit == 0 { 50 } else { query.limit };
        sql.push_str(&format!(" LIMIT {limit}"));

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| AppError::Database(e.to_string()))?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let rows = stmt
            .query_map(param_refs.as_slice(), |row| Ok(row_to_patient(row)))
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut patients = Vec::new();
        for row in rows {
            let p = row
                .map_err(|e| AppError::Database(e.to_string()))?
                .map_err(|e| AppError::Database(e.to_string()))?;
            patients.push(p);
        }
        Ok(patients)
    }

    fn update(&self, patient: &Patient) -> AppResult<()> {
        let affected = self
            .conn
            .execute(
                "UPDATE patients SET given_name=?1, family_name=?2, sex=?3, date_of_birth=?4, phone=?5, address=?6, national_id=?7, updated_at=?8, version=?9
                 WHERE id=?10 AND version=?11",
                rusqlite::params![
                    patient.given_name,
                    patient.family_name,
                    format!("{:?}", patient.sex),
                    patient.date_of_birth.map(|d| d.to_string()),
                    patient.phone,
                    patient.address,
                    patient.national_id,
                    patient.updated_at.to_rfc3339(),
                    patient.version,
                    patient.id.to_string(),
                    patient.version - 1,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        if affected == 0 {
            return Err(AppError::Conflict {
                message: "patient was modified by another user".to_string(),
            });
        }
        Ok(())
    }
}

fn row_to_patient(row: &rusqlite::Row) -> Result<Patient, rusqlite::Error> {
    let id_str: String = row.get(0)?;
    let facility_str: String = row.get(1)?;
    let sex_str: String = row.get(4)?;
    let dob_str: Option<String> = row.get(5)?;
    let created_str: String = row.get(9)?;
    let updated_str: String = row.get(10)?;

    Ok(Patient {
        id: Uuid::parse_str(&id_str).unwrap_or_default(),
        facility_id: Uuid::parse_str(&facility_str).unwrap_or_default(),
        given_name: row.get(2)?,
        family_name: row.get(3)?,
        sex: match sex_str.as_str() {
            "Female" => Sex::Female,
            "Male" => Sex::Male,
            "Other" => Sex::Other,
            _ => Sex::Unknown,
        },
        date_of_birth: dob_str.and_then(|s| chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok()),
        phone: row.get(6)?,
        address: row.get(7)?,
        national_id: row.get(8)?,
        created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now()),
        updated_at: chrono::DateTime::parse_from_rfc3339(&updated_str)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now()),
        version: row.get(11)?,
    })
}

/// Extension trait for optional query results.
trait OptionalExt<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalExt<T> for Result<T, rusqlite::Error> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
