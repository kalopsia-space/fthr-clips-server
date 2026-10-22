use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};

use axum::extract::multipart::Field;
use axum::extract::{ConnectInfo, Multipart, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use rand::RngCore;
use serde::Serialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::io::AsyncWriteExt;

use crate::app::AppState;
use crate::db::media::{self, NewMedia};
use crate::db::{tokens, Media, MediaKind, Source};
use crate::media::filename_date::recorded_at_from_name;
use crate::media::ids::{media_id, slug_candidates};
use crate::media::kind::kind_for;
use crate::settings::public_base;

const SNIFF_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenIdentity {
    Env,
    Db(i64),
}

impl TokenIdentity {
    fn limiter_key(self) -> String {
        match self {
            TokenIdentity::Env => "env".into(),
            TokenIdentity::Db(id) => format!("db:{id}"),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct UploadResponse {
    pub url: String,
    pub id: String,
    pub bytes: u64,
    pub listed: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    #[error("upload too large")]
    TooLarge,
    #[error("unsupported media type")]
    Unsupported,
    #[error("missing clip field")]
    MissingField,
    #[error("malformed multipart body")]
    BadRequest,
    #[error("authentication failed")]
    Auth(StatusCode),
    #[error("rate limited")]
    RateLimited,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Db(anyhow::Error),
}

impl IntoResponse for UploadError {
    fn into_response(self) -> Response {
        let status = match &self {
            UploadError::Auth(s) => *s,
            UploadError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            UploadError::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            UploadError::Unsupported => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            UploadError::MissingField | UploadError::BadRequest => StatusCode::BAD_REQUEST,
            UploadError::Io(_) | UploadError::Db(_) => {
                tracing::error!("upload failed: {self}");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (status, self.to_string()).into_response()
    }
}

pub async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<TokenIdentity, StatusCode> {
    let value = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let (scheme, token) = value.split_once(' ').ok_or(StatusCode::UNAUTHORIZED)?;
    let token = token.trim();
    if !scheme.eq_ignore_ascii_case("bearer") || token.is_empty() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if let Some(env_token) = state.config.upload_token.as_deref() {
        if bool::from(env_token.as_bytes().ct_eq(token.as_bytes())) {
            return Ok(TokenIdentity::Env);
        }
    }
    let hash = hex::encode(Sha256::digest(token.as_bytes()));
    match tokens::find_active_by_hash(&state.db, &hash).await {
        Ok(Some(row)) => {
            if let Err(e) = tokens::touch(&state.db, row.id).await {
                tracing::warn!("touching upload token {}: {e}", row.id);
            }
            Ok(TokenIdentity::Db(row.id))
        }
        Ok(None) => Err(StatusCode::UNAUTHORIZED),
        Err(e) => {
            tracing::error!("token lookup failed: {e}");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

pub async fn head_upload(State(state): State<AppState>, headers: HeaderMap) -> StatusCode {
    match authenticate(&state, &headers).await {
        Ok(_) => StatusCode::OK,
        Err(s) => s,
    }
}

pub async fn post_upload(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<UploadResponse>, UploadError> {
    let identity = authenticate(&state, &headers)
        .await
        .map_err(UploadError::Auth)?;
    if !state.upload_limiter.check(&identity.limiter_key()) {
        return Err(UploadError::RateLimited);
    }
    let media = loop {
        let field = multipart
            .next_field()
            .await
            .map_err(|_| UploadError::BadRequest)?
            .ok_or(UploadError::MissingField)?;
        if field.name() == Some("clip") {
            break store_upload(&state, field).await?;
        }
    };
    let settings = state.settings.get();
    let base = public_base(
        &settings,
        &headers,
        peer.ip(),
        &state.config.trusted_proxies,
    );
    let prefix = match media.kind {
        MediaKind::Video => "w",
        MediaKind::Image => "i",
    };
    Ok(Json(UploadResponse {
        url: format!("{base}/{prefix}/{}", media.slug),
        id: media.id,
        bytes: media.size.max(0) as u64,
        listed: media.public,
    }))
}

/// Removes the temp file on drop unless disarmed (covers every error path).
struct TempGuard(Option<PathBuf>);

impl TempGuard {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            let _ = std::fs::remove_file(p);
        }
    }
}

pub async fn store_upload(state: &AppState, mut field: Field<'_>) -> Result<Media, UploadError> {
    let settings = state.settings.get();
    let max = state.config.max_upload_bytes;
    let original = field.file_name().unwrap_or("").to_string();
    let ext = Path::new(&basename(&original))
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .ok_or(UploadError::Unsupported)?;

    // Buffer the head so the type check happens before anything touches disk.
    let mut head: Vec<u8> = Vec::with_capacity(SNIFF_BYTES);
    let mut done = false;
    while head.len() < SNIFF_BYTES {
        match field.chunk().await.map_err(|_| UploadError::BadRequest)? {
            Some(c) => head.extend_from_slice(&c),
            None => {
                done = true;
                break;
            }
        }
    }
    let sniff = &head[..head.len().min(SNIFF_BYTES)];
    let kind = kind_for(&ext, sniff).ok_or(UploadError::Unsupported)?;
    if head.len() as u64 > max {
        return Err(UploadError::TooLarge);
    }

    let dir = state
        .config
        .data_dir
        .join(safe_folder(&settings.upload_folder));
    tokio::fs::create_dir_all(&dir).await?;
    let mut tmp_name = [0u8; 8];
    rand::rng().fill_bytes(&mut tmp_name);
    let temp = dir.join(format!(".incoming-{}", hex::encode(tmp_name)));
    let mut guard = TempGuard(Some(temp.clone()));
    let mut file = tokio::fs::File::create(&temp).await?;
    file.write_all(&head).await?;
    let mut total = head.len() as u64;
    drop(head);
    while !done {
        match field.chunk().await.map_err(|_| UploadError::BadRequest)? {
            Some(c) => {
                total += c.len() as u64;
                if total > max {
                    return Err(UploadError::TooLarge);
                }
                file.write_all(&c).await?;
            }
            None => done = true,
        }
    }
    file.flush().await?;
    drop(file);

    let id = {
        let t = temp.clone();
        tokio::task::spawn_blocking(move || media_id(&t))
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))??
    };

    let stale = media::get(&state.db, &id).await.map_err(UploadError::Db)?;
    if let Some(existing) = &stale {
        if tokio::fs::try_exists(&existing.path).await.unwrap_or(false) {
            return Ok(existing.clone());
        }
    }

    let mut slug = None;
    for cand in slug_candidates(&id) {
        if !media::slug_exists(&state.db, &cand)
            .await
            .map_err(UploadError::Db)?
        {
            slug = Some(cand);
            break;
        }
    }
    let slug = slug.ok_or_else(|| UploadError::Db(anyhow::anyhow!("no free slug for {id}")))?;

    let name = sanitise_name(&original, &id, &ext);
    let final_path = unique_path(&dir, &name).await;
    tokio::fs::rename(&temp, &final_path).await?;
    guard.disarm();
    if stale.is_some() {
        // Known content whose file went missing: point the row at the new copy.
        let path = final_path.to_string_lossy().into_owned();
        if let Err(e) = media::relink(&state.db, &id, &path).await {
            let _ = tokio::fs::remove_file(&final_path).await;
            return Err(UploadError::Db(e));
        }
        return media::get(&state.db, &id)
            .await
            .map_err(UploadError::Db)?
            .ok_or_else(|| UploadError::Db(anyhow::anyhow!("row vanished after relink")));
    }
    let final_name = final_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let new = NewMedia {
        id: id.clone(),
        slug,
        kind,
        path: final_path.to_string_lossy().into_owned(),
        source: Source::Upload,
        folder: String::new(),
        extension: ext,
        size: total as i64,
        title: Path::new(&final_name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        public: settings.upload_default_public,
        recorded_at: recorded_at_from_name(&final_name),
    };
    if let Err(e) = media::insert(&state.db, &new).await {
        // A concurrent identical upload may have won the race.
        if let Ok(Some(existing)) = media::get(&state.db, &id).await {
            if existing.path != new.path {
                let _ = tokio::fs::remove_file(&final_path).await;
            }
            return Ok(existing);
        }
        let _ = tokio::fs::remove_file(&final_path).await;
        return Err(UploadError::Db(e));
    }
    let row = media::get(&state.db, &id)
        .await
        .map_err(UploadError::Db)?
        .ok_or_else(|| UploadError::Db(anyhow::anyhow!("row vanished after insert")))?;
    crate::jobs::enqueue_for_new_media(state, &row).await;
    Ok(row)
}

/// The upload folder must stay inside the data dir.
fn safe_folder(folder: &str) -> PathBuf {
    let p = Path::new(folder);
    if !folder.is_empty() && p.components().all(|c| matches!(c, Component::Normal(_))) {
        p.to_path_buf()
    } else {
        PathBuf::from("uploads")
    }
}

fn basename(name: &str) -> String {
    name.rsplit(['/', '\\']).next().unwrap_or("").to_string()
}

fn sanitise_name(original: &str, id: &str, ext: &str) -> String {
    let kept: String = basename(original)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ' '))
        .collect();
    let mut collapsed = String::new();
    for c in kept.chars() {
        if c == ' ' && collapsed.ends_with(' ') {
            continue;
        }
        collapsed.push(c);
    }
    let name = collapsed.trim().trim_start_matches('.').trim().to_string();
    let stem = Path::new(&name)
        .file_stem()
        .map(|s| s.to_string_lossy().trim().to_string())
        .unwrap_or_default();
    if stem.is_empty() || stem.chars().all(|c| c == '.') {
        format!("{id}.{ext}")
    } else {
        name
    }
}

async fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !tokio::fs::try_exists(&first).await.unwrap_or(false) {
        return first;
    }
    let p = Path::new(name);
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = p
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let mut n = 2;
    loop {
        let cand = dir.join(format!("{stem}-{n}{ext}"));
        if !tokio::fs::try_exists(&cand).await.unwrap_or(false) {
            return cand;
        }
        n += 1;
    }
}
