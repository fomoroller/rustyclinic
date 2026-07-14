//! Record a payment against an encounter or claim.

use chrono::Utc;
use rustyclinic_billing::payment::{Payment, PaymentMethod, PaymentRepo};
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_core::types::{ActorContext, new_id};
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
use uuid::Uuid;

pub struct RecordPaymentInput {
    pub patient_id: Uuid,
    pub encounter_id: Option<Uuid>,
    pub claim_id: Option<Uuid>,
    pub amount: f64,
    pub currency: String,
    pub method: PaymentMethod,
    pub reference_number: Option<String>,
    pub notes: Option<String>,
}

pub struct RecordPaymentOutput {
    pub payment_id: Uuid,
}

pub fn execute(
    uow: &mut UnitOfWork<'_>,
    repo: &dyn PaymentRepo,
    actor: &ActorContext,
    input: RecordPaymentInput,
) -> AppResult<RecordPaymentOutput> {
    if input.amount <= 0.0 {
        return Err(AppError::Validation {
            message: "payment amount must be positive".to_string(),
        });
    }

    let now = Utc::now();
    let payment_id = new_id();

    let payment = Payment {
        id: payment_id,
        facility_id: actor.facility_id,
        patient_id: input.patient_id,
        encounter_id: input.encounter_id,
        claim_id: input.claim_id,
        amount: input.amount,
        currency: input.currency.clone(),
        method: input.method,
        reference_number: input.reference_number,
        received_by: actor.user_id,
        received_at: now,
        notes: input.notes,
    };

    repo.create(&payment)?;

    uow.record_audit(
        actor,
        "payment.recorded",
        "Payment",
        payment_id,
        serde_json::json!({
            "patient_id": input.patient_id,
            "amount": input.amount,
            "currency": input.currency,
            "method": payment.method.to_string(),
        }),
    );

    uow.record_outbox(
        actor.facility_id,
        "Payment",
        payment_id,
        "payment.recorded",
        serde_json::json!({ "payment_id": payment_id }),
    );

    uow.record_op_log(
        actor,
        "Payment",
        payment_id,
        serde_json::json!({
            "action": "record",
            "patient_id": input.patient_id,
            "amount": input.amount,
        }),
    );

    tracing::info!(payment_id = %payment_id, amount = input.amount, "payment recorded");

    Ok(RecordPaymentOutput { payment_id })
}
