//! Encounter capture routes — form-engine-powered encounter forms.

use std::collections::{HashMap, HashSet};

use axum::Form;
use axum::extract::{Path, Query, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::WebAppState;
use crate::form_renderer;
use crate::forms;
use crate::middleware::session::WebSession;
use crate::routes::patients::{lookup_patient_name, lookup_user_name};
use crate::templates::{EncounterCapturePage, TriagePage};

#[derive(Deserialize)]
pub struct NewEncounterQuery {
    pub queue_entry_id: String,
}

pub async fn new_encounter(
    State(state): State<WebAppState>,
    session: WebSession,
    Query(query): Query<NewEncounterQuery>,
) -> Response {
    let queue_entry_id = match Uuid::parse_str(&query.queue_entry_id) {
        Ok(u) => u,
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };

    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };

    let actor = session.session.to_actor_context();
    let queue_repo = rustyclinic_db::sqlite::queue_repo::SqliteQueueRepo::new(&conn);

    // Check if queue entry already has an encounter (from triage)
    use rustyclinic_clinical::queue::QueueEntryRepo;
    let entry = match queue_repo.find_by_id(queue_entry_id) {
        Ok(Some(e)) => e,
        _ => return Redirect::to("/web/queue").into_response(),
    };

    let encounter_id = if let Some(existing_enc_id) = entry.encounter_id {
        // Reuse existing encounter from triage — just ensure queue is in_service
        if entry.status.to_string() != "in_service" {
            let mut entry_mut = entry.clone();
            if entry_mut.status.to_string() == "called" {
                use rustyclinic_core::state_machine::StateMachine;
                let _ = entry_mut.apply_transition(
                    rustyclinic_clinical::queue::QueueTransition::BeginService,
                    &actor,
                );
                let _ = queue_repo.update(&entry_mut);
            }
        }
        existing_enc_id
    } else {
        // No encounter yet — create one
        let mut uow = match rustyclinic_db::sqlite::unit_of_work::UnitOfWork::try_new(&conn) {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!(error = %e, "failed to begin transaction");
                return Redirect::to("/web/queue").into_response();
            }
        };
        let input = rustyclinic_services::commands::create_encounter::CreateEncounterInput {
            queue_entry_id,
            provider_id: actor.user_id,
        };

        match rustyclinic_services::commands::create_encounter::execute(
            &mut uow,
            &queue_repo,
            &actor,
            input,
        ) {
            Ok(output) => {
                if let Err(e) = uow.commit() {
                    tracing::warn!(error = %e, "commit failed");
                }
                output.encounter_id
            }
            Err(e) => {
                tracing::warn!(error = %e, "create encounter failed");
                return Redirect::to("/web/queue").into_response();
            }
        }
    };

    // Refresh entry after potential changes
    let entry = match queue_repo.find_by_id(queue_entry_id) {
        Ok(Some(e)) => e,
        _ => return Redirect::to("/web/queue").into_response(),
    };

    let patient_name = lookup_patient_name(&conn, actor.facility_id, entry.patient_id);

    let patient_initials = WebSession::initials_from(&patient_name);
    let service_type = entry.service_type.clone();

    // Load form engine and merge existing visit_notes (from triage) + draft
    let encounter_id_str = encounter_id.to_string();
    let form_def = forms::resolve_form_for_encounter(&conn, &encounter_id_str, "encounter-capture");
    let engine = match rustyclinic_forms::engine::FormEngine::new(form_def.clone()) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!(error = %e, "failed to create form engine");
            return Redirect::to("/web/queue").into_response();
        }
    };

    // Start with any vitals saved during triage (from encounter visit_notes)
    let mut field_values = load_encounter_vitals(&conn, &encounter_id_str);

    // Overlay any draft values (draft takes priority)
    let draft_values = load_draft_values(
        &conn,
        &session.session.user_id.to_string(),
        &encounter_id_str,
    );
    for (k, v) in draft_values {
        field_values.insert(k, v);
    }

    let eval_state = engine.evaluate(&field_values);
    let validate_url = format!("/web/encounters/{}/validate", encounter_id);
    let form_html = form_renderer::render_form(&form_def, &eval_state, &validate_url);

    let (display_name, user_initials) = lookup_user_name(&state, session.session.user_id);

    let page = EncounterCapturePage {
        active_nav: "queue".to_string(),
        display_name,
        initials: user_initials,
        flash_success: None,
        flash_error: None,
        encounter_id: encounter_id.to_string(),
        queue_entry_id: queue_entry_id.to_string(),
        patient_name,
        patient_initials,
        service_type,
        form_html,
        form_error: None,
    };
    Html(page.to_string()).into_response()
}

#[derive(Deserialize)]
pub struct EncounterForm {
    pub encounter_id: String,
    pub queue_entry_id: String,
    // All other fields are dynamic — captured via the HashMap
    #[serde(flatten)]
    pub fields: HashMap<String, String>,
}

