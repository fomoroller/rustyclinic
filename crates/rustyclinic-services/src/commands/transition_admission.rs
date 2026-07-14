//! Transition an admission through its state machine.

use rustyclinic_clinical::admission::{AdmissionRepo, AdmissionTransition};
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_core::state_machine::StateMachine;
use rustyclinic_core::types::ActorContext;
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
use uuid::Uuid;

pub struct TransitionAdmissionInput {
    pub admission_id: Uuid,
    pub transition: AdmissionTransition,
}

pub fn execute(
    uow: &mut UnitOfWork<'_>,
    repo: &dyn AdmissionRepo,
    actor: &ActorContext,
    input: TransitionAdmissionInput,
) -> AppResult<()> {
    let mut admission = repo
        .find_by_id(input.admission_id)?
        .ok_or(AppError::NotFound {
            entity: "Admission",
            id: input.admission_id,
        })?;

    let transition_name = input.transition.to_string();
    admission.apply_transition(input.transition, actor)?;
    repo.update(&admission)?;

    uow.record_audit(
        actor,
        &format!("admission.{transition_name}"),
        "Admission",
        input.admission_id,
        serde_json::json!({
            "transition": transition_name,
            "new_status": admission.status.to_string(),
        }),
    );

    uow.record_outbox(
        actor.facility_id,
        "Admission",
        input.admission_id,
        &format!("admission.{transition_name}"),
        serde_json::json!({
            "admission_id": input.admission_id,
            "new_status": admission.status.to_string(),
        }),
    );

    uow.record_op_log(
        actor,
        "Admission",
        input.admission_id,
        serde_json::json!({
            "action": transition_name,
            "new_status": admission.status.to_string(),
        }),
    );

    tracing::info!(
        admission_id = %input.admission_id,
        transition = %transition_name,
        "admission transitioned"
    );

    Ok(())
}
