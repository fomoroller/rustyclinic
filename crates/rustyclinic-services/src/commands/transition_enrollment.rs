//! Transition a program enrollment through its state machine.

use rustyclinic_clinical::program::{EnrollmentTransition, ProgramEnrollmentRepo};
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_core::state_machine::StateMachine;
use rustyclinic_core::types::ActorContext;
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
use uuid::Uuid;

pub struct TransitionEnrollmentInput {
    pub enrollment_id: Uuid,
    pub transition: EnrollmentTransition,
}

pub fn execute(
    uow: &mut UnitOfWork<'_>,
    repo: &dyn ProgramEnrollmentRepo,
    actor: &ActorContext,
    input: TransitionEnrollmentInput,
) -> AppResult<()> {
    let mut enrollment = repo
        .find_by_id(input.enrollment_id)?
        .ok_or(AppError::NotFound {
            entity: "ProgramEnrollment",
            id: input.enrollment_id,
        })?;

    let transition_name = input.transition.to_string();
    enrollment.apply_transition(input.transition, actor)?;
    repo.update(&enrollment)?;

    uow.record_audit(
        actor,
        &format!("enrollment.{transition_name}"),
        "ProgramEnrollment",
        input.enrollment_id,
        serde_json::json!({
            "transition": transition_name,
            "new_status": enrollment.status.to_string(),
        }),
    );

    uow.record_outbox(
        actor.facility_id,
        "ProgramEnrollment",
        input.enrollment_id,
        &format!("enrollment.{transition_name}"),
        serde_json::json!({
            "enrollment_id": input.enrollment_id,
            "new_status": enrollment.status.to_string(),
        }),
    );

    uow.record_op_log(
        actor,
        "ProgramEnrollment",
        input.enrollment_id,
        serde_json::json!({
            "action": transition_name,
            "new_status": enrollment.status.to_string(),
        }),
    );

    tracing::info!(
        enrollment_id = %input.enrollment_id,
        transition = %transition_name,
        "program enrollment transitioned"
    );

    Ok(())
}
