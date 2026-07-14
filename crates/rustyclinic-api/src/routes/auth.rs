//! Authentication endpoints.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::middleware::session::ApiSession;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub session_id: Uuid,
    pub user_id: Uuid,
    pub display_name: String,
    pub roles: Vec<String>,
}

#[derive(Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub display_name: String,
    pub password: String,
    pub roles: Vec<String>,
}

pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
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

    let user_repo = rustyclinic_db::sqlite::user_repo::SqliteUserRepo::new(&conn);
    let session_repo = rustyclinic_db::sqlite::session_repo::SqliteSessionRepo::new(&conn);

    // Use nil facility_id for now — in production, derived from the request context
    let input = rustyclinic_services::commands::login::LoginInput {
        facility_id: state.facility_id,
        username: req.username,
        password: req.password,
        device_id: state.device_id,
    };

    match rustyclinic_services::commands::login::execute(&user_repo, &session_repo, input) {
        Ok(output) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "session_id": output.session_id,
                "user_id": output.user_id,
                "display_name": output.display_name,
                "roles": output.roles,
            })),
        ),
        Err(e) => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub async fn create_user(
    State(state): State<AppState>,
    session: ApiSession,
    Json(req): Json<CreateUserRequest>,
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

    let user_repo = rustyclinic_db::sqlite::user_repo::SqliteUserRepo::new(&conn);

    let existing_count: u32 = conn
        .query_row(
            "SELECT COUNT(*) FROM users WHERE facility_id = ?1",
            rusqlite::params![state.facility_id.to_string()],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if existing_count == 0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "use bootstrap endpoint for first user" })),
        );
    }

    let roles = &session.actor.roles;
    let allowed = roles
        .iter()
        .any(|r| r == "system_admin" || r == "facility_admin");
    if !allowed {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "forbidden" })),
        );
    }

    let input = rustyclinic_services::commands::create_user::CreateUserInput {
        facility_id: state.facility_id,
        username: req.username,
        display_name: req.display_name,
        password: req.password,
        roles: req.roles,
    };

    match rustyclinic_services::commands::create_user::execute(&user_repo, input) {
        Ok(user_id) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "id": user_id,
                "message": "user created"
            })),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

pub async fn bootstrap_user(
    State(state): State<AppState>,
    Json(req): Json<CreateUserRequest>,
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

    let existing_count: u32 = conn
        .query_row(
            "SELECT COUNT(*) FROM users WHERE facility_id = ?1",
            rusqlite::params![state.facility_id.to_string()],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if existing_count > 0 {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "bootstrap not allowed" })),
        );
    }

    let user_repo = rustyclinic_db::sqlite::user_repo::SqliteUserRepo::new(&conn);

    let input = rustyclinic_services::commands::create_user::CreateUserInput {
        facility_id: state.facility_id,
        username: req.username,
        display_name: req.display_name,
        password: req.password,
        roles: req.roles,
    };

    match rustyclinic_services::commands::create_user::execute(&user_repo, input) {
        Ok(user_id) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "id": user_id, "message": "user created" })),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}