pub async fn save_encounter(
    State(state): State<WebAppState>,
    session: WebSession,
    Form(form): Form<EncounterForm>,
) -> Response {
    let encounter_id = match Uuid::parse_str(&form.encounter_id) {
        Ok(u) => u,
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };
    let queue_entry_id = match Uuid::parse_str(&form.queue_entry_id) {
        Ok(u) => u,
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };

    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };

    // Parse form fields into JSON values for the form engine
    let field_values = enrich_field_values(parse_form_fields(&form.fields));

    // Validate with form engine
    let form_def =
        forms::resolve_form_for_encounter(&conn, &encounter_id.to_string(), "encounter-capture");
    let engine = match rustyclinic_forms::engine::FormEngine::new(form_def.clone()) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!(error = %e, "failed to create form engine");
            return Redirect::to("/web/queue").into_response();
        }
    };

    let eval_state = engine.evaluate(&field_values);

    if !eval_state.is_submittable {
        // Re-render with errors
        let queue_repo = rustyclinic_db::sqlite::queue_repo::SqliteQueueRepo::new(&conn);
        use rustyclinic_clinical::queue::QueueEntryRepo;
        let entry = queue_repo.find_by_id(queue_entry_id);

        let (patient_name, patient_initials, service_type) = if let Ok(Some(entry)) = entry {
            let patient_name =
                lookup_patient_name(&conn, session.session.facility_id, entry.patient_id);
            let initials = WebSession::initials_from(&patient_name);
            (patient_name, initials, entry.service_type)
        } else {
            (
                "Unknown".to_string(),
                "?".to_string(),
                "consultation".to_string(),
            )
        };

        let validate_url = format!("/web/encounters/{}/validate", encounter_id);
        let form_html = form_renderer::render_form(&form_def, &eval_state, &validate_url);
        let (display_name, user_initials) = lookup_user_name(&state, session.session.user_id);

        let page = EncounterCapturePage {
            active_nav: "queue".to_string(),
            display_name,
            initials: user_initials,
            flash_success: None,
            flash_error: None,
            encounter_id: encounter_id.to_string(),
            queue_entry_id: queue_entry_id.to_string(),
            patient_name,
            patient_initials,
            service_type,
            form_html,
            form_error: Some("Please correct the errors below before submitting.".to_string()),
        };
        return Html(page.to_string()).into_response();
    }

    // Valid — save the encounter
    let actor = session.session.to_actor_context();
    let queue_repo = rustyclinic_db::sqlite::queue_repo::SqliteQueueRepo::new(&conn);
    let mut uow = match rustyclinic_db::sqlite::unit_of_work::UnitOfWork::try_new(&conn) {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!(error = %e, "failed to begin transaction");
            return Redirect::to("/web/queue").into_response();
        }
    };

    // Serialize all field values as the visit notes JSON
    let visit_notes = serde_json::to_string(&field_values).unwrap_or_default();

    let input = rustyclinic_services::commands::complete_encounter::CompleteEncounterInput {
        encounter_id,
        queue_entry_id,
        visit_notes,
    };

    match rustyclinic_services::commands::complete_encounter::execute(
        &mut uow,
        &queue_repo,
        &actor,
        input,
    ) {
        Ok(()) => {
            if let Err(e) = uow.commit() {
                tracing::warn!(error = %e, "commit failed");
            }
            // Clean up draft
            let _ = conn.execute(
                "DELETE FROM form_drafts WHERE user_id = ?1 AND encounter_id = ?2",
                rusqlite::params![actor.user_id.to_string(), encounter_id.to_string(),],
            );
        }
        Err(e) => {
            tracing::warn!(error = %e, "complete encounter failed");
        }
    }

    Redirect::to("/web/queue").into_response()
}

#[derive(Deserialize)]
pub struct DraftForm {
    #[serde(flatten)]
    pub fields: HashMap<String, String>,
}

pub async fn save_draft(
    State(state): State<WebAppState>,
    session: WebSession,
    Path(encounter_id): Path<String>,
    Form(form): Form<DraftForm>,
) -> impl IntoResponse {
    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Html("error".to_string()),
    };

    let field_values = enrich_field_values(parse_form_fields(&form.fields));
    let json_str = serde_json::to_string(&field_values).unwrap_or_default();

    let now = chrono::Utc::now();
    let (draft_form_family, draft_form_version) = load_pinned_form_identity(&conn, &encounter_id)
        .unwrap_or_else(|| ("encounter-capture".to_string(), "1.1.0".to_string()));
    let _ = conn.execute(
        "INSERT OR REPLACE INTO form_drafts (user_id, encounter_id, form_family, form_version, field_values, saved_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            session.session.user_id.to_string(),
            encounter_id,
            draft_form_family,
            draft_form_version,
            json_str,
            now.to_rfc3339(),
        ],
    );

    Html("saved".to_string())
}

/// Validate form fields via htmx — returns updated form HTML partial.
pub async fn validate_fields(
    State(_state): State<WebAppState>,
    _session: WebSession,
    Path(_encounter_id): Path<String>,
    Form(form): Form<DraftForm>,
) -> impl IntoResponse {
    let field_values = enrich_field_values(parse_form_fields(&form.fields));

    let conn = match rusqlite::Connection::open(&_state.db_path) {
        Ok(c) => c,
        Err(_) => return Html(String::new()),
    };

    let form_def = forms::resolve_form_for_encounter(&conn, &_encounter_id, "encounter-capture");
    let engine = match rustyclinic_forms::engine::FormEngine::new(form_def.clone()) {
        Ok(e) => e,
        Err(_) => return Html(String::new()),
    };

    let eval_state = engine.evaluate(&field_values);
    let validate_url = format!("/web/encounters/{}/validate", _encounter_id);
    let html = form_renderer::render_validation_partial(&form_def, &eval_state, &validate_url);
    Html(html)
}

