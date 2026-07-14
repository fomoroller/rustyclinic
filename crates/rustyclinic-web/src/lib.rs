//! Web/PWA frontend: Askama templates + htmx + Alpine.js.
//!
//! All assets are embedded in the binary via `include_bytes!`.
//! No npm, no Node.js, no separate frontend build.

pub mod assets;
pub mod form_renderer;
pub mod forms;
pub mod middleware;
pub mod routes;
pub mod templates;

use uuid::Uuid;

/// Shared state for all web handlers.
#[derive(Clone)]
pub struct WebAppState {
    pub db_path: String,
    pub device_id: Uuid,
    pub facility_id: Uuid,
}

/// Build the web router with all routes.
pub fn web_router(state: WebAppState) -> axum::Router {
    use axum::routing::{get, post};

    axum::Router::new()
        // Static assets (no auth)
        .route("/static/{*path}", get(assets::serve_static))
        // Auth routes (no auth)
        .route(
            "/web/login",
            get(routes::auth::login_page).post(routes::auth::login_submit),
        )
        .route(
            "/web/pin/setup",
            get(routes::auth::pin_setup_page).post(routes::auth::pin_setup_submit),
        )
        // Lock routes (no auth — reads cookie directly)
        .route(
            "/web/lock",
            get(routes::lock::lock_screen).post(routes::lock::lock_session),
        )
        .route("/web/unlock", post(routes::lock::unlock_session))
        .route("/web/sessions", get(routes::lock::sessions_fragment))
        // Logout (auth required)
        .route("/web/logout", post(routes::auth::logout))
        // Queue (auth required)
        .route("/web/queue", get(routes::queue::board_page))
        .route(
            "/web/queue/board-content",
            get(routes::queue::board_content),
        )
        .route(
            "/web/queue/{id}/transition",
            post(routes::queue::transition_entry),
        )
        .route(
            "/web/queue/{id}/print",
            get(routes::queue::queue_ticket_print_page),
        )
        .route("/web/queue/enqueue", post(routes::queue::enqueue_patient))
        .route(
            "/web/queue/simple-board-content",
            get(routes::queue::simple_board_content),
        )
        // Patients (auth required)
        .route("/web/patients", get(routes::patients::search_page))
        .route("/web/patients/{id}", get(routes::patients::detail_page))
        .route(
            "/web/patients/{id}/print",
            get(routes::patients::print_page),
        )
        .route(
            "/web/patients/search-results",
            get(routes::patients::search_results),
        )
        .route(
            "/web/patients/register",
            get(routes::patients::register_page).post(routes::patients::register_submit),
        )
        // Encounters (auth required)
        .route(
            "/web/encounters/triage",
            get(routes::encounters::triage_page).post(routes::encounters::save_triage),
        )
        .route(
            "/web/encounters/new",
            get(routes::encounters::new_encounter),
        )
        .route("/web/encounters", post(routes::encounters::save_encounter))
        .route(
            "/web/encounters/{id}/draft",
            post(routes::encounters::save_draft),
        )
        .route(
            "/web/encounters/{id}/validate",
            post(routes::encounters::validate_fields),
        )
        .route(
            "/web/encounters/{id}/order-lab",
            get(routes::encounters::order_lab_page).post(routes::encounters::submit_lab_order),
        )
        .route(
            "/web/encounters/{id}/prescribe",
            get(routes::encounters::prescribe_page).post(routes::encounters::submit_prescription),
        )
        // Lab (auth required)
        .route("/web/lab", get(routes::lab::queue_page))
        .route("/web/lab/{id}/results", get(routes::lab::results_page))
        .route("/web/lab/submit-results", post(routes::lab::submit_results))
        // Pharmacy (auth required)
        .route("/web/pharmacy", get(routes::pharmacy::queue_page))
        .route(
            "/web/pharmacy/{id}/dispense",
            get(routes::pharmacy::dispense_page),
        )
        .route(
            "/web/pharmacy/{id}/print",
            get(routes::pharmacy::dispense_slip_print_page),
        )
        .route(
            "/web/pharmacy/submit-dispense",
            post(routes::pharmacy::submit_dispense),
        )
        .route("/web/sync", get(routes::sync::status_page))
        // Root redirect
        .route(
            "/",
            get(|| async { axum::response::Redirect::to("/web/queue") }),
        )
        .with_state(state)
}
