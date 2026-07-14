//! PostgreSQL implementation of PatientRepo.

use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_core::types::Sex;
use rustyclinic_identity::{Patient, PatientRepo, PatientSearch};
use tokio_postgres::Client;
use uuid::Uuid;

pub struct PgPatientRepo<'a> {
    client: &'a Client,
}

impl<'a> PgPatientRepo<'a> {
    pub fn new(client: &'a Client) -> Self {
        Self { client }
    }

    fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        // We rely on the caller having a tokio runtime active.
        // Use block_in_place + block_on to bridge async PG calls to sync trait methods.
        tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f))
    }
}

impl PatientRepo for PgPatientRepo<'_> {
    fn create(&self, patient: &Patient) -> AppResult<()> {
        self.block_on(async {
            self.client
                .execute(
                    "INSERT INTO patients (id, facility_id, given_name, family_name, sex, date_of_birth, phone, address, national_id, created_at, updated_at, version)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
                    &[
                        &patient.id,
                        &patient.facility_id,
                        &patient.given_name,
                        &patient.family_name,
                        &format!("{:?}", patient.sex),
                        &patient.date_of_birth,
                        &patient.phone,
                        &patient.address,
                        &patient.national_id,
                        &patient.created_at,
                        &patient.updated_at,
                        &(patient.version as i32),
                    ],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;
            Ok(())
        })
    }

    fn find_by_id(&self, id: Uuid) -> AppResult<Option<Patient>> {
        self.block_on(async {
            let row = self.client
                .query_opt(
                    "SELECT id, facility_id, given_name, family_name, sex, date_of_birth, phone, address, national_id, created_at, updated_at, version
                     FROM patients WHERE id = $1",
                    &[&id],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            match row {
                Some(row) => Ok(Some(row_to_patient(&row)?)),
                None => Ok(None),
            }
        })
    }

    fn search(&self, query: &PatientSearch) -> AppResult<Vec<Patient>> {
        self.block_on(async {
            let mut sql = String::from(
                "SELECT id, facility_id, given_name, family_name, sex, date_of_birth, phone, address, national_id, created_at, updated_at, version FROM patients WHERE true",
            );
            let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync>> = Vec::new();

            if let Some(ref name) = query.given_name {
                params.push(Box::new(format!("%{name}%")));
                sql.push_str(&format!(" AND given_name LIKE ${}", params.len()));
            }
            if let Some(ref name) = query.family_name {
                params.push(Box::new(format!("%{name}%")));
                sql.push_str(&format!(" AND family_name LIKE ${}", params.len()));
            }
            if let Some(ref nid) = query.national_id {
                params.push(Box::new(nid.clone()));
                sql.push_str(&format!(" AND national_id = ${}", params.len()));
            }

            let limit = if query.limit == 0 { 50 } else { query.limit };
            sql.push_str(&format!(" LIMIT {limit}"));

            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                params.iter().map(|p| p.as_ref()).collect();

            let rows = self.client
                .query(&sql, &param_refs)
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            let mut patients = Vec::new();
            for row in &rows {
                patients.push(row_to_patient(row)?);
            }
            Ok(patients)
        })
    }

    fn update(&self, patient: &Patient) -> AppResult<()> {
        self.block_on(async {
            let affected = self.client
                .execute(
                    "UPDATE patients SET given_name=$1, family_name=$2, sex=$3, date_of_birth=$4, phone=$5, address=$6, national_id=$7, updated_at=$8, version=$9
                     WHERE id=$10 AND version=$11",
                    &[
                        &patient.given_name,
                        &patient.family_name,
                        &format!("{:?}", patient.sex),
                        &patient.date_of_birth,
                        &patient.phone,
                        &patient.address,
                        &patient.national_id,
                        &patient.updated_at,
                        &(patient.version as i32),
                        &patient.id,
                        &((patient.version - 1) as i32),
                    ],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            if affected == 0 {
                return Err(AppError::Conflict {
                    message: "patient was modified by another user".to_string(),
                });
            }
            Ok(())
        })
    }
}

fn row_to_patient(row: &tokio_postgres::Row) -> AppResult<Patient> {
    let sex_str: String = row
        .try_get(4)
        .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(Patient {
        id: row
            .try_get(0)
            .map_err(|e| AppError::Database(e.to_string()))?,
        facility_id: row
            .try_get(1)
            .map_err(|e| AppError::Database(e.to_string()))?,
        given_name: row
            .try_get(2)
            .map_err(|e| AppError::Database(e.to_string()))?,
        family_name: row
            .try_get(3)
            .map_err(|e| AppError::Database(e.to_string()))?,
        sex: match sex_str.as_str() {
            "Female" => Sex::Female,
            "Male" => Sex::Male,
            "Other" => Sex::Other,
            _ => Sex::Unknown,
        },
        date_of_birth: row
            .try_get(5)
            .map_err(|e| AppError::Database(e.to_string()))?,
        phone: row
            .try_get(6)
            .map_err(|e| AppError::Database(e.to_string()))?,
        address: row
            .try_get(7)
            .map_err(|e| AppError::Database(e.to_string()))?,
        national_id: row
            .try_get(8)
            .map_err(|e| AppError::Database(e.to_string()))?,
        created_at: row
            .try_get(9)
            .map_err(|e| AppError::Database(e.to_string()))?,
        updated_at: row
            .try_get(10)
            .map_err(|e| AppError::Database(e.to_string()))?,
        version: row
            .try_get::<_, i32>(11)
            .map_err(|e| AppError::Database(e.to_string()))? as u32,
    })
}
