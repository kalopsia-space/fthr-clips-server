use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::app::AppState;
use crate::db::media;
use crate::legacy::valid_old_id;
use crate::web::library::page_path;

/// `/files/{old_id}`: permanent redirect from the old service's links.
pub async fn redirect(State(state): State<AppState>, Path(old_id): Path<String>) -> Response {
    if !valid_old_id(&old_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let found = async {
        match media::legacy_lookup(&state.db, &old_id).await? {
            // An empty media_id marks an unimportable pair: no redirect.
            Some(id) if !id.is_empty() => media::get(&state.db, &id).await,
            _ => Ok(None),
        }
    }
    .await;
    match found {
        Ok(Some(m)) => (
            StatusCode::MOVED_PERMANENTLY,
            [
                (header::LOCATION, page_path(&m)),
                (header::CACHE_CONTROL, "public, max-age=86400".to_string()),
            ],
        )
            .into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!("legacy lookup failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
