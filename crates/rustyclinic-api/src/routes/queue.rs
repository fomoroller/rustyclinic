//! Queue management endpoints.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use super::patients::{AppState, placeholder_actor};

#[derive(Deserialize)]
pub struct EnqueueRequest {
    pub patient_id: Uuid,
    pub service_type: String,
}

pub async fn enqueue_patient(
    State(state): State<AppState>,
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

    let actor = placeholder_actor();

    let input = rustyclinic_services::commands::enqueue_patient::EnqueuePatientInput {
        patient_id: req.patient_id,
        service_type: req.service_type,
    };

    let repo = rustyclinic_db::sqlite::queue_repo::SqliteQueueRepo::new(&conn);
    let mut uow = rustyclinic_db::sqlite::unit_of_work::UnitOfWork::new(&conn);

    match rustyclinic_services::commands::enqueue_patient::execute(&mut uow, &repo, &actor, input) {
        Ok(entry_id) => {
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
