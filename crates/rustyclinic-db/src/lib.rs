//! Database abstraction: SQLite and PostgreSQL implementations.
//!
//! Uses repository traits per aggregate with separate implementations
//! per backend. Both must produce identical results.

pub mod migration;
#[cfg(feature = "postgres")]
pub mod migration_pg;
#[cfg(feature = "postgres")]
pub mod postgres;
pub mod sqlite;
pub mod sync_repo;

#[cfg(test)]
mod tests;
