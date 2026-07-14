//! Create a lab order from an encounter and auto-enqueue patient in lab queue.

use chrono::Utc;
use rustyclinic_clinical::Priority;
use rustyclinic_clinical::lab::{LabOrder, LabOrderRepo, LabStatus, LabTest, LabTestRepo};
use rustyclinic_clinical::queue::{QueueEntry, QueueEntryRepo, QueueStatus, QueueTransition};
use rustyclinic_core::error::AppResult;
use rustyclinic_core::state_machine::StateMachine;
use rustyclinic_core::types::{ActorContext, new_id};
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
use uuid::Uuid;

pub struct CreateLabOrderInput {
    pub encounter_id: Uuid,
    pub patient_id: Uuid,
    pub tests: Vec<LabTest>,
    pub specimen_type: Option<String>,
    pub priority: Priority,
    pub notes: Option<String>,
}

pub struct CreateLabOrderOutput {
    pub order_id: Uuid,
    pub queue_entry_id: Uuid,
}

pub fn execute(
    uow: &mut UnitOfWork<'_>,
    lab_order_repo: &dyn LabOrderRepo,
    queue_repo: &dyn QueueEntryRepo,
    lab_test_repo: &dyn LabTestRepo,
    actor: &ActorContext,
    input: CreateLabOrderInput,
) -> AppResult<CreateLabOrderOutput> {
    let now = Utc::now();
    let order_id = new_id();

    let order = LabOrder {
        id: order_id,
        encounter_id: input.encounter_id,
        patient_id: input.patient_id,
        facility_id: actor.facility_id,
        status: LabStatus::Ordered,
        priority: input.priority,
        ordered_by: actor.user_id,
        tests: input.tests.clone(),
        specimen_type: input.specimen_type,
        collected_at: None,
        collected_by: None,
        resulted_at: None,
        resulted_by: None,
        verified_at: None,
        verified_by: None,
        notes: input.notes,
        created_at: now,
        version: 0,
    };

    lab_order_repo.create(&order)?;

    // Create lab test rows (initially without results)
    lab_test_repo.create_tests(order_id, &input.tests)?;

    // Auto-enqueue patient in lab queue
    let position = queue_repo.next_position(actor.facility_id)?;
    let queue_entry_id = new_id();
    let mut queue_entry = QueueEntry {
        id: queue_entry_id,
        facility_id: actor.facility_id,
        patient_id: input.patient_id,
        service_type: "lab".to_string(),
        department: "lab".to_string(),
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

    uow.record_audit(
        actor,
        "lab_order.created",
        "LabOrder",
        order_id,
        serde_json::json!({
            "encounter_id": input.encounter_id,
            "patient_id": input.patient_id,
            "test_count": input.tests.len(),
        }),
    );

    uow.record_outbox(
        actor.facility_id,
        "LabOrder",
        order_id,
        "lab_order.created",
        serde_json::json!({ "order_id": order_id, "encounter_id": input.encounter_id }),
    );

    uow.record_op_log(
        actor,
        "LabOrder",
        order_id,
        serde_json::json!({
            "action": "create_lab_order",
            "encounter_id": input.encounter_id,
            "patient_id": input.patient_id,
        }),
    );

    tracing::info!(order_id = %order_id, "lab order created");

    Ok(CreateLabOrderOutput {
        order_id,
        queue_entry_id,
    })
}
