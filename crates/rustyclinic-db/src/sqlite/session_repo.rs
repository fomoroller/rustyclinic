//! SQLite implementation of SessionRepo.

use rusqlite::Connection;
use rustyclinic_auth::session::{Session, SessionRepo, SessionState};
use rustyclinic_core::error::{AppError, AppResult};
use uuid::Uuid;

pub struct SqliteSessionRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteSessionRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

impl SessionRepo for SqliteSessionRepo<'_> {
    fn create(&self, session: &Session) -> AppResult<()> {
        let roles_json =
            serde_json::to_string(&session.roles).map_err(|e| AppError::Database(e.to_string()))?;

        self.conn
            .execute(
                "INSERT INTO sessions (id, user_id, facility_id, device_id, roles, auth_method, state, created_at, expires_at, last_active, locked_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                rusqlite::params![
                    session.id.to_string(),
                    session.user_id.to_string(),
                    session.facility_id.to_string(),
                    session.device_id.to_string(),
                    roles_json,
                    session.auth_method,
                    state_to_str(&session.state),
                    session.created_at.to_rfc3339(),
                    session.expires_at.to_rfc3339(),
                    session.last_active.to_rfc3339(),
                    session.locked_at.map(|t| t.to_rfc3339()),
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn find_by_id(&self, id: Uuid) -> AppResult<Option<Session>> {
        let result = self.conn.query_row(
            "SELECT id, user_id, facility_id, device_id, roles, auth_method, state, created_at, expires_at, last_active, locked_at
             FROM sessions WHERE id = ?1",
            rusqlite::params![id.to_string()],
            |row| Ok(row_to_session(row)),
        );
        match result {
            Ok(s) => Ok(Some(s.map_err(|e| AppError::Database(e.to_string()))?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(AppError::Database(e.to_string())),
        }
    }

    fn update(&self, session: &Session) -> AppResult<()> {
        self.conn
            .execute(
                "UPDATE sessions SET state=?1, last_active=?2, expires_at=?3, locked_at=?4 WHERE id=?5",
                rusqlite::params![
                    state_to_str(&session.state),
                    session.last_active.to_rfc3339(),
                    session.expires_at.to_rfc3339(),
                    session.locked_at.map(|t| t.to_rfc3339()),
                    session.id.to_string(),
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn find_active_by_device(&self, device_id: Uuid) -> AppResult<Vec<Session>> {
        let mut stmt = self.conn
            .prepare(
                "SELECT id, user_id, facility_id, device_id, roles, auth_method, state, created_at, expires_at, last_active, locked_at
                 FROM sessions WHERE device_id = ?1 AND state IN ('active', 'locked')",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params![device_id.to_string()], |row| {
                Ok(row_to_session(row))
            })
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut sessions = Vec::new();
        for row in rows {
            let s = row
                .map_err(|e| AppError::Database(e.to_string()))?
                .map_err(|e| AppError::Database(e.to_string()))?;
            sessions.push(s);
        }
        Ok(sessions)
    }

    fn count_locked_by_device(&self, device_id: Uuid) -> AppResult<u32> {
        let count: u32 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE device_id = ?1 AND state = 'locked'",
                rusqlite::params![device_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(count)
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

fn row_to_session(row: &rusqlite::Row) -> Result<Session, rusqlite::Error> {
    let id_str: String = row.get(0)?;
    let user_str: String = row.get(1)?;
    let facility_str: String = row.get(2)?;
    let device_str: String = row.get(3)?;
    let roles_str: String = row.get(4)?;
    let state_str: String = row.get(6)?;
    let created_str: String = row.get(7)?;
    let expires_str: String = row.get(8)?;
    let active_str: String = row.get(9)?;
    let locked_str: Option<String> = row.get(10)?;

    let parse_dt = |s: &str| {
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now())
    };

    Ok(Session {
        id: Uuid::parse_str(&id_str).unwrap_or_default(),
        user_id: Uuid::parse_str(&user_str).unwrap_or_default(),
        facility_id: Uuid::parse_str(&facility_str).unwrap_or_default(),
        device_id: Uuid::parse_str(&device_str).unwrap_or_default(),
        roles: serde_json::from_str(&roles_str).unwrap_or_default(),
        auth_method: row.get(5)?,
        state: match state_str.as_str() {
            "active" => SessionState::Active,
            "locked" => SessionState::Locked,
            "expired" => SessionState::Expired,
            "revoked" => SessionState::Revoked,
            _ => SessionState::Terminated,
        },
        created_at: parse_dt(&created_str),
        expires_at: parse_dt(&expires_str),
        last_active: parse_dt(&active_str),
        locked_at: locked_str.as_deref().map(parse_dt),
    })
}
