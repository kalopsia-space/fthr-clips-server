//! Settings API (`/admin/api/settings`), shared validation and the
//! form-to-patch conversion used by the settings pages.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde_json::{json, Map, Value};

use super::api_media::{parse_body, ApiError};
use super::{require_csrf, AdminSession};
use crate::app::AppState;
use crate::settings::Settings;

const LADDER_RUNGS: [u64; 3] = [480, 720, 1080];

/// Name of the detected encoder, if any.
pub async fn effective_encoder(state: &AppState) -> Option<&'static str> {
    state.encoder.read().await.map(|e| e.name())
}

/// Detect the encoder again in the background after the choice changed.
pub fn redetect(state: &AppState, choice: crate::config::EncoderChoice) {
    let (ffmpeg, slot) = (state.ffmpeg.clone(), state.encoder.clone());
    tokio::spawn(async move {
        let found = crate::ffmpeg::encoders::detect(&ffmpeg, choice).await;
        *slot.write().await = found;
    });
}

/// The settings plus the effective encoder (null until detected).
async fn view(state: &AppState, s: &Settings) -> Value {
    let mut v = serde_json::to_value(s).unwrap_or_else(|_| json!({}));
    if let Value::Object(m) = &mut v {
        m.insert(
            "effective_encoder".into(),
            json!(effective_encoder(state).await),
        );
    }
    v
}

pub async fn get(AdminSession(_): AdminSession, State(state): State<AppState>) -> Json<Value> {
    Json(view(&state, &state.settings.get()).await)
}

pub async fn put(
    AdminSession(session): AdminSession,
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    require_csrf(&session, &headers, None)?;
    let mut patch: Value = parse_body(&body)?;
    // Allow clients to send back what GET returned.
    if let Value::Object(m) = &mut patch {
        m.remove("effective_encoder");
    }
    validate(&mut patch).map_err(ApiError::bad)?;
    let old = state.settings.get().encoder;
    let new = state
        .settings
        .update(&state.db, patch)
        .await
        .map_err(|e| ApiError::bad(format!("{e:#}")))?;
    if new.encoder != old {
        redetect(&state, new.encoder);
    }
    Ok(Json(view(&state, &new).await))
}

fn int_in(m: &Map<String, Value>, key: &str, lo: u64, hi: u64) -> Result<(), String> {
    match m.get(key) {
        None => Ok(()),
        Some(v) => match v.as_u64() {
            Some(n) if (lo..=hi).contains(&n) => Ok(()),
            _ => Err(format!("{key} must be a whole number from {lo} to {hi}")),
        },
    }
}

fn url_or_null(m: &mut Map<String, Value>, key: &str, strip_slash: bool) -> Result<(), String> {
    let Some(v) = m.get_mut(key) else {
        return Ok(());
    };
    match v {
        Value::Null => Ok(()),
        Value::String(s) => {
            let t = s.trim();
            let t = if strip_slash {
                t.trim_end_matches('/')
            } else {
                t
            };
            if t.is_empty() {
                *v = Value::Null;
            } else if t.starts_with("http://") || t.starts_with("https://") {
                *v = Value::String(t.to_string());
            } else {
                return Err(format!("{key} must start with http:// or https://"));
            }
            Ok(())
        }
        _ => Err(format!("{key} must be a string or null")),
    }
}

fn ext_list(m: &mut Map<String, Value>, key: &str) -> Result<(), String> {
    let Some(v) = m.get_mut(key) else {
        return Ok(());
    };
    let err = || format!("{key} must be a non-empty list of lowercase alphanumeric extensions");
    let Value::Array(items) = v else {
        return Err(err());
    };
    let mut out: Vec<Value> = Vec::new();
    for item in items.iter() {
        let s = item
            .as_str()
            .ok_or_else(err)?
            .trim()
            .trim_start_matches('.');
        if s.is_empty()
            || !s
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        {
            return Err(err());
        }
        if !out.iter().any(|o| o.as_str() == Some(s)) {
            out.push(Value::String(s.to_string()));
        }
    }
    if out.is_empty() {
        return Err(err());
    }
    *v = Value::Array(out);
    Ok(())
}

