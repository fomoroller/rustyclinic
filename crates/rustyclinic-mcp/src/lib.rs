//! MCP (Model Context Protocol) transport.
//!
//! Invokes the same service commands as other interfaces.
//! Agents get no privileged bypass.

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use rustyclinic_auth::session::{Session, SessionRepo, SessionState};
use rustyclinic_core::types::ActorContext;
use rustyclinic_db::sqlite::session_repo::SqliteSessionRepo;

#[derive(Clone)]
pub struct McpState {
    pub db_path: String,
}

#[derive(Serialize)]
struct ToolsResponse {
    tools: Vec<&'static str>,
}

#[derive(Debug, Deserialize)]
struct InvokeRequest {
    tool: String,
    #[serde(default)]
    args: serde_json::Value,
    #[serde(default)]
    idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RegisterPatientArgs {
    given_name: String,
    family_name: String,
    sex: String,
    #[serde(default)]
    date_of_birth: Option<String>,
    #[serde(default)]
    phone: Option<String>,
    #[serde(default)]
    address: Option<String>,
    #[serde(default)]
    national_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EnqueuePatientArgs {
    patient_id: Uuid,
    service_type: String,
}

pub fn mcp_router(state: McpState) -> Router {
    Router::new()
        .route("/mcp/health", get(health))
        .route("/mcp/tools", get(list_tools))
        .route("/mcp/invoke", post(invoke))
        .with_state(state)
}

async fn health(State(_state): State<McpState>) -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"})))
}

async fn list_tools(State(_state): State<McpState>) -> impl IntoResponse {
    Json(ToolsResponse {
        tools: vec![
            "register_patient",
            "enqueue_patient",
            "create_encounter",
            "install_package",
        ],
    })
}

async fn invoke(
    State(state): State<McpState>,
    headers: HeaderMap,
    Json(req): Json<InvokeRequest>,
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

    let mut session = match resolve_session(&conn, &headers) {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    if session.state == SessionState::Locked {
        return unauthorized("session locked");
    }
    if !session.is_valid() {
        return unauthorized("session expired");
    }

    session.touch();
    let session_repo = SqliteSessionRepo::new(&conn);
    let _ = session_repo.update(&session);

    let mut actor = session.to_actor_context();
    actor.purpose = "mcp".to_string();

    let idempotency_key = header_idempotency_key(&headers).or_else(|| {
        req.idempotency_key
            .as_deref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    });

    if let Some(key) = &idempotency_key
        && let Ok(Some(cached)) =
            rustyclinic_db::sqlite::idempotency::check_idempotency(&conn, actor.facility_id, key)
    {
        return replay_idempotent(cached);
    }

    match req.tool.as_str() {
        "register_patient" => invoke_register_patient(&conn, &actor, idempotency_key, req.args),
        "enqueue_patient" => invoke_enqueue_patient(&conn, &actor, idempotency_key, req.args),
        "create_encounter" | "install_package" => (
            StatusCode::NOT_IMPLEMENTED,
            Json(serde_json::json!({ "error": "tool not implemented" })),
        ),
        other => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("unknown tool: {other}") })),
        ),
    }
}

