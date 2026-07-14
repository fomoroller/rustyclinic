use std::sync::Arc;

use uuid::Uuid;

pub type AppState = Arc<AppStateInner>;

pub struct AppStateInner {
    pub db_path: String,
    pub facility_id: Uuid,
    pub device_id: Uuid,
}
