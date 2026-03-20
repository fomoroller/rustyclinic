//! SQLite implementation of UserRepo.

use rusqlite::Connection;
use uuid::Uuid;
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_auth::users::{User, UserRepo};

pub struct SqliteUserRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteUserRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

impl UserRepo for SqliteUserRepo<'_> {
    fn create(&self, user: &User, password_hash: &str) -> AppResult<()> {
        let roles_json = serde_json::to_string(&user.roles)
            .map_err(|e| AppError::Database(e.to_string()))?;

        self.conn
            .execute(
                "INSERT INTO users (id, facility_id, username, display_name, password_hash, roles, active, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    user.id.to_string(),
                    user.facility_id.to_string(),
                    user.username,
                    user.display_name,
                    password_hash,
                    roles_json,
                    user.active as i32,
                    user.created_at.to_rfc3339(),
                    user.updated_at.to_rfc3339(),
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn find_by_id(&self, id: Uuid) -> AppResult<Option<User>> {
        let result = self.conn.query_row(
            "SELECT id, facility_id, username, display_name, roles, active, created_at, updated_at
             FROM users WHERE id = ?1",
            rusqlite::params![id.to_string()],
            |row| Ok(row_to_user(row)),
        );

        match result {
            Ok(u) => Ok(Some(u.map_err(|e| AppError::Database(e.to_string()))?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(AppError::Database(e.to_string())),
        }
    }

    fn find_by_username(&self, facility_id: Uuid, username: &str) -> AppResult<Option<(User, String)>> {
        let result = self.conn.query_row(
            "SELECT id, facility_id, username, display_name, password_hash, roles, active, created_at, updated_at
             FROM users WHERE facility_id = ?1 AND username = ?2",
            rusqlite::params![facility_id.to_string(), username],
            |row| {
                let pw_hash: String = row.get(4)?;
                let user = row_to_user_with_offset(row);
                Ok((user, pw_hash))
            },
        );

        match result {
            Ok((u, hash)) => Ok(Some((u.map_err(|e| AppError::Database(e.to_string()))?, hash))),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(AppError::Database(e.to_string())),
        }
    }
}

fn row_to_user(row: &rusqlite::Row) -> Result<User, rusqlite::Error> {
    let id_str: String = row.get(0)?;
    let facility_str: String = row.get(1)?;
    let roles_str: String = row.get(4)?;
    let created_str: String = row.get(6)?;
    let updated_str: String = row.get(7)?;
    let roles: Vec<String> = serde_json::from_str(&roles_str).unwrap_or_default();

    Ok(User {
        id: Uuid::parse_str(&id_str).unwrap_or_default(),
        facility_id: Uuid::parse_str(&facility_str).unwrap_or_default(),
        username: row.get(2)?,
        display_name: row.get(3)?,
        roles,
        active: row.get::<_, i32>(5)? != 0,
        created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now()),
        updated_at: chrono::DateTime::parse_from_rfc3339(&updated_str)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now()),
    })
}

// Same but columns are offset by 1 because password_hash is at index 4
fn row_to_user_with_offset(row: &rusqlite::Row) -> Result<User, rusqlite::Error> {
    let id_str: String = row.get(0)?;
    let facility_str: String = row.get(1)?;
    let roles_str: String = row.get(5)?;
    let created_str: String = row.get(7)?;
    let updated_str: String = row.get(8)?;
    let roles: Vec<String> = serde_json::from_str(&roles_str).unwrap_or_default();

    Ok(User {
        id: Uuid::parse_str(&id_str).unwrap_or_default(),
        facility_id: Uuid::parse_str(&facility_str).unwrap_or_default(),
        username: row.get(2)?,
        display_name: row.get(3)?,
        roles,
        active: row.get::<_, i32>(6)? != 0,
        created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now()),
        updated_at: chrono::DateTime::parse_from_rfc3339(&updated_str)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now()),
    })
}
