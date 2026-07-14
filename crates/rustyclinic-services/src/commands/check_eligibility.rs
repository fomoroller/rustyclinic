//! Check patient coverage eligibility.

use chrono::Utc;
use rustyclinic_billing::coverage::{
    CoverageRepo, CoverageStatus, EligibilityCheck, EligibilityCheckRepo,
};
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_core::types::{ActorContext, new_id};
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
use uuid::Uuid;

pub struct CheckEligibilityInput {
    pub coverage_id: Uuid,
}

pub struct CheckEligibilityOutput {
    pub check_id: Uuid,
    pub is_eligible: bool,
    pub denial_reason: Option<String>,
}

pub fn execute(
    uow: &mut UnitOfWork<'_>,
    coverage_repo: &dyn CoverageRepo,
    eligibility_repo: &dyn EligibilityCheckRepo,
    actor: &ActorContext,
    input: CheckEligibilityInput,
) -> AppResult<CheckEligibilityOutput> {
    let coverage = coverage_repo
        .find_by_id(input.coverage_id)?
        .ok_or(AppError::NotFound {
            entity: "Coverage",
            id: input.coverage_id,
        })?;

    let now = Utc::now();
    let today = now.date_naive();
    let check_id = new_id();

    // Determine eligibility based on coverage status and dates
    let (is_eligible, denial_reason) = if coverage.status != CoverageStatus::Active {
        (
            false,
            Some(format!("coverage status is {}", coverage.status)),
        )
    } else if today < coverage.effective_start {
        (false, Some("coverage has not started yet".to_string()))
    } else if coverage.effective_end.is_some_and(|end| today > end) {
        (false, Some("coverage has expired".to_string()))
    } else {
        (true, None)
    };

    let check = EligibilityCheck {
        id: check_id,
        coverage_id: input.coverage_id,
        patient_id: coverage.patient_id,
        facility_id: actor.facility_id,
        checked_at: now,
        is_eligible,
        denial_reason: denial_reason.clone(),
        checked_by: actor.user_id,
    };

    eligibility_repo.create(&check)?;

    uow.record_audit(
        actor,
        "eligibility.checked",
        "EligibilityCheck",
        check_id,
        serde_json::json!({
            "coverage_id": input.coverage_id,
            "patient_id": coverage.patient_id,
            "is_eligible": is_eligible,
        }),
    );

    uow.record_outbox(
        actor.facility_id,
        "EligibilityCheck",
        check_id,
        "eligibility.checked",
        serde_json::json!({
            "check_id": check_id,
            "is_eligible": is_eligible,
        }),
    );

    uow.record_op_log(
        actor,
        "EligibilityCheck",
        check_id,
        serde_json::json!({
            "action": "check",
            "coverage_id": input.coverage_id,
            "is_eligible": is_eligible,
        }),
    );

    tracing::info!(
        check_id = %check_id,
        coverage_id = %input.coverage_id,
        is_eligible = is_eligible,
        "eligibility checked"
    );

    Ok(CheckEligibilityOutput {
        check_id,
        is_eligible,
        denial_reason,
    })
}
