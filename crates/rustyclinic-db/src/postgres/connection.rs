//! PostgreSQL connection management via deadpool.

use deadpool_postgres::{Config, Pool, Runtime};
use tokio_postgres::NoTls;

use crate::migration_pg;

/// Create a connection pool and run migrations.
pub async fn create_pool_and_migrate(
    host: &str,
    port: u16,
    dbname: &str,
    user: &str,
    password: &str,
) -> Result<Pool, String> {
    let mut cfg = Config::new();
    cfg.host = Some(host.to_string());
    cfg.port = Some(port);
    cfg.dbname = Some(dbname.to_string());
    cfg.user = Some(user.to_string());
    cfg.password = Some(password.to_string());

    let pool = cfg
        .create_pool(Some(Runtime::Tokio1), NoTls)
        .map_err(|e| format!("failed to create pool: {e}"))?;

    // Run migrations on a checkout
    let client = pool
        .get()
        .await
        .map_err(|e| format!("failed to get connection: {e}"))?;

    migration_pg::run_migrations(&client)
        .await
        .map_err(|e| format!("migration failed: {e}"))?;

    Ok(pool)
}
