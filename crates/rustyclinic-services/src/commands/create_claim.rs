//! Create a claim case from an encounter with tariff lookup.

use chrono::Utc;
use rustyclinic_billing::claims::{ClaimCase, ClaimCaseRepo, ClaimStatus};
use rustyclinic_billing::tariff::{ClaimItem, TariffRepo};
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_core::types::{ActorContext, new_id};
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
use uuid::Uuid;

pub struct CreateClaimInput {
    pub patient_id: Uuid,
    pub encounter_id: Option<Uuid>,
    pub payer_id: String,
    /// Service codes to include in the claim; quantities looked up from tariffs.
    pub service_codes: Vec<ServiceCodeEntry>,
}

pub struct ServiceCodeEntry {
    pub service_code: String,
    pub quantity: u32,
}

pub struct CreateClaimOutput {
    pub claim_id: Uuid,
    pub total_amount: f64,
}

pub fn execute(
    uow: &mut UnitOfWork<'_>,
    claim_repo: &dyn ClaimCaseRepo,
    tariff_repo: &dyn TariffRepo,
    actor: &ActorContext,
    input: CreateClaimInput,
) -> AppResult<CreateClaimOutput> {
    if input.service_codes.is_empty() {
        return Err(AppError::Validation {
            message: "claim must have at least one service code".to_string(),
        });
    }

    let now = Utc::now();
    let claim_id = new_id();

    // Look up tariffs and build claim items
    let mut items = Vec::new();
    let mut total_amount = 0.0;

    for entry in &input.service_codes {
        let tariff = tariff_repo
            .find_by_service_code(
                actor.facility_id,
                &entry.service_code,
                Some(&input.payer_id),
            )?
            .ok_or_else(|| AppError::Validation {
                message: format!("no tariff found for service code: {}", entry.service_code),
            })?;

        let line_total = tariff.unit_price * f64::from(entry.quantity);
        items.push(ClaimItem {
            service_code: tariff.service_code.clone(),
            service_name: tariff.service_name.clone(),
            quantity: entry.quantity,
            unit_price: tariff.unit_price,
            total: line_total,
            approved_amount: None,
        });
        total_amount += line_total;
    }

    let claim = ClaimCase {
        id: claim_id,
        facility_id: actor.facility_id,
        patient_id: input.patient_id,
        encounter_id: input.encounter_id,
        payer_id: input.payer_id.clone(),
        claim_number: None,
        status: ClaimStatus::Draft,
        total_amount,
        approved_amount: None,
        items,
        submitted_at: None,
        adjudicated_at: None,
        paid_at: None,
        rejection_reason: None,
        created_at: now,
        version: 0,
    };

    claim_repo.create(&claim)?;

    uow.record_audit(
        actor,
        "claim.created",
        "ClaimCase",
        claim_id,
        serde_json::json!({
            "patient_id": input.patient_id,
            "payer_id": input.payer_id,
            "total_amount": total_amount,
            "item_count": claim.items.len(),
        }),
    );

    uow.record_outbox(
        actor.facility_id,
        "ClaimCase",
        claim_id,
        "claim.created",
        serde_json::json!({ "claim_id": claim_id }),
    );

    uow.record_op_log(
        actor,
        "ClaimCase",
        claim_id,
        serde_json::json!({
            "action": "create",
            "patient_id": input.patient_id,
            "payer_id": input.payer_id,
            "total_amount": total_amount,
        }),
    );

    tracing::info!(claim_id = %claim_id, total = total_amount, "claim created");

    Ok(CreateClaimOutput {
        claim_id,
        total_amount,
    })
}