// ===== Lab Order from Encounter =====

#[derive(Deserialize)]
pub struct OrderLabQuery {
    pub queue_entry_id: String,
}

pub async fn order_lab_page(
    State(state): State<WebAppState>,
    session: WebSession,
    Path(encounter_id): Path<String>,
    Query(query): Query<OrderLabQuery>,
) -> Response {
    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };

    let (display_name, initials) = lookup_user_name(&state, session.session.user_id);

    // Look up patient from encounter
    let patient_id = conn.query_row(
        "SELECT patient_id FROM encounters WHERE id = ?1",
        rusqlite::params![encounter_id],
        |row| row.get::<_, String>(0),
    );

    let (patient_id, patient_name) = match patient_id {
        Ok(patient_id) => {
            let patient_uuid = Uuid::parse_str(&patient_id).unwrap_or_default();
            let patient_name =
                lookup_patient_name(&conn, session.session.facility_id, patient_uuid);
            (patient_id, patient_name)
        }
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };

    let page = crate::templates::OrderLabPage {
        active_nav: "queue".to_string(),
        display_name,
        initials,
        flash_success: None,
        flash_error: None,
        encounter_id: encounter_id.clone(),
        queue_entry_id: query.queue_entry_id,
        patient_id,
        patient_name,
    };
    Html(page.to_string()).into_response()
}

#[derive(Deserialize)]
pub struct LabOrderForm {
    pub encounter_id: String,
    pub queue_entry_id: String,
    pub patient_id: String,
    pub specimen_type: Option<String>,
    pub priority: String,
    pub notes: Option<String>,
    #[serde(flatten)]
    pub fields: HashMap<String, String>,
}

fn parse_selected_lab_tests(
    fields: &HashMap<String, String>,
) -> Vec<rustyclinic_clinical::lab::LabTest> {
    let mut seen_codes = HashSet::new();
    let mut tests = Vec::new();

    let mut push_test = |test_code: String, test_name: String| {
        if test_code.is_empty() || test_name.is_empty() || !seen_codes.insert(test_code.clone()) {
            return;
        }
        tests.push(rustyclinic_clinical::lab::LabTest {
            test_code,
            test_name,
            result: None,
            result_value: None,
            unit: None,
            reference_range: None,
            is_abnormal: false,
            resulted_at: None,
            resulted_by: None,
        });
    };

    for (code, name) in rustyclinic_clinical::lab::COMMON_TESTS {
        let key = format!("test_{code}");
        if fields.contains_key(&key) {
            push_test(code.to_string(), name.to_string());
        }
    }

    for (key, value) in fields {
        if let Some(code) = key.strip_prefix("test_loinc_") {
            push_test(code.trim().to_string(), value.trim().to_string());
        }
    }

    tests
}

pub async fn submit_lab_order(
    State(state): State<WebAppState>,
    session: WebSession,
    Path(_encounter_id): Path<String>,
    Form(form): Form<LabOrderForm>,
) -> Response {
    let encounter_id = match Uuid::parse_str(&form.encounter_id) {
        Ok(u) => u,
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };
    let patient_id = match Uuid::parse_str(&form.patient_id) {
        Ok(u) => u,
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };

    let tests = parse_selected_lab_tests(&form.fields);

    if tests.is_empty() {
        // At least one test must be selected, redirect back
        return Redirect::to(&format!(
            "/web/encounters/{}/order-lab?queue_entry_id={}",
            form.encounter_id, form.queue_entry_id
        ))
        .into_response();
    }

    let priority = rustyclinic_clinical::Priority::from_str_safe(&form.priority);

    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };

    let actor = session.session.to_actor_context();
    let order_repo = rustyclinic_db::sqlite::lab_repo::SqliteLabOrderRepo::new(&conn);
    let queue_repo = rustyclinic_db::sqlite::queue_repo::SqliteQueueRepo::new(&conn);
    let lab_repo = rustyclinic_db::sqlite::lab_repo::SqliteLabTestRepo::new(&conn);
    let mut uow = match rustyclinic_db::sqlite::unit_of_work::UnitOfWork::try_new(&conn) {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!(error = %e, "failed to begin transaction");
            return Redirect::to("/web/queue").into_response();
        }
    };

    let input = rustyclinic_services::commands::create_lab_order::CreateLabOrderInput {
        encounter_id,
        patient_id,
        tests,
        specimen_type: form.specimen_type.filter(|s| !s.is_empty()),
        priority,
        notes: form.notes.filter(|s| !s.is_empty()),
    };

    match rustyclinic_services::commands::create_lab_order::execute(
        &mut uow,
        &order_repo,
        &queue_repo,
        &lab_repo,
        &actor,
        input,
    ) {
        Ok(_output) => {
            if let Err(e) = uow.commit() {
                tracing::warn!(error = %e, "commit failed");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "create lab order failed");
        }
    }

    // Return to encounter capture
    Redirect::to(&format!(
        "/web/encounters/new?queue_entry_id={}",
        form.queue_entry_id
    ))
    .into_response()
}

// ===== Prescription from Encounter =====

