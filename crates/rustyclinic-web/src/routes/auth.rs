//! Login and logout routes.

use axum::Form;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use rustyclinic_auth::credentials::hash_credential;
use rustyclinic_auth::session::SessionRepo;
use rustyclinic_auth::users::UserRepo;
use serde::Deserialize;

use crate::WebAppState;
use crate::middleware::session::WebSession;
use crate::routes::patients::lookup_user_name;
use crate::templates::LoginPage;

pub async fn login_page() -> impl IntoResponse {
    let page = LoginPage { error: None };
    Html(page.to_string())
}

#[derive(Deserialize)]
pub struct LoginForm {
    pub username: String,
    pub password: String,
}

pub async fn login_submit(
    State(state): State<WebAppState>,
    Form(form): Form<LoginForm>,
) -> Response {
    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(e) => {
            let page = LoginPage {
                error: Some(format!("Database error: {e}")),
            };
            return Html(page.to_string()).into_response();
        }
    };

    let user_repo = rustyclinic_db::sqlite::user_repo::SqliteUserRepo::new(&conn);
    let session_repo = rustyclinic_db::sqlite::session_repo::SqliteSessionRepo::new(&conn);

    let input = rustyclinic_services::commands::login::LoginInput {
        facility_id: state.facility_id,
        username: form.username,
        password: form.password,
        device_id: state.device_id,
    };

    match rustyclinic_services::commands::login::execute(&user_repo, &session_repo, input) {
        Ok(output) => {
            let redirect_path = if output.requires_pin_setup {
                "/web/pin/setup"
            } else {
                "/web/queue"
            };
            let cookie = format!(
                "rustyclinic_session={}; HttpOnly; SameSite=Strict; Path=/; Max-Age=3600",
                output.session_id
            );
            Response::builder()
                .status(StatusCode::SEE_OTHER)
                .header(axum::http::header::LOCATION, redirect_path)
                .header(axum::http::header::SET_COOKIE, &cookie)
                .body(axum::body::Body::empty())
                .expect("valid response")
                .into_response()
        }
        Err(e) => {
            let page = LoginPage {
                error: Some(e.to_string()),
            };
            Html(page.to_string()).into_response()
        }
    }
}

pub async fn pin_setup_page(State(state): State<WebAppState>, session: WebSession) -> Response {
    let (display_name, initials) = lookup_user_name(&state, session.session.user_id);
    Html(render_pin_setup_page(&display_name, &initials, None)).into_response()
}

#[derive(Deserialize)]
pub struct PinSetupForm {
    pub pin: String,
    pub pin_confirm: String,
}

pub async fn pin_setup_submit(
    State(state): State<WebAppState>,
    session: WebSession,
    Form(form): Form<PinSetupForm>,
) -> Response {
    let (display_name, initials) = lookup_user_name(&state, session.session.user_id);

    if form.pin.len() < 4 || !form.pin.chars().all(|c| c.is_ascii_digit()) {
        return Html(render_pin_setup_page(
            &display_name,
            &initials,
            Some("PIN must be at least 4 digits."),
        ))
        .into_response();
    }

    if form.pin != form.pin_confirm {
        return Html(render_pin_setup_page(
            &display_name,
            &initials,
            Some("PIN confirmation does not match."),
        ))
        .into_response();
    }

    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => {
            return Html(render_pin_setup_page(
                &display_name,
                &initials,
                Some("Database error while saving PIN."),
            ))
            .into_response();
        }
    };

    let user_repo = rustyclinic_db::sqlite::user_repo::SqliteUserRepo::new(&conn);
    let pin_hash = match hash_credential(&form.pin) {
        Ok(h) => h,
        Err(e) => {
            return Html(render_pin_setup_page(
                &display_name,
                &initials,
                Some(&e.to_string()),
            ))
            .into_response();
        }
    };

    match user_repo.update_pin_hash(session.session.user_id, &pin_hash) {
        Ok(()) => Redirect::to("/web/queue").into_response(),
        Err(e) => Html(render_pin_setup_page(
            &display_name,
            &initials,
            Some(&e.to_string()),
        ))
        .into_response(),
    }
}

fn render_pin_setup_page(display_name: &str, initials: &str, error: Option<&str>) -> String {
    let error_html = match error {
        Some(message) => format!(
            r#"<div class="flash flash-error mb-md"><svg width="20" height="20" viewBox="0 0 20 20" fill="currentColor"><path fill-rule="evenodd" d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-7 4a1 1 0 11-2 0 1 1 0 012 0zm-1-9a1 1 0 00-1 1v4a1 1 0 102 0V6a1 1 0 00-1-1z" clip-rule="evenodd"/></svg>{message}</div>"#
        ),
        None => String::new(),
    };

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Set PIN — RustyClinic</title>
  <link rel="stylesheet" href="/static/css/app.css">
</head>
<body>
  <div class="auth-layout">
    <div class="auth-card">
      <h1 class="auth-title">Set your unlock PIN</h1>
      <p class="auth-subtitle">Hi {display_name} ({initials}), create a PIN for quick unlock.</p>
      {error_html}
      <form method="post" action="/web/pin/setup">
        <div class="form-group">
          <label class="form-label" for="pin">PIN</label>
          <input class="form-input" type="password" id="pin" name="pin" inputmode="numeric" pattern="[0-9]*" minlength="4" maxlength="8" required autofocus>
        </div>
        <div class="form-group">
          <label class="form-label" for="pin_confirm">Confirm PIN</label>
          <input class="form-input" type="password" id="pin_confirm" name="pin_confirm" inputmode="numeric" pattern="[0-9]*" minlength="4" maxlength="8" required>
        </div>
        <button type="submit" class="btn btn-accent w-full mt-md">Save PIN</button>
      </form>
    </div>
  </div>
</body>
</html>"#
    )
}

pub async fn logout(State(state): State<WebAppState>, session: WebSession) -> Response {
    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return Redirect::to("/web/login").into_response(),
    };

    let session_repo = rustyclinic_db::sqlite::session_repo::SqliteSessionRepo::new(&conn);

    let mut s = session.session;
    s.state = rustyclinic_auth::session::SessionState::Terminated;
    let _ = session_repo.update(&s);

    let cookie = "rustyclinic_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0";
    (
        StatusCode::SEE_OTHER,
        [
            (axum::http::header::LOCATION, "/web/login"),
            (axum::http::header::SET_COOKIE, cookie),
        ],
    )
        .into_response()
}
