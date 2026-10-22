pub mod security;
pub mod upload;

use axum::Json;
use serde_json::{json, Value};

pub async fn healthz() -> Json<Value> {
    Json(json!({"status": "ok"}))
}

pub fn upload_routes(max_upload_bytes: u64) -> axum::Router<crate::app::AppState> {
    use axum::extract::DefaultBodyLimit;
    use axum::routing::head;
    let limit = usize::try_from(max_upload_bytes)
        .unwrap_or(usize::MAX - (1 << 20))
        .saturating_add(1 << 20);
    axum::Router::new()
        .route(
            "/upload",
            head(upload::head_upload).post(upload::post_upload),
        )
        .layer(DefaultBodyLimit::max(limit))
}