fn invoke_register_patient(
    conn: &rusqlite::Connection,
    actor: &ActorContext,
    idempotency_key: Option<String>,
    args: serde_json::Value,
) -> (StatusCode, Json<serde_json::Value>) {
    let args: RegisterPatientArgs = match serde_json::from_value(args) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("invalid args: {e}") })),
            );
        }
    };

    let sex = match args.sex.to_lowercase().as_str() {
        "female" | "f" => rustyclinic_core::types::Sex::Female,
        "male" | "m" => rustyclinic_core::types::Sex::Male,
        "other" => rustyclinic_core::types::Sex::Other,
        _ => rustyclinic_core::types::Sex::Unknown,
    };

    let dob = args
        .date_of_birth
        .as_deref()
        .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());

    let input = rustyclinic_services::commands::register_patient::RegisterPatientInput {
        given_name: args.given_name,
        family_name: args.family_name,
        sex,
        date_of_birth: dob,
        phone: args.phone,
        address: args.address,
        national_id: args.national_id,
    };

    let repo = rustyclinic_db::sqlite::patient_repo::SqlitePatientRepo::new(conn);
    let mut uow = match rustyclinic_db::sqlite::unit_of_work::UnitOfWork::try_new(conn) {
        Ok(u) => u,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("uow error: {e}") })),
            );
        }
    };

    match rustyclinic_services::commands::register_patient::execute(&mut uow, &repo, actor, input) {
        Ok(patient_id) => {
            let body = serde_json::json!({
                "id": patient_id,
                "message": "patient registered"
            });
            if let Some(key) = idempotency_key {
                uow.record_idempotency(
                    actor.facility_id,
                    key,
                    serde_json::json!({"status": 201, "body": body}),
                );
            }
            if let Err(e) = uow.commit() {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("commit failed: {e}") })),
                );
            }
            (StatusCode::CREATED, Json(body))
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

fn invoke_enqueue_patient(
    conn: &rusqlite::Connection,
    actor: &ActorContext,
    idempotency_key: Option<String>,
    args: serde_json::Value,
) -> (StatusCode, Json<serde_json::Value>) {
    let args: EnqueuePatientArgs = match serde_json::from_value(args) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("invalid args: {e}") })),
            );
        }
    };

    let input = rustyclinic_services::commands::enqueue_patient::EnqueuePatientInput {
        patient_id: args.patient_id,
        service_type: args.service_type,
    };

    let repo = rustyclinic_db::sqlite::queue_repo::SqliteQueueRepo::new(conn);
    let mut uow = match rustyclinic_db::sqlite::unit_of_work::UnitOfWork::try_new(conn) {
        Ok(u) => u,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("uow error: {e}") })),
            );
        }
    };

    match rustyclinic_services::commands::enqueue_patient::execute(&mut uow, &repo, actor, input) {
        Ok(entry_id) => {
            let body = serde_json::json!({
                "id": entry_id,
                "message": "patient enqueued"
            });
            if let Some(key) = idempotency_key {
                uow.record_idempotency(
                    actor.facility_id,
                    key,
                    serde_json::json!({"status": 201, "body": body}),
                );
            }
            if let Err(e) = uow.commit() {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("commit failed: {e}") })),
                );
            }
            (StatusCode::CREATED, Json(body))
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

fn resolve_session(
    conn: &rusqlite::Connection,
    headers: &HeaderMap,
) -> Result<Session, (StatusCode, Json<serde_json::Value>)> {
    let session_id = authorization_bearer(headers)
        .map(|s| s.to_string())
        .or_else(|| cookie_session(headers))
        .ok_or_else(|| unauthorized("missing session"))?;

    let session_id =
        Uuid::parse_str(session_id.trim()).map_err(|_| unauthorized("invalid session"))?;

    let session_repo = SqliteSessionRepo::new(conn);
    session_repo
        .find_by_id(session_id)
        .map_err(|_| unauthorized("invalid session"))?
        .ok_or_else(|| unauthorized("invalid session"))
}

fn authorization_bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ")
}

fn cookie_session(headers: &HeaderMap) -> Option<String> {
    let cookie_header = headers.get(header::COOKIE)?.to_str().ok()?;
    parse_cookie(cookie_header, "rustyclinic_session")
}

fn parse_cookie(header_value: &str, name: &str) -> Option<String> {
    for pair in header_value.split(';') {
        let pair = pair.trim();
        if let Some(value) = pair.strip_prefix(&format!("{name}=")) {
            return Some(value.to_string());
        }
    }
    None
}

fn header_idempotency_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
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

fn unauthorized(message: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({ "error": message })),
    )
}
