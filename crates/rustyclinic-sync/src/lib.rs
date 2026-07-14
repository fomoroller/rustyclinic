//! Sync protocol: op-log replication, conflict detection, and health metrics.
//!
//! This crate implements Phase 3 of the RustyClinic build. It provides:
//!
//! - **Op-log sync engine**: push pending operations upstream, pull new operations
//!   from upstream, track cursors per device.
//! - **Conflict detection**: domain-specific merge rules (field-aware for patient
//!   demographics, append-only for encounters, last-write-wins for queues).
//! - **Hash chain validation**: verify tamper-proof integrity of the op-log.
//! - **Gap detection**: halt sync if sequence gaps are found.
//! - **HTTP routes**: `/sync/push`, `/sync/pull`, `/sync/ack`, `/sync/cursor`,
//!   `/sync/health`.

pub mod conflict;
pub mod engine;
pub mod health;
pub mod routes;
pub mod types;

pub use engine::SyncEngine;
pub use health::SyncHealth;
pub use routes::{SyncState, sync_router};
pub use types::{ApplyResult, PushResult, SyncConflict, SyncCursor};
