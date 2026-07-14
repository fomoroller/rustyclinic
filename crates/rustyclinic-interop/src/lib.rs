//! Minimal FHIR-facing interoperability layer.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use rustyclinic_terminology::{
    CodeSystem, has_imported_concepts, lab_test_binding, medication_binding, observation_binding,
    search_imported, search_imported_any, ucum_for_display,
};

const IMPORTED_TERMINOLOGY_SEARCH_LIMIT: usize = 50;

#[derive(Clone)]
pub struct InteropState {
    inner: Arc<InteropStateInner>,
}

struct InteropStateInner {
    db_path: String,
}

impl InteropState {
    pub fn new(db_path: String) -> Self {
        Self {
            inner: Arc::new(InteropStateInner { db_path }),
        }
    }
}

pub fn interop_router(state: InteropState) -> Router {
    Router::new()
        .route("/api/terminology/search/{system}", get(search_terminology))
        .route("/api/fhir/patients/{id}", get(get_fhir_patient))
        .route("/api/fhir/encounters/{id}", get(get_fhir_encounter_bundle))
        .with_state(state)
}

#[derive(serde::Deserialize)]
pub struct SearchParams {
    pub q: Option<String>,
}

pub async fn search_terminology(
    State(state): State<InteropState>,
    Path(system): Path<String>,
    axum::extract::Query(params): axum::extract::Query<SearchParams>,
) -> impl IntoResponse {
    let Some(system) = CodeSystem::parse(&system) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "unknown terminology system" })),
        );
    };

    let query = params.q.as_deref().unwrap_or_default();
    let results = search_terminology_results(&state.inner.db_path, system, query);
    (StatusCode::OK, Json(json!({ "results": results })))
}

fn search_terminology_results(
    db_path: &str,
    system: CodeSystem,
    query: &str,
) -> Vec<rustyclinic_terminology::TerminologyConcept> {
    let starter_results = rustyclinic_terminology::search(system, query);

    let conn = match rusqlite::Connection::open(db_path) {
        Ok(conn) => conn,
        Err(error) => {
            tracing::warn!(
                ?error,
                db_path,
                "terminology db open failed; using starter search"
            );
            return starter_results;
        }
    };

    let overlay_results = search_packaged_terminology_artifacts(&conn, system, query);

    let has_imports = match has_imported_concepts(&conn, system) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(
                ?error,
                "failed to check imported terminology concepts; using starter search"
            );
            return merge_terminology_results(overlay_results, starter_results);
        }
    };

    if !has_imports {
        let fallback_results =
            match search_imported_any(&conn, query, IMPORTED_TERMINOLOGY_SEARCH_LIMIT) {
                Ok(results) => results,
                Err(error) => {
                    tracing::warn!(?error, "fallback terminology search failed");
                    Vec::new()
                }
            };
        return merge_terminology_results(
            overlay_results,
            merge_terminology_results(starter_results, fallback_results),
        );
    }

    match search_imported(&conn, system, query, IMPORTED_TERMINOLOGY_SEARCH_LIMIT) {
        Ok(results) => merge_terminology_results(overlay_results, results),
        Err(error) => {
            tracing::warn!(
                ?error,
                "imported terminology search failed; using starter search"
            );
            merge_terminology_results(overlay_results, starter_results)
        }
    }
}

fn search_packaged_terminology_artifacts(
    conn: &rusqlite::Connection,
    system: CodeSystem,
    query: &str,
) -> Vec<rustyclinic_terminology::TerminologyConcept> {
    let mut stmt = match conn.prepare(
        "SELECT artifact_json
         FROM package_terminology_artifacts
         INNER JOIN installed_packages ON installed_packages.id = package_terminology_artifacts.package_row_id
         WHERE installed_packages.status = 'activated'
           AND package_terminology_artifacts.terminology_system IN (?1, ?2)
         ORDER BY installed_packages.activated_at DESC, package_terminology_artifacts.artifact_id ASC",
    ) {
        Ok(stmt) => stmt,
        Err(error) => {
            tracing::warn!(?error, "failed to prepare package terminology artifact query");
            return Vec::new();
        }
    };

    let rows = match stmt.query_map(
        rusqlite::params![system.canonical_url(), code_system_key(system)],
        |row| row.get::<_, String>(0),
    ) {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(?error, "failed to query package terminology artifacts");
            return Vec::new();
        }
    };

    let lowered = query.trim().to_ascii_lowercase();
    rows.flatten()
        .filter_map(|json| {
            match serde_json::from_str::<rustyclinic_terminology::TerminologyConcept>(&json) {
                Ok(concept) => Some(concept),
                Err(error) => {
                    tracing::warn!(?error, "failed to parse package terminology artifact");
                    None
                }
            }
        })
        .filter(|concept| {
            if lowered.is_empty() {
                return true;
            }
            concept.coding.code.to_ascii_lowercase().contains(&lowered)
                || concept
                    .coding
                    .display
                    .to_ascii_lowercase()
                    .contains(&lowered)
                || concept
                    .synonyms
                    .iter()
                    .any(|syn| syn.to_ascii_lowercase().contains(&lowered))
        })
        .collect()
}

fn merge_terminology_results(
    primary: Vec<rustyclinic_terminology::TerminologyConcept>,
    secondary: Vec<rustyclinic_terminology::TerminologyConcept>,
) -> Vec<rustyclinic_terminology::TerminologyConcept> {
    let mut merged = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for concept in primary.into_iter().chain(secondary) {
        let key = format!("{}|{}", concept.coding.system, concept.coding.code);
        if seen.insert(key) {
            merged.push(concept);
        }
    }

    merged
}

