//! Database abstraction: SQLite and PostgreSQL implementations.
//!
//! Uses repository traits per aggregate with separate implementations
//! per backend. Both must produce identical results.

pub mod migration;
pub mod sqlite;

#[cfg(test)]
mod tests;
