//! Pharmacy workflow routes — prescription queue and dispensing.

use axum::Form;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;
use uuid::Uuid;

use crate::WebAppState;
use crate::middleware::session::WebSession;
use crate::routes::patients::{lookup_patient_name, lookup_user_name};
use crate::templates::{
    PharmacyDispensePage, PharmacyDispenseSlipPrintPage, PharmacyOrderView, PharmacyQueuePage,
    PrescriptionItemView,
};

/// Pharmacy queue page — shows pending prescriptions.
pub async fn queue_page(
    State(state): State<WebAppState>,
    session: WebSession,
) -> impl IntoResponse {
    let (display_name, initials) = lookup_user_name(&state, session.session.user_id);
    let facility_id = session.session.facility_id;

    let orders = load_pharmacy_orders(&state, facility_id);

    let page = PharmacyQueuePage {
        active_nav: "pharmacy".to_string(),
        display_name,
        initials,
        flash_success: None,
        flash_error: None,
        orders,
    };
    Html(page.to_string())
}

/// Dispense page for a specific prescription order.
pub async fn dispense_page(
    State(state): State<WebAppState>,
    session: WebSession,
    Path(order_id): Path<String>,
) -> Response {
    let order_uuid = match Uuid::parse_str(&order_id) {
        Ok(u) => u,
        Err(_) => return Redirect::to("/web/pharmacy").into_response(),
    };

    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/web/pharmacy").into_response(),
    };

    let (display_name, initials) = lookup_user_name(&state, session.session.user_id);

    // Load medication dispense
    let dispense_repo =
        rustyclinic_db::sqlite::pharmacy_repo::SqliteMedicationDispenseRepo::new(&conn);
    use rustyclinic_clinical::pharmacy::MedicationDispenseRepo;
    let dispense = match dispense_repo.find_by_id(order_uuid) {
        Ok(Some(d)) => d,
        _ => return Redirect::to("/web/pharmacy").into_response(),
    };

    // Find queue entry
    let queue_entry_id = conn
        .query_row(
            "SELECT id FROM queue_entries WHERE encounter_id = ?1 AND department = 'pharmacy' AND status NOT IN ('completed', 'cancelled') LIMIT 1",
            rusqlite::params![dispense.encounter_id.to_string()],
            |row| {
                let id: String = row.get(0)?;
                Ok(id)
            },
        )
        .unwrap_or_default();

    // Load patient name
    let patient_name = lookup_patient_name(&conn, session.session.facility_id, dispense.patient_id);

    // Load dispense items
    let item_repo = rustyclinic_db::sqlite::pharmacy_repo::SqliteDispenseItemRepo::new(&conn);
    use rustyclinic_clinical::pharmacy::DispenseItemRepo;
    let items = item_repo.find_by_dispense(order_uuid).unwrap_or_default();

    let item_views: Vec<PrescriptionItemView> = items
        .iter()
        .map(|i| PrescriptionItemView {
            medication_name: i.medication_name.clone(),
            medication_field_name: i.medication_name.replace(' ', "_"),
            dosage: i.dosage.clone(),
            frequency: i.frequency.clone(),
            duration: i.duration.clone(),
            quantity: i.quantity,
            dispensed_quantity: i
                .dispensed_quantity
                .map(|q| q.to_string())
                .unwrap_or_default(),
        })
        .collect();

    let page = PharmacyDispensePage {
        active_nav: "pharmacy".to_string(),
        display_name,
        initials,
        flash_success: None,
        flash_error: None,
        order_id: order_id.clone(),
        queue_entry_id,
        patient_name,
        items: item_views,
    };
    Html(page.to_string()).into_response()
}

pub async fn dispense_slip_print_page(
    State(state): State<WebAppState>,
    session: WebSession,
    Path(order_id): Path<String>,
) -> Response {
    let order_uuid = match Uuid::parse_str(&order_id) {
        Ok(u) => u,
        Err(_) => return Redirect::to("/web/pharmacy").into_response(),
    };

    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/web/pharmacy").into_response(),
    };

    // Load medication dispense
    let dispense_repo =
        rustyclinic_db::sqlite::pharmacy_repo::SqliteMedicationDispenseRepo::new(&conn);
    use rustyclinic_clinical::pharmacy::MedicationDispenseRepo;
    let dispense = match dispense_repo.find_by_id(order_uuid) {
        Ok(Some(d)) => d,
        _ => return Redirect::to("/web/pharmacy").into_response(),
    };

    // Load patient name
    let patient_name = lookup_patient_name(&conn, session.session.facility_id, dispense.patient_id);

    // Load dispense items
    let item_repo = rustyclinic_db::sqlite::pharmacy_repo::SqliteDispenseItemRepo::new(&conn);
    use rustyclinic_clinical::pharmacy::DispenseItemRepo;
    let items = item_repo.find_by_dispense(order_uuid).unwrap_or_default();
    if items.is_empty() {
        return Redirect::to("/web/pharmacy").into_response();
    }

    let item_views: Vec<PrescriptionItemView> = items
        .iter()
        .map(|i| PrescriptionItemView {
            medication_name: i.medication_name.clone(),
            medication_field_name: i.medication_name.replace(' ', "_"),
            dosage: i.dosage.clone(),
            frequency: i.frequency.clone(),
            duration: i.duration.clone(),
            quantity: i.quantity,
            dispensed_quantity: i.dispensed_quantity.unwrap_or(i.quantity).to_string(),
        })
        .collect();

    let page = PharmacyDispenseSlipPrintPage {
        order_id: order_id.clone(),
        patient_name,
        items: item_views,
    };

    Html(page.to_string()).into_response()
}