fn code_system_key(system: CodeSystem) -> &'static str {
    match system {
        CodeSystem::Icd11 => "icd11",
        CodeSystem::SnomedCt => "snomed",
        CodeSystem::Loinc => "loinc",
        CodeSystem::Ucum => "ucum",
    }
}

pub async fn get_fhir_patient(
    State(state): State<InteropState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let patient_id = match Uuid::parse_str(&id) {
        Ok(value) => value,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "invalid patient id" })),
            );
        }
    };

    let conn = match rusqlite::Connection::open(&state.inner.db_path) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("db error: {e}") })),
            );
        }
    };

    match patient_resource(&conn, patient_id) {
        Ok(Some(resource)) => (StatusCode::OK, Json(resource)),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "patient not found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("failed to build patient resource: {e}") })),
        ),
    }
}

pub async fn get_fhir_encounter_bundle(
    State(state): State<InteropState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let encounter_id = match Uuid::parse_str(&id) {
        Ok(value) => value,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "invalid encounter id" })),
            );
        }
    };

    let conn = match rusqlite::Connection::open(&state.inner.db_path) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("db error: {e}") })),
            );
        }
    };

    match encounter_bundle(&conn, encounter_id) {
        Ok(Some(bundle)) => (StatusCode::OK, Json(bundle)),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "encounter not found" })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("failed to build encounter bundle: {e}") })),
        ),
    }
}

