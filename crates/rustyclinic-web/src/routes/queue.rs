//! Queue board routes.

use axum::Form;
use axum::extract::{Path, Query, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use rustyclinic_clinical::queue::QueueTransition;
use rustyclinic_projections::QueueBoardProjection;

use crate::WebAppState;
use crate::middleware::session::WebSession;
use crate::routes::patients::lookup_user_name;
use crate::templates::{
    QueueBoardPage, QueueEntryRowPartial, QueueEntryView, QueueStatsPartial, QueueTicketPrintPage,
    SimpleQueueBoardPage, SimpleQueuePatientView, SimpleQueueRowPartial,
};

#[derive(Deserialize)]
pub struct QueueQuery {
    pub department: Option<String>,
}

/// Build queue data with a single JOIN query (no N+1).
fn build_queue_data(
    state: &WebAppState,
    facility_id: Uuid,
    department: &str,
) -> (Vec<QueueEntryView>, u32, u32, u32, String) {
    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return (vec![], 0, 0, 0, "—".to_string()),
    };

    let projection = rustyclinic_projections::sqlite::SqliteQueueBoardProjection::new(&conn);
    let mut board = match projection.get_board(facility_id) {
        Ok(b) => b,
        Err(_) => return (vec![], 0, 0, 0, "—".to_string()),
    };

    if department != "all" {
        board.retain(|e| e.department == department);
    }

    let mut waiting_count = 0u32;
    let mut in_service_count = 0u32;
    let mut total_wait_seconds = 0i64;
    let mut wait_count = 0u32;

    let entries: Vec<QueueEntryView> = board
        .into_iter()
        .map(|e| {
            match e.status {
                rustyclinic_clinical::queue::QueueStatus::Created
                | rustyclinic_clinical::queue::QueueStatus::Waiting
                | rustyclinic_clinical::queue::QueueStatus::Called => waiting_count += 1,
                rustyclinic_clinical::queue::QueueStatus::InService
                | rustyclinic_clinical::queue::QueueStatus::Transferred => in_service_count += 1,
                _ => {}
            }

            total_wait_seconds += e.wait_minutes * 60;
            wait_count += 1;

            let wait_mins = e.wait_minutes;
            let wait_time = if wait_mins >= 60 {
                format!("{}h {}m", wait_mins / 60, wait_mins % 60)
            } else {
                format!("{}m", wait_mins)
            };

            QueueEntryView {
                id: e.queue_entry_id.to_string(),
                position: e.position,
                patient_name: e.patient_name,
                service_type: e.service_type,
                department: e.department,
                status: e.status.to_string(),
                wait_time,
                assigned_to_name: e.assigned_to_name.unwrap_or_else(|| "—".to_string()),
            }
        })
        .collect();

    // Count completed separately
    let completed_count: u32 = if department == "all" {
        conn.query_row(
            "SELECT COUNT(*) FROM queue_entries
             WHERE facility_id = ?1 AND status = 'completed'
               AND date(arrived_at) = date('now')",
            rusqlite::params![facility_id.to_string()],
            |row| row.get(0),
        )
        .unwrap_or(0)
    } else {
        conn.query_row(
            "SELECT COUNT(*) FROM queue_entries
             WHERE facility_id = ?1 AND department = ?2 AND status = 'completed'
               AND date(arrived_at) = date('now')",
            rusqlite::params![facility_id.to_string(), department],
            |row| row.get(0),
        )
        .unwrap_or(0)
    };

    let avg_wait = if wait_count > 0 {
        let avg_mins = (total_wait_seconds / wait_count as i64) / 60;
        format!("{}m", avg_mins)
    } else {
        "—".to_string()
    };

    (
        entries,
        waiting_count,
        in_service_count,
        completed_count,
        avg_wait,
    )
}

/// Check which queue mode is active for this facility.
fn get_queue_mode(state: &WebAppState, facility_id: uuid::Uuid) -> String {
    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return "simple".to_string(),
    };
    get_effective_setting(&conn, facility_id, "queue_mode").unwrap_or_else(|| "simple".to_string())
}

