//! Authenticate a user and create a session.

use uuid::Uuid;
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_auth::credentials::verify_credential;
use rustyclinic_auth::session::Session;
use rustyclinic_auth::users::UserRepo;
use rustyclinic_auth::session::SessionRepo;

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
}

/// Authenticate user and create a session.
pub fn execute(
    user_repo: &dyn UserRepo,
    session_repo: &dyn SessionRepo,
    input: LoginInput,
) -> AppResult<LoginOutput> {
    // Find user
    let (user, password_hash) = user_repo
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
    })
}