pub async fn prescribe_page(
    State(state): State<WebAppState>,
    session: WebSession,
    Path(encounter_id): Path<String>,
    Query(query): Query<OrderLabQuery>,
) -> Response {
    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };

    let (display_name, initials) = lookup_user_name(&state, session.session.user_id);

    let patient_id = conn.query_row(
        "SELECT patient_id FROM encounters WHERE id = ?1",
        rusqlite::params![encounter_id],
        |row| row.get::<_, String>(0),
    );

    let (patient_id, patient_name) = match patient_id {
        Ok(patient_id) => {
            let patient_uuid = Uuid::parse_str(&patient_id).unwrap_or_default();
            let patient_name =
                lookup_patient_name(&conn, session.session.facility_id, patient_uuid);
            (patient_id, patient_name)
        }
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };

    let page = crate::templates::PrescribePage {
        active_nav: "queue".to_string(),
        display_name,
        initials,
        flash_success: None,
        flash_error: None,
        encounter_id: encounter_id.clone(),
        queue_entry_id: query.queue_entry_id,
        patient_id,
        patient_name,
    };
    Html(page.to_string()).into_response()
}

#[derive(Deserialize)]
pub struct PrescribeForm {
    pub encounter_id: String,
    pub queue_entry_id: String,
    pub patient_id: String,
    pub item_count: Option<String>,
    pub priority: String,
    pub notes: Option<String>,
    #[serde(flatten)]
    pub fields: HashMap<String, String>,
}

pub async fn submit_prescription(
    State(state): State<WebAppState>,
    session: WebSession,
    Path(_encounter_id): Path<String>,
    Form(form): Form<PrescribeForm>,
) -> Response {
    let encounter_id = match Uuid::parse_str(&form.encounter_id) {
        Ok(u) => u,
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };
    let patient_id = match Uuid::parse_str(&form.patient_id) {
        Ok(u) => u,
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };

    // Parse medication items from numbered fields
    let count: usize = form
        .item_count
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let mut items = Vec::new();
    for i in 0..count {
        let name = form
            .fields
            .get(&format!("med_name_{i}"))
            .cloned()
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let dosage = form
            .fields
            .get(&format!("med_dosage_{i}"))
            .cloned()
            .unwrap_or_default();
        let frequency = form
            .fields
            .get(&format!("med_frequency_{i}"))
            .cloned()
            .unwrap_or_default();
        let duration = form
            .fields
            .get(&format!("med_duration_{i}"))
            .cloned()
            .unwrap_or_default();
        let quantity: u32 = form
            .fields
            .get(&format!("med_quantity_{i}"))
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);

        // Resolve medication coding: posted fields override, otherwise use terminology binding
        let (medication_system, medication_code, medication_display) =
            resolve_medication_coding(&form.fields, i, &name);

        items.push(rustyclinic_clinical::pharmacy::DispenseItem {
            medication_name: name,
            medication_system,
            medication_code,
            medication_display,
            dosage,
            frequency,
            duration,
            quantity,
            dispensed_quantity: None,
            substituted: false,
            substitution_reason: None,
        });
    }

    if items.is_empty() {
        return Redirect::to(&format!(
            "/web/encounters/{}/prescribe?queue_entry_id={}",
            form.encounter_id, form.queue_entry_id
        ))
        .into_response();
    }

    let priority = rustyclinic_clinical::Priority::from_str_safe(&form.priority);

    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };

    let actor = session.session.to_actor_context();
    let dispense_repo =
        rustyclinic_db::sqlite::pharmacy_repo::SqliteMedicationDispenseRepo::new(&conn);
    let queue_repo = rustyclinic_db::sqlite::queue_repo::SqliteQueueRepo::new(&conn);
    let item_repo = rustyclinic_db::sqlite::pharmacy_repo::SqliteDispenseItemRepo::new(&conn);
    let mut uow = match rustyclinic_db::sqlite::unit_of_work::UnitOfWork::try_new(&conn) {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!(error = %e, "failed to begin transaction");
            return Redirect::to("/web/queue").into_response();
        }
    };

    let input = rustyclinic_services::commands::create_prescription::CreatePrescriptionInput {
        encounter_id,
        patient_id,
        items,
        priority,
        notes: form.notes.filter(|s| !s.is_empty()),
    };

    match rustyclinic_services::commands::create_prescription::execute(
        &mut uow,
        &dispense_repo,
        &queue_repo,
        &item_repo,
        &actor,
        input,
    ) {
        Ok(_output) => {
            if let Err(e) = uow.commit() {
                tracing::warn!(error = %e, "commit failed");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "create prescription failed");
        }
    }

    Redirect::to(&format!(
        "/web/encounters/new?queue_entry_id={}",
        form.queue_entry_id
    ))
    .into_response()
}

// ===== Triage =====

#[derive(Deserialize)]
pub struct TriageQuery {
    pub queue_entry_id: String,
}