fn get_effective_setting(
    conn: &rusqlite::Connection,
    facility_id: uuid::Uuid,
    key: &str,
) -> Option<String> {
    if let Some(value) =
        rustyclinic_services::commands::update_facility_setting::get_setting(conn, facility_id, key)
    {
        return Some(value);
    }

    conn.query_row(
        "SELECT package_deployment_settings.setting_value
         FROM package_deployment_settings
         INNER JOIN installed_packages ON installed_packages.id = package_deployment_settings.package_row_id
         WHERE installed_packages.status = 'activated'
           AND installed_packages.package_type = 'deployment'
           AND installed_packages.facility_id = ?1
           AND package_deployment_settings.setting_key = ?2
         ORDER BY installed_packages.activated_at DESC
         LIMIT 1",
        rusqlite::params![facility_id.to_string(), key],
        |row| row.get(0),
    )
    .ok()
}

/// Determine journey status for a patient in the simple queue.
///
/// Looks at all queue entries for the patient today + lab/pharmacy orders
/// to derive a human-readable status.
fn determine_journey_status(
    conn: &rusqlite::Connection,
    patient_id: &str,
    facility_id: &str,
) -> String {
    // Check for active pharmacy queue entries
    let has_active_pharmacy: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM queue_entries
             WHERE patient_id = ?1 AND facility_id = ?2 AND department = 'pharmacy'
               AND status NOT IN ('completed', 'cancelled', 'no_show')
               AND date(arrived_at) = date('now')",
            rusqlite::params![patient_id, facility_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;

    if has_active_pharmacy {
        return "Pharmacy".to_string();
    }

    // Check for active lab queue entries
    let has_active_lab: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM queue_entries
             WHERE patient_id = ?1 AND facility_id = ?2 AND department = 'lab'
               AND status NOT IN ('completed', 'cancelled', 'no_show')
               AND date(arrived_at) = date('now')",
            rusqlite::params![patient_id, facility_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;

    if has_active_lab {
        return "Lab".to_string();
    }

    // Check for active billing queue entries
    let has_active_billing: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM queue_entries
             WHERE patient_id = ?1 AND facility_id = ?2 AND department = 'billing'
               AND status NOT IN ('completed', 'cancelled', 'no_show')
               AND date(arrived_at) = date('now')",
            rusqlite::params![patient_id, facility_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;

    if has_active_billing {
        return "Billing".to_string();
    }

    // Look at the consultation queue entry
    let consult_status: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT status, encounter_id FROM queue_entries
             WHERE patient_id = ?1 AND facility_id = ?2 AND department = 'consultation'
               AND status NOT IN ('completed', 'cancelled', 'no_show')
               AND date(arrived_at) = date('now')
             ORDER BY position DESC LIMIT 1",
            rusqlite::params![patient_id, facility_id],
            |row| {
                let status: String = row.get(0)?;
                let enc_id: Option<String> = row.get(1)?;
                Ok((status, enc_id))
            },
        )
        .ok();

    match consult_status {
        Some((status, encounter_id)) => {
            match status.as_str() {
                "waiting" => "Waiting".to_string(),
                "called" => {
                    // If encounter exists, they're in triage; otherwise just called
                    if encounter_id.is_some() {
                        "Triage".to_string()
                    } else {
                        "Waiting".to_string()
                    }
                }
                "in_service" => {
                    // Check if encounter has vitals but no full notes yet (still with doctor)
                    if let Some(enc_id) = &encounter_id {
                        let enc_status: Option<String> = conn
                            .query_row(
                                "SELECT status FROM encounters WHERE id = ?1",
                                rusqlite::params![enc_id],
                                |row| row.get(0),
                            )
                            .ok();
                        match enc_status.as_deref() {
                            Some("in_progress") => "With Doctor".to_string(),
                            Some("completed") => "Done".to_string(),
                            _ => "With Doctor".to_string(),
                        }
                    } else {
                        "With Doctor".to_string()
                    }
                }
                _ => "Waiting".to_string(),
            }
        }
        None => {
            // No active consultation entry — check if all are completed
            let all_completed: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM queue_entries
                     WHERE patient_id = ?1 AND facility_id = ?2
                       AND status = 'completed'
                       AND date(arrived_at) = date('now')",
                    rusqlite::params![patient_id, facility_id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0;

            if all_completed {
                "Done".to_string()
            } else {
                "Waiting".to_string()
            }
        }
    }
}

/// Build simple board data: one row per unique patient today.
fn build_simple_board_data(
    state: &WebAppState,
    facility_id: Uuid,
) -> (Vec<SimpleQueuePatientView>, u32, u32, u32, String) {
    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return (vec![], 0, 0, 0, "\u{2014}".to_string()),
    };

    let fid = facility_id.to_string();

    // Get distinct patients who have queue entries today, ordered by earliest arrival
    let mut stmt = match conn.prepare(
        "SELECT DISTINCT q.patient_id, p.given_name, p.family_name,
                MIN(q.position) as first_pos, MIN(q.arrived_at) as first_arrived,
                q.id as queue_entry_id, q.encounter_id
         FROM queue_entries q
         JOIN patients p ON p.id = q.patient_id
         WHERE q.facility_id = ?1
           AND date(q.arrived_at) = date('now')
         GROUP BY q.patient_id
         ORDER BY first_pos ASC",
    ) {
        Ok(s) => s,
        Err(_) => return (vec![], 0, 0, 0, "\u{2014}".to_string()),
    };

    let now = Utc::now();
    let mut waiting_count = 0u32;
    let mut in_service_count = 0u32;
    let mut total_wait_seconds = 0i64;
    let mut wait_count = 0u32;

    let rows = match stmt.query_map(rusqlite::params![&fid], |row| {
        let patient_id: String = row.get(0)?;
        let given_name: String = row.get(1)?;
        let family_name: String = row.get(2)?;
        let position: u32 = row.get(3)?;
        let arrived_str: String = row.get(4)?;
        let queue_entry_id: String = row.get(5)?;
        let encounter_id: Option<String> = row.get(6)?;
        Ok((
            patient_id,
            given_name,
            family_name,
            position,
            arrived_str,
            queue_entry_id,
            encounter_id,
        ))
    }) {
        Ok(r) => r.filter_map(|r| r.ok()).collect::<Vec<_>>(),
        Err(_) => vec![],
    };

    let mut patients = Vec::new();
    for (
        patient_id,
        given_name,
        family_name,
        position,
        arrived_str,
        queue_entry_id,
        encounter_id,
    ) in rows
    {
        let journey_status = determine_journey_status(&conn, &patient_id, &fid);

        match journey_status.as_str() {
            "Waiting" => waiting_count += 1,
            "Triage" | "With Doctor" | "Lab" | "Pharmacy" | "Billing" => in_service_count += 1,
            _ => {}
        }

        let arrived = chrono::DateTime::parse_from_rfc3339(&arrived_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or(now);
        let wait = now.signed_duration_since(arrived);
        let wait_mins = wait.num_minutes();
        total_wait_seconds += wait.num_seconds();
        wait_count += 1;

        let wait_time = if wait_mins >= 60 {
            format!("{}h {}m", wait_mins / 60, wait_mins % 60)
        } else {
            format!("{}m", wait_mins)
        };

        patients.push(SimpleQueuePatientView {
            queue_entry_id,
            encounter_id: encounter_id.unwrap_or_default(),
            position,
            patient_name: format!("{family_name}, {given_name}"),
            journey_status,
            wait_time,
        });
    }

    let completed_count: u32 = conn
        .query_row(
            "SELECT COUNT(DISTINCT patient_id) FROM queue_entries
             WHERE facility_id = ?1 AND status = 'completed'
               AND date(arrived_at) = date('now')
               AND patient_id NOT IN (
                 SELECT patient_id FROM queue_entries
                 WHERE facility_id = ?1 AND status NOT IN ('completed', 'cancelled', 'no_show')
                   AND date(arrived_at) = date('now')
               )",
            rusqlite::params![&fid],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let avg_wait = if wait_count > 0 {
        let avg_mins = (total_wait_seconds / wait_count as i64) / 60;
        format!("{}m", avg_mins)
    } else {
        "\u{2014}".to_string()
    };

    (
        patients,
        waiting_count,
        in_service_count,
        completed_count,
        avg_wait,
    )
}

pub async fn board_page(
    State(state): State<WebAppState>,
    session: WebSession,
    Query(query): Query<QueueQuery>,
) -> Response {
    let (display_name, initials) = lookup_user_name(&state, session.session.user_id);
    let facility_id = session.session.facility_id;
    let queue_mode = get_queue_mode(&state, facility_id);

    if queue_mode == "simple" {
        let (patients, waiting_count, in_service_count, completed_count, avg_wait) =
            build_simple_board_data(&state, facility_id);

        let page = SimpleQueueBoardPage {
            active_nav: "queue".to_string(),
            display_name,
            initials,
            flash_success: None,
            flash_error: None,
            patients,
            waiting_count,
            in_service_count,
            completed_count,
            avg_wait,
        };
        return Html(page.to_string()).into_response();
    }

    // Department mode
    let department = query.department.unwrap_or_else(|| "all".to_string());

    let (entries, waiting_count, in_service_count, completed_count, avg_wait) =
        build_queue_data(&state, facility_id, &department);

    let page = QueueBoardPage {
        active_nav: "queue".to_string(),
        display_name,
        initials,
        flash_success: None,
        flash_error: None,
        entries,
        waiting_count,
        in_service_count,
        completed_count,
        avg_wait,
        department_filter: department,
    };
    Html(page.to_string()).into_response()
}

pub async fn queue_ticket_print_page(
    State(state): State<WebAppState>,
    session: WebSession,
    Path(id): Path<String>,
) -> Response {
    let queue_entry_id = match Uuid::parse_str(&id) {
        Ok(uuid) => uuid,
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };

    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };

    let ticket = match load_queue_ticket_view(&conn, &session.session.facility_id, &queue_entry_id)
    {
        Some(ticket) => ticket,
        None => return Redirect::to("/web/queue").into_response(),
    };

    let page = QueueTicketPrintPage {
        queue_entry_id: ticket.id,
        position: ticket.position,
        patient_name: ticket.patient_name,
        department: ticket.department,
        service_type: ticket.service_type,
        status: ticket.status,
    };

    Html(page.to_string()).into_response()
}

pub async fn board_content(
    State(state): State<WebAppState>,
    session: WebSession,
    Query(query): Query<QueueQuery>,
) -> impl IntoResponse {
    let facility_id = session.session.facility_id;
    let department = query.department.unwrap_or_else(|| "all".to_string());

    let (entries, waiting_count, in_service_count, completed_count, avg_wait) =
        build_queue_data(&state, facility_id, &department);

    let stats = QueueStatsPartial {
        waiting_count,
        in_service_count,
        completed_count,
        avg_wait: avg_wait.clone(),
    };

    let mut html = stats.to_string();

    if entries.is_empty() {
        html.push_str(r#"<div class="empty-state"><svg viewBox="0 0 48 48" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="6" y="10" width="36" height="28" rx="4"/><path d="M6 18h36M18 26h12M20 32h8"/></svg><p>No patients in the queue today.</p><a href="/web/patients" class="btn btn-accent">Find Patient</a></div>"#);
    } else {
        html.push_str(r#"<div class="card table-wrap"><table><thead><tr><th>#</th><th>Patient</th><th>Dept</th><th>Service</th><th>Status</th><th>Wait Time</th><th>Assigned To</th><th>Actions</th></tr></thead><tbody>"#);
        for e in &entries {
            let row = QueueEntryRowPartial { e: e.clone() };
            html.push_str(&row.to_string());
        }
        html.push_str("</tbody></table></div>");
    }

    Html(html)
}

/// htmx polling endpoint for simple board mode.
pub async fn simple_board_content(
    State(state): State<WebAppState>,
    session: WebSession,
) -> impl IntoResponse {
    let facility_id = session.session.facility_id;

    let (patients, waiting_count, in_service_count, completed_count, avg_wait) =
        build_simple_board_data(&state, facility_id);

    let stats = QueueStatsPartial {
        waiting_count,
        in_service_count,
        completed_count,
        avg_wait: avg_wait.clone(),
    };

    let mut html = stats.to_string();

    if patients.is_empty() {
        html.push_str(r#"<div class="empty-state"><svg viewBox="0 0 48 48" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="6" y="10" width="36" height="28" rx="4"/><path d="M6 18h36M18 26h12M20 32h8"/></svg><p>No patients in the queue today.</p><a href="/web/patients" class="btn btn-accent">Find Patient</a></div>"#);
    } else {
        html.push_str(r#"<div class="card table-wrap"><table><thead><tr><th>#</th><th>Patient</th><th>Status</th><th>Since</th><th>Actions</th></tr></thead><tbody>"#);
        for p in &patients {
            let row = SimpleQueueRowPartial { p: p.clone() };
            html.push_str(&row.to_string());
        }
        html.push_str("</tbody></table></div>");
    }

    Html(html)
}

#[derive(Deserialize)]
pub struct TransitionForm {
    pub transition: String,
}

pub async fn transition_entry(
    State(state): State<WebAppState>,
    session: WebSession,
    Path(id): Path<String>,
    Form(form): Form<TransitionForm>,
) -> Response {
    let entry_id = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };

    let transition = match form.transition.as_str() {
        "call" => QueueTransition::Call,
        "begin_service" => QueueTransition::BeginService,
        "complete" => QueueTransition::Complete,
        "mark_no_show" => QueueTransition::MarkNoShow,
        "transfer" => QueueTransition::Transfer,
        "cancel" => QueueTransition::Cancel,
        _ => return Redirect::to("/web/queue").into_response(),
    };

    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/web/queue").into_response(),
    };

    let actor = session.session.to_actor_context();
    let repo = rustyclinic_db::sqlite::queue_repo::SqliteQueueRepo::new(&conn);
    let mut uow = match rustyclinic_db::sqlite::unit_of_work::UnitOfWork::try_new(&conn) {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!(error = %e, "failed to begin transaction");
            return Redirect::to("/web/queue").into_response();
        }
    };

    let input = rustyclinic_services::commands::transition_queue::TransitionQueueInput {
        queue_entry_id: entry_id,
        transition,
        assigned_to: None,
    };

    match rustyclinic_services::commands::transition_queue::execute(&mut uow, &repo, &actor, input)
    {
        Ok(()) => {
            if let Err(e) = uow.commit() {
                tracing::warn!(error = %e, "commit failed");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "queue transition failed");
        }
    }

    // Return updated row partial (for htmx) — use JOIN for single query
    let row_data = conn.query_row(
        "SELECT q.id, q.position, q.service_type, q.status, q.arrived_at,
                p.given_name, p.family_name,
                u.display_name,
                q.department
         FROM queue_entries q
         JOIN patients p ON p.id = q.patient_id
         LEFT JOIN users u ON u.id = q.assigned_to
         WHERE q.id = ?1",
        rusqlite::params![entry_id.to_string()],
        |row| {
            let id: String = row.get(0)?;
            let position: u32 = row.get(1)?;
            let service_type: String = row.get(2)?;
            let status: String = row.get(3)?;
            let arrived_str: String = row.get(4)?;
            let given_name: String = row.get(5)?;
            let family_name: String = row.get(6)?;
            let assigned_name: Option<String> = row.get(7)?;
            let dept: String = row.get(8)?;
            Ok((
                id,
                position,
                service_type,
                status,
                arrived_str,
                given_name,
                family_name,
                assigned_name,
                dept,
            ))
        },
    );

    if let Ok((
        id,
        position,
        service_type,
        status,
        arrived_str,
        given_name,
        family_name,
        assigned_name,
        dept,
    )) = row_data
    {
        let now = Utc::now();
        let arrived = chrono::DateTime::parse_from_rfc3339(&arrived_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or(now);
        let wait_mins = now.signed_duration_since(arrived).num_minutes();
        let wait_time = if wait_mins >= 60 {
            format!("{}h {}m", wait_mins / 60, wait_mins % 60)
        } else {
            format!("{}m", wait_mins)
        };

        let view = QueueEntryView {
            id,
            position,
            patient_name: format!("{family_name}, {given_name}"),
            service_type,
            department: dept,
            status,
            wait_time,
            assigned_to_name: assigned_name.unwrap_or_default(),
        };

        let row = QueueEntryRowPartial { e: view };
        Html(row.to_string()).into_response()
    } else {
        Redirect::to("/web/queue").into_response()
    }
}

#[derive(Deserialize)]
pub struct EnqueueForm {
    pub patient_id: String,
    pub service_type: String,
}

pub async fn enqueue_patient(
    State(state): State<WebAppState>,
    session: WebSession,
    Form(form): Form<EnqueueForm>,
) -> Response {
    let patient_id = match Uuid::parse_str(&form.patient_id) {
        Ok(u) => u,
        Err(_) => return Redirect::to("/web/patients").into_response(),
    };

    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/web/patients").into_response(),
    };

    let actor = session.session.to_actor_context();
    let repo = rustyclinic_db::sqlite::queue_repo::SqliteQueueRepo::new(&conn);
    let mut uow = match rustyclinic_db::sqlite::unit_of_work::UnitOfWork::try_new(&conn) {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!(error = %e, "failed to begin transaction");
            return Redirect::to("/web/patients").into_response();
        }
    };

    let input = rustyclinic_services::commands::enqueue_patient::EnqueuePatientInput {
        patient_id,
        service_type: form.service_type,
    };

    match rustyclinic_services::commands::enqueue_patient::execute(&mut uow, &repo, &actor, input) {
        Ok(_entry_id) => {
            if let Err(e) = uow.commit() {
                tracing::warn!(error = %e, "commit failed");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "enqueue failed");
        }
    }

    Redirect::to("/web/queue").into_response()
}

/// Minimal queue ticket view for print rendering.
///
/// Holds the subset of fields needed for a print-friendly queue ticket:
/// position, patient name, department, service type, plus id/status for context.
#[derive(Clone, Debug)]
pub struct QueueTicketView {
    pub id: String,
    pub position: u32,
    pub patient_name: String,
    pub department: String,
    pub service_type: String,
    pub status: String,
}

/// Load a single queue ticket view by queue entry id.
///
/// Returns `None` if the queue entry does not exist or belongs to a different facility.
///
/// # Arguments
/// * `conn` — open SQLite connection
/// * `facility_id` — the facility to scope the query to
/// * `queue_entry_id` — the queue entry UUID string
pub fn load_queue_ticket_view(
    conn: &rusqlite::Connection,
    facility_id: &Uuid,
    queue_entry_id: &Uuid,
) -> Option<QueueTicketView> {
    conn.query_row(
        "SELECT q.id, q.position, q.department, q.service_type, q.status,
                p.given_name, p.family_name
         FROM queue_entries q
         JOIN patients p ON p.id = q.patient_id
         WHERE q.id = ?1 AND q.facility_id = ?2",
        rusqlite::params![queue_entry_id.to_string(), facility_id.to_string()],
        |row| {
            let id: String = row.get(0)?;
            let position: u32 = row.get(1)?;
            let department: String = row.get(2)?;
            let service_type: String = row.get(3)?;
            let status: String = row.get(4)?;
            let given_name: String = row.get(5)?;
            let family_name: String = row.get(6)?;
            Ok((
                id,
                position,
                department,
                service_type,
                status,
                given_name,
                family_name,
            ))
        },
    )
    .ok()
    .map(
        |(id, position, department, service_type, status, given_name, family_name)| {
            QueueTicketView {
                id,
                position,
                department,
                service_type,
                status,
                patient_name: format!("{family_name}, {given_name}"),
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyclinic_core::types::new_id;

    fn setup_in_memory_db() -> (rusqlite::Connection, Uuid, Uuid) {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
        conn.pragma_update(None, "foreign_keys", "on")
            .expect("enable fk");

        use rustyclinic_db::migration::run_migrations;
        run_migrations(&conn).expect("migrations");

        let facility_id = new_id();
        let patient_id = new_id();

        conn.execute(
            "INSERT INTO patients (id, facility_id, given_name, family_name, sex,
                                  date_of_birth, created_at, updated_at, version)
             VALUES (?1, ?2, 'Jane', 'Smith', 'female', '1990-05-15',
                     datetime('now'), datetime('now'), 1)",
            rusqlite::params![patient_id.to_string(), facility_id.to_string()],
        )
        .expect("patient insert");

        let queue_entry_id = new_id();
        conn.execute(
            "INSERT INTO queue_entries (id, facility_id, patient_id, service_type, department,
                                        status, position, arrived_at, created_at, version)
             VALUES (?1, ?2, ?3, 'consultation', 'consultation', 'waiting', 3,
                     datetime('now'), datetime('now'), 1)",
            rusqlite::params![
                queue_entry_id.to_string(),
                facility_id.to_string(),
                patient_id.to_string()
            ],
        )
        .expect("queue entry insert");

        (conn, facility_id, queue_entry_id)
    }

    #[test]
    fn load_queue_ticket_view_returns_all_required_print_fields() {
        let (conn, facility_id, queue_entry_id) = setup_in_memory_db();

        let ticket = load_queue_ticket_view(&conn, &facility_id, &queue_entry_id);

        let ticket = ticket.expect("queue ticket should be found");
        assert_eq!(ticket.position, 3, "position should match");
        assert_eq!(
            ticket.patient_name, "Smith, Jane",
            "patient_name should be 'Family, Given' format"
        );
        assert_eq!(ticket.department, "consultation", "department should match");
        assert_eq!(
            ticket.service_type, "consultation",
            "service_type should match"
        );
    }

    #[test]
    fn load_queue_ticket_view_returns_none_for_missing_entry() {
        let (conn, facility_id, _) = setup_in_memory_db();
        let nonexistent_id = new_id();

        let ticket = load_queue_ticket_view(&conn, &facility_id, &nonexistent_id);

        assert!(
            ticket.is_none(),
            "should return None for non-existent queue entry"
        );
    }

    #[test]
    fn load_queue_ticket_view_returns_none_for_wrong_facility() {
        let (conn, _, queue_entry_id) = setup_in_memory_db();
        let wrong_facility_id = new_id();

        let ticket = load_queue_ticket_view(&conn, &wrong_facility_id, &queue_entry_id);

        assert!(
            ticket.is_none(),
            "should return None when facility_id does not match"
        );
    }

    #[test]
    fn load_queue_ticket_view_includes_status_field() {
        let (conn, facility_id, queue_entry_id) = setup_in_memory_db();

        let ticket =
            load_queue_ticket_view(&conn, &facility_id, &queue_entry_id).expect("ticket exists");

        assert_eq!(ticket.status, "waiting", "status field should be present");
    }

    #[test]
    fn get_effective_setting_uses_activated_deployment_default() {
        let (conn, facility_id, _) = setup_in_memory_db();
        let package_row_id = new_id();
        let installed_by = new_id();

        let manifest = serde_json::json!({
            "package_id": "queue-defaults",
            "package_type": "deployment",
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
             VALUES (?1, ?2, 'queue-defaults', 'deployment', '1.0.0', 'activated', ?3, datetime('now'), datetime('now'), NULL, ?4, 1)",
            rusqlite::params![package_row_id.to_string(), facility_id.to_string(), manifest, installed_by.to_string()],
        )
        .expect("insert deployment package");
        conn.execute(
            "INSERT INTO package_deployment_settings (package_row_id, setting_key, setting_value)
             VALUES (?1, 'queue_mode', 'department')",
            rusqlite::params![package_row_id.to_string()],
        )
        .expect("insert deployment setting");

        assert_eq!(
            get_effective_setting(&conn, facility_id, "queue_mode").as_deref(),
            Some("department")
        );
    }

    #[test]
    fn get_effective_setting_prefers_facility_override_over_package_default() {
        let (conn, facility_id, _) = setup_in_memory_db();
        let package_row_id = new_id();
        let installed_by = new_id();

        let manifest = serde_json::json!({
            "package_id": "queue-defaults",
            "package_type": "deployment",
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
             VALUES (?1, ?2, 'queue-defaults', 'deployment', '1.0.0', 'activated', ?3, datetime('now'), datetime('now'), NULL, ?4, 1)",
            rusqlite::params![package_row_id.to_string(), facility_id.to_string(), manifest, installed_by.to_string()],
        )
        .expect("insert deployment package");
        conn.execute(
            "INSERT INTO package_deployment_settings (package_row_id, setting_key, setting_value)
             VALUES (?1, 'queue_mode', 'department')",
            rusqlite::params![package_row_id.to_string()],
        )
        .expect("insert deployment setting");
        rustyclinic_services::commands::update_facility_setting::update_setting(
            &conn,
            facility_id,
            "queue_mode",
            "simple",
        )
        .expect("upsert facility setting");

        assert_eq!(
            get_effective_setting(&conn, facility_id, "queue_mode").as_deref(),
            Some("simple")
        );
    }
}
