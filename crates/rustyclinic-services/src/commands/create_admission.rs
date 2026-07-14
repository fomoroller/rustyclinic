//! Create an admission record for a patient.

use chrono::Utc;
use rustyclinic_clinical::admission::{Admission, AdmissionRepo, AdmissionStatus};
use rustyclinic_core::error::AppResult;
use rustyclinic_core::types::{ActorContext, new_id};
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
use uuid::Uuid;

pub struct CreateAdmissionInput {
    pub encounter_id: Uuid,
    pub patient_id: Uuid,
    pub ward: String,
    pub bed: Option<String>,
    pub notes: Option<String>,
}

pub struct CreateAdmissionOutput {
    pub admission_id: Uuid,
}

pub fn execute(
    uow: &mut UnitOfWork<'_>,
    repo: &dyn AdmissionRepo,
    actor: &ActorContext,
    input: CreateAdmissionInput,
) -> AppResult<CreateAdmissionOutput> {
    let now = Utc::now();
    let admission_id = new_id();

    let admission = Admission {
        id: admission_id,
        encounter_id: input.encounter_id,
        patient_id: input.patient_id,
        facility_id: actor.facility_id,
        status: AdmissionStatus::Planned,
        ward: input.ward,
        bed: input.bed,
        admitted_by: actor.user_id,
        admitted_at: None,
        transferred_to_ward: None,
        transferred_at: None,
        discharged_at: None,
        discharged_by: None,
        discharge_reason: None,
        notes: input.notes,
        created_at: now,
        version: 0,
    };

    repo.create(&admission)?;

    uow.record_audit(
        actor,
        "admission.created",
        "Admission",
        admission_id,
        serde_json::json!({
            "encounter_id": input.encounter_id,
            "patient_id": input.patient_id,
        }),
    );

    uow.record_outbox(
        actor.facility_id,
        "Admission",
        admission_id,
        "admission.created",
        serde_json::json!({ "admission_id": admission_id }),
    );

    uow.record_op_log(
        actor,
        "Admission",
        admission_id,
        serde_json::json!({
            "action": "create_admission",
            "encounter_id": input.encounter_id,
        }),
    );

    tracing::info!(admission_id = %admission_id, "admission created");

    Ok(CreateAdmissionOutput { admission_id })
}
