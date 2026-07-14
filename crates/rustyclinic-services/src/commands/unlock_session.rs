//! Unlock a locked session with PIN verification.

use rustyclinic_auth::credentials::verify_credential;
use rustyclinic_auth::session::SessionRepo;
use rustyclinic_auth::users::UserRepo;
use rustyclinic_core::error::{AppError, AppResult};
use uuid::Uuid;

pub struct UnlockSessionInput {
    pub session_id: Uuid,
    pub pin: String,
}

pub fn execute(
    session_repo: &dyn SessionRepo,
    user_repo: &dyn UserRepo,
    input: UnlockSessionInput,
) -> AppResult<()> {
    let mut session = session_repo
        .find_by_id(input.session_id)?
        .ok_or(AppError::NotFound {
            entity: "Session",
            id: input.session_id,
        })?;

    // Load user to get pin_hash
    let user = user_repo
        .find_by_id(session.user_id)?
        .ok_or(AppError::NotFound {
            entity: "User",
            id: session.user_id,
        })?;

    let (_user, password_hash, pin_hash) = user_repo
        .find_by_username(user.facility_id, &user.username)?
        .ok_or(AppError::NotFound {
            entity: "User",
            id: session.user_id,
        })?;

    let verification_hash = pin_hash.as_deref().unwrap_or(&password_hash);
    let valid = verify_credential(&input.pin, verification_hash)?;
    if !valid {
        return Err(AppError::AuthorizationDenied {
            reason: "invalid PIN".to_string(),
        });
    }

    session.unlock();
    session_repo.update(&session)?;

    tracing::info!(session_id = %input.session_id, "session unlocked");
    Ok(())
}
