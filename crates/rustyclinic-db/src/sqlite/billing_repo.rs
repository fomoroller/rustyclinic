//! SQLite implementations of billing repository traits.

use chrono::NaiveDate;
use rusqlite::Connection;
use rustyclinic_billing::claims::{ClaimCase, ClaimCaseRepo, ClaimStatus};
use rustyclinic_billing::coverage::{
    Coverage, CoverageRepo, CoverageStatus, EligibilityCheck, EligibilityCheckRepo,
};
use rustyclinic_billing::payment::{
    Payment, PaymentMethod, PaymentRepo, Waiver, WaiverReason, WaiverRepo,
};
use rustyclinic_billing::tariff::{ClaimItem, Tariff, TariffRepo};
use rustyclinic_core::error::{AppError, AppResult};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// ClaimCaseRepo
// ---------------------------------------------------------------------------

pub struct SqliteClaimCaseRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteClaimCaseRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

impl ClaimCaseRepo for SqliteClaimCaseRepo<'_> {
    fn create(&self, claim: &ClaimCase) -> AppResult<()> {
        let items_json =
            serde_json::to_string(&claim.items).map_err(|e| AppError::Database(e.to_string()))?;
        self.conn
            .execute(
                "INSERT INTO claim_cases (id, facility_id, patient_id, encounter_id, payer_id, claim_number, status, total_amount, approved_amount, items, submitted_at, adjudicated_at, paid_at, rejection_reason, created_at, version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                rusqlite::params![
                    claim.id.to_string(),
                    claim.facility_id.to_string(),
                    claim.patient_id.to_string(),
                    claim.encounter_id.map(|u| u.to_string()),
                    claim.payer_id,
                    claim.claim_number,
                    claim.status.to_string(),
                    claim.total_amount,
                    claim.approved_amount,
                    items_json,
                    claim.submitted_at.map(|t| t.to_rfc3339()),
                    claim.adjudicated_at.map(|t| t.to_rfc3339()),
                    claim.paid_at.map(|t| t.to_rfc3339()),
                    claim.rejection_reason,
                    claim.created_at.to_rfc3339(),
                    claim.version,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn find_by_id(&self, id: Uuid) -> AppResult<Option<ClaimCase>> {
        let result = self.conn.query_row(
            "SELECT id, facility_id, patient_id, encounter_id, payer_id, claim_number, status, total_amount, approved_amount, items, submitted_at, adjudicated_at, paid_at, rejection_reason, created_at, version
             FROM claim_cases WHERE id = ?1",
            rusqlite::params![id.to_string()],
            |row| Ok(row_to_claim_case(row)),
        );
        match result {
            Ok(claim) => Ok(Some(claim.map_err(|e| AppError::Database(e.to_string()))?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(AppError::Database(e.to_string())),
        }
    }

    fn find_by_patient(&self, patient_id: Uuid) -> AppResult<Vec<ClaimCase>> {
        let mut stmt = self.conn
            .prepare(
                "SELECT id, facility_id, patient_id, encounter_id, payer_id, claim_number, status, total_amount, approved_amount, items, submitted_at, adjudicated_at, paid_at, rejection_reason, created_at, version
                 FROM claim_cases WHERE patient_id = ?1 ORDER BY created_at DESC",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        collect_claims(&mut stmt, rusqlite::params![patient_id.to_string()])
    }

    fn find_by_facility_and_status(
        &self,
        facility_id: Uuid,
        status: &ClaimStatus,
    ) -> AppResult<Vec<ClaimCase>> {
        let mut stmt = self.conn
            .prepare(
                "SELECT id, facility_id, patient_id, encounter_id, payer_id, claim_number, status, total_amount, approved_amount, items, submitted_at, adjudicated_at, paid_at, rejection_reason, created_at, version
                 FROM claim_cases WHERE facility_id = ?1 AND status = ?2 ORDER BY created_at DESC",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        collect_claims(
            &mut stmt,
            rusqlite::params![facility_id.to_string(), status.to_string()],
        )
    }

    fn update(&self, claim: &ClaimCase) -> AppResult<()> {
        let items_json =
            serde_json::to_string(&claim.items).map_err(|e| AppError::Database(e.to_string()))?;
        let affected = self.conn
            .execute(
                "UPDATE claim_cases SET status=?1, total_amount=?2, approved_amount=?3, items=?4, submitted_at=?5, adjudicated_at=?6, paid_at=?7, rejection_reason=?8, claim_number=?9, version=?10
                 WHERE id=?11 AND version=?12",
                rusqlite::params![
                    claim.status.to_string(),
                    claim.total_amount,
                    claim.approved_amount,
                    items_json,
                    claim.submitted_at.map(|t| t.to_rfc3339()),
                    claim.adjudicated_at.map(|t| t.to_rfc3339()),
                    claim.paid_at.map(|t| t.to_rfc3339()),
                    claim.rejection_reason,
                    claim.claim_number,
                    claim.version,
                    claim.id.to_string(),
                    claim.version - 1,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        if affected == 0 {
            return Err(AppError::Conflict {
                message: "claim case was modified concurrently".to_string(),
            });
        }
        Ok(())
    }
}

fn collect_claims(
    stmt: &mut rusqlite::Statement<'_>,
    params: impl rusqlite::Params,
) -> AppResult<Vec<ClaimCase>> {
    let rows = stmt
        .query_map(params, |row| Ok(row_to_claim_case(row)))
        .map_err(|e| AppError::Database(e.to_string()))?;
    let mut result = Vec::new();
    for row in rows {
        let claim = row
            .map_err(|e| AppError::Database(e.to_string()))?
            .map_err(|e| AppError::Database(e.to_string()))?;
        result.push(claim);
    }
    Ok(result)
}

fn row_to_claim_case(row: &rusqlite::Row) -> Result<ClaimCase, rusqlite::Error> {
    let id_str: String = row.get(0)?;
    let facility_str: String = row.get(1)?;
    let patient_str: String = row.get(2)?;
    let encounter_str: Option<String> = row.get(3)?;
    let payer_id: String = row.get(4)?;
    let claim_number: Option<String> = row.get(5)?;
    let status_str: String = row.get(6)?;
    let total_amount: f64 = row.get(7)?;
    let approved_amount: Option<f64> = row.get(8)?;
    let items_json: String = row.get(9)?;
    let submitted_str: Option<String> = row.get(10)?;
    let adjudicated_str: Option<String> = row.get(11)?;
    let paid_str: Option<String> = row.get(12)?;
    let rejection_reason: Option<String> = row.get(13)?;
    let created_str: String = row.get(14)?;
    let version: u32 = row.get(15)?;

    let parse_dt = |s: &str| {
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now())
    };

    let items: Vec<ClaimItem> = serde_json::from_str(&items_json).unwrap_or_default();

    Ok(ClaimCase {
        id: Uuid::parse_str(&id_str).unwrap_or_default(),
        facility_id: Uuid::parse_str(&facility_str).unwrap_or_default(),
        patient_id: Uuid::parse_str(&patient_str).unwrap_or_default(),
        encounter_id: encounter_str.and_then(|s| Uuid::parse_str(&s).ok()),
        payer_id,
        claim_number,
        status: match status_str.as_str() {
            "draft" => ClaimStatus::Draft,
            "validated" => ClaimStatus::Validated,
            "batched" => ClaimStatus::Batched,
            "submitted" => ClaimStatus::Submitted,
            "acknowledged" => ClaimStatus::Acknowledged,
            "adjudicated" => ClaimStatus::Adjudicated,
            "paid" => ClaimStatus::Paid,
            "rejected" => ClaimStatus::Rejected,
            "voided" => ClaimStatus::Voided,
            "reopened" => ClaimStatus::Reopened,
            _ => ClaimStatus::Draft,
        },
        total_amount,
        approved_amount,
        items,
        submitted_at: submitted_str.as_deref().map(parse_dt),
        adjudicated_at: adjudicated_str.as_deref().map(parse_dt),
        paid_at: paid_str.as_deref().map(parse_dt),
        rejection_reason,
        created_at: parse_dt(&created_str),
        version,
    })
}

// ---------------------------------------------------------------------------
// CoverageRepo
// ---------------------------------------------------------------------------

pub struct SqliteCoverageRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteCoverageRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

impl CoverageRepo for SqliteCoverageRepo<'_> {
    fn create(&self, cov: &Coverage) -> AppResult<()> {
        self.conn
            .execute(
                "INSERT INTO coverage (id, patient_id, facility_id, payer_id, payer_name, member_id, plan_name, effective_start, effective_end, status, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                rusqlite::params![
                    cov.id.to_string(),
                    cov.patient_id.to_string(),
                    cov.facility_id.to_string(),
                    cov.payer_id,
                    cov.payer_name,
                    cov.member_id,
                    cov.plan_name,
                    cov.effective_start.to_string(),
                    cov.effective_end.map(|d| d.to_string()),
                    cov.status.to_string(),
                    cov.created_at.to_rfc3339(),
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn find_by_patient(&self, patient_id: Uuid) -> AppResult<Vec<Coverage>> {
        let mut stmt = self.conn
            .prepare(
                "SELECT id, patient_id, facility_id, payer_id, payer_name, member_id, plan_name, effective_start, effective_end, status, created_at
                 FROM coverage WHERE patient_id = ?1 ORDER BY created_at DESC",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        collect_coverage(&mut stmt, rusqlite::params![patient_id.to_string()])
    }

    fn find_active_by_patient(&self, patient_id: Uuid) -> AppResult<Vec<Coverage>> {
        let mut stmt = self.conn
            .prepare(
                "SELECT id, patient_id, facility_id, payer_id, payer_name, member_id, plan_name, effective_start, effective_end, status, created_at
                 FROM coverage WHERE patient_id = ?1 AND status = 'active' ORDER BY created_at DESC",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        collect_coverage(&mut stmt, rusqlite::params![patient_id.to_string()])
    }

    fn find_by_id(&self, id: Uuid) -> AppResult<Option<Coverage>> {
        let result = self.conn.query_row(
            "SELECT id, patient_id, facility_id, payer_id, payer_name, member_id, plan_name, effective_start, effective_end, status, created_at
             FROM coverage WHERE id = ?1",
            rusqlite::params![id.to_string()],
            |row| Ok(row_to_coverage(row)),
        );
        match result {
            Ok(cov) => Ok(Some(cov.map_err(|e| AppError::Database(e.to_string()))?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(AppError::Database(e.to_string())),
        }
    }

    fn update(&self, cov: &Coverage) -> AppResult<()> {
        self.conn
            .execute(
                "UPDATE coverage SET status=?1, effective_end=?2 WHERE id=?3",
                rusqlite::params![
                    cov.status.to_string(),
                    cov.effective_end.map(|d| d.to_string()),
                    cov.id.to_string(),
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }
}

fn collect_coverage(
    stmt: &mut rusqlite::Statement<'_>,
    params: impl rusqlite::Params,
) -> AppResult<Vec<Coverage>> {
    let rows = stmt
        .query_map(params, |row| Ok(row_to_coverage(row)))
        .map_err(|e| AppError::Database(e.to_string()))?;
    let mut result = Vec::new();
    for row in rows {
        let cov = row
            .map_err(|e| AppError::Database(e.to_string()))?
            .map_err(|e| AppError::Database(e.to_string()))?;
        result.push(cov);
    }
    Ok(result)
}

fn row_to_coverage(row: &rusqlite::Row) -> Result<Coverage, rusqlite::Error> {
    let id_str: String = row.get(0)?;
    let patient_str: String = row.get(1)?;
    let facility_str: String = row.get(2)?;
    let payer_id: String = row.get(3)?;
    let payer_name: String = row.get(4)?;
    let member_id: String = row.get(5)?;
    let plan_name: Option<String> = row.get(6)?;
    let start_str: String = row.get(7)?;
    let end_str: Option<String> = row.get(8)?;
    let status_str: String = row.get(9)?;
    let created_str: String = row.get(10)?;

    let parse_dt = |s: &str| {
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now())
    };

    let parse_date = |s: &str| {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap_or_else(|_| chrono::Utc::now().date_naive())
    };

    Ok(Coverage {
        id: Uuid::parse_str(&id_str).unwrap_or_default(),
        patient_id: Uuid::parse_str(&patient_str).unwrap_or_default(),
        facility_id: Uuid::parse_str(&facility_str).unwrap_or_default(),
        payer_id,
        payer_name,
        member_id,
        plan_name,
        effective_start: parse_date(&start_str),
        effective_end: end_str.as_deref().map(parse_date),
        status: match status_str.as_str() {
            "active" => CoverageStatus::Active,
            "suspended" => CoverageStatus::Suspended,
            "cancelled" => CoverageStatus::Cancelled,
            "expired" => CoverageStatus::Expired,
            _ => CoverageStatus::Active,
        },
        created_at: parse_dt(&created_str),
    })
}

// ---------------------------------------------------------------------------
// EligibilityCheckRepo
// ---------------------------------------------------------------------------

pub struct SqliteEligibilityCheckRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteEligibilityCheckRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

impl EligibilityCheckRepo for SqliteEligibilityCheckRepo<'_> {
    fn create(&self, check: &EligibilityCheck) -> AppResult<()> {
        self.conn
            .execute(
                "INSERT INTO eligibility_checks (id, coverage_id, patient_id, facility_id, checked_at, is_eligible, denial_reason, checked_by)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    check.id.to_string(),
                    check.coverage_id.to_string(),
                    check.patient_id.to_string(),
                    check.facility_id.to_string(),
                    check.checked_at.to_rfc3339(),
                    check.is_eligible as i32,
                    check.denial_reason,
                    check.checked_by.to_string(),
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn find_by_coverage(&self, coverage_id: Uuid) -> AppResult<Vec<EligibilityCheck>> {
        let mut stmt = self.conn
            .prepare(
                "SELECT id, coverage_id, patient_id, facility_id, checked_at, is_eligible, denial_reason, checked_by
                 FROM eligibility_checks WHERE coverage_id = ?1 ORDER BY checked_at DESC",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params![coverage_id.to_string()], |row| {
                Ok(row_to_eligibility_check(row))
            })
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut result = Vec::new();
        for row in rows {
            let check = row
                .map_err(|e| AppError::Database(e.to_string()))?
                .map_err(|e| AppError::Database(e.to_string()))?;
            result.push(check);
        }
        Ok(result)
    }
}

fn row_to_eligibility_check(row: &rusqlite::Row) -> Result<EligibilityCheck, rusqlite::Error> {
    let id_str: String = row.get(0)?;
    let coverage_str: String = row.get(1)?;
    let patient_str: String = row.get(2)?;
    let facility_str: String = row.get(3)?;
    let checked_str: String = row.get(4)?;
    let is_eligible: i32 = row.get(5)?;
    let denial_reason: Option<String> = row.get(6)?;
    let checked_by_str: String = row.get(7)?;

    let parse_dt = |s: &str| {
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now())
    };

    Ok(EligibilityCheck {
        id: Uuid::parse_str(&id_str).unwrap_or_default(),
        coverage_id: Uuid::parse_str(&coverage_str).unwrap_or_default(),
        patient_id: Uuid::parse_str(&patient_str).unwrap_or_default(),
        facility_id: Uuid::parse_str(&facility_str).unwrap_or_default(),
        checked_at: parse_dt(&checked_str),
        is_eligible: is_eligible != 0,
        denial_reason,
        checked_by: Uuid::parse_str(&checked_by_str).unwrap_or_default(),
    })
}

// ---------------------------------------------------------------------------
// TariffRepo
// ---------------------------------------------------------------------------

pub struct SqliteTariffRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteTariffRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

impl TariffRepo for SqliteTariffRepo<'_> {
    fn create(&self, tariff: &Tariff) -> AppResult<()> {
        self.conn
            .execute(
                "INSERT INTO tariffs (id, facility_id, service_code, service_name, unit_price, currency, effective_start, effective_end, payer_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    tariff.id.to_string(),
                    tariff.facility_id.to_string(),
                    tariff.service_code,
                    tariff.service_name,
                    tariff.unit_price,
                    tariff.currency,
                    tariff.effective_start.to_string(),
                    tariff.effective_end.map(|d| d.to_string()),
                    tariff.payer_id,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn find_by_service_code(
        &self,
        facility_id: Uuid,
        service_code: &str,
        payer_id: Option<&str>,
    ) -> AppResult<Option<Tariff>> {
        // Try payer-specific tariff first, then fall back to facility default
        let result = if let Some(pid) = payer_id {
            self.conn.query_row(
                "SELECT id, facility_id, service_code, service_name, unit_price, currency, effective_start, effective_end, payer_id
                 FROM tariffs
                 WHERE facility_id = ?1 AND service_code = ?2 AND payer_id = ?3
                   AND date(effective_start) <= date('now')
                   AND (effective_end IS NULL OR date(effective_end) >= date('now'))
                 ORDER BY effective_start DESC LIMIT 1",
                rusqlite::params![facility_id.to_string(), service_code, pid],
                |row| Ok(row_to_tariff(row)),
            )
        } else {
            Err(rusqlite::Error::QueryReturnedNoRows)
        };

        match result {
            Ok(t) => Ok(Some(t.map_err(|e| AppError::Database(e.to_string()))?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                // Fall back to facility default (no payer_id)
                let fallback = self.conn.query_row(
                    "SELECT id, facility_id, service_code, service_name, unit_price, currency, effective_start, effective_end, payer_id
                     FROM tariffs
                     WHERE facility_id = ?1 AND service_code = ?2 AND payer_id IS NULL
                       AND date(effective_start) <= date('now')
                       AND (effective_end IS NULL OR date(effective_end) >= date('now'))
                     ORDER BY effective_start DESC LIMIT 1",
                    rusqlite::params![facility_id.to_string(), service_code],
                    |row| Ok(row_to_tariff(row)),
                );
                match fallback {
                    Ok(t) => Ok(Some(t.map_err(|e| AppError::Database(e.to_string()))?)),
                    Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                    Err(e) => Err(AppError::Database(e.to_string())),
                }
            }
            Err(e) => Err(AppError::Database(e.to_string())),
        }
    }

    fn find_by_facility(&self, facility_id: Uuid) -> AppResult<Vec<Tariff>> {
        let mut stmt = self.conn
            .prepare(
                "SELECT id, facility_id, service_code, service_name, unit_price, currency, effective_start, effective_end, payer_id
                 FROM tariffs WHERE facility_id = ?1 ORDER BY service_code",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params![facility_id.to_string()], |row| {
                Ok(row_to_tariff(row))
            })
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut result = Vec::new();
        for row in rows {
            let tariff = row
                .map_err(|e| AppError::Database(e.to_string()))?
                .map_err(|e| AppError::Database(e.to_string()))?;
            result.push(tariff);
        }
        Ok(result)
    }
}

fn row_to_tariff(row: &rusqlite::Row) -> Result<Tariff, rusqlite::Error> {
    let id_str: String = row.get(0)?;
    let facility_str: String = row.get(1)?;
    let service_code: String = row.get(2)?;
    let service_name: String = row.get(3)?;
    let unit_price: f64 = row.get(4)?;
    let currency: String = row.get(5)?;
    let start_str: String = row.get(6)?;
    let end_str: Option<String> = row.get(7)?;
    let payer_id: Option<String> = row.get(8)?;

    let parse_date = |s: &str| {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap_or_else(|_| chrono::Utc::now().date_naive())
    };

    Ok(Tariff {
        id: Uuid::parse_str(&id_str).unwrap_or_default(),
        facility_id: Uuid::parse_str(&facility_str).unwrap_or_default(),
        service_code,
        service_name,
        unit_price,
        currency,
        effective_start: parse_date(&start_str),
        effective_end: end_str.as_deref().map(parse_date),
        payer_id,
    })
}

// ---------------------------------------------------------------------------
// PaymentRepo
// ---------------------------------------------------------------------------

pub struct SqlitePaymentRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqlitePaymentRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

impl PaymentRepo for SqlitePaymentRepo<'_> {
    fn create(&self, payment: &Payment) -> AppResult<()> {
        self.conn
            .execute(
                "INSERT INTO payments (id, facility_id, patient_id, encounter_id, claim_id, amount, currency, method, reference_number, received_by, received_at, notes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                rusqlite::params![
                    payment.id.to_string(),
                    payment.facility_id.to_string(),
                    payment.patient_id.to_string(),
                    payment.encounter_id.map(|u| u.to_string()),
                    payment.claim_id.map(|u| u.to_string()),
                    payment.amount,
                    payment.currency,
                    payment.method.to_string(),
                    payment.reference_number,
                    payment.received_by.to_string(),
                    payment.received_at.to_rfc3339(),
                    payment.notes,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn find_by_encounter(&self, encounter_id: Uuid) -> AppResult<Vec<Payment>> {
        let mut stmt = self.conn
            .prepare(
                "SELECT id, facility_id, patient_id, encounter_id, claim_id, amount, currency, method, reference_number, received_by, received_at, notes
                 FROM payments WHERE encounter_id = ?1 ORDER BY received_at DESC",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        collect_payments(&mut stmt, rusqlite::params![encounter_id.to_string()])
    }

    fn find_by_patient(&self, patient_id: Uuid) -> AppResult<Vec<Payment>> {
        let mut stmt = self.conn
            .prepare(
                "SELECT id, facility_id, patient_id, encounter_id, claim_id, amount, currency, method, reference_number, received_by, received_at, notes
                 FROM payments WHERE patient_id = ?1 ORDER BY received_at DESC",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        collect_payments(&mut stmt, rusqlite::params![patient_id.to_string()])
    }
}

fn collect_payments(
    stmt: &mut rusqlite::Statement<'_>,
    params: impl rusqlite::Params,
) -> AppResult<Vec<Payment>> {
    let rows = stmt
        .query_map(params, |row| Ok(row_to_payment(row)))
        .map_err(|e| AppError::Database(e.to_string()))?;
    let mut result = Vec::new();
    for row in rows {
        let p = row
            .map_err(|e| AppError::Database(e.to_string()))?
            .map_err(|e| AppError::Database(e.to_string()))?;
        result.push(p);
    }
    Ok(result)
}

fn row_to_payment(row: &rusqlite::Row) -> Result<Payment, rusqlite::Error> {
    let id_str: String = row.get(0)?;
    let facility_str: String = row.get(1)?;
    let patient_str: String = row.get(2)?;
    let encounter_str: Option<String> = row.get(3)?;
    let claim_str: Option<String> = row.get(4)?;
    let amount: f64 = row.get(5)?;
    let currency: String = row.get(6)?;
    let method_str: String = row.get(7)?;
    let reference_number: Option<String> = row.get(8)?;
    let received_by_str: String = row.get(9)?;
    let received_str: String = row.get(10)?;
    let notes: Option<String> = row.get(11)?;

    let parse_dt = |s: &str| {
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now())
    };

    Ok(Payment {
        id: Uuid::parse_str(&id_str).unwrap_or_default(),
        facility_id: Uuid::parse_str(&facility_str).unwrap_or_default(),
        patient_id: Uuid::parse_str(&patient_str).unwrap_or_default(),
        encounter_id: encounter_str.and_then(|s| Uuid::parse_str(&s).ok()),
        claim_id: claim_str.and_then(|s| Uuid::parse_str(&s).ok()),
        amount,
        currency,
        method: match method_str.as_str() {
            "cash" => PaymentMethod::Cash,
            "mobile_money" => PaymentMethod::MobileMoney,
            "insurance" => PaymentMethod::Insurance,
            "bank_transfer" => PaymentMethod::BankTransfer,
            "waiver" => PaymentMethod::Waiver,
            _ => PaymentMethod::Cash,
        },
        reference_number,
        received_by: Uuid::parse_str(&received_by_str).unwrap_or_default(),
        received_at: parse_dt(&received_str),
        notes,
    })
}

// ---------------------------------------------------------------------------
// WaiverRepo
// ---------------------------------------------------------------------------

pub struct SqliteWaiverRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteWaiverRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

impl WaiverRepo for SqliteWaiverRepo<'_> {
    fn create(&self, waiver: &Waiver) -> AppResult<()> {
        self.conn
            .execute(
                "INSERT INTO waivers (id, patient_id, facility_id, encounter_id, amount_waived, reason, approved_by, approved_at, notes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    waiver.id.to_string(),
                    waiver.patient_id.to_string(),
                    waiver.facility_id.to_string(),
                    waiver.encounter_id.map(|u| u.to_string()),
                    waiver.amount_waived,
                    waiver.reason.to_string(),
                    waiver.approved_by.to_string(),
                    waiver.approved_at.to_rfc3339(),
                    waiver.notes,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn find_by_patient(&self, patient_id: Uuid) -> AppResult<Vec<Waiver>> {
        let mut stmt = self.conn
            .prepare(
                "SELECT id, patient_id, facility_id, encounter_id, amount_waived, reason, approved_by, approved_at, notes
                 FROM waivers WHERE patient_id = ?1 ORDER BY approved_at DESC",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params![patient_id.to_string()], |row| {
                Ok(row_to_waiver(row))
            })
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut result = Vec::new();
        for row in rows {
            let w = row
                .map_err(|e| AppError::Database(e.to_string()))?
                .map_err(|e| AppError::Database(e.to_string()))?;
            result.push(w);
        }
        Ok(result)
    }
}

fn row_to_waiver(row: &rusqlite::Row) -> Result<Waiver, rusqlite::Error> {
    let id_str: String = row.get(0)?;
    let patient_str: String = row.get(1)?;
    let facility_str: String = row.get(2)?;
    let encounter_str: Option<String> = row.get(3)?;
    let amount_waived: f64 = row.get(4)?;
    let reason_str: String = row.get(5)?;
    let approved_by_str: String = row.get(6)?;
    let approved_str: String = row.get(7)?;
    let notes: Option<String> = row.get(8)?;

    let parse_dt = |s: &str| {
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now())
    };

    Ok(Waiver {
        id: Uuid::parse_str(&id_str).unwrap_or_default(),
        patient_id: Uuid::parse_str(&patient_str).unwrap_or_default(),
        facility_id: Uuid::parse_str(&facility_str).unwrap_or_default(),
        encounter_id: encounter_str.and_then(|s| Uuid::parse_str(&s).ok()),
        amount_waived,
        reason: match reason_str.as_str() {
            "indigent" => WaiverReason::Indigent,
            "emergency" => WaiverReason::Emergency,
            "government_program" => WaiverReason::GovernmentProgram,
            "staff" => WaiverReason::Staff,
            "minor" => WaiverReason::Minor,
            "other" => WaiverReason::Other,
            _ => WaiverReason::Other,
        },
        approved_by: Uuid::parse_str(&approved_by_str).unwrap_or_default(),
        approved_at: parse_dt(&approved_str),
        notes,
    })
}
