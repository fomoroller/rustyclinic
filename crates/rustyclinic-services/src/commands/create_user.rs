//! Create a new user account.

use chrono::Utc;
use rustyclinic_auth::credentials::hash_credential;
use rustyclinic_auth::users::{User, UserRepo};
use rustyclinic_core::error::AppResult;
use rustyclinic_core::types::new_id;
use uuid::Uuid;

pub struct CreateUserInput {
    pub facility_id: Uuid,
    pub username: String,
    pub display_name: String,
    pub password: String,
    pub roles: Vec<String>,
}

/// Create a new user. Returns user ID.
pub fn execute(repo: &dyn UserRepo, input: CreateUserInput) -> AppResult<Uuid> {
    let user_id = new_id();
    let now = Utc::now();

    let password_hash = hash_credential(&input.password)?;

    let user = User {
        id: user_id,
        facility_id: input.facility_id,
        username: input.username.clone(),
        display_name: input.display_name,
        roles: input.roles,
        active: true,
        created_at: now,
        updated_at: now,
    };

    repo.create(&user, &password_hash)?;

    tracing::info!(user_id = %user_id, username = %input.username, "user created");

    Ok(user_id)
}