/// GET /web/encounters/triage — show triage vitals form.
///
/// Creates an encounter if one doesn't exist for the queue entry.
pub async fn triage_page(
    State(state): State<WebAppState>,
    session: WebSession,
    Query(query): Query<TriageQuery>,
) -> Response {
    let queue_entry_id = match Uuid::parse_str(&query.queue_entry_id) {
        Ok(u) => u,
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };

    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };

    let actor = session.session.to_actor_context();
    let queue_repo = rustyclinic_db::sqlite::queue_repo::SqliteQueueRepo::new(&conn);

    use rustyclinic_clinical::queue::QueueEntryRepo;
    let entry = match queue_repo.find_by_id(queue_entry_id) {
        Ok(Some(e)) => e,
        _ => return Redirect::to("/web/queue").into_response(),
    };

    // Create encounter if none exists, and transition queue to called/in-service
    let encounter_id = if let Some(existing_enc_id) = entry.encounter_id {
        existing_enc_id
    } else {
        // Transition to called first if waiting
        let mut entry_mut = entry.clone();
        let mut uow = match rustyclinic_db::sqlite::unit_of_work::UnitOfWork::try_new(&conn) {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!(error = %e, "failed to begin transaction");
                return Redirect::to("/web/queue").into_response();
            }
        };

        if entry_mut.status.to_string() == "waiting" {
            use rustyclinic_core::state_machine::StateMachine;
            let _ = entry_mut
                .apply_transition(rustyclinic_clinical::queue::QueueTransition::Call, &actor);
            let _ = queue_repo.update(&entry_mut);
        }

        let input = rustyclinic_services::commands::create_encounter::CreateEncounterInput {
            queue_entry_id,
            provider_id: actor.user_id,
        };

        match rustyclinic_services::commands::create_encounter::execute(
            &mut uow,
            &queue_repo,
            &actor,
            input,
        ) {
            Ok(output) => {
                // Link encounter to queue entry
                let _ = conn.execute(
                    "UPDATE queue_entries SET encounter_id = ?1 WHERE id = ?2",
                    rusqlite::params![output.encounter_id.to_string(), queue_entry_id.to_string(),],
                );
                if let Err(e) = uow.commit() {
                    tracing::warn!(error = %e, "commit failed");
                }
                output.encounter_id
            }
            Err(e) => {
                tracing::warn!(error = %e, "create encounter for triage failed");
                return Redirect::to("/web/queue").into_response();
            }
        }
    };

    let patient_name = lookup_patient_name(&conn, session.session.facility_id, entry.patient_id);

    let patient_initials = WebSession::initials_from(&patient_name);

    // Load triage form
    let form_def = forms::triage_form();
    let engine = match rustyclinic_forms::engine::FormEngine::new(form_def.clone()) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!(error = %e, "failed to create triage form engine");
            return Redirect::to("/web/queue").into_response();
        }
    };

    // Load any existing vitals from the encounter
    let field_values = load_encounter_vitals(&conn, &encounter_id.to_string());
    let eval_state = engine.evaluate(&field_values);
    let form_html = form_renderer::render_form(
        &form_def,
        &eval_state,
        &format!("/web/encounters/{}/validate", encounter_id),
    );

    let (display_name, user_initials) = lookup_user_name(&state, session.session.user_id);

    let page = TriagePage {
        active_nav: "queue".to_string(),
        display_name,
        initials: user_initials,
        flash_success: None,
        flash_error: None,
        encounter_id: encounter_id.to_string(),
        queue_entry_id: queue_entry_id.to_string(),
        patient_name,
        patient_initials,
        form_html,
        form_error: None,
    };
    Html(page.to_string()).into_response()
}

#[derive(Deserialize)]
pub struct TriageForm {
    pub encounter_id: String,
    pub queue_entry_id: String,
    #[serde(flatten)]
    pub fields: HashMap<String, String>,
}

