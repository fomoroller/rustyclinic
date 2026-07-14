//! Payments and waivers — recording money received and fee waivers.

use chrono::{DateTime, Utc};
use rustyclinic_core::error::AppResult;
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Payment method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaymentMethod {
    Cash,
    MobileMoney,
    Insurance,
    BankTransfer,
    Waiver,
}

impl fmt::Display for PaymentMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cash => write!(f, "cash"),
            Self::MobileMoney => write!(f, "mobile_money"),
            Self::Insurance => write!(f, "insurance"),
            Self::BankTransfer => write!(f, "bank_transfer"),
            Self::Waiver => write!(f, "waiver"),
        }
    }
}

/// A payment received against an encounter or claim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payment {
    pub id: Uuid,
    pub facility_id: Uuid,
    pub patient_id: Uuid,
    pub encounter_id: Option<Uuid>,
    pub claim_id: Option<Uuid>,
    pub amount: f64,
    pub currency: String,
    pub method: PaymentMethod,
    pub reference_number: Option<String>,
    pub received_by: Uuid,
    pub received_at: DateTime<Utc>,
    pub notes: Option<String>,
}

/// Reason for waiving fees.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WaiverReason {
    Indigent,
    Emergency,
    GovernmentProgram,
    Staff,
    Minor,
    Other,
}

impl fmt::Display for WaiverReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Indigent => write!(f, "indigent"),
            Self::Emergency => write!(f, "emergency"),
            Self::GovernmentProgram => write!(f, "government_program"),
            Self::Staff => write!(f, "staff"),
            Self::Minor => write!(f, "minor"),
            Self::Other => write!(f, "other"),
        }
    }
}

/// A fee waiver — partial or full forgiveness of patient charges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Waiver {
    pub id: Uuid,
    pub patient_id: Uuid,
    pub facility_id: Uuid,
    pub encounter_id: Option<Uuid>,
    pub amount_waived: f64,
    pub reason: WaiverReason,
    pub approved_by: Uuid,
    pub approved_at: DateTime<Utc>,
    pub notes: Option<String>,
}

/// Repository trait for payment persistence.
pub trait PaymentRepo {
    fn create(&self, payment: &Payment) -> AppResult<()>;
    fn find_by_encounter(&self, encounter_id: Uuid) -> AppResult<Vec<Payment>>;
    fn find_by_patient(&self, patient_id: Uuid) -> AppResult<Vec<Payment>>;
}

/// Repository trait for waiver persistence.
pub trait WaiverRepo {
    fn create(&self, waiver: &Waiver) -> AppResult<()>;
    fn find_by_patient(&self, patient_id: Uuid) -> AppResult<Vec<Waiver>>;
}
