//! Login, logout and session endpoints.

use std::net::SocketAddr;
use std::time::Duration;

use argon2::password_hash::{PasswordHash, PasswordVerifier};
use argon2::Argon2;
use askama::Template;
use axum::extract::{ConnectInfo, FromRequest, Query, Request, State};
use axum::http::header::{CONTENT_TYPE, LOCATION, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Form, Json};
use serde::Deserialize;
use serde_json::json;
use subtle::ConstantTimeEq;

use super::{require_csrf, safe_next, AdminSession, COOKIE_NAME};
use crate::app::AppState;
use crate::db::sessions;
use crate::web::assets::asset_v;
use crate::web::library::{render, PageError};
use crate::web::security::{client_ip, request_is_https};

const SESSION_TTL: Duration = Duration::from_secs(8 * 3600);

#[derive(Template)]
#[template(path = "login.html")]
struct LoginPage {
    asset_v: String,
    next: Option<String>,
    error: bool,
}

#[derive(Deserialize)]
pub struct LoginQuery {
    next: Option<String>,
    error: Option<String>,
}

pub async fn login_page(Query(q): Query<LoginQuery>) -> Result<impl IntoResponse, PageError> {
    let next = q
        .next
        .as_deref()
        .map(|n| safe_next(Some(n)).to_string())
        .filter(|n| n != "/admin");
    render(&LoginPage {
        asset_v: asset_v().to_string(),
        next,
        error: q.error.is_some(),
    })
}

#[derive(Deserialize)]
struct LoginBody {
    #[serde(default)]
    password: String,
    next: Option<String>,
}

fn password_ok(state: &AppState, supplied: &str) -> bool {
    let cfg = &state.config;
    if let Some(hash) = cfg.admin_password_hash.as_deref() {
        return match PasswordHash::new(hash) {
            Ok(h) => Argon2::default()
                .verify_password(supplied.as_bytes(), &h)
                .is_ok(),
            Err(_) => false,
        };
    }
    if let Some(expected) = cfg.admin_password.as_deref() {
        return bool::from(supplied.as_bytes().ct_eq(expected.as_bytes()));
    }
    false
}

fn cookie_value(id: &str, max_age: u64, secure: bool) -> HeaderValue {
    let secure = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{COOKIE_NAME}={id}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}"
    ))
    .expect("cookie is ascii")
}

fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.trim_start()
                .to_ascii_lowercase()
                .starts_with("application/json")
        })
}

fn see_other(location: &str) -> Response {
    match HeaderValue::from_str(location) {
        Ok(v) => (StatusCode::SEE_OTHER, [(LOCATION, v)]).into_response(),
        Err(_) => StatusCode::SEE_OTHER.into_response(),
    }
}

pub async fn login_submit(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
) -> Response {
    let headers = request.headers().clone();
    let json_mode = is_json(&headers);
    let trusted = &state.config.trusted_proxies;
    let ip = client_ip(&headers, peer.ip(), trusted);
    // Evaluate both so each limiter records the attempt.
    let per_ip = state.login_limiter.check(&ip.to_string());
    let global = state.login_limiter.check("global");
    if !(per_ip && global) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }

    let body = if json_mode {
        Json::<LoginBody>::from_request(request, &state)
            .await
            .map(|j| j.0)
            .ok()
    } else {
        Form::<LoginBody>::from_request(request, &state)
            .await
            .map(|f| f.0)
            .ok()
    };
    let Some(body) = body else {
        return StatusCode::BAD_REQUEST.into_response();
    };

    // Argon2 is CPU heavy; keep it off the async workers.
    let verify_state = state.clone();
    let password = body.password;
    let ok = tokio::task::spawn_blocking(move || password_ok(&verify_state, &password))
        .await
        .unwrap_or(false);

    if !ok {
        return if json_mode {
            (StatusCode::UNAUTHORIZED, Json(json!({"error": "invalid"}))).into_response()
        } else {
            see_other("/admin/login?error=1")
        };
    }

    let session = match sessions::create(&state.db, SESSION_TTL).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("creating admin session: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    if let Err(e) = sessions::purge_expired(&state.db).await {
        tracing::warn!("purging expired sessions: {e}");
    }
    let secure = request_is_https(&headers, peer.ip(), trusted);
    let cookie = cookie_value(&session.id, SESSION_TTL.as_secs(), secure);
    let mut resp = if json_mode {
        Json(json!({"csrf": session.csrf})).into_response()
    } else {
        see_other(safe_next(body.next.as_deref()))
    };
    resp.headers_mut().append(SET_COOKIE, cookie);
    resp
}

#[derive(Deserialize, Default)]
struct LogoutForm {
    csrf: Option<String>,
}

pub async fn logout(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    AdminSession(session): AdminSession,
    request: Request,
) -> Response {
    let headers = request.headers().clone();
    let json_mode = is_json(&headers);
    let form: LogoutForm = if headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/x-www-form-urlencoded"))
    {
        Form::<LogoutForm>::from_request(request, &state)
            .await
            .map(|f| f.0)
            .unwrap_or_default()
    } else {
        LogoutForm::default()
    };
    if let Err(code) = require_csrf(&session, &headers, form.csrf.as_deref()) {
        return code.into_response();
    }
    if let Err(e) = sessions::delete(&state.db, &session.id).await {
        tracing::error!("deleting admin session: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let secure = request_is_https(&headers, peer.ip(), &state.config.trusted_proxies);
    let mut resp = if json_mode || headers.contains_key("x-csrf-token") {
        StatusCode::NO_CONTENT.into_response()
    } else {
        see_other("/")
    };
    resp.headers_mut()
        .append(SET_COOKIE, cookie_value("", 0, secure));
    resp
}

pub async fn api_session(AdminSession(session): AdminSession) -> Json<serde_json::Value> {
    Json(json!({"csrf": session.csrf}))
}
