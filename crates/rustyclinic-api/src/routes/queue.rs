//! Queue management endpoints.

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use uuid::Uuid;

use crate::middleware::session::ApiSession;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct EnqueueRequest {
    pub patient_id: Uuid,
    pub service_type: String,
}

pub async fn enqueue_patient(
    State(state): State<AppState>,
    session: ApiSession,
    headers: HeaderMap,
    Json(req): Json<EnqueueRequest>,
) -> impl IntoResponse {
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

    let input = rustyclinic_services::commands::enqueue_patient::EnqueuePatientInput {
        patient_id: req.patient_id,
        service_type: req.service_type,
    };

    let repo = rustyclinic_db::sqlite::queue_repo::SqliteQueueRepo::new(&conn);
    let mut uow = match rustyclinic_db::sqlite::unit_of_work::UnitOfWork::try_new(&conn) {
        Ok(u) => u,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("uow error: {e}") })),
            );
        }
    };

    match rustyclinic_services::commands::enqueue_patient::execute(&mut uow, &repo, &actor, input) {
        Ok(entry_id) => {
            if let Some(key) = idempotency_key {
                uow.record_idempotency(
                    actor.facility_id,
                    key,
                    serde_json::json!({
                        "status": 201,
                        "body": {"id": entry_id, "message": "patient enqueued"}
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
                    "id": entry_id,
                    "message": "patient enqueued"
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
