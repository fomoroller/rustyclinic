//! Lab workflow routes — queue view and results entry.

use axum::Form;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;
use uuid::Uuid;

use crate::WebAppState;
use crate::middleware::session::WebSession;
use crate::routes::patients::{lookup_patient_name, lookup_user_name};
use crate::templates::{LabOrderView, LabQueuePage, LabResultsPage, LabTestView};

/// Lab queue page — shows pending lab orders.
pub async fn queue_page(
    State(state): State<WebAppState>,
    session: WebSession,
) -> impl IntoResponse {
    let (display_name, initials) = lookup_user_name(&state, session.session.user_id);
    let facility_id = session.session.facility_id;

    let orders = load_lab_orders(&state, facility_id);

    let page = LabQueuePage {
        active_nav: "lab".to_string(),
        display_name,
        initials,
        flash_success: None,
        flash_error: None,
        orders,
    };
    Html(page.to_string())
}

/// Lab results entry page for a specific order.
pub async fn results_page(
    State(state): State<WebAppState>,
    session: WebSession,
    Path(order_id): Path<String>,
) -> Response {
    let order_uuid = match Uuid::parse_str(&order_id) {
        Ok(u) => u,
        Err(_) => return Redirect::to("/web/lab").into_response(),
    };

    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/web/lab").into_response(),
    };

    let (display_name, initials) = lookup_user_name(&state, session.session.user_id);

    // Load lab order
    let lab_order_repo = rustyclinic_db::sqlite::lab_repo::SqliteLabOrderRepo::new(&conn);
    use rustyclinic_clinical::lab::LabOrderRepo;
    let order = match lab_order_repo.find_by_id(order_uuid) {
        Ok(Some(o)) => o,
        _ => return Redirect::to("/web/lab").into_response(),
    };

    // Find the queue entry for this order's encounter in the lab department
    let queue_entry_id = conn
        .query_row(
            "SELECT id FROM queue_entries WHERE encounter_id = ?1 AND department = 'lab' AND status NOT IN ('completed', 'cancelled') LIMIT 1",
            rusqlite::params![order.encounter_id.to_string()],
            |row| {
                let id: String = row.get(0)?;
                Ok(id)
            },
        )
        .unwrap_or_default();

    // Load patient name
    let patient_name = lookup_patient_name(&conn, session.session.facility_id, order.patient_id);

    // Load lab test results
    let lab_test_repo = rustyclinic_db::sqlite::lab_repo::SqliteLabTestRepo::new(&conn);
    use rustyclinic_clinical::lab::LabTestRepo;
    let tests = lab_test_repo.find_by_order(order_uuid).unwrap_or_default();

    let specimen_type = order.specimen_type.clone().unwrap_or_default();

    let test_views: Vec<LabTestView> = tests
        .iter()
        .map(|t| LabTestView {
            test_code: t.test_code.clone(),
            test_name: t.test_name.clone(),
            result: t.result.clone().unwrap_or_default(),
            result_value: t.result_value.map(|v| v.to_string()).unwrap_or_default(),
            unit: t.unit.clone().unwrap_or_default(),
            reference_range: t.reference_range.clone().unwrap_or_default(),
            is_abnormal: t.is_abnormal,
        })
        .collect();

    let page = LabResultsPage {
        active_nav: "lab".to_string(),
        display_name,
        initials,
        flash_success: None,
        flash_error: None,
        order_id: order_id.clone(),
        queue_entry_id,
        patient_name,
        specimen_type,
        tests: test_views,
    };
    Html(page.to_string()).into_response()
}

#[derive(Deserialize)]
pub struct LabResultForm {
    pub order_id: String,
    pub queue_entry_id: String,
    #[serde(flatten)]
    pub fields: std::collections::HashMap<String, String>,
}

