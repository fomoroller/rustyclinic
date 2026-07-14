//! Static asset serving with `include_bytes!` for single-binary deployment.

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

static HTMX_JS: &[u8] = include_bytes!("../static/js/htmx.min.js");
static ALPINE_JS: &[u8] = include_bytes!("../static/js/alpine.min.js");
static APP_CSS: &[u8] = include_bytes!("../static/css/app.css");
static FONT_WOFF2: &[u8] = include_bytes!("../static/fonts/source-sans-3.woff2");
static SW_JS: &[u8] = include_bytes!("../static/sw.js");

pub async fn serve_static(axum::extract::Path(path): axum::extract::Path<String>) -> Response {
    let (body, content_type) = match path.as_str() {
        "js/htmx.min.js" => (HTMX_JS, "application/javascript"),
        "js/alpine.min.js" => (ALPINE_JS, "application/javascript"),
        "css/app.css" => (APP_CSS, "text/css"),
        "fonts/source-sans-3.woff2" => (FONT_WOFF2, "font/woff2"),
        "sw.js" => (SW_JS, "application/javascript"),
        _ => {
            return (StatusCode::NOT_FOUND, "not found").into_response();
        }
    };

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        body,
    )
        .into_response()
}