fn patient_resource(
    conn: &rusqlite::Connection,
    patient_id: Uuid,
) -> rusqlite::Result<Option<Value>> {
    let result: rusqlite::Result<Value> = conn.query_row(
        "SELECT id, given_name, family_name, sex, date_of_birth, phone, national_id
         FROM patients WHERE id = ?1",
        rusqlite::params![patient_id.to_string()],
        |row| {
            let id: String = row.get(0)?;
            let given_name: String = row.get(1)?;
            let family_name: String = row.get(2)?;
            let sex: String = row.get(3)?;
            let date_of_birth: Option<String> = row.get(4)?;
            let phone: Option<String> = row.get(5)?;
            let national_id: Option<String> = row.get(6)?;

            let mut resource = json!({
                "resourceType": "Patient",
                "id": id,
                "name": [{
                    "use": "official",
                    "family": family_name,
                    "given": [given_name],
                }],
                "gender": fhir_gender(&sex),
            });

            if let Some(dob) = date_of_birth {
                resource["birthDate"] = Value::String(dob);
            }
            if let Some(phone) = phone {
                resource["telecom"] = json!([{ "system": "phone", "value": phone }]);
            }
            if let Some(national_id) = national_id {
                resource["identifier"] = json!([{
                    "system": "https://rustyclinic.example/identifiers/national-id",
                    "value": national_id,
                }]);
            }

            Ok(resource)
        },
    );

    match result {
        Ok(value) => Ok(Some(value)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

fn encounter_bundle(
    conn: &rusqlite::Connection,
    encounter_id: Uuid,
) -> rusqlite::Result<Option<Value>> {
    type EncounterRow = (
        String,
        String,
        String,
        String,
        Option<String>,
        String,
        Option<String>,
        String,
    );
    let encounter_row: rusqlite::Result<EncounterRow> =
        conn.query_row(
            "SELECT e.id, e.patient_id, e.provider_id, e.started_at, e.ended_at, e.status, e.queue_entry_id, e.visit_notes
             FROM encounters e WHERE e.id = ?1",
            rusqlite::params![encounter_id.to_string()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        );

    let (id, patient_id, provider_id, started_at, ended_at, status, queue_entry_id, visit_notes) =
        match encounter_row {
            Ok(row) => row,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(e) => return Err(e),
        };

    let patient_uuid = Uuid::parse_str(&patient_id).unwrap_or(encounter_id);
    let patient = patient_resource(conn, patient_uuid)?.unwrap_or_else(|| {
        json!({
            "resourceType": "Patient",
            "id": patient_id,
        })
    });

    let encounter_resource = json!({
        "resourceType": "Encounter",
        "id": id,
        "status": if status == "completed" { "finished" } else { "in-progress" },
        "subject": { "reference": format!("Patient/{patient_id}") },
        "participant": [{
            "individual": { "reference": format!("Practitioner/{provider_id}") }
        }],
        "period": {
            "start": started_at,
            "end": ended_at,
        },
    });

    let note_map: Map<String, Value> =
        serde_json::from_str::<Map<String, Value>>(&visit_notes).unwrap_or_default();

    let mut entries = vec![bundle_entry(patient), bundle_entry(encounter_resource)];

    for condition in condition_resources(&note_map, &patient_id, &id) {
        entries.push(bundle_entry(condition));
    }
    for obs in observation_resources(&note_map, &patient_id, &id) {
        entries.push(bundle_entry(obs));
    }
    for sr in service_request_resources(conn, encounter_id, &patient_id)? {
        entries.push(bundle_entry(sr));
    }
    for dr in diagnostic_report_resources(conn, encounter_id, &patient_id)? {
        entries.push(bundle_entry(dr));
    }
    for mr in medication_request_resources(conn, encounter_id, &patient_id)? {
        entries.push(bundle_entry(mr));
    }
    for md in medication_dispense_resources(conn, encounter_id, &patient_id)? {
        entries.push(bundle_entry(md));
    }

    let mut bundle = json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": entries,
    });

    if let Some(queue_entry_id) = queue_entry_id {
        bundle["identifier"] = json!({
            "system": "https://rustyclinic.example/encounter-queue-link",
            "value": queue_entry_id,
        });
    }

    Ok(Some(bundle))
}

fn condition_resources(
    note_map: &Map<String, Value>,
    patient_id: &str,
    encounter_id: &str,
) -> Vec<Value> {
    let mut resources = Vec::new();

    if let Some(primary) = note_map.get("_diagnosis").and_then(|v| v.get("primary")) {
        let code = build_codeable_concept(primary);
        let label = primary
            .get("clinician_label")
            .and_then(Value::as_str)
            .unwrap_or("Diagnosis");
        resources.push(json!({
            "resourceType": "Condition",
            "id": format!("{encounter_id}-condition-primary"),
            "clinicalStatus": {
                "coding": [{
                    "system": "http://terminology.hl7.org/CodeSystem/condition-clinical",
                    "code": "active",
                    "display": "Active",
                }]
            },
            "subject": { "reference": format!("Patient/{patient_id}") },
            "encounter": { "reference": format!("Encounter/{encounter_id}") },
            "code": code,
            "note": [{ "text": label }],
        }));
    }

    resources
}

fn observation_resources(
    note_map: &Map<String, Value>,
    patient_id: &str,
    encounter_id: &str,
) -> Vec<Value> {
    let mut observations = Vec::new();

    for (key, value) in note_map {
        let Some(binding) = observation_binding(key) else {
            continue;
        };
        let Some(observation_value) = build_observation_value(value, binding.ucum.as_ref()) else {
            continue;
        };

        observations.push(json!({
            "resourceType": "Observation",
            "id": format!("{encounter_id}-{key}"),
            "status": "final",
            "subject": { "reference": format!("Patient/{patient_id}") },
            "encounter": { "reference": format!("Encounter/{encounter_id}") },
            "code": {
                "coding": [{
                    "system": binding.loinc.system,
                    "code": binding.loinc.code,
                    "display": binding.loinc.display,
                }],
                "text": binding.label,
            },
            "valueQuantity": observation_value,
        }));
    }

    observations
}

fn service_request_resources(
    conn: &rusqlite::Connection,
    encounter_id: Uuid,
    patient_id: &str,
) -> rusqlite::Result<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT id, status, priority, specimen_type, notes FROM lab_orders WHERE encounter_id = ?1",
    )?;
    let rows = stmt.query_map(rusqlite::params![encounter_id.to_string()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;

    let mut resources = Vec::new();
    for row in rows {
        let (order_id, status, priority, specimen_type, notes) = row?;
        let tests = load_lab_tests(conn, &order_id)?;
        let contained: Vec<Value> = tests
            .iter()
            .filter_map(|test| {
                let binding = lab_test_binding(&test.test_code);
                binding.loinc.map(|loinc| {
                    let display = loinc_display_for_test(test, &loinc);
                    json!({
                        "coding": [{
                            "system": loinc.system,
                            "code": loinc.code,
                            "display": display,
                        }],
                        "text": test.test_name,
                    })
                })
            })
            .collect();

        let mut resource = json!({
            "resourceType": "ServiceRequest",
            "id": order_id,
            "status": fhir_service_request_status(&status),
            "intent": "order",
            "priority": priority.to_ascii_lowercase(),
            "subject": { "reference": format!("Patient/{patient_id}") },
            "encounter": { "reference": format!("Encounter/{encounter_id}") },
            "code": {
                "text": "Laboratory order"
            },
        });

        if !contained.is_empty() {
            resource["orderDetail"] = Value::Array(contained);
        }
        if let Some(specimen_type) = specimen_type {
            resource["specimen"] = json!([{ "display": specimen_type }]);
        }
        if let Some(notes) = notes {
            resource["note"] = json!([{ "text": notes }]);
        }

        resources.push(resource);
    }

    Ok(resources)
}

fn diagnostic_report_resources(
    conn: &rusqlite::Connection,
    encounter_id: Uuid,
    patient_id: &str,
) -> rusqlite::Result<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT id, status, resulted_at, verified_at FROM lab_orders
         WHERE encounter_id = ?1 AND status IN ('resulted', 'verified', 'amended')",
    )?;
    let rows = stmt.query_map(rusqlite::params![encounter_id.to_string()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;

    let mut resources = Vec::new();
    for row in rows {
        let (order_id, status, resulted_at, verified_at) = row?;
        let tests = load_lab_tests(conn, &order_id)?;
        let results: Vec<Value> = tests
            .iter()
            .map(|test| {
                let binding = lab_test_binding(&test.test_code);
                let mut result = json!({
                    "code": {
                        "text": test.test_name
                    }
                });
                if let Some(loinc) = binding.loinc {
                    let display = loinc_display_for_test(test, &loinc);
                    result["code"] = json!({
                        "coding": [{
                            "system": loinc.system,
                            "code": loinc.code,
                            "display": display,
                        }],
                        "text": test.test_name
                    });
                }
                if let Some(value) = test.result_value {
                    result["valueQuantity"] = build_numeric_quantity(value, test.unit.as_deref());
                } else if let Some(text) = &test.result {
                    result["valueString"] = Value::String(text.clone());
                }
                if let Some(range) = &test.reference_range {
                    result["referenceRange"] = json!([{ "text": range }]);
                }
                result["interpretation"] = json!([{
                    "text": if test.is_abnormal { "abnormal" } else { "normal" }
                }]);
                result
            })
            .collect();

        resources.push(json!({
            "resourceType": "DiagnosticReport",
            "id": format!("{order_id}-report"),
            "status": if status == "verified" { "final" } else { "preliminary" },
            "subject": { "reference": format!("Patient/{patient_id}") },
            "encounter": { "reference": format!("Encounter/{encounter_id}") },
            "issued": verified_at.or(resulted_at),
            "result": results,
        }));
    }

    Ok(resources)
}

fn medication_request_resources(
    conn: &rusqlite::Connection,
    encounter_id: Uuid,
    patient_id: &str,
) -> rusqlite::Result<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT id, status, priority, notes FROM medication_dispenses WHERE encounter_id = ?1",
    )?;
    let rows = stmt.query_map(rusqlite::params![encounter_id.to_string()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;

    let mut resources = Vec::new();
    for row in rows {
        let (dispense_id, status, priority, notes) = row?;
        let items = load_dispense_items(conn, &dispense_id)?;
        for (index, item) in items.iter().enumerate() {
            let med = medication_codeable_concept(item);

            let mut resource = json!({
                "resourceType": "MedicationRequest",
                "id": format!("{dispense_id}-rx-{index}"),
                "status": if status == "voided" { "stopped" } else { "active" },
                "intent": "order",
                "priority": priority.to_ascii_lowercase(),
                "subject": { "reference": format!("Patient/{patient_id}") },
                "encounter": { "reference": format!("Encounter/{encounter_id}") },
                "medicationCodeableConcept": med,
                "dosageInstruction": [{
                    "text": format!("{} {}, {}", item.dosage, item.frequency, item.duration)
                }],
                "dispenseRequest": {
                    "quantity": { "value": item.quantity }
                }
            });
            if let Some(notes) = &notes {
                resource["note"] = json!([{ "text": notes }]);
            }
            resources.push(resource);
        }
    }
    Ok(resources)
}

fn medication_dispense_resources(
    conn: &rusqlite::Connection,
    encounter_id: Uuid,
    patient_id: &str,
) -> rusqlite::Result<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT id, status, dispensed_at FROM medication_dispenses
         WHERE encounter_id = ?1 AND status IN ('dispensed', 'partial', 'returned')",
    )?;
    let rows = stmt.query_map(rusqlite::params![encounter_id.to_string()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;

    let mut resources = Vec::new();
    for row in rows {
        let (dispense_id, status, dispensed_at) = row?;
        let items = load_dispense_items(conn, &dispense_id)?;
        for (index, item) in items.iter().enumerate() {
            resources.push(json!({
                "resourceType": "MedicationDispense",
                "id": format!("{dispense_id}-dispense-{index}"),
                "status": if status == "returned" { "stopped" } else { "completed" },
                "subject": { "reference": format!("Patient/{patient_id}") },
                "whenHandedOver": dispensed_at,
                "medicationCodeableConcept": medication_codeable_concept(item),
                "quantity": {
                    "value": item.dispensed_quantity.unwrap_or(item.quantity),
                }
            }));
        }
    }
    Ok(resources)
}

#[derive(Debug)]
struct LabResultRow {
    test_code: String,
    test_name: String,
    result: Option<String>,
    result_value: Option<f64>,
    unit: Option<String>,
    reference_range: Option<String>,
    is_abnormal: bool,
}

#[derive(Debug)]
struct DispenseItemRow {
    medication_name: String,
    medication_system: Option<String>,
    medication_code: Option<String>,
    medication_display: Option<String>,
    dosage: String,
    frequency: String,
    duration: String,
    quantity: u32,
    dispensed_quantity: Option<u32>,
}

fn loinc_display_for_test(test: &LabResultRow, loinc: &rustyclinic_terminology::Coding) -> String {
    if test.test_code == loinc.code {
        return test.test_name.clone();
    }

    loinc.display.clone()
}

fn load_lab_tests(
    conn: &rusqlite::Connection,
    order_id: &str,
) -> rusqlite::Result<Vec<LabResultRow>> {
    let mut stmt = conn.prepare(
        "SELECT test_code, test_name, result, result_value, unit, reference_range, is_abnormal
         FROM lab_tests WHERE order_id = ?1",
    )?;
    let rows = stmt.query_map(rusqlite::params![order_id], |row| {
        Ok(LabResultRow {
            test_code: row.get(0)?,
            test_name: row.get(1)?,
            result: row.get(2)?,
            result_value: row.get(3)?,
            unit: row.get(4)?,
            reference_range: row.get(5)?,
            is_abnormal: row.get::<_, i64>(6)? != 0,
        })
    })?;
    rows.collect()
}

fn load_dispense_items(
    conn: &rusqlite::Connection,
    dispense_id: &str,
) -> rusqlite::Result<Vec<DispenseItemRow>> {
    let mut stmt = conn.prepare(
        "SELECT medication_name, medication_system, medication_code, medication_display,
                dosage, frequency, duration, quantity, dispensed_quantity
         FROM dispense_items WHERE dispense_id = ?1",
    )?;
    let rows = stmt.query_map(rusqlite::params![dispense_id], |row| {
        Ok(DispenseItemRow {
            medication_name: row.get(0)?,
            medication_system: row.get(1)?,
            medication_code: row.get(2)?,
            medication_display: row.get(3)?,
            dosage: row.get(4)?,
            frequency: row.get(5)?,
            duration: row.get(6)?,
            quantity: row.get(7)?,
            dispensed_quantity: row.get(8)?,
        })
    })?;
    rows.collect()
}

fn medication_codeable_concept(item: &DispenseItemRow) -> Value {
    if let (Some(system), Some(code), Some(display)) = (
        item.medication_system.as_deref(),
        item.medication_code.as_deref(),
        item.medication_display.as_deref(),
    ) {
        return json!({
            "coding": [{
                "system": system,
                "code": code,
                "display": display,
            }],
            "text": item.medication_name,
        });
    }

    let binding = medication_binding(&item.medication_name);
    if let Some(snomed) = binding.snomed {
        return json!({
            "coding": [{
                "system": snomed.system,
                "code": snomed.code,
                "display": snomed.display,
            }],
            "text": item.medication_name,
        });
    }

    json!({ "text": item.medication_name })
}

fn bundle_entry(resource: Value) -> Value {
    json!({ "resource": resource })
}

fn fhir_gender(value: &str) -> &str {
    match value.to_ascii_lowercase().as_str() {
        "male" | "m" => "male",
        "female" | "f" => "female",
        _ => "unknown",
    }
}

fn fhir_service_request_status(status: &str) -> &str {
    match status {
        "cancelled" => "revoked",
        "verified" | "resulted" => "completed",
        _ => "active",
    }
}

fn build_codeable_concept(primary: &Value) -> Value {
    let mut codings = Vec::new();
    if let Some(icd11) = primary.get("icd11").and_then(coding_value) {
        codings.push(icd11);
    }
    if let Some(snomed) = primary.get("snomed").and_then(coding_value) {
        codings.push(snomed);
    }

    json!({
        "coding": codings,
        "text": primary
            .get("clinician_label")
            .and_then(Value::as_str)
            .unwrap_or("Diagnosis"),
    })
}

fn coding_value(value: &Value) -> Option<Value> {
    Some(json!({
        "system": value.get("system")?.as_str()?,
        "code": value.get("code")?.as_str()?,
        "display": value.get("display")?.as_str()?,
    }))
}

fn build_observation_value(
    value: &Value,
    ucum: Option<&rustyclinic_terminology::Coding>,
) -> Option<Value> {
    match value {
        Value::Number(number) => {
            let numeric = number.as_f64()?;
            Some(build_numeric_quantity(
                numeric,
                ucum.map(|u| u.display.as_str()),
            ))
        }
        Value::String(text) => Some(json!({ "value": text })),
        _ => None,
    }
}

fn build_numeric_quantity(value: f64, unit_display: Option<&str>) -> Value {
    if let Some(unit_display) = unit_display
        && let Some(ucum) = ucum_for_display(unit_display)
    {
        return json!({
            "value": value,
            "unit": ucum.display,
            "system": ucum.system,
            "code": ucum.code,
        });
    }

    json!({ "value": value })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    #[test]
    fn condition_uses_coded_concept() {
        let note_map = serde_json::from_value::<Map<String, Value>>(json!({
            "_diagnosis": {
                "primary": rustyclinic_terminology::diagnosis_binding("hypertension", None),
            }
        }))
        .expect("map");
        let conditions = condition_resources(&note_map, "patient-1", "enc-1");
        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0]["code"]["coding"][0]["code"], "BA00");
    }

    #[test]
    fn vitals_become_observations() {
        let note_map = serde_json::from_value::<Map<String, Value>>(json!({
            "weight_kg": 72.5,
            "pulse_rate": 88,
        }))
        .expect("map");
        let observations = observation_resources(&note_map, "patient-1", "enc-1");
        assert_eq!(observations.len(), 2);
        assert_eq!(observations[0]["resourceType"], "Observation");
    }

    #[test]
    fn terminology_search_uses_imported_concepts_when_present() {
        let db_path = temp_db_path("imported-concepts");
        let conn = rusqlite::Connection::open(&db_path).expect("open temp db");
        rustyclinic_terminology::import::ensure_schema(&conn).expect("ensure schema");
        conn.execute(
            "INSERT INTO terminology_concepts (system, code, display, active, properties, imported_at)
             VALUES (?1, ?2, ?3, 1, '{}', ?4)",
            params![
                CodeSystem::Loinc.canonical_url(),
                "99999-9",
                "Imported Custom Chemistry Test",
                "2026-01-01T00:00:00Z"
            ],
        )
        .expect("insert imported concept");
        drop(conn);

        let results = search_terminology_results(&db_path, CodeSystem::Loinc, "custom chemistry");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].coding.code, "99999-9");
        assert_eq!(results[0].coding.display, "Imported Custom Chemistry Test");

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn terminology_search_falls_back_when_system_has_no_imported_concepts() {
        let db_path = temp_db_path("fallback-no-imports");
        let conn = rusqlite::Connection::open(&db_path).expect("open temp db");
        rustyclinic_terminology::import::ensure_schema(&conn).expect("ensure schema");
        conn.execute(
            "INSERT INTO terminology_concepts (system, code, display, active, properties, imported_at)
             VALUES (?1, ?2, ?3, 1, '{}', ?4)",
            params![
                CodeSystem::SnomedCt.canonical_url(),
                "123456",
                "Some SNOMED Concept",
                "2026-01-01T00:00:00Z"
            ],
        )
        .expect("insert non-loinc concept");
        drop(conn);

        let actual = search_terminology_results(&db_path, CodeSystem::Loinc, "glucose");
        let expected = rustyclinic_terminology::search(CodeSystem::Loinc, "glucose");
        assert_eq!(
            serde_json::to_value(actual).expect("serialize actual"),
            serde_json::to_value(expected).expect("serialize expected")
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn terminology_search_uses_imported_cross_system_fallback_for_empty_requested_catalog() {
        let db_path = temp_db_path("fallback-cross-system");
        let conn = rusqlite::Connection::open(&db_path).expect("open temp db");
        rustyclinic_terminology::import::ensure_schema(&conn).expect("ensure schema");
        conn.execute(
            "INSERT INTO terminology_concepts (system, code, display, active, properties, imported_at)
             VALUES (?1, ?2, ?3, 1, '{}', ?4)",
            params![
                "http://dicom.nema.org/resources/ontology/DCM",
                "112054",
                "Secondary pulmonary lobule",
                "2026-01-01T00:00:00Z"
            ],
        )
        .expect("insert pulmonary concept");
        drop(conn);

        let results = search_terminology_results(&db_path, CodeSystem::Icd11, "pulmonary");
        assert!(results.iter().any(|result| {
            result.coding.system == "http://dicom.nema.org/resources/ontology/DCM"
                && result.coding.code == "112054"
                && result.coding.display == "Secondary pulmonary lobule"
        }));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn terminology_search_includes_activated_package_overlay_results() {
        let db_path = temp_db_path("package-overlay-active");
        let conn = rusqlite::Connection::open(&db_path).expect("open temp db");
        create_package_terminology_tables(&conn);

        let package_row_id = Uuid::now_v7();
        let facility_id = Uuid::now_v7();
        let installed_by = Uuid::now_v7();
        let now = "2026-01-01T00:00:00Z";
        let manifest = serde_json::json!({
            "package_id": "loinc-overlay",
            "package_type": "terminology",
            "version": "1.0.0",
            "compatible_versions": "*",
            "dependencies": [],
            "effective_start": null,
            "effective_end": null,
            "scope": "facility",
            "checksum": "abc",
            "localization_coverage": []
        })
        .to_string();

        conn.execute(
            "INSERT INTO installed_packages (id, facility_id, package_id, package_type, version, status, manifest, installed_at, activated_at, rolled_back_at, installed_by, version_num)
             VALUES (?1, ?2, 'loinc-overlay', 'terminology', '1.0.0', 'activated', ?3, ?4, ?4, NULL, ?5, 1)",
            params![package_row_id.to_string(), facility_id.to_string(), manifest, now, installed_by.to_string()],
        )
        .expect("insert activated package");
        conn.execute(
            "INSERT INTO package_terminology_artifacts (package_row_id, artifact_id, terminology_system, artifact_type, artifact_json)
             VALUES (?1, 'artifact-1', 'http://loinc.org', 'concept', ?2)",
            params![
                package_row_id.to_string(),
                serde_json::json!({
                    "coding": {
                        "system": "http://loinc.org",
                        "code": "LPK-1",
                        "display": "Packaged Glucose Panel"
                    },
                    "synonyms": ["overlay glucose"]
                })
                .to_string()
            ],
        )
        .expect("insert terminology artifact");
        drop(conn);

        let results = search_terminology_results(&db_path, CodeSystem::Loinc, "overlay glucose");
        assert!(results.iter().any(|result| result.coding.code == "LPK-1"));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn terminology_search_ignores_non_activated_package_overlay_results() {
        let db_path = temp_db_path("package-overlay-staged");
        let conn = rusqlite::Connection::open(&db_path).expect("open temp db");
        create_package_terminology_tables(&conn);

        let package_row_id = Uuid::now_v7();
        let facility_id = Uuid::now_v7();
        let installed_by = Uuid::now_v7();
        let now = "2026-01-01T00:00:00Z";
        let manifest = serde_json::json!({
            "package_id": "loinc-overlay",
            "package_type": "terminology",
            "version": "1.0.0",
            "compatible_versions": "*",
            "dependencies": [],
            "effective_start": null,
            "effective_end": null,
            "scope": "facility",
            "checksum": "abc",
            "localization_coverage": []
        })
        .to_string();

        conn.execute(
            "INSERT INTO installed_packages (id, facility_id, package_id, package_type, version, status, manifest, installed_at, activated_at, rolled_back_at, installed_by, version_num)
             VALUES (?1, ?2, 'loinc-overlay', 'terminology', '1.0.0', 'staged', ?3, ?4, NULL, NULL, ?5, 1)",
            params![package_row_id.to_string(), facility_id.to_string(), manifest, now, installed_by.to_string()],
        )
        .expect("insert staged package");
        conn.execute(
            "INSERT INTO package_terminology_artifacts (package_row_id, artifact_id, terminology_system, artifact_type, artifact_json)
             VALUES (?1, 'artifact-1', 'http://loinc.org', 'concept', ?2)",
            params![
                package_row_id.to_string(),
                serde_json::json!({
                    "coding": {
                        "system": "http://loinc.org",
                        "code": "LPK-2",
                        "display": "Hidden Packaged Glucose Panel"
                    },
                    "synonyms": ["hidden overlay"]
                })
                .to_string()
            ],
        )
        .expect("insert terminology artifact");
        drop(conn);

        let results = search_terminology_results(&db_path, CodeSystem::Loinc, "hidden overlay");
        assert!(!results.iter().any(|result| result.coding.code == "LPK-2"));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn medication_request_prefers_persisted_medication_coding() {
        let conn = rusqlite::Connection::open_in_memory().expect("open db");
        create_medication_export_tables(&conn);

        let encounter_id = Uuid::parse_str("44444444-4444-4444-4444-444444444444").expect("uuid");
        conn.execute(
            "INSERT INTO medication_dispenses (id, encounter_id, status, priority, notes, dispensed_at)
             VALUES (?1, ?2, 'prepared', 'routine', NULL, NULL)",
            params!["dispense-1", encounter_id.to_string()],
        )
        .expect("insert dispense");
        conn.execute(
            "INSERT INTO dispense_items
             (dispense_id, medication_name, dosage, frequency, duration, quantity, dispensed_quantity,
              medication_system, medication_code, medication_display)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8, ?9)",
            params![
                "dispense-1",
                "Amoxicillin",
                "500mg",
                "3x daily",
                "7 days",
                21,
                "http://example.org/meds",
                "RX-123",
                "Custom Amoxicillin"
            ],
        )
        .expect("insert item");

        let resources =
            medication_request_resources(&conn, encounter_id, "patient-1").expect("resources");
        assert_eq!(resources.len(), 1);
        let coding = &resources[0]["medicationCodeableConcept"]["coding"][0];
        assert_eq!(coding["system"], "http://example.org/meds");
        assert_eq!(coding["code"], "RX-123");
        assert_eq!(coding["display"], "Custom Amoxicillin");
    }

    fn create_package_terminology_tables(conn: &rusqlite::Connection) {
        conn.execute_batch(
            "CREATE TABLE installed_packages (
                 id TEXT PRIMARY KEY,
                 facility_id TEXT NOT NULL,
                 package_id TEXT NOT NULL,
                 package_type TEXT NOT NULL,
                 version TEXT NOT NULL,
                 status TEXT NOT NULL,
                 manifest TEXT NOT NULL,
                 installed_at TEXT NOT NULL,
                 activated_at TEXT,
                 rolled_back_at TEXT,
                 installed_by TEXT NOT NULL,
                 version_num INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE package_terminology_artifacts (
                 package_row_id TEXT NOT NULL,
                 artifact_id TEXT NOT NULL,
                 terminology_system TEXT NOT NULL,
                 artifact_type TEXT NOT NULL,
                 artifact_json TEXT NOT NULL,
                 PRIMARY KEY (package_row_id, artifact_id)
             );",
        )
        .expect("create package terminology tables");
    }

    #[test]
    fn medication_request_falls_back_to_legacy_binding_without_persisted_coding() {
        let conn = rusqlite::Connection::open_in_memory().expect("open db");
        create_medication_export_tables(&conn);

        let encounter_id = Uuid::parse_str("55555555-5555-5555-5555-555555555555").expect("uuid");
        conn.execute(
            "INSERT INTO medication_dispenses (id, encounter_id, status, priority, notes, dispensed_at)
             VALUES (?1, ?2, 'prepared', 'routine', NULL, NULL)",
            params!["dispense-1", encounter_id.to_string()],
        )
        .expect("insert dispense");
        conn.execute(
            "INSERT INTO dispense_items
             (dispense_id, medication_name, dosage, frequency, duration, quantity, dispensed_quantity,
              medication_system, medication_code, medication_display)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL, NULL, NULL)",
            params!["dispense-1", "Paracetamol", "500mg", "4x daily", "3 days", 12],
        )
        .expect("insert item");

        let resources =
            medication_request_resources(&conn, encounter_id, "patient-1").expect("resources");
        assert_eq!(resources.len(), 1);
        let coding = &resources[0]["medicationCodeableConcept"]["coding"][0];
        assert_eq!(coding["system"], "http://snomed.info/sct");
        assert_eq!(coding["code"], "387517004");
        assert_eq!(coding["display"], "Paracetamol");
    }

    #[test]
    fn medication_dispense_prefers_persisted_medication_coding() {
        let conn = rusqlite::Connection::open_in_memory().expect("open db");
        create_medication_export_tables(&conn);

        let encounter_id = Uuid::parse_str("66666666-6666-6666-6666-666666666666").expect("uuid");
        conn.execute(
            "INSERT INTO medication_dispenses (id, encounter_id, status, priority, notes, dispensed_at)
             VALUES (?1, ?2, 'dispensed', 'routine', NULL, '2026-01-01T00:00:00Z')",
            params!["dispense-1", encounter_id.to_string()],
        )
        .expect("insert dispense");
        conn.execute(
            "INSERT INTO dispense_items
             (dispense_id, medication_name, dosage, frequency, duration, quantity, dispensed_quantity,
              medication_system, medication_code, medication_display)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8, ?9)",
            params![
                "dispense-1",
                "Amoxicillin",
                "500mg",
                "3x daily",
                "7 days",
                21,
                "http://example.org/meds",
                "RX-123",
                "Custom Amoxicillin"
            ],
        )
        .expect("insert item");

        let resources =
            medication_dispense_resources(&conn, encounter_id, "patient-1").expect("resources");
        assert_eq!(resources.len(), 1);
        let coding = &resources[0]["medicationCodeableConcept"]["coding"][0];
        assert_eq!(coding["system"], "http://example.org/meds");
        assert_eq!(coding["code"], "RX-123");
        assert_eq!(coding["display"], "Custom Amoxicillin");
    }

    #[test]
    fn service_request_order_detail_uses_raw_loinc_code_and_test_name_display() {
        let conn = rusqlite::Connection::open_in_memory().expect("open db");
        create_lab_export_tables(&conn);

        let encounter_id = Uuid::parse_str("11111111-1111-1111-1111-111111111111").expect("uuid");
        conn.execute(
            "INSERT INTO lab_orders (id, encounter_id, status, priority, specimen_type, notes)
             VALUES (?1, ?2, 'ordered', 'routine', NULL, NULL)",
            params!["order-1", encounter_id.to_string()],
        )
        .expect("insert order");
        conn.execute(
            "INSERT INTO lab_tests (order_id, test_code, test_name, result, result_value, unit, reference_range, is_abnormal)
             VALUES (?1, ?2, ?3, NULL, NULL, NULL, NULL, 0)",
            params!["order-1", "57021-8", "Complete Blood Count"],
        )
        .expect("insert test");

        let resources =
            service_request_resources(&conn, encounter_id, "patient-1").expect("resources");
        assert_eq!(resources.len(), 1);
        let coding = &resources[0]["orderDetail"][0]["coding"][0];
        assert_eq!(coding["system"], "http://loinc.org");
        assert_eq!(coding["code"], "57021-8");
        assert_eq!(coding["display"], "Complete Blood Count");
        assert_eq!(
            resources[0]["orderDetail"][0]["text"],
            "Complete Blood Count"
        );
    }

    #[test]
    fn diagnostic_report_result_uses_raw_loinc_code_and_test_name_display() {
        let conn = rusqlite::Connection::open_in_memory().expect("open db");
        create_lab_export_tables(&conn);

        let encounter_id = Uuid::parse_str("22222222-2222-2222-2222-222222222222").expect("uuid");
        conn.execute(
            "INSERT INTO lab_orders (id, encounter_id, status, priority, specimen_type, notes, resulted_at)
             VALUES (?1, ?2, 'resulted', 'routine', NULL, NULL, '2026-01-01T00:00:00Z')",
            params!["order-1", encounter_id.to_string()],
        )
        .expect("insert order");
        conn.execute(
            "INSERT INTO lab_tests (order_id, test_code, test_name, result, result_value, unit, reference_range, is_abnormal)
             VALUES (?1, ?2, ?3, 'normal', NULL, NULL, NULL, 0)",
            params!["order-1", "57021-8", "Complete Blood Count"],
        )
        .expect("insert test");

        let reports =
            diagnostic_report_resources(&conn, encounter_id, "patient-1").expect("reports");
        assert_eq!(reports.len(), 1);
        let coding = &reports[0]["result"][0]["code"]["coding"][0];
        assert_eq!(coding["system"], "http://loinc.org");
        assert_eq!(coding["code"], "57021-8");
        assert_eq!(coding["display"], "Complete Blood Count");
        assert_eq!(
            reports[0]["result"][0]["code"]["text"],
            "Complete Blood Count"
        );
    }

    #[test]
    fn local_lab_code_keeps_bound_loinc_display() {
        let conn = rusqlite::Connection::open_in_memory().expect("open db");
        create_lab_export_tables(&conn);

        let encounter_id = Uuid::parse_str("33333333-3333-3333-3333-333333333333").expect("uuid");
        conn.execute(
            "INSERT INTO lab_orders (id, encounter_id, status, priority, specimen_type, notes)
             VALUES (?1, ?2, 'ordered', 'routine', NULL, NULL)",
            params!["order-1", encounter_id.to_string()],
        )
        .expect("insert order");
        conn.execute(
            "INSERT INTO lab_tests (order_id, test_code, test_name, result, result_value, unit, reference_range, is_abnormal)
             VALUES (?1, ?2, ?3, NULL, NULL, NULL, NULL, 0)",
            params!["order-1", "cbc", "Complete Blood Count"],
        )
        .expect("insert test");

        let resources =
            service_request_resources(&conn, encounter_id, "patient-1").expect("resources");
        let coding = &resources[0]["orderDetail"][0]["coding"][0];
        assert_eq!(coding["code"], "57021-8");
        assert_eq!(coding["display"], "CBC W Auto Differential panel");
    }

    fn create_lab_export_tables(conn: &rusqlite::Connection) {
        conn.execute_batch(
            "CREATE TABLE lab_orders (
                id TEXT PRIMARY KEY,
                encounter_id TEXT NOT NULL,
                status TEXT NOT NULL,
                priority TEXT NOT NULL,
                specimen_type TEXT,
                notes TEXT,
                resulted_at TEXT,
                verified_at TEXT
            );
            CREATE TABLE lab_tests (
                order_id TEXT NOT NULL,
                test_code TEXT NOT NULL,
                test_name TEXT NOT NULL,
                result TEXT,
                result_value REAL,
                unit TEXT,
                reference_range TEXT,
                is_abnormal INTEGER NOT NULL
            );",
        )
        .expect("create lab tables");
    }

    fn create_medication_export_tables(conn: &rusqlite::Connection) {
        conn.execute_batch(
            "CREATE TABLE medication_dispenses (
                id TEXT PRIMARY KEY,
                encounter_id TEXT NOT NULL,
                status TEXT NOT NULL,
                priority TEXT NOT NULL,
                notes TEXT,
                dispensed_at TEXT
            );
            CREATE TABLE dispense_items (
                dispense_id TEXT NOT NULL,
                medication_name TEXT NOT NULL,
                dosage TEXT NOT NULL,
                frequency TEXT NOT NULL,
                duration TEXT NOT NULL,
                quantity INTEGER NOT NULL,
                dispensed_quantity INTEGER,
                medication_system TEXT,
                medication_code TEXT,
                medication_display TEXT
            );",
        )
        .expect("create medication tables");
    }

    fn temp_db_path(label: &str) -> String {
        let mut path = std::env::temp_dir();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        path.push(format!("rustyclinic-interop-{label}-{nonce}.sqlite"));
        path.to_string_lossy().into_owned()
    }
}
