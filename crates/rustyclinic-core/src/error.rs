//! Application error types.

use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("not found: {entity} with id {id}")]
    NotFound { entity: &'static str, id: Uuid },

    #[error("authorization denied: {reason}")]
    AuthorizationDenied { reason: String },

    #[error("validation error: {message}")]
    Validation { message: String },

    #[error("conflict: {message}")]
    Conflict { message: String },

    #[error("idempotency replay for key {key}")]
    IdempotencyReplay { key: String },

    #[error("state transition not allowed: {from} -> {to}")]
    InvalidTransition { from: String, to: String },

    #[error("database error: {0}")]
    Database(String),
}

pub type AppResult<T> = Result<T, AppError>;
