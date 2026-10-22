//! Upload token API: `/admin/api/tokens`. Plaintext tokens are returned once
//! at creation; only a SHA-256 hash is stored and hashes are never served.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::api_media::{parse_body, ApiError};
use super::{require_csrf, AdminSession};
use crate::app::AppState;
use crate::db::tokens;

#[derive(Serialize)]
pub struct TokenView {
    id: i64,
    name: String,
    prefix: String,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
}

impl From<tokens::UploadToken> for TokenView {
    fn from(t: tokens::UploadToken) -> Self {
        Self {
            id: t.id,
            name: t.name,
            prefix: t.prefix,
            created_at: t.created_at,
            last_used_at: t.last_used_at,
            revoked_at: t.revoked_at,
        }
    }
}

pub async fn list(
    AdminSession(_): AdminSession,
    State(state): State<AppState>,
) -> Result<Json<Vec<TokenView>>, ApiError> {
    let rows = tokens::list(&state.db).await?;
    Ok(Json(rows.into_iter().map(TokenView::from).collect()))
}

#[derive(Deserialize)]
struct CreateBody {
    name: String,
}

pub async fn create(
    AdminSession(session): AdminSession,
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    require_csrf(&session, &headers, None)?;
    let b: CreateBody = parse_body(&body)?;
    let name = b.name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(ApiError::bad("token name must be 1 to 60 characters"));
    }
    let token = format!(
        "fthr_{}",
        URL_SAFE_NO_PAD.encode(rand::random::<[u8; 24]>())
    );
    let prefix = &token[..12];
    let hash = hex::encode(Sha256::digest(token.as_bytes()));
    let row = tokens::create(&state.db, name, &hash, prefix).await?;
    Ok(Json(json!({
        "id": row.id,
        "name": row.name,
        "prefix": row.prefix,
        "token": token,
    })))
}

pub async fn revoke(
    AdminSession(session): AdminSession,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    require_csrf(&session, &headers, None)?;
    let all = tokens::list(&state.db).await?;
    if !all.iter().any(|t| t.id == id) {
        return Err(ApiError::not_found());
    }
    tokens::revoke(&state.db, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
