//! Complete an encounter: save visit notes and transition queue to Completed.

use chrono::Utc;
use rustyclinic_clinical::queue::{QueueEntryRepo, QueueTransition};
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_core::state_machine::StateMachine;
use rustyclinic_core::types::ActorContext;
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
use uuid::Uuid;

pub struct CompleteEncounterInput {
    pub encounter_id: Uuid,
    pub queue_entry_id: Uuid,
    pub visit_notes: String,
}

pub fn execute(
    uow: &mut UnitOfWork<'_>,
    queue_repo: &dyn QueueEntryRepo,
    actor: &ActorContext,
    input: CompleteEncounterInput,
) -> AppResult<()> {
    let now = Utc::now();

    // Update encounter
    let affected = uow.conn().execute(
        "UPDATE encounters SET visit_notes=?1, ended_at=?2, status='completed', version=version+1
         WHERE id=?3 AND status='in_progress'",
        rusqlite::params![
            input.visit_notes,
            now.to_rfc3339(),
            input.encounter_id.to_string(),
        ],
    ).map_err(|e| AppError::Database(e.to_string()))?;

    if affected == 0 {
        return Err(AppError::NotFound {
            entity: "Encounter",
            id: input.encounter_id,
        });
    }

    // Transition queue entry to completed
    let mut entry = queue_repo
        .find_by_id(input.queue_entry_id)?
        .ok_or(AppError::NotFound {
            entity: "QueueEntry",
            id: input.queue_entry_id,
        })?;

    entry.apply_transition(QueueTransition::Complete, actor)?;
    queue_repo.update(&entry)?;

    uow.record_audit(
        actor,
        "encounter.completed",
        "Encounter",
        input.encounter_id,
        serde_json::json!({
            "queue_entry_id": input.queue_entry_id,
        }),
    );

    uow.record_outbox(
        actor.facility_id,
        "Encounter",
        input.encounter_id,
        "encounter.completed",
        serde_json::json!({ "encounter_id": input.encounter_id }),
    );

    uow.record_op_log(
        actor,
        "Encounter",
        input.encounter_id,
        serde_json::json!({
            "action": "complete",
            "queue_entry_id": input.queue_entry_id,
        }),
    );

    tracing::info!(encounter_id = %input.encounter_id, "encounter completed");

    Ok(())
}
