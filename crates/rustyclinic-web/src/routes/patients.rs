//! Patient registration and search routes.

use axum::extract::{Query, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;
use uuid::Uuid;

use rustyclinic_core::types::Sex;
use rustyclinic_projections::{LongitudinalTimelineProjection, PatientSummaryProjection};

use crate::WebAppState;
use crate::middleware::session::WebSession;
use crate::templates::{
    PatientCardPrintPage, PatientDetailPage, PatientListPartial, PatientRegisterPage,
    PatientSearchPage, PatientTimelineItemView, PatientView,
};

/// OR-based patient search: matches given_name, family_name, or national_id.
fn search_patients_or(conn: &rusqlite::Connection, q: &str) -> Vec<PatientView> {
    if q.is_empty() {
        // Return recent patients
        let mut stmt = match conn.prepare(
            "SELECT id, facility_id, given_name, family_name, sex, date_of_birth, phone, address, national_id
             FROM patients ORDER BY created_at DESC LIMIT 20",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        let rows = match stmt.query_map([], |row| {
            Ok(PatientView {
                id: row.get::<_, String>(0)?,
                given_name: row.get(2)?,
                family_name: row.get(3)?,
                sex: row.get::<_, String>(4)?,
                date_of_birth: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                national_id: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
                phone: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
            })
        }) {
            Ok(r) => r,
            Err(_) => return vec![],
        };

        rows.filter_map(|r| r.ok()).collect()
    } else {
        let like = format!("%{q}%");
        let mut stmt = match conn.prepare(
            "SELECT id, facility_id, given_name, family_name, sex, date_of_birth, phone, address, national_id
             FROM patients
             WHERE given_name LIKE ?1 OR family_name LIKE ?1 OR national_id LIKE ?1
             LIMIT 20",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        let rows = match stmt.query_map(rusqlite::params![like], |row| {
            Ok(PatientView {
                id: row.get::<_, String>(0)?,
                given_name: row.get(2)?,
                family_name: row.get(3)?,
                sex: row.get::<_, String>(4)?,
                date_of_birth: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                national_id: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
                phone: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
            })
        }) {
            Ok(r) => r,
            Err(_) => return vec![],
        };

        rows.filter_map(|r| r.ok()).collect()
    }
}

pub async fn search_page(State(state): State<WebAppState>, session: WebSession) -> Response {
    let (display_name, initials) = lookup_user_name(&state, session.session.user_id);

    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/web/login").into_response(),
    };

    let patients = search_patients_or(&conn, "");

    let page = PatientSearchPage {
        active_nav: "patients".to_string(),
        display_name,
        initials,
        flash_success: None,
        flash_error: None,
        patients,
    };
    Html(page.to_string()).into_response()
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
}

pub async fn search_results(
    State(state): State<WebAppState>,
    _session: WebSession,
    Query(query): Query<SearchQuery>,
) -> impl IntoResponse {
    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Html(String::new()),
    };

    let q = query.q.unwrap_or_default();
    let results = search_patients_or(&conn, &q);

    let partial = PatientListPartial { patients: results };
    Html(partial.to_string())
}

pub async fn register_page(
    State(state): State<WebAppState>,
    session: WebSession,
) -> impl IntoResponse {
    let (display_name, initials) = lookup_user_name(&state, session.session.user_id);

    let page = PatientRegisterPage {
        active_nav: "register".to_string(),
        display_name,
        initials,
        flash_success: None,
        flash_error: None,
        error: None,
    };
    Html(page.to_string())
}

#[derive(Deserialize)]
pub struct RegisterForm {
    pub given_name: String,
    pub family_name: String,
    pub sex: String,
    pub date_of_birth: Option<String>,
    pub phone: Option<String>,
    pub address: Option<String>,
    pub national_id: Option<String>,
}

pub async fn register_submit(
    State(state): State<WebAppState>,
    session: WebSession,
    axum::Form(form): axum::Form<RegisterForm>,
) -> Response {
    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(e) => {
            return render_register_error(
                &state,
                session.session.user_id,
                &format!("Database error: {e}"),
            );
        }
    };

    let actor = session.session.to_actor_context();

    let sex = match form.sex.to_lowercase().as_str() {
        "female" | "f" => Sex::Female,
        "male" | "m" => Sex::Male,
        "other" => Sex::Other,
        _ => Sex::Unknown,
    };

    let dob = form
        .date_of_birth
        .as_deref()
        .filter(|s| !s.is_empty())
        .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());

    let input = rustyclinic_services::commands::register_patient::RegisterPatientInput {
        given_name: form.given_name,
        family_name: form.family_name,
        sex,
        date_of_birth: dob,
        phone: form.phone.filter(|s| !s.is_empty()),
        address: form.address.filter(|s| !s.is_empty()),
        national_id: form.national_id.filter(|s| !s.is_empty()),
    };

    let repo = rustyclinic_db::sqlite::patient_repo::SqlitePatientRepo::new(&conn);
    let mut uow = match rustyclinic_db::sqlite::unit_of_work::UnitOfWork::try_new(&conn) {
        Ok(u) => u,
        Err(e) => {
            return render_register_error(
                &state,
                session.session.user_id,
                &format!("Transaction error: {e}"),
            );
        }
    };

    match rustyclinic_services::commands::register_patient::execute(&mut uow, &repo, &actor, input)
    {
        Ok(_patient_id) => {
            if let Err(e) = uow.commit() {
                return render_register_error(
                    &state,
                    session.session.user_id,
                    &format!("Commit failed: {e}"),
                );
            }
            Redirect::to("/web/queue").into_response()
        }
        Err(e) => render_register_error(&state, session.session.user_id, &e.to_string()),
    }
}

