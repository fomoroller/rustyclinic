//! Complete a lab order: enter results, verify, and complete the lab queue entry.

use rustyclinic_clinical::lab::{LabOrderRepo, LabTest, LabTestRepo, LabTransition};
use rustyclinic_clinical::queue::{QueueEntryRepo, QueueTransition};
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_core::state_machine::StateMachine;
use rustyclinic_core::types::ActorContext;
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
use uuid::Uuid;

pub struct CompleteLabOrderInput {
    pub order_id: Uuid,
    pub queue_entry_id: Uuid,
    pub results: Vec<LabTest>,
}

pub fn execute(
    uow: &mut UnitOfWork<'_>,
    lab_order_repo: &dyn LabOrderRepo,
    queue_repo: &dyn QueueEntryRepo,
    lab_test_repo: &dyn LabTestRepo,
    actor: &ActorContext,
    input: CompleteLabOrderInput,
) -> AppResult<()> {
    // Load the lab order
    let mut order = lab_order_repo
        .find_by_id(input.order_id)?
        .ok_or(AppError::NotFound {
            entity: "LabOrder",
            id: input.order_id,
        })?;

    // Update lab test results
    for test in &input.results {
        lab_test_repo.update_test(input.order_id, test)?;
    }

    // Walk the lab order through remaining transitions to get to Verified.
    let transitions_needed = transitions_to_verified(&order.status.to_string());
    for transition in transitions_needed {
        order.apply_transition(transition, actor)?;
        lab_order_repo.update(&order)?;
    }

    // Complete the lab queue entry — transition step by step with intermediate saves
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
        "lab_order.completed",
        "LabOrder",
        input.order_id,
        serde_json::json!({
            "result_count": input.results.len(),
        }),
    );

    uow.record_outbox(
        actor.facility_id,
        "LabOrder",
        input.order_id,
        "lab_order.completed",
        serde_json::json!({ "order_id": input.order_id }),
    );

    uow.record_op_log(
        actor,
        "LabOrder",
        input.order_id,
        serde_json::json!({
            "action": "complete_lab_order",
        }),
    );

    tracing::info!(order_id = %input.order_id, "lab order completed");

    Ok(())
}

/// Determine the transitions needed to get from a given status to Verified.
fn transitions_to_verified(status: &str) -> Vec<LabTransition> {
    match status {
        "ordered" => vec![
            LabTransition::RequestSample,
            LabTransition::CollectSample,
            LabTransition::ReceiveAtLab,
            LabTransition::BeginProcessing,
            LabTransition::EnterResults,
            LabTransition::Verify,
        ],
        "sample_pending" => vec![
            LabTransition::CollectSample,
            LabTransition::ReceiveAtLab,
            LabTransition::BeginProcessing,
            LabTransition::EnterResults,
            LabTransition::Verify,
        ],
        "collected" => vec![
            LabTransition::ReceiveAtLab,
            LabTransition::BeginProcessing,
            LabTransition::EnterResults,
            LabTransition::Verify,
        ],
        "received" => vec![
            LabTransition::BeginProcessing,
            LabTransition::EnterResults,
            LabTransition::Verify,
        ],
        "in_process" => vec![LabTransition::EnterResults, LabTransition::Verify],
        "resulted" => vec![LabTransition::Verify],
        _ => vec![],
    }
}
