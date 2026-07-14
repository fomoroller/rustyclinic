//! Transition a referral through its state machine.

use rustyclinic_clinical::referral::{ReferralRepo, ReferralTransition};
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_core::state_machine::StateMachine;
use rustyclinic_core::types::ActorContext;
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
use uuid::Uuid;

pub struct TransitionReferralInput {
    pub referral_id: Uuid,
    pub transition: ReferralTransition,
}

pub fn execute(
    uow: &mut UnitOfWork<'_>,
    repo: &dyn ReferralRepo,
    actor: &ActorContext,
    input: TransitionReferralInput,
) -> AppResult<()> {
    let mut referral = repo
        .find_by_id(input.referral_id)?
        .ok_or(AppError::NotFound {
            entity: "Referral",
            id: input.referral_id,
        })?;

    let transition_name = input.transition.to_string();
    referral.apply_transition(input.transition, actor)?;
    repo.update(&referral)?;

    uow.record_audit(
        actor,
        &format!("referral.{transition_name}"),
        "Referral",
        input.referral_id,
        serde_json::json!({
            "transition": transition_name,
            "new_status": referral.status.to_string(),
        }),
    );

    uow.record_outbox(
        actor.facility_id,
        "Referral",
        input.referral_id,
        &format!("referral.{transition_name}"),
        serde_json::json!({
            "referral_id": input.referral_id,
            "new_status": referral.status.to_string(),
        }),
    );

    uow.record_op_log(
        actor,
        "Referral",
        input.referral_id,
        serde_json::json!({
            "action": transition_name,
            "new_status": referral.status.to_string(),
        }),
    );

    tracing::info!(
        referral_id = %input.referral_id,
        transition = %transition_name,
        "referral transitioned"
    );

    Ok(())
}
