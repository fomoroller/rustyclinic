//! Authentication, sessions, and credential management.
//!
//! Supports password and PIN auth with Argon2id hashing.
//! Offline cached credentials with configurable lifetime.
//! Shared-device session isolation.

pub mod credentials;
pub mod session;
pub mod users;