/// POST /web/encounters/triage — save triage vitals and advance patient to consultation.
pub async fn save_triage(
    State(state): State<WebAppState>,
    session: WebSession,
    Form(form): Form<TriageForm>,
) -> Response {
    let encounter_id = match Uuid::parse_str(&form.encounter_id) {
        Ok(u) => u,
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };
    let queue_entry_id = match Uuid::parse_str(&form.queue_entry_id) {
        Ok(u) => u,
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };

    let field_values = enrich_field_values(parse_form_fields(&form.fields));

    // Validate with triage form engine
    let form_def = forms::triage_form();
    let engine = match rustyclinic_forms::engine::FormEngine::new(form_def.clone()) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!(error = %e, "failed to create triage form engine");
            return Redirect::to("/web/queue").into_response();
        }
    };

    let eval_state = engine.evaluate(&field_values);

    if !eval_state.is_submittable {
        // Re-render with errors
        let conn = match rusqlite::Connection::open(&state.db_path) {
            Ok(c) => c,
            Err(_) => return Redirect::to("/web/queue").into_response(),
        };

        let queue_repo = rustyclinic_db::sqlite::queue_repo::SqliteQueueRepo::new(&conn);
        use rustyclinic_clinical::queue::QueueEntryRepo;
        let entry = queue_repo.find_by_id(queue_entry_id);

        let patient_name = if let Ok(Some(ref entry)) = entry {
            lookup_patient_name(&conn, session.session.facility_id, entry.patient_id)
        } else {
            "Unknown".to_string()
        };

        let patient_initials = WebSession::initials_from(&patient_name);
        let form_html = form_renderer::render_form(
            &form_def,
            &eval_state,
            &format!("/web/encounters/{}/validate", encounter_id),
        );
        let (display_name, user_initials) = lookup_user_name(&state, session.session.user_id);

        let page = TriagePage {
            active_nav: "queue".to_string(),
            display_name,
            initials: user_initials,
            flash_success: None,
            flash_error: None,
            encounter_id: encounter_id.to_string(),
            queue_entry_id: queue_entry_id.to_string(),
            patient_name,
            patient_initials,
            form_html,
            form_error: Some("Please correct the errors before submitting.".to_string()),
        };
        return Html(page.to_string()).into_response();
    }

    // Valid — save vitals to encounter visit_notes
    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };

    let visit_notes_json = serde_json::to_string(&field_values).unwrap_or_default();

    // Update encounter visit_notes with triage vitals (don't complete the encounter)
    let _ = conn.execute(
        "UPDATE encounters SET visit_notes = ?1 WHERE id = ?2 AND status = 'in_progress'",
        rusqlite::params![visit_notes_json, encounter_id.to_string()],
    );

    // Transition queue entry: ensure it's in_service (ready for doctor)
    let actor = session.session.to_actor_context();
    let queue_repo = rustyclinic_db::sqlite::queue_repo::SqliteQueueRepo::new(&conn);
    use rustyclinic_clinical::queue::QueueEntryRepo;

    if let Ok(Some(mut entry)) = queue_repo.find_by_id(queue_entry_id) {
        let status_str = entry.status.to_string();
        if status_str == "called" || status_str == "waiting" {
            use rustyclinic_core::state_machine::StateMachine;
            // If waiting, call first
            if status_str == "waiting" {
                let _ = entry
                    .apply_transition(rustyclinic_clinical::queue::QueueTransition::Call, &actor);
                let _ = queue_repo.update(&entry);
            }
            // Then begin service
            if entry.status.to_string() == "called" {
                let _ = entry.apply_transition(
                    rustyclinic_clinical::queue::QueueTransition::BeginService,
                    &actor,
                );
                let _ = queue_repo.update(&entry);
            }
        }
    }

    // Audit the triage completion
    let mut uow = match rustyclinic_db::sqlite::unit_of_work::UnitOfWork::try_new(&conn) {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!(error = %e, "failed to begin transaction");
            return Redirect::to("/web/queue").into_response();
        }
    };
    uow.record_audit(
        &actor,
        "encounter.triage_completed",
        "Encounter",
        encounter_id,
        serde_json::json!({
            "queue_entry_id": queue_entry_id,
        }),
    );
    if let Err(e) = uow.commit() {
        tracing::warn!(error = %e, "commit failed");
    }

    tracing::info!(encounter_id = %encounter_id, "triage completed");

    Redirect::to("/web/queue").into_response()
}

// ===== Helpers =====

/// Parse form field strings into JSON values suitable for the form engine.
///
/// Numbers are parsed as numbers, "true"/"false" as booleans, empty strings as null,
/// and everything else stays as a string.
fn parse_form_fields(fields: &HashMap<String, String>) -> HashMap<String, Value> {
    // Skip hidden form fields that aren't part of the form definition
    let skip = ["encounter_id", "queue_entry_id"];

    let mut values = HashMap::new();
    for (key, val) in fields {
        if skip.contains(&key.as_str()) {
            continue;
        }
        if val.is_empty() {
            continue; // Don't insert empty strings — the engine treats missing as empty
        }
        let json_val = if val == "true" {
            Value::Bool(true)
        } else if val == "false" {
            Value::Bool(false)
        } else if let Ok(n) = val.parse::<i64>() {
            Value::Number(serde_json::Number::from(n))
        } else if let Ok(n) = val.parse::<f64>() {
            serde_json::Number::from_f64(n)
                .map(Value::Number)
                .unwrap_or_else(|| Value::String(val.clone()))
        } else {
            Value::String(val.clone())
        };
        values.insert(key.clone(), json_val);
    }
    values
}

fn enrich_field_values(mut values: HashMap<String, Value>) -> HashMap<String, Value> {
    if let Some(binding) = diagnosis_binding_from_coded_selection(&values) {
        let diagnosis = serde_json::json!({
            "primary": binding,
        });
        values.insert("_diagnosis".to_string(), diagnosis);
    } else if let Some(primary) = non_empty_field(&values, "primary_diagnosis") {
        let other = values.get("other_diagnosis").and_then(Value::as_str);
        let binding = rustyclinic_terminology::diagnosis_binding(primary, other);
        let diagnosis = serde_json::json!({
            "primary": binding,
        });
        values.insert("_diagnosis".to_string(), diagnosis);
    }

    let observations: Vec<Value> = values
        .iter()
        .filter_map(|(key, value)| {
            let binding = rustyclinic_terminology::observation_binding(key)?;
            Some(serde_json::json!({
                "link_id": binding.link_id,
                "label": binding.label,
                "loinc": binding.loinc,
                "ucum": binding.ucum,
                "value": value,
            }))
        })
        .collect();

    if !observations.is_empty() {
        values.insert("_observations".to_string(), Value::Array(observations));
    }

    values
}

