//! Transition a lab order through its state machine.

use rustyclinic_clinical::lab::{LabOrderRepo, LabTransition};
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_core::state_machine::StateMachine;
use rustyclinic_core::types::ActorContext;
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
use uuid::Uuid;

pub struct TransitionLabInput {
    pub order_id: Uuid,
    pub transition: LabTransition,
}

pub fn execute(
    uow: &mut UnitOfWork<'_>,
    repo: &dyn LabOrderRepo,
    actor: &ActorContext,
    input: TransitionLabInput,
) -> AppResult<()> {
    let mut order = repo.find_by_id(input.order_id)?.ok_or(AppError::NotFound {
        entity: "LabOrder",
        id: input.order_id,
    })?;

    let transition_name = input.transition.to_string();
    order.apply_transition(input.transition, actor)?;
    repo.update(&order)?;

    uow.record_audit(
        actor,
        &format!("lab_order.{transition_name}"),
        "LabOrder",
        input.order_id,
        serde_json::json!({
            "transition": transition_name,
            "new_status": order.status.to_string(),
        }),
    );

    uow.record_outbox(
        actor.facility_id,
        "LabOrder",
        input.order_id,
        &format!("lab_order.{transition_name}"),
        serde_json::json!({
            "order_id": input.order_id,
            "new_status": order.status.to_string(),
        }),
    );

    uow.record_op_log(
        actor,
        "LabOrder",
        input.order_id,
        serde_json::json!({
            "action": transition_name,
            "new_status": order.status.to_string(),
        }),
    );

    tracing::info!(
        order_id = %input.order_id,
        transition = %transition_name,
        "lab order transitioned"
    );

    Ok(())
}
