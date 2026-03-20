//! REST API interface.
//!
//! Validates transport input, shapes responses. Does NOT implement business rules.
//! All mutations go through rustyclinic-services.

pub mod routes;

pub async fn health_check() -> &'static str {
    "ok"
}
