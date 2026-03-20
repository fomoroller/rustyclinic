//! Session management for shared-device environments.
//!
//! ```text
//! SESSION STATE MACHINE:
//!
//!   [Login] ──▶ ACTIVE ──idle timeout──▶ LOCKED ──re-auth──▶ ACTIVE
//!                 │                        │
//!                 │                        ├──expired──▶ EXPIRED
//!                 │                        └──revoked──▶ REVOKED
//!                 └──logout──▶ TERMINATED
//! ```

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use rustyclinic_core::types::{new_id, ActorContext};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    Active,
    Locked,
    Expired,
    Revoked,
    Terminated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub user_id: Uuid,
    pub facility_id: Uuid,
    pub device_id: Uuid,
    pub roles: Vec<String>,
    pub auth_method: String,
    pub state: SessionState,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_active: DateTime<Utc>,
    pub locked_at: Option<DateTime<Utc>>,
}

impl Session {
    /// Create a new active session (1-hour default expiry).
    pub fn new(
        user_id: Uuid,
        facility_id: Uuid,
        device_id: Uuid,
        roles: Vec<String>,
        auth_method: &str,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: new_id(),
            user_id,
            facility_id,
            device_id,
            roles,
            auth_method: auth_method.to_string(),
            state: SessionState::Active,
            created_at: now,
            expires_at: now + Duration::hours(1),
            last_active: now,
            locked_at: None,
        }
    }

    /// Check if the session is usable (active and not expired).
    pub fn is_valid(&self) -> bool {
        self.state == SessionState::Active && Utc::now() < self.expires_at
    }

    /// Lock the session (idle timeout). Context is preserved.
    pub fn lock(&mut self) {
        if self.state == SessionState::Active {
            self.state = SessionState::Locked;
            self.locked_at = Some(Utc::now());
        }
    }

    /// Unlock with re-authentication. Extends expiry.
    pub fn unlock(&mut self) {
        if self.state == SessionState::Locked {
            let now = Utc::now();
            self.state = SessionState::Active;
            self.last_active = now;
            self.expires_at = now + Duration::hours(1);
            self.locked_at = None;
        }
    }

    /// Touch — update last_active timestamp.
    pub fn touch(&mut self) {
        if self.state == SessionState::Active {
            self.last_active = Utc::now();
        }
    }

    /// Check if the session should be locked due to idle timeout.
    pub fn should_lock(&self, idle_timeout_minutes: i64) -> bool {
        self.state == SessionState::Active
            && Utc::now() > self.last_active + Duration::minutes(idle_timeout_minutes)
    }

    /// Convert session to actor context for service layer.
    pub fn to_actor_context(&self) -> ActorContext {
        ActorContext {
            user_id: self.user_id,
            facility_id: self.facility_id,
            device_id: self.device_id,
            roles: self.roles.clone(),
            purpose: "clinical_care".to_string(),
            session_id: self.id,
        }
    }
}

/// Repository trait for session persistence.
pub trait SessionRepo {
    fn create(&self, session: &Session) -> rustyclinic_core::error::AppResult<()>;
    fn find_by_id(&self, id: Uuid) -> rustyclinic_core::error::AppResult<Option<Session>>;
    fn update(&self, session: &Session) -> rustyclinic_core::error::AppResult<()>;
    fn find_active_by_device(
        &self,
        device_id: Uuid,
    ) -> rustyclinic_core::error::AppResult<Vec<Session>>;
    fn count_locked_by_device(&self, device_id: Uuid) -> rustyclinic_core::error::AppResult<u32>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_session_is_active() {
        let s = Session::new(new_id(), new_id(), new_id(), vec!["nurse".into()], "password");
        assert!(s.is_valid());
        assert_eq!(s.state, SessionState::Active);
    }

    #[test]
    fn test_lock_and_unlock() {
        let mut s = Session::new(new_id(), new_id(), new_id(), vec!["nurse".into()], "password");
        s.lock();
        assert_eq!(s.state, SessionState::Locked);
        assert!(!s.is_valid());

        s.unlock();
        assert_eq!(s.state, SessionState::Active);
        assert!(s.is_valid());
    }

    #[test]
    fn test_to_actor_context() {
        let user_id = new_id();
        let facility_id = new_id();
        let s = Session::new(user_id, facility_id, new_id(), vec!["nurse".into()], "pin");

        let actor = s.to_actor_context();
        assert_eq!(actor.user_id, user_id);
        assert_eq!(actor.facility_id, facility_id);
        assert_eq!(actor.session_id, s.id);
    }
}
