//! PostgreSQL implementation of UserRepo.

use rustyclinic_auth::users::{User, UserRepo};
use rustyclinic_core::error::{AppError, AppResult};
use tokio_postgres::Client;
use uuid::Uuid;

pub struct PgUserRepo<'a> {
    client: &'a Client,
}

impl<'a> PgUserRepo<'a> {
    pub fn new(client: &'a Client) -> Self {
        Self { client }
    }

    fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f))
    }
}

impl UserRepo for PgUserRepo<'_> {
    fn create(&self, user: &User, password_hash: &str) -> AppResult<()> {
        let roles_json =
            serde_json::to_string(&user.roles).map_err(|e| AppError::Database(e.to_string()))?;

        self.block_on(async {
            self.client
                .execute(
                    "INSERT INTO users (id, facility_id, username, display_name, password_hash, roles, active, created_at, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                    &[
                        &user.id,
                        &user.facility_id,
                        &user.username,
                        &user.display_name,
                        &password_hash,
                        &roles_json,
                        &user.active,
                        &user.created_at,
                        &user.updated_at,
                    ],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;
            Ok(())
        })
    }

    fn find_by_id(&self, id: Uuid) -> AppResult<Option<User>> {
        self.block_on(async {
            let row = self.client
                .query_opt(
                    "SELECT id, facility_id, username, display_name, roles, active, created_at, updated_at
                     FROM users WHERE id = $1",
                    &[&id],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            match row {
                Some(row) => Ok(Some(row_to_user(&row)?)),
                None => Ok(None),
            }
        })
    }

    fn find_by_username(
        &self,
        facility_id: Uuid,
        username: &str,
    ) -> AppResult<Option<(User, String, Option<String>)>> {
        self.block_on(async {
            let row = self.client
                .query_opt(
                    "SELECT id, facility_id, username, display_name, password_hash, pin_hash, roles, active, created_at, updated_at
                     FROM users WHERE facility_id = $1 AND username = $2",
                    &[&facility_id, &username],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            match row {
                Some(row) => {
                    let pw_hash: String = row.try_get(4).map_err(|e| AppError::Database(e.to_string()))?;
                    let pin_hash: Option<String> = row.try_get(5).map_err(|e| AppError::Database(e.to_string()))?;
                    let user = row_to_user_with_offset(&row)?;
                    Ok(Some((user, pw_hash, pin_hash)))
                }
                None => Ok(None),
            }
        })
    }

    fn update_pin_hash(&self, user_id: Uuid, pin_hash: &str) -> AppResult<()> {
        self.block_on(async {
            let updated = self
                .client
                .execute(
                    "UPDATE users SET pin_hash = $1, updated_at = $2 WHERE id = $3",
                    &[&pin_hash, &chrono::Utc::now(), &user_id],
                )
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;

            if updated == 0 {
                return Err(AppError::NotFound {
                    entity: "User",
                    id: user_id,
                });
            }

            Ok(())
        })
    }
}

fn row_to_user(row: &tokio_postgres::Row) -> AppResult<User> {
    let roles_str: String = row
        .try_get(4)
        .map_err(|e| AppError::Database(e.to_string()))?;
    let roles: Vec<String> = serde_json::from_str(&roles_str).unwrap_or_default();
    Ok(User {
        id: row
            .try_get(0)
            .map_err(|e| AppError::Database(e.to_string()))?,
        facility_id: row
            .try_get(1)
            .map_err(|e| AppError::Database(e.to_string()))?,
        username: row
            .try_get(2)
            .map_err(|e| AppError::Database(e.to_string()))?,
        display_name: row
            .try_get(3)
            .map_err(|e| AppError::Database(e.to_string()))?,
        roles,
        active: row
            .try_get(5)
            .map_err(|e| AppError::Database(e.to_string()))?,
        created_at: row
            .try_get(6)
            .map_err(|e| AppError::Database(e.to_string()))?,
        updated_at: row
            .try_get(7)
            .map_err(|e| AppError::Database(e.to_string()))?,
    })
}

fn row_to_user_with_offset(row: &tokio_postgres::Row) -> AppResult<User> {
    let roles_str: String = row
        .try_get(6)
        .map_err(|e| AppError::Database(e.to_string()))?;
    let roles: Vec<String> = serde_json::from_str(&roles_str).unwrap_or_default();
    Ok(User {
        id: row
            .try_get(0)
            .map_err(|e| AppError::Database(e.to_string()))?,
        facility_id: row
            .try_get(1)
            .map_err(|e| AppError::Database(e.to_string()))?,
        username: row
            .try_get(2)
            .map_err(|e| AppError::Database(e.to_string()))?,
        display_name: row
            .try_get(3)
            .map_err(|e| AppError::Database(e.to_string()))?,
        roles,
        active: row
            .try_get(7)
            .map_err(|e| AppError::Database(e.to_string()))?,
        created_at: row
            .try_get(8)
            .map_err(|e| AppError::Database(e.to_string()))?,
        updated_at: row
            .try_get(9)
            .map_err(|e| AppError::Database(e.to_string()))?,
    })
}