/// Submit lab results and complete the order.
pub async fn submit_results(
    State(state): State<WebAppState>,
    session: WebSession,
    Form(form): Form<LabResultForm>,
) -> Response {
    let order_id = match Uuid::parse_str(&form.order_id) {
        Ok(u) => u,
        Err(_) => return Redirect::to("/web/lab").into_response(),
    };
    let queue_entry_id = match Uuid::parse_str(&form.queue_entry_id) {
        Ok(u) => u,
        Err(_) => return Redirect::to("/web/lab").into_response(),
    };

    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/web/lab").into_response(),
    };

    let actor = session.session.to_actor_context();
    let now = chrono::Utc::now();

    // Load existing lab test results to get test codes
    let lab_test_repo = rustyclinic_db::sqlite::lab_repo::SqliteLabTestRepo::new(&conn);
    use rustyclinic_clinical::lab::LabTestRepo;
    let existing_tests = lab_test_repo.find_by_order(order_id).unwrap_or_default();

    // Parse form fields into LabTest updates
    let results: Vec<rustyclinic_clinical::lab::LabTest> = existing_tests
        .into_iter()
        .map(|mut test| {
            let result_key = format!("result_{}", test.test_code);
            let value_key = format!("value_{}", test.test_code);
            let abnormal_key = format!("abnormal_{}", test.test_code);

            if let Some(result) = form.fields.get(&result_key)
                && !result.is_empty()
            {
                test.result = Some(result.clone());
            }
            if let Some(value_str) = form.fields.get(&value_key)
                && let Ok(v) = value_str.parse::<f64>()
            {
                test.result_value = Some(v);
            }
            test.is_abnormal = form.fields.contains_key(&abnormal_key);
            test.resulted_at = Some(now);
            test.resulted_by = Some(actor.user_id);
            test
        })
        .collect();

    let lab_order_repo = rustyclinic_db::sqlite::lab_repo::SqliteLabOrderRepo::new(&conn);
    let queue_repo = rustyclinic_db::sqlite::queue_repo::SqliteQueueRepo::new(&conn);
    let mut uow = match rustyclinic_db::sqlite::unit_of_work::UnitOfWork::try_new(&conn) {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!(error = %e, "failed to begin transaction");
            return Redirect::to("/web/lab").into_response();
        }
    };

    let input = rustyclinic_services::commands::complete_lab_order::CompleteLabOrderInput {
        order_id,
        queue_entry_id,
        results,
    };

    match rustyclinic_services::commands::complete_lab_order::execute(
        &mut uow,
        &lab_order_repo,
        &queue_repo,
        &lab_test_repo,
        &actor,
        input,
    ) {
        Ok(()) => {
            if let Err(e) = uow.commit() {
                tracing::warn!(error = %e, "commit failed");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "complete lab order failed");
        }
    }

    Redirect::to("/web/lab").into_response()
}

fn load_lab_orders(state: &WebAppState, facility_id: Uuid) -> Vec<LabOrderView> {
    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let mut stmt = match conn.prepare(
        "SELECT lo.id, lo.status, lo.priority, lo.created_at, lo.patient_id,
                (SELECT COUNT(*) FROM lab_tests lt WHERE lt.order_id = lo.id) as test_count,
                 (SELECT q.id FROM queue_entries q WHERE q.encounter_id = lo.encounter_id AND q.department = 'lab' AND q.status NOT IN ('completed', 'cancelled') LIMIT 1) as queue_entry_id
         FROM lab_orders lo
         WHERE lo.facility_id = ?1 AND lo.status NOT IN ('verified', 'cancelled')
         ORDER BY CASE lo.priority WHEN 'stat' THEN 0 WHEN 'urgent' THEN 1 ELSE 2 END, lo.created_at ASC",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    match stmt.query_map(rusqlite::params![facility_id.to_string()], |row| {
        let id: String = row.get(0)?;
        let status: String = row.get(1)?;
        let priority: String = row.get(2)?;
        let created_str: String = row.get(3)?;
        let patient_id: String = row.get(4)?;
        let test_count: usize = row.get(5)?;
        let queue_entry_id: Option<String> = row.get(6)?;
        let patient_name = Uuid::parse_str(&patient_id)
            .ok()
            .map(|patient_id| lookup_patient_name(&conn, facility_id, patient_id))
            .unwrap_or_else(|| "Unknown".to_string());
        Ok(LabOrderView {
            order_id: id,
            queue_entry_id: queue_entry_id.unwrap_or_default(),
            patient_name,
            test_count,
            priority,
            status,
            created_at: created_str,
        })
    }) {
        Ok(r) => r.filter_map(|r| r.ok()).collect::<Vec<_>>(),
        Err(_) => vec![],
    }
}
