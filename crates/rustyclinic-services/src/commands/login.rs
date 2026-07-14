//! Authenticate a user and create a session.

use rustyclinic_auth::credentials::verify_credential;
use rustyclinic_auth::session::{Session, SessionRepo, SessionState};
use rustyclinic_auth::users::UserRepo;
use rustyclinic_core::error::{AppError, AppResult};
use uuid::Uuid;

pub struct LoginInput {
    pub facility_id: Uuid,
    pub username: String,
    pub password: String,
    pub device_id: Uuid,
}

pub struct LoginOutput {
    pub session_id: Uuid,
    pub user_id: Uuid,
    pub display_name: String,
    pub roles: Vec<String>,
    pub requires_pin_setup: bool,
}

/// Authenticate user and create a session.
pub fn execute(
    user_repo: &dyn UserRepo,
    session_repo: &dyn SessionRepo,
    input: LoginInput,
) -> AppResult<LoginOutput> {
    // Find user
    let (user, password_hash, pin_hash) = user_repo
        .find_by_username(input.facility_id, &input.username)?
        .ok_or_else(|| AppError::AuthorizationDenied {
            reason: "invalid username or password".to_string(),
        })?;

    if !user.active {
        return Err(AppError::AuthorizationDenied {
            reason: "account is disabled".to_string(),
        });
    }

    // Verify password
    let valid = verify_credential(&input.password, &password_hash)?;
    if !valid {
        return Err(AppError::AuthorizationDenied {
            reason: "invalid username or password".to_string(),
        });
    }

    if session_repo.count_locked_by_device(input.device_id)? >= 3 {
        let mut locked_sessions = session_repo
            .find_active_by_device(input.device_id)?
            .into_iter()
            .filter(|session| session.state == SessionState::Locked)
            .collect::<Vec<_>>();

        locked_sessions.sort_by_key(|session| session.locked_at.unwrap_or(session.created_at));

        if let Some(mut oldest_locked) = locked_sessions.into_iter().next() {
            oldest_locked.state = SessionState::Terminated;
            oldest_locked.locked_at = None;
            session_repo.update(&oldest_locked)?;
        }
    }

    // Create session
    let session = Session::new(
        user.id,
        user.facility_id,
        input.device_id,
        user.roles.clone(),
        "password",
    );

    session_repo.create(&session)?;

    tracing::info!(
        user_id = %user.id,
        username = %user.username,
        session_id = %session.id,
        "user logged in"
    );

    Ok(LoginOutput {
        session_id: session.id,
        user_id: user.id,
        display_name: user.display_name,
        roles: user.roles,
        requires_pin_setup: pin_hash.is_none(),
    })
}
