//! Create an encounter from a queue entry and transition queue to InService.

use chrono::Utc;
use rusqlite::OptionalExtension;
use rustyclinic_clinical::queue::{QueueEntryRepo, QueueTransition};
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_core::state_machine::StateMachine;
use rustyclinic_core::types::{ActorContext, new_id};
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
use uuid::Uuid;

pub struct CreateEncounterInput {
    pub queue_entry_id: Uuid,
    pub provider_id: Uuid,
}

pub struct CreateEncounterOutput {
    pub encounter_id: Uuid,
}

pub fn execute(
    uow: &mut UnitOfWork<'_>,
    queue_repo: &dyn QueueEntryRepo,
    actor: &ActorContext,
    input: CreateEncounterInput,
) -> AppResult<CreateEncounterOutput> {
    let now = Utc::now();
    let encounter_id = new_id();
    let (
        pinned_form_family,
        pinned_form_version,
        pinned_form_package_row_id,
        pinned_form_source_form_id,
    ) = resolve_active_encounter_form_pin(uow.conn())?;

    // Load queue entry
    let mut entry = queue_repo
        .find_by_id(input.queue_entry_id)?
        .ok_or(AppError::NotFound {
            entity: "QueueEntry",
            id: input.queue_entry_id,
        })?;

    // Transition queue to InService if not already
    let status_str = entry.status.to_string();
    if status_str == "called" {
        entry.apply_transition(QueueTransition::BeginService, actor)?;
        queue_repo.update(&entry)?;
    }

    // Create encounter row
    uow.conn().execute(
        "INSERT INTO encounters (id, facility_id, patient_id, queue_entry_id, provider_id, started_at, visit_notes, status, created_at, version, pinned_form_family, pinned_form_version, pinned_form_package_row_id, pinned_form_source_form_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, '', 'in_progress', ?7, 0, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            encounter_id.to_string(),
            actor.facility_id.to_string(),
            entry.patient_id.to_string(),
            input.queue_entry_id.to_string(),
            input.provider_id.to_string(),
            now.to_rfc3339(),
            now.to_rfc3339(),
            pinned_form_family,
            pinned_form_version,
            pinned_form_package_row_id,
            pinned_form_source_form_id,
        ],
    ).map_err(|e| AppError::Database(e.to_string()))?;

    uow.record_audit(
        actor,
        "encounter.created",
        "Encounter",
        encounter_id,
        serde_json::json!({
            "queue_entry_id": input.queue_entry_id,
            "patient_id": entry.patient_id,
            "provider_id": input.provider_id,
        }),
    );

    uow.record_outbox(
        actor.facility_id,
        "Encounter",
        encounter_id,
        "encounter.created",
        serde_json::json!({ "encounter_id": encounter_id }),
    );

    uow.record_op_log(
        actor,
        "Encounter",
        encounter_id,
        serde_json::json!({
            "action": "create",
            "queue_entry_id": input.queue_entry_id,
            "patient_id": entry.patient_id,
        }),
    );

    tracing::info!(encounter_id = %encounter_id, "encounter created");

    Ok(CreateEncounterOutput { encounter_id })
}

fn resolve_active_encounter_form_pin(
    conn: &rusqlite::Connection,
) -> AppResult<(String, String, Option<String>, String)> {
    let package_form: Option<(String, String, String)> = conn
        .query_row(
            "SELECT package_forms.form_id, installed_packages.version, installed_packages.id
             FROM package_forms
             INNER JOIN installed_packages ON installed_packages.id = package_forms.package_row_id
             WHERE package_forms.form_id = 'encounter-capture'
               AND installed_packages.status = 'activated'
             ORDER BY installed_packages.activated_at DESC
             LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|e| AppError::Database(e.to_string()))?;

    Ok(match package_form {
        Some((form_id, version, package_row_id)) => {
            (form_id.clone(), version, Some(package_row_id), form_id)
        }
        None => (
            "encounter-capture".to_string(),
            "1.1.0".to_string(),
            None,
            "encounter-capture".to_string(),
        ),
    })
}
