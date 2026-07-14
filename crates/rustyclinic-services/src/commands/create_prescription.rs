//! Create a medication dispense from an encounter and auto-enqueue patient in pharmacy queue.

use chrono::Utc;
use rustyclinic_clinical::Priority;
use rustyclinic_clinical::pharmacy::{
    DispenseItem, DispenseItemRepo, DispenseStatus, MedicationDispense, MedicationDispenseRepo,
};
use rustyclinic_clinical::queue::{QueueEntry, QueueEntryRepo, QueueStatus, QueueTransition};
use rustyclinic_core::error::AppResult;
use rustyclinic_core::state_machine::StateMachine;
use rustyclinic_core::types::{ActorContext, new_id};
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
use uuid::Uuid;

pub struct CreatePrescriptionInput {
    pub encounter_id: Uuid,
    pub patient_id: Uuid,
    pub items: Vec<DispenseItem>,
    pub priority: Priority,
    pub notes: Option<String>,
}

pub struct CreatePrescriptionOutput {
    pub order_id: Uuid,
    pub queue_entry_id: Uuid,
}

pub fn execute(
    uow: &mut UnitOfWork<'_>,
    dispense_repo: &dyn MedicationDispenseRepo,
    queue_repo: &dyn QueueEntryRepo,
    item_repo: &dyn DispenseItemRepo,
    actor: &ActorContext,
    input: CreatePrescriptionInput,
) -> AppResult<CreatePrescriptionOutput> {
    let now = Utc::now();
    let dispense_id = new_id();

    let dispense = MedicationDispense {
        id: dispense_id,
        encounter_id: input.encounter_id,
        patient_id: input.patient_id,
        facility_id: actor.facility_id,
        status: DispenseStatus::Draft,
        priority: input.priority,
        prescribed_by: actor.user_id,
        dispensed_by: None,
        items: input.items.clone(),
        notes: input.notes,
        created_at: now,
        prepared_at: None,
        dispensed_at: None,
        version: 0,
    };

    dispense_repo.create(&dispense)?;

    // Create dispense item rows
    item_repo.create_items(dispense_id, &input.items)?;

    // Check if patient already has an active pharmacy queue entry for this encounter
    let existing = queue_repo.find_by_encounter(input.encounter_id)?;
    let has_pharmacy = existing.iter().any(|e| {
        e.department == "pharmacy"
            && e.status != QueueStatus::Completed
            && e.status != QueueStatus::Cancelled
    });

    let queue_entry_id = if has_pharmacy {
        existing
            .iter()
            .find(|e| {
                e.department == "pharmacy"
                    && e.status != QueueStatus::Completed
                    && e.status != QueueStatus::Cancelled
            })
            .map(|e| e.id)
            .unwrap_or_else(new_id)
    } else {
        let position = queue_repo.next_position(actor.facility_id)?;
        let entry_id = new_id();
        let mut queue_entry = QueueEntry {
            id: entry_id,
            facility_id: actor.facility_id,
            patient_id: input.patient_id,
            service_type: "pharmacy".to_string(),
            department: "pharmacy".to_string(),
            encounter_id: Some(input.encounter_id),
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
        queue_entry.apply_transition(QueueTransition::Enqueue, actor)?;
        queue_repo.create(&queue_entry)?;
        entry_id
    };

    uow.record_audit(
        actor,
        "dispense.created",
        "MedicationDispense",
        dispense_id,
        serde_json::json!({
            "encounter_id": input.encounter_id,
            "patient_id": input.patient_id,
            "item_count": input.items.len(),
        }),
    );

    uow.record_outbox(
        actor.facility_id,
        "MedicationDispense",
        dispense_id,
        "dispense.created",
        serde_json::json!({ "dispense_id": dispense_id, "encounter_id": input.encounter_id }),
    );

    uow.record_op_log(
        actor,
        "MedicationDispense",
        dispense_id,
        serde_json::json!({
            "action": "create_prescription",
            "encounter_id": input.encounter_id,
            "patient_id": input.patient_id,
        }),
    );

    tracing::info!(dispense_id = %dispense_id, "medication dispense created");

    Ok(CreatePrescriptionOutput {
        order_id: dispense_id,
        queue_entry_id,
    })
}
