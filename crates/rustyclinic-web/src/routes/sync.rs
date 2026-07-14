use axum::extract::State;
use axum::response::{Html, IntoResponse, Response};
use rustyclinic_db::sqlite::sync_repo::SqliteSyncRepo;
use rustyclinic_db::sync_repo::{OpLogSyncRepo, SyncConflictRepo, SyncCursorRepo};

use crate::WebAppState;
use crate::middleware::session::WebSession;
use crate::routes::patients::lookup_user_name;
use crate::templates::{SyncConflictView, SyncStatusPage};

pub async fn status_page(State(state): State<WebAppState>, session: WebSession) -> Response {
    let conn = match rusqlite::Connection::open(&state.db_path) {
        Ok(c) => c,
        Err(_) => return axum::response::Redirect::to("/web/queue").into_response(),
    };

    let repo = SqliteSyncRepo::new(&conn);
    let pending_ops_count = repo.count_pending(session.session.facility_id).unwrap_or(0);
    let conflicts =
        SyncConflictRepo::list_pending(&repo, session.session.facility_id).unwrap_or_default();
    let conflict_queue_depth = conflicts.len() as u32;
    let cursor = repo
        .get(session.session.device_id, session.session.facility_id)
        .unwrap_or(None);
    let last_push_at = cursor
        .as_ref()
        .map(|cursor| cursor.updated_at.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "Never".to_string());
    let last_pull_at = cursor
        .as_ref()
        .map(|cursor| cursor.updated_at.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "Never".to_string());
    let cursor_lag = pending_ops_count;

    let conflicts = conflicts
        .into_iter()
        .map(|conflict| SyncConflictView {
            aggregate_type: conflict.aggregate_type,
            aggregate_id: conflict.aggregate_id.to_string(),
            conflict_type: conflict.conflict_type.to_string(),
            created_at: conflict.created_at.format("%Y-%m-%d %H:%M").to_string(),
        })
        .collect();

    let (display_name, initials) = lookup_user_name(&state, session.session.user_id);
    let page = SyncStatusPage {
        active_nav: "sync".to_string(),
        display_name,
        initials,
        flash_success: None,
        flash_error: None,
        pending_ops_count,
        conflict_queue_depth,
        cursor_lag,
        last_push_at,
        last_pull_at,
        conflicts,
    };

    Html(page.to_string()).into_response()
}
