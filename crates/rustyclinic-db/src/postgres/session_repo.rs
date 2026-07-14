//! PostgreSQL implementation of SessionRepo.

use rustyclinic_auth::session::{Session, SessionRepo, SessionState};
use rustyclinic_core::error::{AppError, AppResult};
use tokio_postgres::Client;
use uuid::Uuid;

pub struct PgSessionRepo<'a> {
    client: &'a Client,
}

impl<'a> PgSessionRepo<'a> {
    pub fn new(client: &'a Client) -> Self {
        Self { client }
    }

    fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f))
    }
}

impl SessionRepo for PgSessionRepo<'_> {
    fn create(&self, session: &Session) -> AppResult<()> {
        let roles_json =
            serde_json::to_string(&session.roles).map_err(|e| AppError::Database(e.to_string()))?;

        self.block_on(async {
            self.client
                .execute(
                    "INSERT INTO sessions (id, user_id, facility_id, device_id, roles, auth_method, state, created_at, expires_at, last_active, locked_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
                    &[
                        &session.id,
                        &session.user_id,
                        &session.facility_id,
                        &session.device_id,
                        &roles_json,
                        &session.auth_method,
                        &state_to_str(&session.state),
                        &session.created_at,
                        &session.expires_at,
                        &session.last_active,
                        &session.locked_at,
                    ],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;
            Ok(())
        })
    }

    fn find_by_id(&self, id: Uuid) -> AppResult<Option<Session>> {
        self.block_on(async {
            let row = self.client
                .query_opt(
                    "SELECT id, user_id, facility_id, device_id, roles, auth_method, state, created_at, expires_at, last_active, locked_at
                     FROM sessions WHERE id = $1",
                    &[&id],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            match row {
                Some(row) => Ok(Some(row_to_session(&row)?)),
                None => Ok(None),
            }
        })
    }

    fn update(&self, session: &Session) -> AppResult<()> {
        self.block_on(async {
            self.client
                .execute(
                    "UPDATE sessions SET state=$1, last_active=$2, expires_at=$3, locked_at=$4 WHERE id=$5",
                    &[
                        &state_to_str(&session.state),
                        &session.last_active,
                        &session.expires_at,
                        &session.locked_at,
                        &session.id,
                    ],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;
            Ok(())
        })
    }

    fn find_active_by_device(&self, device_id: Uuid) -> AppResult<Vec<Session>> {
        self.block_on(async {
            let rows = self.client
                .query(
                    "SELECT id, user_id, facility_id, device_id, roles, auth_method, state, created_at, expires_at, last_active, locked_at
                     FROM sessions WHERE device_id = $1 AND state IN ('active', 'locked')",
                    &[&device_id],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            let mut sessions = Vec::new();
            for row in &rows {
                sessions.push(row_to_session(row)?);
            }
            Ok(sessions)
        })
    }

    fn count_locked_by_device(&self, device_id: Uuid) -> AppResult<u32> {
        self.block_on(async {
            let row = self
                .client
                .query_one(
                    "SELECT COUNT(*)::int FROM sessions WHERE device_id = $1 AND state = 'locked'",
                    &[&device_id],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            let count: i32 = row
                .try_get(0)
                .map_err(|e| AppError::Database(e.to_string()))?;
            Ok(count as u32)
        })
    }
}

fn state_to_str(state: &SessionState) -> &'static str {
    match state {
        SessionState::Active => "active",
        SessionState::Locked => "locked",
        SessionState::Expired => "expired",
        SessionState::Revoked => "revoked",
        SessionState::Terminated => "terminated",
    }
}

fn row_to_session(row: &tokio_postgres::Row) -> AppResult<Session> {
    let roles_str: String = row
        .try_get(4)
        .map_err(|e| AppError::Database(e.to_string()))?;
    let state_str: String = row
        .try_get(6)
        .map_err(|e| AppError::Database(e.to_string()))?;

    Ok(Session {
        id: row
            .try_get(0)
            .map_err(|e| AppError::Database(e.to_string()))?,
        user_id: row
            .try_get(1)
            .map_err(|e| AppError::Database(e.to_string()))?,
        facility_id: row
            .try_get(2)
            .map_err(|e| AppError::Database(e.to_string()))?,
        device_id: row
            .try_get(3)
            .map_err(|e| AppError::Database(e.to_string()))?,
        roles: serde_json::from_str(&roles_str).unwrap_or_default(),
        auth_method: row
            .try_get(5)
            .map_err(|e| AppError::Database(e.to_string()))?,
        state: match state_str.as_str() {
            "active" => SessionState::Active,
            "locked" => SessionState::Locked,
            "expired" => SessionState::Expired,
            "revoked" => SessionState::Revoked,
            _ => SessionState::Terminated,
        },
        created_at: row
            .try_get(7)
            .map_err(|e| AppError::Database(e.to_string()))?,
        expires_at: row
            .try_get(8)
            .map_err(|e| AppError::Database(e.to_string()))?,
        last_active: row
            .try_get(9)
            .map_err(|e| AppError::Database(e.to_string()))?,
        locked_at: row
            .try_get(10)
            .map_err(|e| AppError::Database(e.to_string()))?,
    })
}
