//! Lock screen and PIN unlock routes.

use axum::Form;
use axum::extract::Query;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;
use uuid::Uuid;

use rustyclinic_auth::session::SessionRepo;
use rustyclinic_auth::users::UserRepo;

use crate::WebAppState;
use crate::middleware::session::{WebSession, parse_cookie};
use crate::templates::LockScreenPage;

#[derive(Deserialize, Default)]
pub struct LockQuery {
    pub session: Option<String>,
}

pub async fn lock_screen(
    State(state): State<WebAppState>,
    Query(query): Query<LockQuery>,
    headers: axum::http::HeaderMap,
) -> Response {
    let cookie_header = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let session_id = query
        .session
        .or_else(|| parse_cookie(cookie_header, "rustyclinic_session"));

    let (display_name, initials, sid) = match session_id {
        Some(sid) => {
            if let Ok(uuid) = Uuid::parse_str(&sid) {
                if let Ok(conn) = rusqlite::Connection::open(&state.db_path) {
                    let session_repo =
                        rustyclinic_db::sqlite::session_repo::SqliteSessionRepo::new(&conn);
                    if let Ok(Some(session)) = session_repo.find_by_id(uuid) {
                        let user_repo =
                            rustyclinic_db::sqlite::user_repo::SqliteUserRepo::new(&conn);
                        let name = match user_repo.find_by_id(session.user_id) {
                            Ok(Some(u)) => u.display_name,
                            _ => "User".to_string(),
                        };
                        let initials = WebSession::initials_from(&name);
                        (name, initials, sid)
                    } else {
                        return Redirect::to("/web/login").into_response();
                    }
                } else {
                    return Redirect::to("/web/login").into_response();
                }
            } else {
                return Redirect::to("/web/login").into_response();
            }
        }
        None => return Redirect::to("/web/login").into_response(),
    };

    let page = LockScreenPage {
        session_id: sid,
        display_name,
        initials,
        error: None,
    };
    Html(page.to_string()).into_response()
}

pub async fn lock_session(State(state): State<WebAppState>, session: WebSession) -> Response {
    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/web/login").into_response(),
    };

    let session_repo = rustyclinic_db::sqlite::session_repo::SqliteSessionRepo::new(&conn);

    let input = rustyclinic_services::commands::lock_session::LockSessionInput {
        session_id: session.session.id,
    };

    let _ = rustyclinic_services::commands::lock_session::execute(&session_repo, input);

    Redirect::to("/web/lock").into_response()
}

#[derive(Deserialize)]
pub struct UnlockForm {
    pub session_id: String,
    pub pin: String,
}

pub async fn unlock_session(
    State(state): State<WebAppState>,
    Form(form): Form<UnlockForm>,
) -> Response {
    let session_uuid = match Uuid::parse_str(&form.session_id) {
        Ok(u) => u,
        Err(_) => return Redirect::to("/web/login").into_response(),
    };

    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/web/login").into_response(),
    };

    let session_repo = rustyclinic_db::sqlite::session_repo::SqliteSessionRepo::new(&conn);
    let user_repo = rustyclinic_db::sqlite::user_repo::SqliteUserRepo::new(&conn);

    let input = rustyclinic_services::commands::unlock_session::UnlockSessionInput {
        session_id: session_uuid,
        pin: form.pin,
    };

    match rustyclinic_services::commands::unlock_session::execute(&session_repo, &user_repo, input)
    {
        Ok(()) => {
            let cookie = format!(
                "rustyclinic_session={}; HttpOnly; SameSite=Strict; Path=/; Max-Age=3600",
                session_uuid
            );

            Response::builder()
                .status(StatusCode::SEE_OTHER)
                .header(axum::http::header::LOCATION, "/web/queue")
                .header(axum::http::header::SET_COOKIE, &cookie)
                .body(axum::body::Body::empty())
                .expect("valid response")
                .into_response()
        }
        Err(e) => {
            let (display_name, initials) =
                if let Ok(Some(session)) = session_repo.find_by_id(session_uuid) {
                    let name = match user_repo.find_by_id(session.user_id) {
                        Ok(Some(u)) => u.display_name,
                        _ => "User".to_string(),
                    };
                    let initials = WebSession::initials_from(&name);
                    (name, initials)
                } else {
                    ("User".to_string(), "U".to_string())
                };

            let page = LockScreenPage {
                session_id: form.session_id,
                display_name,
                initials,
                error: Some(e.to_string()),
            };
            Html(page.to_string()).into_response()
        }
    }
}

