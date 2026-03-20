//! Patient identity, matching, and identifier management.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use rustyclinic_core::types::Sex;

/// Facility-level patient record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Patient {
    pub id: Uuid,
    pub facility_id: Uuid,
    pub given_name: String,
    pub family_name: String,
    pub sex: Sex,
    pub date_of_birth: Option<NaiveDate>,
    pub phone: Option<String>,
    pub address: Option<String>,
    pub national_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub version: u32,
}

/// Search criteria for patient lookup.
#[derive(Debug, Default)]
pub struct PatientSearch {
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub national_id: Option<String>,
    pub date_of_birth: Option<NaiveDate>,
    pub phone: Option<String>,
    pub limit: u32,
}

/// Patient identifier from the identifier registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatientIdentifier {
    pub id: Uuid,
    pub patient_id: Uuid,
    pub identifier_type: String,
    pub value: String,
    pub active: bool,
    pub created_at: DateTime<Utc>,
}

/// Repository trait for patient persistence.
pub trait PatientRepo {
    fn create(&self, patient: &Patient) -> rustyclinic_core::error::AppResult<()>;
    fn find_by_id(&self, id: Uuid) -> rustyclinic_core::error::AppResult<Option<Patient>>;
    fn search(&self, query: &PatientSearch) -> rustyclinic_core::error::AppResult<Vec<Patient>>;
    fn update(&self, patient: &Patient) -> rustyclinic_core::error::AppResult<()>;
}
