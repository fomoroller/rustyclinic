//! Add a patient to today's queue.

use chrono::Utc;
use uuid::Uuid;
use rustyclinic_core::error::AppResult;
use rustyclinic_core::types::{new_id, ActorContext};
use rustyclinic_core::state_machine::StateMachine;
use rustyclinic_clinical::queue::{QueueEntry, QueueEntryRepo, QueueStatus, QueueTransition};
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;

/// Input for enqueuing a patient.
pub struct EnqueuePatientInput {
    pub patient_id: Uuid,
    pub service_type: String,
}

/// Add a patient to the queue with full audit trail. Returns the queue entry ID.
pub fn execute(
    uow: &mut UnitOfWork<'_>,
    repo: &dyn QueueEntryRepo,
    actor: &ActorContext,
    input: EnqueuePatientInput,
) -> AppResult<Uuid> {
    let now = Utc::now();
    let entry_id = new_id();
    let position = repo.next_position(actor.facility_id)?;

    let mut entry = QueueEntry {
        id: entry_id,
        facility_id: actor.facility_id,
        patient_id: input.patient_id,
        service_type: input.service_type.clone(),
        status: QueueStatus::Created,
        assigned_to: None,
        position,
        arrived_at: now,
        called_at: None,
        service_started_at: None,
        completed_at: None,
        created_at: now,
        version: 0,
    };

    entry.apply_transition(QueueTransition::Enqueue, actor)?;
    repo.create(&entry)?;

    uow.record_audit(
        actor,
        "queue.enqueued",
        "QueueEntry",
        entry_id,
        serde_json::json!({
            "patient_id": input.patient_id,
            "service_type": input.service_type,
            "position": position,
        }),
    );

    uow.record_outbox(
        actor.facility_id,
        "QueueEntry",
        entry_id,
        "queue.enqueued",
        serde_json::json!({ "entry_id": entry_id, "patient_id": input.patient_id }),
    );

    uow.record_op_log(
        actor,
        "QueueEntry",
        entry_id,
        serde_json::json!({
            "action": "enqueue",
            "patient_id": input.patient_id,
            "service_type": input.service_type,
        }),
    );

    tracing::info!(entry_id = %entry_id, position = position, "patient enqueued");

    Ok(entry_id)
}