fn diagnosis_binding_from_coded_selection(
    values: &HashMap<String, Value>,
) -> Option<rustyclinic_terminology::DiagnosisBinding> {
    let system = non_empty_field(values, "primary_diagnosis_system")?;
    let code = non_empty_field(values, "primary_diagnosis_code")?;
    let display = non_empty_field(values, "primary_diagnosis_display")?;
    let normalized = system.trim().to_ascii_lowercase();

    let canonical_system = match normalized.as_str() {
        "icd11" | rustyclinic_terminology::ICD11_SYSTEM => rustyclinic_terminology::ICD11_SYSTEM,
        "snomed" | "snomedct" | "snomed_ct" | rustyclinic_terminology::SNOMED_SYSTEM => {
            rustyclinic_terminology::SNOMED_SYSTEM
        }
        _ => return None,
    };

    let local_value = non_empty_field(values, "primary_diagnosis")
        .unwrap_or(code)
        .to_string();
    let coding = rustyclinic_terminology::Coding {
        system: canonical_system.to_string(),
        code: code.to_string(),
        display: display.to_string(),
    };

    let mut binding = rustyclinic_terminology::DiagnosisBinding {
        local_value,
        clinician_label: display.to_string(),
        icd11: None,
        snomed: None,
    };

    if canonical_system == rustyclinic_terminology::ICD11_SYSTEM {
        binding.icd11 = Some(coding);
    } else {
        binding.snomed = Some(coding);
    }

    Some(binding)
}

fn non_empty_field<'a>(values: &'a HashMap<String, Value>, key: &str) -> Option<&'a str> {
    values
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn resolve_medication_coding(
    fields: &HashMap<String, String>,
    index: usize,
    name: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    let system = fields
        .get(&format!("med_system_{index}"))
        .filter(|s| !s.is_empty())
        .cloned();
    let code = fields
        .get(&format!("med_code_{index}"))
        .filter(|s| !s.is_empty())
        .cloned();
    let display = fields
        .get(&format!("med_display_{index}"))
        .filter(|s| !s.is_empty())
        .cloned();

    if system.is_some() && code.is_some() && display.is_some() {
        return (system, code, display);
    }

    let binding = rustyclinic_terminology::medication_binding(name);
    if let Some(snomed) = binding.snomed {
        return (Some(snomed.system), Some(snomed.code), Some(snomed.display));
    }

    (None, None, None)
}

/// Load vitals from an encounter's visit_notes JSON (saved during triage).
fn load_encounter_vitals(
    conn: &rusqlite::Connection,
    encounter_id: &str,
) -> HashMap<String, Value> {
    let result: Result<String, _> = conn.query_row(
        "SELECT visit_notes FROM encounters WHERE id = ?1",
        rusqlite::params![encounter_id],
        |row| row.get(0),
    );

    match result {
        Ok(json_str) if !json_str.is_empty() => {
            serde_json::from_str::<HashMap<String, Value>>(&json_str).unwrap_or_default()
        }
        _ => HashMap::new(),
    }
}

/// Load draft field values from the database, if any exist.
fn load_draft_values(
    conn: &rusqlite::Connection,
    user_id: &str,
    encounter_id: &str,
) -> HashMap<String, Value> {
    let result: Result<String, _> = conn.query_row(
        "SELECT field_values FROM form_drafts WHERE user_id = ?1 AND encounter_id = ?2",
        rusqlite::params![user_id, encounter_id],
        |row| row.get(0),
    );

    match result {
        Ok(json_str) => {
            serde_json::from_str::<HashMap<String, Value>>(&json_str).unwrap_or_default()
        }
        Err(_) => HashMap::new(),
    }
}

