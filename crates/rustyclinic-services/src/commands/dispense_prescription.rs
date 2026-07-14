//! Dispense a prescription: prepare, dispense items, and complete the pharmacy queue entry.

use rustyclinic_clinical::pharmacy::{
    DispenseItemRepo, DispenseTransition, MedicationDispenseRepo,
};
use rustyclinic_clinical::queue::{QueueEntryRepo, QueueTransition};
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_core::state_machine::StateMachine;
use rustyclinic_core::types::ActorContext;
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
use uuid::Uuid;

pub struct DispenseItemInput {
    pub medication_name: String,
    pub dispensed_quantity: u32,
    pub substituted: bool,
    pub substitution_reason: Option<String>,
}

pub struct DispensePrescriptionInput {
    pub order_id: Uuid,
    pub queue_entry_id: Uuid,
    pub items: Vec<DispenseItemInput>,
}

pub fn execute(
    uow: &mut UnitOfWork<'_>,
    dispense_repo: &dyn MedicationDispenseRepo,
    queue_repo: &dyn QueueEntryRepo,
    item_repo: &dyn DispenseItemRepo,
    actor: &ActorContext,
    input: DispensePrescriptionInput,
) -> AppResult<()> {
    // Load the medication dispense
    let mut dispense = dispense_repo
        .find_by_id(input.order_id)?
        .ok_or(AppError::NotFound {
            entity: "MedicationDispense",
            id: input.order_id,
        })?;

    // Update dispensed quantities
    for item in &input.items {
        item_repo.update_dispensed(
            input.order_id,
            &item.medication_name,
            item.dispensed_quantity,
            item.substituted,
            item.substitution_reason.as_deref(),
        )?;
    }

    // Transition through prepare → dispense if needed
    let transitions = transitions_to_dispensed(&dispense.status.to_string());
    for transition in transitions {
        dispense.apply_transition(transition, actor)?;
        dispense_repo.update(&dispense)?;
    }

    // Complete the pharmacy queue entry
    let queue_transitions = {
        let queue_entry =
            queue_repo
                .find_by_id(input.queue_entry_id)?
                .ok_or(AppError::NotFound {
                    entity: "QueueEntry",
                    id: input.queue_entry_id,
                })?;
        match queue_entry.status.to_string().as_str() {
            "waiting" => vec![
                QueueTransition::Call,
                QueueTransition::BeginService,
                QueueTransition::Complete,
            ],
            "called" => vec![QueueTransition::BeginService, QueueTransition::Complete],
            "in_service" => vec![QueueTransition::Complete],
            _ => vec![],
        }
    };

    for transition in queue_transitions {
        let mut queue_entry =
            queue_repo
                .find_by_id(input.queue_entry_id)?
                .ok_or(AppError::NotFound {
                    entity: "QueueEntry",
                    id: input.queue_entry_id,
                })?;
        queue_entry.apply_transition(transition, actor)?;
        queue_repo.update(&queue_entry)?;
    }

    uow.record_audit(
        actor,
        "dispense.dispensed",
        "MedicationDispense",
        input.order_id,
        serde_json::json!({
            "item_count": input.items.len(),
        }),
    );

    uow.record_outbox(
        actor.facility_id,
        "MedicationDispense",
        input.order_id,
        "dispense.dispensed",
        serde_json::json!({ "dispense_id": input.order_id }),
    );

    uow.record_op_log(
        actor,
        "MedicationDispense",
        input.order_id,
        serde_json::json!({
            "action": "dispense_prescription",
        }),
    );

    tracing::info!(dispense_id = %input.order_id, "prescription dispensed");

    Ok(())
}

fn transitions_to_dispensed(status: &str) -> Vec<DispenseTransition> {
    match status {
        "draft" => vec![DispenseTransition::Prepare, DispenseTransition::Dispense],
        "prepared" => vec![DispenseTransition::Dispense],
        "partial" => vec![DispenseTransition::Prepare, DispenseTransition::Dispense],
        _ => vec![],
    }
}
