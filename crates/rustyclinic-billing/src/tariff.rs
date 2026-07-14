//! Tariff schedule — unit prices for services by facility and payer.

use chrono::NaiveDate;
use rustyclinic_core::error::AppResult;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A claim line item — one service within a claim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimItem {
    pub service_code: String,
    pub service_name: String,
    pub quantity: u32,
    pub unit_price: f64,
    pub total: f64,
    pub approved_amount: Option<f64>,
}

/// A tariff entry — the price for a service at a facility, optionally
/// scoped to a specific payer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tariff {
    pub id: Uuid,
    pub facility_id: Uuid,
    pub service_code: String,
    pub service_name: String,
    pub unit_price: f64,
    pub currency: String,
    pub effective_start: NaiveDate,
    pub effective_end: Option<NaiveDate>,
    pub payer_id: Option<String>,
}

/// Repository trait for tariff persistence.
pub trait TariffRepo {
    fn create(&self, tariff: &Tariff) -> AppResult<()>;
    fn find_by_service_code(
        &self,
        facility_id: Uuid,
        service_code: &str,
        payer_id: Option<&str>,
    ) -> AppResult<Option<Tariff>>;
    fn find_by_facility(&self, facility_id: Uuid) -> AppResult<Vec<Tariff>>;
}