fn load_pinned_form_identity(
    conn: &rusqlite::Connection,
    encounter_id: &str,
) -> Option<(String, String)> {
    conn.query_row(
        "SELECT pinned_form_family, pinned_form_version
         FROM encounters
         WHERE id = ?1
           AND pinned_form_family IS NOT NULL
           AND pinned_form_version IS NOT NULL",
        rusqlite::params![encounter_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enrich_field_values_prefers_coded_primary_diagnosis_binding() {
        let mut values = HashMap::new();
        values.insert(
            "primary_diagnosis".to_string(),
            Value::String("headache".to_string()),
        );
        values.insert(
            "primary_diagnosis_system".to_string(),
            Value::String(rustyclinic_terminology::SNOMED_SYSTEM.to_string()),
        );
        values.insert(
            "primary_diagnosis_code".to_string(),
            Value::String("25064002".to_string()),
        );
        values.insert(
            "primary_diagnosis_display".to_string(),
            Value::String("Headache".to_string()),
        );

        let enriched = enrich_field_values(values);
        let primary = enriched
            .get("_diagnosis")
            .and_then(|value| value.get("primary"))
            .expect("primary diagnosis binding should exist");

        assert_eq!(
            primary.get("clinician_label").and_then(Value::as_str),
            Some("Headache")
        );
        assert_eq!(
            primary.get("local_value").and_then(Value::as_str),
            Some("headache")
        );
        assert_eq!(
            primary
                .get("snomed")
                .and_then(|coding| coding.get("code"))
                .and_then(Value::as_str),
            Some("25064002")
        );
        assert!(primary.get("icd11").is_none());
    }

    #[test]
    fn enrich_field_values_falls_back_to_legacy_binding_when_coded_fields_absent() {
        let mut values = HashMap::new();
        values.insert(
            "primary_diagnosis".to_string(),
            Value::String("hypertension".to_string()),
        );

        let enriched = enrich_field_values(values);
        let expected = serde_json::to_value(rustyclinic_terminology::diagnosis_binding(
            "hypertension",
            None,
        ))
        .expect("legacy binding should serialize");

        assert_eq!(
            enriched
                .get("_diagnosis")
                .and_then(|value| value.get("primary")),
            Some(&expected)
        );
    }

    #[test]
    fn enrich_field_values_falls_back_to_legacy_binding_when_coded_system_unknown() {
        let mut values = HashMap::new();
        values.insert(
            "primary_diagnosis".to_string(),
            Value::String("hypertension".to_string()),
        );
        values.insert(
            "primary_diagnosis_system".to_string(),
            Value::String("custom-system".to_string()),
        );
        values.insert(
            "primary_diagnosis_code".to_string(),
            Value::String("X1".to_string()),
        );
        values.insert(
            "primary_diagnosis_display".to_string(),
            Value::String("Custom diagnosis".to_string()),
        );

        let enriched = enrich_field_values(values);
        let primary = enriched
            .get("_diagnosis")
            .and_then(|value| value.get("primary"))
            .expect("legacy binding should be used");

        assert_eq!(
            primary
                .get("icd11")
                .and_then(|coding| coding.get("code"))
                .and_then(Value::as_str),
            Some("BA00")
        );
        assert_eq!(
            primary.get("clinician_label").and_then(Value::as_str),
            Some("Hypertension")
        );
    }

    #[test]
    fn parse_selected_lab_tests_supports_checkbox_common_tests() {
        let (expected_code, expected_name) = rustyclinic_clinical::lab::COMMON_TESTS[0];
        let mut fields = HashMap::new();
        fields.insert(format!("test_{expected_code}"), "on".to_string());

        let tests = parse_selected_lab_tests(&fields);

        assert_eq!(tests.len(), 1);
        assert_eq!(tests[0].test_code, expected_code);
        assert_eq!(tests[0].test_name, expected_name);
    }

    #[test]
    fn parse_selected_lab_tests_supports_dynamic_loinc_fields() {
        let mut fields = HashMap::new();
        fields.insert(
            "test_loinc_718-7".to_string(),
            "Hemoglobin [Mass/volume] in Blood".to_string(),
        );

        let tests = parse_selected_lab_tests(&fields);

        assert_eq!(tests.len(), 1);
        assert_eq!(tests[0].test_code, "718-7");
        assert_eq!(tests[0].test_name, "Hemoglobin [Mass/volume] in Blood");
    }

    #[test]
    fn parse_selected_lab_tests_dedupes_by_test_code() {
        let (shared_code, shared_name) = rustyclinic_clinical::lab::COMMON_TESTS[0];
        let mut fields = HashMap::new();
        fields.insert(format!("test_{shared_code}"), "on".to_string());
        fields.insert(
            format!("test_loinc_{shared_code}"),
            "Different display name".to_string(),
        );

        let tests = parse_selected_lab_tests(&fields);

        assert_eq!(tests.len(), 1);
        assert_eq!(tests[0].test_code, shared_code);
        assert_eq!(tests[0].test_name, shared_name);
    }

    #[test]
    fn parse_selected_lab_tests_returns_empty_when_none_selected() {
        let fields = HashMap::new();

        let tests = parse_selected_lab_tests(&fields);

        assert!(tests.is_empty());
    }

    #[test]
    fn resolve_medication_coding_returns_snomed_for_known_medication() {
        let fields = HashMap::new();
        let (system, code, display) = resolve_medication_coding(&fields, 0, "Amoxicillin");

        assert_eq!(system, Some("http://snomed.info/sct".to_string()));
        assert_eq!(code, Some("27658006".to_string()));
        assert_eq!(display, Some("Amoxicillin".to_string()));
    }

    #[test]
    fn resolve_medication_coding_returns_none_for_unknown_medication() {
        let fields = HashMap::new();
        let (system, code, display) = resolve_medication_coding(&fields, 0, "Unknown Drug");

        assert!(system.is_none());
        assert!(code.is_none());
        assert!(display.is_none());
    }

    #[test]
    fn resolve_medication_coding_prefers_posted_fields_over_terminology() {
        let mut fields = HashMap::new();
        fields.insert("med_system_0".to_string(), "http://custom.org".to_string());
        fields.insert("med_code_0".to_string(), "CUSTOM123".to_string());
        fields.insert(
            "med_display_0".to_string(),
            "Custom Amoxicillin".to_string(),
        );

        let (system, code, display) = resolve_medication_coding(&fields, 0, "Amoxicillin");

        assert_eq!(system, Some("http://custom.org".to_string()));
        assert_eq!(code, Some("CUSTOM123".to_string()));
        assert_eq!(display, Some("Custom Amoxicillin".to_string()));
    }

    #[test]
    fn resolve_medication_coding_requires_all_posted_fields() {
        let mut fields = HashMap::new();
        fields.insert("med_system_0".to_string(), "http://custom.org".to_string());
        // missing med_code_0 and med_display_0

        let (system, code, display) = resolve_medication_coding(&fields, 0, "Amoxicillin");

        assert_eq!(system, Some("http://snomed.info/sct".to_string()));
        assert_eq!(code, Some("27658006".to_string()));
        assert_eq!(display, Some("Amoxicillin".to_string()));
    }
}