/// htmx fragment: list locked sessions on this device.
pub async fn sessions_fragment(
    State(state): State<WebAppState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let cookie_header = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let session_id = parse_cookie(cookie_header, "rustyclinic_session");

    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Html(String::new()),
    };

    // Find the device_id from the current session
    let device_id = match session_id {
        Some(sid) => {
            if let Ok(uuid) = Uuid::parse_str(&sid) {
                let session_repo =
                    rustyclinic_db::sqlite::session_repo::SqliteSessionRepo::new(&conn);
                match session_repo.find_by_id(uuid) {
                    Ok(Some(s)) => s.device_id,
                    _ => return Html(String::new()),
                }
            } else {
                return Html(String::new());
            }
        }
        None => return Html(String::new()),
    };

    // Find locked sessions on this device
    let mut stmt = match conn.prepare(
        "SELECT s.id, u.display_name
         FROM sessions s
         JOIN users u ON u.id = s.user_id
         WHERE s.device_id = ?1 AND s.state = 'locked'
         ORDER BY s.locked_at DESC",
    ) {
        Ok(s) => s,
        Err(_) => return Html(String::new()),
    };

    let rows: Vec<(String, String)> = match stmt
        .query_map(rusqlite::params![device_id.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) {
        Ok(r) => r.filter_map(|r| r.ok()).collect(),
        Err(_) => vec![],
    };

    if rows.is_empty() {
        return Html(String::new());
    }

    let mut html = String::from(
        r#"<div class="mt-lg"><p class="text-sm text-secondary mb-sm">Switch user:</p><div class="session-list">"#,
    );
    for (sid, name) in &rows {
        let initials = WebSession::initials_from(name);
        html.push_str(&format!(
            r#"<a href="/web/lock?session={sid}" class="session-item"><div class="avatar">{initials}</div><span>{name}</span></a>"#,
        ));
    }
    html.push_str("</div></div>");
    Html(html)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, header};
    use chrono::Utc;
    use rustyclinic_auth::session::Session;
    use rustyclinic_auth::users::User;
    use rustyclinic_core::types::new_id;

    fn setup_state_with_db() -> WebAppState {
        let db_path = std::env::temp_dir().join(format!("rustyclinic-web-lock-{}.db", new_id()));
        let conn = rusqlite::Connection::open(&db_path).expect("open sqlite file");
        conn.pragma_update(None, "foreign_keys", "on").expect("fk");
        rustyclinic_db::migration::run_migrations(&conn).expect("migrations");

        WebAppState {
            db_path: db_path.to_string_lossy().to_string(),
            device_id: new_id(),
            facility_id: new_id(),
        }
    }

    fn create_locked_user_session(
        state: &WebAppState,
        username: &str,
        display_name: &str,
        password: &str,
        device_id: Uuid,
    ) -> Session {
        let conn = rusqlite::Connection::open(&state.db_path).expect("open sqlite file");
        let user_repo = rustyclinic_db::sqlite::user_repo::SqliteUserRepo::new(&conn);
        let session_repo = rustyclinic_db::sqlite::session_repo::SqliteSessionRepo::new(&conn);

        let now = Utc::now();
        let user = User {
            id: new_id(),
            facility_id: state.facility_id,
            username: username.to_string(),
            display_name: display_name.to_string(),
            roles: vec!["nurse".to_string()],
            active: true,
            created_at: now,
            updated_at: now,
        };
        let password_hash =
            rustyclinic_auth::credentials::hash_credential(password).expect("hash password");
        user_repo
            .create(&user, &password_hash)
            .expect("create user");

        let mut session = Session::new(
            user.id,
            state.facility_id,
            device_id,
            vec!["nurse".to_string()],
            "password",
        );
        session_repo.create(&session).expect("create session");
        session.lock();
        session_repo.update(&session).expect("lock session");
        session
    }

    #[tokio::test]
    async fn lock_screen_prefers_query_session_over_cookie() {
        let state = setup_state_with_db();
        let shared_device_id = new_id();
        let cookie_session = create_locked_user_session(
            &state,
            "cookie.user",
            "Cookie User",
            "1111",
            shared_device_id,
        );
        let selected_session = create_locked_user_session(
            &state,
            "selected.user",
            "Selected User",
            "2222",
            shared_device_id,
        );

        let mut headers = HeaderMap::new();
        let cookie = format!("rustyclinic_session={}", cookie_session.id);
        headers.insert(
            header::COOKIE,
            axum::http::HeaderValue::from_str(&cookie).expect("valid cookie header"),
        );

        let response = lock_screen(
            State(state.clone()),
            Query(LockQuery {
                session: Some(selected_session.id.to_string()),
            }),
            headers,
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let body_text = String::from_utf8(body.to_vec()).expect("utf8 body");

        assert!(body_text.contains("Selected User"));
        assert!(body_text.contains(&format!("value=\"{}\"", selected_session.id)));
        assert!(!body_text.contains("Cookie User"));

        let _ = std::fs::remove_file(&state.db_path);
    }

    #[tokio::test]
    async fn unlock_session_sets_session_cookie_for_unlocked_session() {
        let state = setup_state_with_db();
        let session = create_locked_user_session(
            &state,
            "unlock.user",
            "Unlock User",
            "1234",
            state.device_id,
        );

        let response = unlock_session(
            State(state.clone()),
            Form(UnlockForm {
                session_id: session.id.to_string(),
                pin: "1234".to_string(),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response
                .headers()
                .get(header::LOCATION)
                .expect("location header")
                .to_str()
                .expect("location as str"),
            "/web/queue"
        );

        let set_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .expect("set-cookie header")
            .to_str()
            .expect("set-cookie as str");
        assert!(set_cookie.contains(&format!("rustyclinic_session={}", session.id)));

        let _ = std::fs::remove_file(&state.db_path);
    }
}
