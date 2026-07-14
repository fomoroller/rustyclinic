//! Patient registration and search endpoints.

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use rustyclinic_core::types::Sex;

use crate::middleware::session::ApiSession;
use crate::state::AppState;

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
    session: ApiSession,
    headers: HeaderMap,
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

    let actor = session.actor;

    let idempotency_key = headers
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string());

    if let Some(key) = &idempotency_key
        && let Ok(Some(cached)) =
            rustyclinic_db::sqlite::idempotency::check_idempotency(&conn, actor.facility_id, key)
    {
        return replay_idempotent(cached);
    }

    let sex = match req.sex.to_lowercase().as_str() {
        "female" | "f" => Sex::Female,
        "male" | "m" => Sex::Male,
        "other" => Sex::Other,
        _ => Sex::Unknown,
    };

    let dob = req
        .date_of_birth
        .as_deref()
        .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());

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
    let mut uow = match rustyclinic_db::sqlite::unit_of_work::UnitOfWork::try_new(&conn) {
        Ok(u) => u,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("uow error: {e}") })),
            );
        }
    };

    match rustyclinic_services::commands::register_patient::execute(&mut uow, &repo, &actor, input)
    {
        Ok(patient_id) => {
            if let Some(key) = idempotency_key {
                uow.record_idempotency(
                    actor.facility_id,
                    key,
                    serde_json::json!({
                        "status": 201,
                        "body": {"id": patient_id, "message": "patient registered"}
                    }),
                );
            }
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

fn replay_idempotent(cached: serde_json::Value) -> (StatusCode, Json<serde_json::Value>) {
    let status = cached
        .get("status")
        .and_then(|v| v.as_u64())
        .and_then(|s| u16::try_from(s).ok())
        .and_then(|s| StatusCode::from_u16(s).ok())
        .unwrap_or(StatusCode::OK);
    let body = cached.get("body").cloned().unwrap_or(cached);
    (status, Json(body))
}