/// Semantic checks beyond what `SettingsStore::update` enforces; normalises
/// values in place (URL trimming, extension cleanup).
pub fn validate(patch: &mut Value) -> Result<(), String> {
    let Value::Object(m) = patch else {
        return Err("settings must be a JSON object".into());
    };
    if let Some(v) = m.get("ladder") {
        let ok = v.as_array().is_some_and(|a| {
            !a.is_empty()
                && a.iter()
                    .all(|r| r.as_u64().is_some_and(|n| LADDER_RUNGS.contains(&n)))
        });
        if !ok {
            return Err("ladder must be a non-empty list of 480, 720 or 1080".into());
        }
    }
    int_in(m, "transcode_workers", 1, 8)?;
    int_in(m, "transcode_timeout_minutes", 1, 1440)?;
    int_in(m, "scan_interval_minutes", 0, 1440)?;
    if let Some(v) = m.get("upload_folder") {
        let ok = v.as_str().is_some_and(|s| {
            !s.is_empty()
                && !s.starts_with('.')
                && !s.contains("..")
                && !s.contains('/')
                && !s.contains('\\')
                && !s.contains('\0')
        });
        if !ok {
            return Err("upload_folder must be a plain relative folder name".into());
        }
    }
    if let Some(v) = m.get("site_title") {
        let ok = v
            .as_str()
            .is_some_and(|s| !s.trim().is_empty() && s.chars().count() <= 80);
        if !ok {
            return Err("site_title must be 1 to 80 characters".into());
        }
        if let Some(Value::String(s)) = m.get_mut("site_title") {
            *s = s.trim().to_string();
        }
    }
    url_or_null(m, "public_base_url", true)?;
    url_or_null(m, "discord_webhook_url", false)?;
    url_or_null(m, "webhook_url", false)?;
    ext_list(m, "video_exts")?;
    ext_list(m, "image_exts")?;
    Ok(())
}

fn text(form: &[(String, String)], key: &str) -> Option<String> {
    form.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.trim().to_string())
}

fn flag(form: &[(String, String)], key: &str) -> Value {
    Value::Bool(
        form.iter()
            .any(|(k, v)| k == key && v != "0" && v != "false"),
    )
}

fn number(
    form: &[(String, String)],
    key: &str,
    out: &mut Map<String, Value>,
) -> Result<(), String> {
    if let Some(t) = text(form, key) {
        let n: u64 = t
            .parse()
            .map_err(|_| format!("{key} must be a whole number"))?;
        out.insert(key.into(), json!(n));
    }
    Ok(())
}

fn string(form: &[(String, String)], key: &str, out: &mut Map<String, Value>) {
    if let Some(t) = text(form, key) {
        out.insert(key.into(), Value::String(t));
    }
}

fn csv(form: &[(String, String)], key: &str, out: &mut Map<String, Value>) {
    if let Some(t) = text(form, key) {
        let items: Vec<Value> = t
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| Value::String(s.to_ascii_lowercase()))
            .collect();
        out.insert(key.into(), Value::Array(items));
    }
}

/// Turns a settings-page form post into a settings patch for `tab`.
/// Unchecked checkboxes (absent from the form) become `false`.
pub fn form_to_patch(tab: &str, form: &[(String, String)]) -> Result<Value, String> {
    let mut m = Map::new();
    match tab {
        "library" => {
            number(form, "scan_interval_minutes", &mut m)?;
            csv(form, "video_exts", &mut m);
            csv(form, "image_exts", &mut m);
            string(form, "upload_folder", &mut m);
            m.insert(
                "upload_default_public".into(),
                flag(form, "upload_default_public"),
            );
        }
        "transcoding" => {
            string(form, "transcoding", &mut m);
            string(form, "encoder", &mut m);
            number(form, "transcode_workers", &mut m)?;
            number(form, "transcode_timeout_minutes", &mut m)?;
            let mut ladder = Vec::new();
            for (k, v) in form.iter().filter(|(k, _)| k == "ladder") {
                let n: u64 = v
                    .trim()
                    .parse()
                    .map_err(|_| format!("{k} values must be numbers"))?;
                if !ladder.contains(&n) {
                    ladder.push(n);
                }
            }
            ladder.sort_unstable();
            m.insert("ladder".into(), json!(ladder));
        }
        "sharing" => {
            string(form, "site_title", &mut m);
            string(form, "public_base_url", &mut m);
            string(form, "default_sort", &mut m);
            m.insert(
                "library_requires_login".into(),
                flag(form, "library_requires_login"),
            );
        }
        "integrations" => {
            string(form, "discord_webhook_url", &mut m);
            string(form, "webhook_url", &mut m);
            m.insert("webhooks_enabled".into(), flag(form, "webhooks_enabled"));
        }
        _ => return Err("unknown settings page".into()),
    }
    Ok(Value::Object(m))
}
