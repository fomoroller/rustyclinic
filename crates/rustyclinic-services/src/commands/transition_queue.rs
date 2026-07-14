//! Transition a queue entry through its state machine.

use rustyclinic_clinical::queue::{QueueEntryRepo, QueueTransition};
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_core::state_machine::StateMachine;
use rustyclinic_core::types::ActorContext;
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
use uuid::Uuid;

pub struct TransitionQueueInput {
    pub queue_entry_id: Uuid,
    pub transition: QueueTransition,
    pub assigned_to: Option<Uuid>,
}

pub fn execute(
    uow: &mut UnitOfWork<'_>,
    repo: &dyn QueueEntryRepo,
    actor: &ActorContext,
    input: TransitionQueueInput,
) -> AppResult<()> {
    let mut entry = repo
        .find_by_id(input.queue_entry_id)?
        .ok_or(AppError::NotFound {
            entity: "QueueEntry",
            id: input.queue_entry_id,
        })?;

    let transition_name = input.transition.to_string();
    entry.apply_transition(input.transition, actor)?;

    if let Some(assigned) = input.assigned_to {
        entry.assigned_to = Some(assigned);
    }

    repo.update(&entry)?;

    uow.record_audit(
        actor,
        &format!("queue.{transition_name}"),
        "QueueEntry",
        input.queue_entry_id,
        serde_json::json!({
            "transition": transition_name,
            "new_status": entry.status.to_string(),
        }),
    );

    uow.record_outbox(
        actor.facility_id,
        "QueueEntry",
        input.queue_entry_id,
        &format!("queue.{transition_name}"),
        serde_json::json!({
            "entry_id": input.queue_entry_id,
            "new_status": entry.status.to_string(),
        }),
    );

    uow.record_op_log(
        actor,
        "QueueEntry",
        input.queue_entry_id,
        serde_json::json!({
            "action": transition_name,
            "new_status": entry.status.to_string(),
        }),
    );

    tracing::info!(
        entry_id = %input.queue_entry_id,
        transition = %transition_name,
        "queue entry transitioned"
    );

    Ok(())
}
