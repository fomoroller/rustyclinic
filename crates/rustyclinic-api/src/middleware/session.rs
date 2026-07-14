use axum::Json;
use axum::extract::FromRequestParts;
use axum::http::{StatusCode, header, request::Parts};
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

use rustyclinic_auth::session::{Session, SessionRepo, SessionState};
use rustyclinic_core::types::ActorContext;
use rustyclinic_db::sqlite::session_repo::SqliteSessionRepo;

use crate::state::AppState;

pub struct ApiSession {
    pub session: Session,
    pub actor: ActorContext,
}

impl FromRequestParts<AppState> for ApiSession {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let session_id = authorization_bearer(parts)
            .map(|s| s.to_string())
            .or_else(|| cookie_session(parts))
            .ok_or_else(|| unauthorized("missing session"))?;

        let session_id =
            Uuid::parse_str(session_id.trim()).map_err(|_| unauthorized("invalid session"))?;

        let conn = rusqlite::Connection::open(&state.db_path)
            .map_err(|_| unauthorized("database error"))?;

        let session_repo = SqliteSessionRepo::new(&conn);
        let mut session = session_repo
            .find_by_id(session_id)
            .map_err(|_| unauthorized("invalid session"))?
            .ok_or_else(|| unauthorized("invalid session"))?;

        if session.state == SessionState::Locked {
            return Err(unauthorized("session locked"));
        }

        if !session.is_valid() {
            return Err(unauthorized("session expired"));
        }

        session.touch();
        let _ = session_repo.update(&session);

        let actor = session.to_actor_context();

        Ok(Self { session, actor })
    }
}

fn authorization_bearer(parts: &Parts) -> Option<&str> {
    let value = parts.headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ")
}

fn cookie_session(parts: &Parts) -> Option<String> {
    let cookie_header = parts.headers.get(header::COOKIE)?.to_str().ok()?;
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

fn unauthorized(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({ "error": message })),
    )
        .into_response()
}