#[derive(Deserialize)]
pub struct DispenseForm {
    pub order_id: String,
    pub queue_entry_id: String,
    #[serde(flatten)]
    pub fields: std::collections::HashMap<String, String>,
}

/// Submit dispensing and complete the prescription order.
pub async fn submit_dispense(
    State(state): State<WebAppState>,
    session: WebSession,
    Form(form): Form<DispenseForm>,
) -> Response {
    let order_id = match Uuid::parse_str(&form.order_id) {
        Ok(u) => u,
        Err(_) => return Redirect::to("/web/pharmacy").into_response(),
    };
    let queue_entry_id = match Uuid::parse_str(&form.queue_entry_id) {
        Ok(u) => u,
        Err(_) => return Redirect::to("/web/pharmacy").into_response(),
    };

    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/web/pharmacy").into_response(),
    };

    let actor = session.session.to_actor_context();

    // Load existing dispense items to get medication names
    let item_repo = rustyclinic_db::sqlite::pharmacy_repo::SqliteDispenseItemRepo::new(&conn);
    use rustyclinic_clinical::pharmacy::DispenseItemRepo;
    let existing_items = item_repo.find_by_dispense(order_id).unwrap_or_default();

    // Parse form fields into dispense items
    let items: Vec<rustyclinic_services::commands::dispense_prescription::DispenseItemInput> =
        existing_items
            .iter()
            .map(|item| {
                let qty_key = format!("dispensed_{}", item.medication_name.replace(' ', "_"));
                let sub_key = format!("substituted_{}", item.medication_name.replace(' ', "_"));
                let reason_key = format!("sub_reason_{}", item.medication_name.replace(' ', "_"));

                let dispensed_quantity = form
                    .fields
                    .get(&qty_key)
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(item.quantity);

                let substituted = form.fields.contains_key(&sub_key);
                let substitution_reason = form.fields.get(&reason_key).cloned();

                rustyclinic_services::commands::dispense_prescription::DispenseItemInput {
                    medication_name: item.medication_name.clone(),
                    dispensed_quantity,
                    substituted,
                    substitution_reason,
                }
            })
            .collect();

    let dispense_repo =
        rustyclinic_db::sqlite::pharmacy_repo::SqliteMedicationDispenseRepo::new(&conn);
    let queue_repo = rustyclinic_db::sqlite::queue_repo::SqliteQueueRepo::new(&conn);
    let mut uow = match rustyclinic_db::sqlite::unit_of_work::UnitOfWork::try_new(&conn) {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!(error = %e, "failed to begin transaction");
            return Redirect::to("/web/pharmacy").into_response();
        }
    };

    let input = rustyclinic_services::commands::dispense_prescription::DispensePrescriptionInput {
        order_id,
        queue_entry_id,
        items,
    };

    match rustyclinic_services::commands::dispense_prescription::execute(
        &mut uow,
        &dispense_repo,
        &queue_repo,
        &item_repo,
        &actor,
        input,
    ) {
        Ok(()) => {
            if let Err(e) = uow.commit() {
                tracing::warn!(error = %e, "commit failed");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "dispense prescription failed");
        }
    }

    Redirect::to("/web/pharmacy").into_response()
}

fn load_pharmacy_orders(state: &WebAppState, facility_id: Uuid) -> Vec<PharmacyOrderView> {
    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let mut stmt = match conn.prepare(
        "SELECT md.id, md.status, md.priority, md.created_at, md.patient_id,
                (SELECT COUNT(*) FROM dispense_items di WHERE di.dispense_id = md.id) as item_count,
                 (SELECT q.id FROM queue_entries q WHERE q.encounter_id = md.encounter_id AND q.department = 'pharmacy' AND q.status NOT IN ('completed', 'cancelled') LIMIT 1) as queue_entry_id
         FROM medication_dispenses md
         WHERE md.facility_id = ?1 AND md.status NOT IN ('dispensed', 'returned', 'voided')
         ORDER BY CASE md.priority WHEN 'stat' THEN 0 WHEN 'urgent' THEN 1 ELSE 2 END, md.created_at ASC",
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
        let item_count: usize = row.get(5)?;
        let queue_entry_id: Option<String> = row.get(6)?;
        let patient_name = Uuid::parse_str(&patient_id)
            .ok()
            .map(|patient_id| lookup_patient_name(&conn, facility_id, patient_id))
            .unwrap_or_else(|| "Unknown".to_string());
        Ok(PharmacyOrderView {
            order_id: id,
            queue_entry_id: queue_entry_id.unwrap_or_default(),
            patient_name,
            item_count,
            priority,
            status,
            created_at: created_str,
        })
    }) {
        Ok(r) => r.filter_map(|r| r.ok()).collect::<Vec<_>>(),
        Err(_) => vec![],
    }
}
