//! Register a new patient at a facility.
//!
//! Full canonical write flow: domain row + audit + outbox + op-log in one transaction.

use chrono::Utc;
use rustyclinic_core::error::AppResult;
use rustyclinic_core::types::{ActorContext, Sex, new_id};
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
use rustyclinic_identity::{Patient, PatientRepo};
use uuid::Uuid;

/// Input for patient registration.
pub struct RegisterPatientInput {
    pub given_name: String,
    pub family_name: String,
    pub sex: Sex,
    pub date_of_birth: Option<chrono::NaiveDate>,
    pub phone: Option<String>,
    pub address: Option<String>,
    pub national_id: Option<String>,
}

/// Register a new patient with full audit trail. Returns the patient ID.
pub fn execute(
    uow: &mut UnitOfWork<'_>,
    repo: &dyn PatientRepo,
    actor: &ActorContext,
    input: RegisterPatientInput,
) -> AppResult<Uuid> {
    let now = Utc::now();
    let patient_id = new_id();

    let patient = Patient {
        id: patient_id,
        facility_id: actor.facility_id,
        given_name: input.given_name.clone(),
        family_name: input.family_name.clone(),
        sex: input.sex,
        date_of_birth: input.date_of_birth,
        phone: input.phone,
        address: input.address,
        national_id: input.national_id,
        created_at: now,
        updated_at: now,
        version: 0,
    };

    // Step 6: persist domain row
    repo.create(&patient)?;

    // Step 7: audit entry
    uow.record_audit(
        actor,
        "patient.registered",
        "Patient",
        patient_id,
        serde_json::json!({
            "given_name": input.given_name,
            "family_name": input.family_name,
        }),
    );

    // Step 8: outbox event
    uow.record_outbox(
        actor.facility_id,
        "Patient",
        patient_id,
        "patient.registered",
        serde_json::json!({ "patient_id": patient_id }),
    );

    // Step 9: op-log for sync
    uow.record_op_log(
        actor,
        "Patient",
        patient_id,
        serde_json::json!({
            "action": "register",
            "given_name": input.given_name,
            "family_name": input.family_name,
        }),
    );

    tracing::info!(patient_id = %patient_id, "patient registered");

    Ok(patient_id)
}
