//! Admin area: session extractors, CSRF helper and routes.

pub mod api_media;
pub mod api_settings;
pub mod api_tags;
pub mod api_tokens;
pub mod auth;
pub mod pages;

use axum::extract::FromRequestParts;
use axum::http::header::{COOKIE, LOCATION};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde_json::json;

use crate::app::AppState;
use crate::db::sessions::{self, Session};
use crate::web::security::csrf_ok;

pub const COOKIE_NAME: &str = "fthr_admin";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin", get(pages::root))
        .route("/admin/files", get(pages::files))
        .route("/admin/tags", get(pages::tags))
        .route("/admin/tokens", get(pages::tokens))
        .route(
            "/admin/library",
            get(pages::library_get).post(pages::library_post),
        )
        .route(
            "/admin/transcoding",
            get(pages::transcoding_get).post(pages::transcoding_post),
        )
        .route(
            "/admin/sharing",
            get(pages::sharing_get).post(pages::sharing_post),
        )
        .route(
            "/admin/integrations",
            get(pages::integrations_get).post(pages::integrations_post),
        )
        .route(
            "/admin/api/settings",
            get(api_settings::get).put(api_settings::put),
        )
        .route(
            "/admin/login",
            get(auth::login_page).post(auth::login_submit),
        )
        .route("/admin/logout", post(auth::logout))
        .route("/admin/api/session", get(auth::api_session))
        .route("/admin/api/media", get(api_media::list))
        .route("/admin/api/media/bulk", post(api_media::bulk))
        .route(
            "/admin/api/media/{id}",
            get(api_media::get_one)
                .patch(api_media::patch)
                .delete(api_media::delete),
        )
        .route(
            "/admin/api/tags",
            get(api_tags::list).post(api_tags::create),
        )
        .route(
            "/admin/api/tokens",
            get(api_tokens::list).post(api_tokens::create),
        )
        .route(
            "/admin/api/tokens/{id}",
            axum::routing::delete(api_tokens::revoke),
        )
        .route(
            "/admin/api/tags/{id}",
            patch(api_tags::patch).delete(api_tags::delete),
        )
}

/// Value of the `fthr_admin` cookie, if present.
fn session_cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == COOKIE_NAME)
        .map(|(_, v)| v)
}

async fn lookup(state: &AppState, headers: &HeaderMap) -> Option<Session> {
    let id = session_cookie(headers)?;
    // Session ids are 64 hex chars; reject anything else without a query.
    if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    sessions::get_valid(&state.db, id).await.ok().flatten()
}

/// A valid admin session, or a rejection (401 JSON for API paths, else a
/// redirect to the login page).
pub struct AdminSession(pub Session);

/// Never rejects; `None` when not signed in.
pub struct OptionalAdmin(pub Option<Session>);

/// Only same-site absolute paths are accepted as a post-login target.
pub fn safe_next(next: Option<&str>) -> &str {
    match next {
        Some(n)
            if n.starts_with('/')
                && !n.starts_with("//")
                && !n.starts_with("/\\")
                && n.bytes().all(|b| b > 0x20 && b != 0x7f && b != b'\\') =>
        {
            n
        }
        _ => "/admin",
    }
}

impl FromRequestParts<AppState> for AdminSession {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if let Some(s) = lookup(state, &parts.headers).await {
            return Ok(AdminSession(s));
        }
        let path = parts.uri.path();
        if path.starts_with("/admin/api/") {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "unauthorized"})),
            )
                .into_response());
        }
        let original = match parts.uri.query() {
            Some(q) => format!("{path}?{q}"),
            None => path.to_string(),
        };
        let next = safe_next(Some(&original));
        let location = format!("/admin/login?next={}", urlencoding::encode(next));
        Err((StatusCode::SEE_OTHER, [(LOCATION, location)]).into_response())
    }
}

impl FromRequestParts<AppState> for OptionalAdmin {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(OptionalAdmin(lookup(state, &parts.headers).await))
    }
}

/// CSRF check for state-changing admin routes; 403 on failure.
pub fn require_csrf(
    session: &Session,
    headers: &HeaderMap,
    form_token: Option<&str>,
) -> Result<(), StatusCode> {
    if csrf_ok(session, headers, form_token) {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}
