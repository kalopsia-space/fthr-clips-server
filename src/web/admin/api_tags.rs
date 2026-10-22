//! Tag management API: `/admin/api/tags`.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::api_media::{parse_body, ApiError};
use super::{require_csrf, AdminSession};
use crate::app::AppState;
use crate::db::tags::{self, Tag};

pub fn valid_colour(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Trimmed tag name, 1..=40 chars.
pub fn clean_name(raw: &str) -> Result<String, ApiError> {
    let n = raw.trim();
    if n.is_empty() || n.chars().count() > 40 {
        return Err(ApiError::bad("tag name must be 1 to 40 characters"));
    }
    if n.contains(',') {
        return Err(ApiError::bad("tag names may not contain commas"));
    }
    Ok(n.to_string())
}

fn check_colour(c: &str) -> Result<(), ApiError> {
    if valid_colour(c) {
        Ok(())
    } else {
        Err(ApiError::bad("colour must look like #rrggbb"))
    }
}

pub async fn list(
    AdminSession(_): AdminSession,
    State(state): State<AppState>,
) -> Result<Json<Vec<Tag>>, ApiError> {
    Ok(Json(tags::list_all(&state.db).await?))
}

#[derive(Deserialize)]
struct CreateBody {
    name: String,
    colour: Option<String>,
}

pub async fn create(
    AdminSession(session): AdminSession,
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Tag>, ApiError> {
    require_csrf(&session, &headers, None)?;
    let b: CreateBody = parse_body(&body)?;
    let name = clean_name(&b.name)?;
    let colour = b.colour.unwrap_or_else(|| tags::DEFAULT_COLOUR.to_string());
    check_colour(&colour)?;
    let all = tags::list_all(&state.db).await?;
    if all.iter().any(|t| t.name.eq_ignore_ascii_case(&name)) {
        return Err(ApiError::new(StatusCode::CONFLICT, "tag already exists"));
    }
    Ok(Json(tags::create(&state.db, &name, &colour).await?))
}

#[derive(Deserialize)]
struct PatchBody {
    name: Option<String>,
    colour: Option<String>,
}

pub async fn patch(
    AdminSession(session): AdminSession,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Tag>, ApiError> {
    require_csrf(&session, &headers, None)?;
    let b: PatchBody = parse_body(&body)?;
    let name = b.name.as_deref().map(clean_name).transpose()?;
    if let Some(c) = &b.colour {
        check_colour(c)?;
    }
    let all = tags::list_all(&state.db).await?;
    if !all.iter().any(|t| t.id == id) {
        return Err(ApiError::not_found());
    }
    if let Some(n) = &name {
        if all
            .iter()
            .any(|t| t.id != id && t.name.eq_ignore_ascii_case(n))
        {
            return Err(ApiError::new(StatusCode::CONFLICT, "tag already exists"));
        }
        tags::rename(&state.db, id, n).await?;
    }
    if let Some(c) = &b.colour {
        tags::recolour(&state.db, id, c).await?;
    }
    let tag = tags::list_all(&state.db)
        .await?
        .into_iter()
        .find(|t| t.id == id)
        .ok_or_else(ApiError::not_found)?;
    Ok(Json(tag))
}

pub async fn delete(
    AdminSession(session): AdminSession,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_csrf(&session, &headers, None)?;
    let all = tags::list_all(&state.db).await?;
    if !all.iter().any(|t| t.id == id) {
        return Err(ApiError::not_found());
    }
    tags::delete(&state.db, id).await?;
    Ok(Json(json!({"ok": true})))
}
