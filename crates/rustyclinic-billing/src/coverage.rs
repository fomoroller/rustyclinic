//! Coverage and eligibility — patient insurance/payer enrollment.

use chrono::{DateTime, NaiveDate, Utc};
use rustyclinic_core::error::AppResult;
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Status of a coverage record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoverageStatus {
    Active,
    Suspended,
    Cancelled,
    Expired,
}

impl fmt::Display for CoverageStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Suspended => write!(f, "suspended"),
            Self::Cancelled => write!(f, "cancelled"),
            Self::Expired => write!(f, "expired"),
        }
    }
}

/// A patient's coverage under a payer or insurance scheme.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Coverage {
    pub id: Uuid,
    pub patient_id: Uuid,
    pub facility_id: Uuid,
    pub payer_id: String,
    pub payer_name: String,
    pub member_id: String,
    pub plan_name: Option<String>,
    pub effective_start: NaiveDate,
    pub effective_end: Option<NaiveDate>,
    pub status: CoverageStatus,
    pub created_at: DateTime<Utc>,
}

/// Result of an eligibility check against a coverage record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EligibilityCheck {
    pub id: Uuid,
    pub coverage_id: Uuid,
    pub patient_id: Uuid,
    pub facility_id: Uuid,
    pub checked_at: DateTime<Utc>,
    pub is_eligible: bool,
    pub denial_reason: Option<String>,
    pub checked_by: Uuid,
}

/// Repository trait for coverage persistence.
pub trait CoverageRepo {
    fn create(&self, coverage: &Coverage) -> AppResult<()>;
    fn find_by_patient(&self, patient_id: Uuid) -> AppResult<Vec<Coverage>>;
    fn find_active_by_patient(&self, patient_id: Uuid) -> AppResult<Vec<Coverage>>;
    fn find_by_id(&self, id: Uuid) -> AppResult<Option<Coverage>>;
    fn update(&self, coverage: &Coverage) -> AppResult<()>;
}

/// Repository trait for eligibility check persistence.
pub trait EligibilityCheckRepo {
    fn create(&self, check: &EligibilityCheck) -> AppResult<()>;
    fn find_by_coverage(&self, coverage_id: Uuid) -> AppResult<Vec<EligibilityCheck>>;
}