pub async fn detail_page(
    State(state): State<WebAppState>,
    session: WebSession,
    axum::extract::Path(patient_id): axum::extract::Path<String>,
) -> Response {
    let patient_id = match Uuid::parse_str(&patient_id) {
        Ok(id) => id,
        Err(_) => return Redirect::to("/web/patients").into_response(),
    };

    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/web/patients").into_response(),
    };

    let patient_projection =
        rustyclinic_projections::sqlite::SqlitePatientSummaryProjection::new(&conn);
    let timeline_projection =
        rustyclinic_projections::sqlite::SqliteLongitudinalTimelineProjection::new(&conn);

    let summary = match patient_projection.get_summary(session.session.facility_id, patient_id) {
        Ok(Some(summary)) => summary,
        _ => return Redirect::to("/web/patients").into_response(),
    };

    let timeline = timeline_projection
        .get_timeline(session.session.facility_id, patient_id, 20)
        .unwrap_or_default()
        .into_iter()
        .map(|entry| PatientTimelineItemView {
            entry_type: entry.entry_type.replace('_', " "),
            title: entry.title,
            detail: entry.detail.unwrap_or_default(),
            occurred_at: entry.occurred_at.format("%Y-%m-%d %H:%M").to_string(),
        })
        .collect::<Vec<_>>();

    let (display_name, initials) = lookup_user_name(&state, session.session.user_id);
    let patient_name = format!("{} {}", summary.given_name, summary.family_name);
    let page = PatientDetailPage {
        active_nav: "patients".to_string(),
        display_name,
        initials,
        flash_success: None,
        flash_error: None,
        patient_id: patient_id.to_string(),
        patient_name: patient_name.clone(),
        patient_initials: WebSession::initials_from(&patient_name),
        sex: summary.sex,
        age: summary
            .age
            .map(|age| age.to_string())
            .unwrap_or_else(|| "Unknown".to_string()),
        national_id: summary.national_id.unwrap_or_else(|| "—".to_string()),
        last_visit: summary
            .last_visit
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "No visits yet".to_string()),
        active_programs: summary.active_programs,
        timeline,
    };

    Html(page.to_string()).into_response()
}

pub async fn print_page(
    State(state): State<WebAppState>,
    session: WebSession,
    axum::extract::Path(patient_id): axum::extract::Path<String>,
) -> Response {
    let patient_id = match Uuid::parse_str(&patient_id) {
        Ok(id) => id,
        Err(_) => return Redirect::to("/web/patients").into_response(),
    };

    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/web/patients").into_response(),
    };

    let patient_projection =
        rustyclinic_projections::sqlite::SqlitePatientSummaryProjection::new(&conn);

    let summary = match patient_projection.get_summary(session.session.facility_id, patient_id) {
        Ok(Some(summary)) => summary,
        _ => return Redirect::to("/web/patients").into_response(),
    };

    let patient_name = format!("{} {}", summary.given_name, summary.family_name);
    let page = PatientCardPrintPage {
        patient_id: patient_id.to_string(),
        patient_name,
        sex: summary.sex,
        age: summary
            .age
            .map(|age| age.to_string())
            .unwrap_or_else(|| "Unknown".to_string()),
        national_id: summary.national_id.unwrap_or_else(|| "—".to_string()),
        last_visit: summary
            .last_visit
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "No visits yet".to_string()),
        active_programs: summary.active_programs,
    };

    Html(page.to_string()).into_response()
}

pub fn lookup_patient_name(
    conn: &rusqlite::Connection,
    facility_id: Uuid,
    patient_id: Uuid,
) -> String {
    let projection = rustyclinic_projections::sqlite::SqlitePatientSummaryProjection::new(conn);
    projection
        .get_summary(facility_id, patient_id)
        .ok()
        .flatten()
        .map(|summary| format!("{} {}", summary.given_name, summary.family_name))
        .unwrap_or_else(|| "Unknown".to_string())
}

fn render_register_error(state: &WebAppState, user_id: Uuid, error: &str) -> Response {
    let (display_name, initials) = lookup_user_name(state, user_id);
    let page = PatientRegisterPage {
        active_nav: "register".to_string(),
        display_name,
        initials,
        flash_success: None,
        flash_error: None,
        error: Some(error.to_string()),
    };
    Html(page.to_string()).into_response()
}

pub(crate) fn lookup_user_name(state: &WebAppState, user_id: Uuid) -> (String, String) {
    if let Ok(conn) = rusqlite::Connection::open(&state.db_path) {
        let user_repo = rustyclinic_db::sqlite::user_repo::SqliteUserRepo::new(&conn);
        use rustyclinic_auth::users::UserRepo;
        if let Ok(Some(user)) = user_repo.find_by_id(user_id) {
            let initials = WebSession::initials_from(&user.display_name);
            return (user.display_name, initials);
        }
    }
    ("User".to_string(), "U".to_string())
}
