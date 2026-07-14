//! Cookie-based session middleware for the web UI.

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Redirect, Response};
use uuid::Uuid;

use rustyclinic_auth::session::{Session, SessionRepo, SessionState};
use rustyclinic_db::sqlite::session_repo::SqliteSessionRepo;

use crate::WebAppState;

/// Extracted from the request — provides the authenticated session and actor context.
pub struct WebSession {
    pub session: Session,
}

impl WebSession {
    pub fn initials_from(name: &str) -> String {
        name.split_whitespace()
            .filter_map(|w| w.chars().next())
            .take(2)
            .collect::<String>()
            .to_uppercase()
    }
}

impl FromRequestParts<WebAppState> for WebSession {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &WebAppState,
    ) -> Result<Self, Self::Rejection> {
        // Extract session cookie
        let cookie_header = parts
            .headers
            .get(axum::http::header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        let session_id = parse_cookie(cookie_header, "rustyclinic_session");

        let session_id = match session_id {
            Some(id) => id,
            None => return Err(Redirect::to("/web/login").into_response()),
        };

        let uuid = match Uuid::parse_str(&session_id) {
            Ok(u) => u,
            Err(_) => return Err(Redirect::to("/web/login").into_response()),
        };

        // Open connection and look up session
        let conn = match rusqlite::Connection::open(&state.db_path) {
            Ok(c) => c,
            Err(_) => {
                return Err((StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response());
            }
        };

        let session_repo = SqliteSessionRepo::new(&conn);

        let mut session = match session_repo.find_by_id(uuid) {
            Ok(Some(s)) => s,
            _ => return Err(Redirect::to("/web/login").into_response()),
        };

        // Check validity
        if session.state == SessionState::Locked {
            return Err(Redirect::to("/web/lock").into_response());
        }

        if !session.is_valid() {
            return Err(Redirect::to("/web/login").into_response());
        }

        // Check idle timeout (15 minutes)
        if session.should_lock(15) {
            session.lock();
            let _ = session_repo.update(&session);
            return Err(Redirect::to("/web/lock").into_response());
        }

        // Touch session
        session.touch();
        let _ = session_repo.update(&session);

        Ok(WebSession { session })
    }
}

pub fn parse_cookie(header: &str, name: &str) -> Option<String> {
    for pair in header.split(';') {
        let pair = pair.trim();
        if let Some(value) = pair.strip_prefix(&format!("{name}=")) {
            return Some(value.to_string());
        }
    }
    None
}
