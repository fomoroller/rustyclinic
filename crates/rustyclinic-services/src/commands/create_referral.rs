//! Create a referral from an encounter.

use chrono::Utc;
use rustyclinic_clinical::Priority;
use rustyclinic_clinical::referral::{Referral, ReferralRepo, ReferralStatus};
use rustyclinic_core::error::AppResult;
use rustyclinic_core::types::{ActorContext, new_id};
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
use uuid::Uuid;

pub struct CreateReferralInput {
    pub encounter_id: Uuid,
    pub patient_id: Uuid,
    pub priority: Priority,
    pub referred_to_facility: Option<String>,
    pub referred_to_department: Option<String>,
    pub reason: String,
    pub clinical_summary: Option<String>,
    pub notes: Option<String>,
}

pub struct CreateReferralOutput {
    pub referral_id: Uuid,
}

pub fn execute(
    uow: &mut UnitOfWork<'_>,
    repo: &dyn ReferralRepo,
    actor: &ActorContext,
    input: CreateReferralInput,
) -> AppResult<CreateReferralOutput> {
    let now = Utc::now();
    let referral_id = new_id();

    let referral = Referral {
        id: referral_id,
        encounter_id: input.encounter_id,
        patient_id: input.patient_id,
        facility_id: actor.facility_id,
        status: ReferralStatus::Drafted,
        priority: input.priority,
        referred_by: actor.user_id,
        referred_to_facility: input.referred_to_facility,
        referred_to_department: input.referred_to_department,
        reason: input.reason,
        clinical_summary: input.clinical_summary,
        sent_at: None,
        received_at: None,
        accepted_at: None,
        completed_at: None,
        notes: input.notes,
        created_at: now,
        version: 0,
    };

    repo.create(&referral)?;

    uow.record_audit(
        actor,
        "referral.created",
        "Referral",
        referral_id,
        serde_json::json!({
            "encounter_id": input.encounter_id,
            "patient_id": input.patient_id,
        }),
    );

    uow.record_outbox(
        actor.facility_id,
        "Referral",
        referral_id,
        "referral.created",
        serde_json::json!({ "referral_id": referral_id }),
    );

    uow.record_op_log(
        actor,
        "Referral",
        referral_id,
        serde_json::json!({
            "action": "create_referral",
            "encounter_id": input.encounter_id,
        }),
    );

    tracing::info!(referral_id = %referral_id, "referral created");

    Ok(CreateReferralOutput { referral_id })
}
