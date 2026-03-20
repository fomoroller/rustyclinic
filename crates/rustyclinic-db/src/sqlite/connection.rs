//! SQLite connection management.

use anyhow::Result;
use rusqlite::Connection;
use crate::migration;

/// Open a SQLite connection with WAL mode and run migrations.
pub fn open_and_migrate(path: &str) -> Result<Connection> {
    let conn = Connection::open(path)?;

    // WAL mode for concurrent reads during writes
    conn.pragma_update(None, "journal_mode", "wal")?;
    // NORMAL sync for balance of durability and performance
    conn.pragma_update(None, "synchronous", "normal")?;
    // Enable foreign keys
    conn.pragma_update(None, "foreign_keys", "on")?;

    migration::run_migrations(&conn)?;

    Ok(conn)
}
