//! Lock a session (idle timeout or manual lock).

use rustyclinic_auth::session::SessionRepo;
use rustyclinic_core::error::{AppError, AppResult};
use uuid::Uuid;

pub struct LockSessionInput {
    pub session_id: Uuid,
}

pub fn execute(session_repo: &dyn SessionRepo, input: LockSessionInput) -> AppResult<()> {
    let mut session = session_repo
        .find_by_id(input.session_id)?
        .ok_or(AppError::NotFound {
            entity: "Session",
            id: input.session_id,
        })?;

    session.lock();
    session_repo.update(&session)?;

    tracing::info!(session_id = %input.session_id, "session locked");
    Ok(())
}
