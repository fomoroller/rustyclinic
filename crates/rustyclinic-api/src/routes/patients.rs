//! Patient registration and search endpoints.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use rustyclinic_core::types::{ActorContext, Sex};

/// Shared application state passed to all handlers.
pub type AppState = Arc<AppStateInner>;

pub struct AppStateInner {
    pub db_path: String,
}

#[derive(Deserialize)]
pub struct RegisterPatientRequest {
    pub given_name: String,
    pub family_name: String,
    pub sex: String,
    pub date_of_birth: Option<String>,
    pub phone: Option<String>,
    pub address: Option<String>,
    pub national_id: Option<String>,
}

#[derive(Serialize)]
pub struct RegisterPatientResponse {
    pub id: Uuid,
    pub message: String,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

pub async fn register_patient(
    State(state): State<AppState>,
    Json(req): Json<RegisterPatientRequest>,
) -> impl IntoResponse {
    // Open connection (in production, this would use a connection pool)
    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("db error: {e}") })),
            );
        }
    };

    // For now, use a placeholder actor context
    // In production, this comes from the auth middleware
    let actor = placeholder_actor();

    let sex = match req.sex.to_lowercase().as_str() {
        "female" | "f" => Sex::Female,
        "male" | "m" => Sex::Male,
        "other" => Sex::Other,
        _ => Sex::Unknown,
    };

    let dob = req.date_of_birth.as_deref().and_then(|s| {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
    });

    let input = rustyclinic_services::commands::register_patient::RegisterPatientInput {
        given_name: req.given_name,
        family_name: req.family_name,
        sex,
        date_of_birth: dob,
        phone: req.phone,
        address: req.address,
        national_id: req.national_id,
    };

    let repo = rustyclinic_db::sqlite::patient_repo::SqlitePatientRepo::new(&conn);
    let mut uow = rustyclinic_db::sqlite::unit_of_work::UnitOfWork::new(&conn);

    match rustyclinic_services::commands::register_patient::execute(&mut uow, &repo, &actor, input) {
        Ok(patient_id) => {
            if let Err(e) = uow.commit() {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("commit failed: {e}") })),
                );
            }
            (
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "id": patient_id,
                    "message": "patient registered"
                })),
            )
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

/// Placeholder actor context until auth is implemented.
pub fn placeholder_actor() -> ActorContext {
    ActorContext {
        user_id: Uuid::nil(),
        facility_id: Uuid::nil(),
        device_id: Uuid::nil(),
        roles: vec!["admin".to_string()],
        purpose: "clinical_care".to_string(),
        session_id: Uuid::nil(),
    }
}
