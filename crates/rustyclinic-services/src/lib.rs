//! Application service layer.
//!
//! All writes flow through this crate. Each domain command is a separate module
//! (one-command-per-module pattern to prevent merge conflicts).
//!
//! ```text
//! CANONICAL WRITE FLOW:
//!
//!   1. Resolve actor, device, scope
//!   2. Resolve active package set (cached, invalidated on activation)
//!   3. Validate permissions + co-sign requirements
//!   4. Check idempotency record
//!   5. Open database transaction
//!   6. Persist domain rows
//!   7. Persist audit entry (hash-chained)
//!   8. Persist outbox event
//!   9. Persist op-log entry
//!  10. Commit
//!  11. Background: publish events, update projections
//! ```

pub mod commands;

#[cfg(test)]
mod tests;
