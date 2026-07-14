//! Transition a medication dispense through its state machine.

use rustyclinic_clinical::pharmacy::{DispenseTransition, MedicationDispenseRepo};
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_core::state_machine::StateMachine;
use rustyclinic_core::types::ActorContext;
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
use uuid::Uuid;

pub struct TransitionDispenseInput {
    pub dispense_id: Uuid,
    pub transition: DispenseTransition,
}

pub fn execute(
    uow: &mut UnitOfWork<'_>,
    repo: &dyn MedicationDispenseRepo,
    actor: &ActorContext,
    input: TransitionDispenseInput,
) -> AppResult<()> {
    let mut dispense = repo
        .find_by_id(input.dispense_id)?
        .ok_or(AppError::NotFound {
            entity: "MedicationDispense",
            id: input.dispense_id,
        })?;

    let transition_name = input.transition.to_string();
    dispense.apply_transition(input.transition, actor)?;
    repo.update(&dispense)?;

    uow.record_audit(
        actor,
        &format!("dispense.{transition_name}"),
        "MedicationDispense",
        input.dispense_id,
        serde_json::json!({
            "transition": transition_name,
            "new_status": dispense.status.to_string(),
        }),
    );

    uow.record_outbox(
        actor.facility_id,
        "MedicationDispense",
        input.dispense_id,
        &format!("dispense.{transition_name}"),
        serde_json::json!({
            "dispense_id": input.dispense_id,
            "new_status": dispense.status.to_string(),
        }),
    );

    uow.record_op_log(
        actor,
        "MedicationDispense",
        input.dispense_id,
        serde_json::json!({
            "action": transition_name,
            "new_status": dispense.status.to_string(),
        }),
    );

    tracing::info!(
        dispense_id = %input.dispense_id,
        transition = %transition_name,
        "medication dispense transitioned"
    );

    Ok(())
}
