//! Grant a fee waiver for a patient.

use chrono::Utc;
use rustyclinic_billing::payment::{Waiver, WaiverReason, WaiverRepo};
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_core::types::{ActorContext, new_id};
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
use uuid::Uuid;

pub struct GrantWaiverInput {
    pub patient_id: Uuid,
    pub encounter_id: Option<Uuid>,
    pub amount_waived: f64,
    pub reason: WaiverReason,
    pub notes: Option<String>,
}

pub struct GrantWaiverOutput {
    pub waiver_id: Uuid,
}

pub fn execute(
    uow: &mut UnitOfWork<'_>,
    repo: &dyn WaiverRepo,
    actor: &ActorContext,
    input: GrantWaiverInput,
) -> AppResult<GrantWaiverOutput> {
    if input.amount_waived <= 0.0 {
        return Err(AppError::Validation {
            message: "waiver amount must be positive".to_string(),
        });
    }

    let now = Utc::now();
    let waiver_id = new_id();

    let waiver = Waiver {
        id: waiver_id,
        patient_id: input.patient_id,
        facility_id: actor.facility_id,
        encounter_id: input.encounter_id,
        amount_waived: input.amount_waived,
        reason: input.reason,
        approved_by: actor.user_id,
        approved_at: now,
        notes: input.notes,
    };

    repo.create(&waiver)?;

    uow.record_audit(
        actor,
        "waiver.granted",
        "Waiver",
        waiver_id,
        serde_json::json!({
            "patient_id": input.patient_id,
            "amount_waived": input.amount_waived,
            "reason": waiver.reason.to_string(),
        }),
    );

    uow.record_outbox(
        actor.facility_id,
        "Waiver",
        waiver_id,
        "waiver.granted",
        serde_json::json!({ "waiver_id": waiver_id }),
    );

    uow.record_op_log(
        actor,
        "Waiver",
        waiver_id,
        serde_json::json!({
            "action": "grant",
            "patient_id": input.patient_id,
            "amount_waived": input.amount_waived,
        }),
    );

    tracing::info!(waiver_id = %waiver_id, amount = input.amount_waived, "waiver granted");

    Ok(GrantWaiverOutput { waiver_id })
}
