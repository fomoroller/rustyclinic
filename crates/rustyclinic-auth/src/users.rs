//! User accounts and roles.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub facility_id: Uuid,
    pub username: String,
    pub display_name: String,
    pub roles: Vec<String>,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Repository trait for user persistence.
pub trait UserRepo {
    fn create(&self, user: &User, password_hash: &str) -> rustyclinic_core::error::AppResult<()>;
    fn find_by_id(&self, id: Uuid) -> rustyclinic_core::error::AppResult<Option<User>>;
    fn find_by_username(
        &self,
        facility_id: Uuid,
        username: &str,
    ) -> rustyclinic_core::error::AppResult<Option<(User, String, Option<String>)>>;
    fn update_pin_hash(
        &self,
        user_id: Uuid,
        pin_hash: &str,
    ) -> rustyclinic_core::error::AppResult<()>;
}
