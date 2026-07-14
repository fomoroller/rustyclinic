//! Create a program enrollment for a patient.

use chrono::Utc;
use rustyclinic_clinical::program::{EnrollmentStatus, ProgramEnrollment, ProgramEnrollmentRepo};
use rustyclinic_core::error::AppResult;
use rustyclinic_core::types::{ActorContext, new_id};
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
use uuid::Uuid;

pub struct EnrollProgramInput {
    pub patient_id: Uuid,
    pub program_code: String,
    pub program_name: String,
    pub notes: Option<String>,
}

pub struct EnrollProgramOutput {
    pub enrollment_id: Uuid,
}

pub fn execute(
    uow: &mut UnitOfWork<'_>,
    repo: &dyn ProgramEnrollmentRepo,
    actor: &ActorContext,
    input: EnrollProgramInput,
) -> AppResult<EnrollProgramOutput> {
    let now = Utc::now();
    let enrollment_id = new_id();

    let enrollment = ProgramEnrollment {
        id: enrollment_id,
        patient_id: input.patient_id,
        facility_id: actor.facility_id,
        program_code: input.program_code,
        program_name: input.program_name,
        status: EnrollmentStatus::Eligible,
        enrolled_by: actor.user_id,
        enrolled_at: None,
        activated_at: None,
        paused_at: None,
        completed_at: None,
        withdrawn_at: None,
        notes: input.notes,
        created_at: now,
        version: 0,
    };

    repo.create(&enrollment)?;

    uow.record_audit(
        actor,
        "enrollment.created",
        "ProgramEnrollment",
        enrollment_id,
        serde_json::json!({
            "patient_id": input.patient_id,
        }),
    );

    uow.record_outbox(
        actor.facility_id,
        "ProgramEnrollment",
        enrollment_id,
        "enrollment.created",
        serde_json::json!({ "enrollment_id": enrollment_id }),
    );

    uow.record_op_log(
        actor,
        "ProgramEnrollment",
        enrollment_id,
        serde_json::json!({
            "action": "enroll_program",
            "patient_id": input.patient_id,
        }),
    );

    tracing::info!(enrollment_id = %enrollment_id, "program enrollment created");

    Ok(EnrollProgramOutput { enrollment_id })
}
