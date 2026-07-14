//! Transition a claim case through its state machine.

use rustyclinic_billing::claims::{ClaimCaseRepo, ClaimTransition};
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_core::state_machine::StateMachine;
use rustyclinic_core::types::ActorContext;
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
use uuid::Uuid;

pub struct TransitionClaimInput {
    pub claim_id: Uuid,
    pub transition: ClaimTransition,
    /// Optional rejection reason (required for Reject transition).
    pub rejection_reason: Option<String>,
    /// Optional approved amount (set during Adjudicate).
    pub approved_amount: Option<f64>,
}

pub fn execute(
    uow: &mut UnitOfWork<'_>,
    repo: &dyn ClaimCaseRepo,
    actor: &ActorContext,
    input: TransitionClaimInput,
) -> AppResult<()> {
    let mut claim = repo.find_by_id(input.claim_id)?.ok_or(AppError::NotFound {
        entity: "ClaimCase",
        id: input.claim_id,
    })?;

    let transition_name = input.transition.to_string();

    if let Some(reason) = &input.rejection_reason {
        claim.rejection_reason = Some(reason.clone());
    }
    if let Some(amount) = input.approved_amount {
        claim.approved_amount = Some(amount);
    }

    claim.apply_transition(input.transition, actor)?;

    repo.update(&claim)?;

    uow.record_audit(
        actor,
        &format!("claim.{transition_name}"),
        "ClaimCase",
        input.claim_id,
        serde_json::json!({
            "transition": transition_name,
            "new_status": claim.status.to_string(),
        }),
    );

    uow.record_outbox(
        actor.facility_id,
        "ClaimCase",
        input.claim_id,
        &format!("claim.{transition_name}"),
        serde_json::json!({
            "claim_id": input.claim_id,
            "new_status": claim.status.to_string(),
        }),
    );

    uow.record_op_log(
        actor,
        "ClaimCase",
        input.claim_id,
        serde_json::json!({
            "action": transition_name,
            "new_status": claim.status.to_string(),
        }),
    );

    tracing::info!(
        claim_id = %input.claim_id,
        transition = %transition_name,
        "claim transitioned"
    );

    Ok(())
}
