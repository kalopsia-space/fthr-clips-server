//! Media management API: `/admin/api/media`.

use std::collections::HashMap;
use std::path::{Component, Path as FsPath, PathBuf};

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::api_tags::clean_name;
use super::{require_csrf, AdminSession};
use crate::app::AppState;
use crate::db::jobs;
use crate::db::media::{self, ListQuery, Media, MediaWithViews};
use crate::db::tags::{self, Tag};
use crate::media::kind::{MediaKind, Source};
use crate::media::paths::DerivedPaths;
use crate::web::library::{
    media_url, page_path, parse_sort, poster_path, LibraryItem, TagView, PER_PAGE,
};
use crate::web::upload::safe_folder;

/// JSON error `{"error": "..."}` with a status code.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
    pub fn bad(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }
    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not found")
    }
}

impl From<StatusCode> for ApiError {
    fn from(status: StatusCode) -> Self {
        let message = match status {
            StatusCode::FORBIDDEN => "forbidden",
            _ => "error",
        };
        Self::new(status, message)
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        tracing::error!(error = %e, "admin api error");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({"error": self.message}))).into_response()
    }
}

/// Parses a JSON body after the CSRF check has passed (so a bad token is
/// always 403, whatever the body).
pub fn parse_body<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|_| ApiError::bad("invalid JSON body"))
}

#[derive(Serialize)]
struct RenditionView {
    height: i64,
    state: String,
    progress: f64,
}

#[derive(Serialize)]
struct JobView {
    id: i64,
    kind: String,
    state: String,
    progress: f64,
    error: Option<String>,
}

#[derive(Serialize)]
pub struct AdminItem {
    #[serde(flatten)]
    item: LibraryItem,
    public: bool,
    available: bool,
    source: &'static str,
    folder: String,
    /// File name only; the absolute path never leaves the server.
    path: String,
    processing_state: String,
    processing_error: Option<String>,
    renditions: Vec<RenditionView>,
    jobs: Vec<JobView>,
}

async fn admin_item(state: &AppState, row: &MediaWithViews) -> anyhow::Result<AdminItem> {
    let m = &row.media;
    let renditions = jobs::renditions_for(&state.db, &m.id)
        .await?
        .into_iter()
        .map(|r| RenditionView {
            height: r.height,
            state: r.state,
            progress: r.progress,
        })
        .collect();
    let jobs = jobs::list_for_media(&state.db, &m.id)
        .await?
        .into_iter()
        .map(|j| JobView {
            id: j.id,
            kind: j.kind,
            state: j.state,
            progress: j.progress,
            error: j.error,
        })
        .collect();
    Ok(AdminItem {
        item: LibraryItem {
            id: m.id.clone(),
            slug: m.slug.clone(),
            kind: m.kind.as_str(),
            title: m.title.clone(),
            url: page_path(m),
            poster_url: poster_path(m),
            preview_url: (m.has_preview && m.kind == MediaKind::Video)
                .then(|| media_url(m, "preview.mp4")),
            duration_ms: m.duration_ms,
            width: m.width,
            height: m.height,
            size: m.size,
            views: row.views,
            created_at: m.created_at,
            recorded_at: m.recorded_at,
            tags: row.tags.iter().map(|t| TagView::new(t, "")).collect(),
        },
        public: m.public,
        available: m.available,
        source: m.source.as_str(),
        folder: m.folder.clone(),
        path: FsPath::new(&m.path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        processing_state: m.processing_state.clone(),
        processing_error: m.processing_error.clone(),
        renditions,
        jobs,
    })
}

async fn load(state: &AppState, id: &str) -> Result<MediaWithViews, ApiError> {
    let media = media::get(&state.db, id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let views = media::view_count(&state.db, id).await?;
    let tags = tags::for_media(&state.db, id).await?;
    Ok(MediaWithViews { media, views, tags })
}

async fn item_json(state: &AppState, id: &str) -> Result<Json<AdminItem>, ApiError> {
    let row = load(state, id).await?;
    Ok(Json(admin_item(state, &row).await?))
}

#[derive(Serialize)]
pub struct ListResponse {
    items: Vec<AdminItem>,
    page: u32,
    total: i64,
}

/// `GET /admin/api/media?kind=&q=&tag=&sort=&page=&per_page=`.
pub async fn list(
    AdminSession(_): AdminSession,
    State(state): State<AppState>,
    Query(map): Query<HashMap<String, String>>,
) -> Result<Json<ListResponse>, ApiError> {
    let get = |k: &str| {
        map.get(k)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let kind = match get("kind").as_deref() {
        None => None,
        Some("video") => Some(MediaKind::Video),
        Some("image") => Some(MediaKind::Image),
        Some(_) => return Err(ApiError::bad("unknown kind")),
    };
    let sort = match get("sort") {
        None => state.settings.get().default_sort,
        Some(s) => parse_sort(&s).ok_or_else(|| ApiError::bad("unknown sort"))?,
    };
    let page = get("page")
        .and_then(|p| p.parse::<u32>().ok())
        .filter(|p| *p >= 1)
        .unwrap_or(1);
    let per_page = get("per_page")
        .and_then(|p| p.parse::<u32>().ok())
        .map_or(PER_PAGE, |p| p.clamp(1, 200));
    let q = ListQuery {
        kind,
        public_only: false,
        q: get("q"),
        tag: get("tag"),
        sort,
        page,
        per_page,
    };
    let total = media::count_matching(&state.db, &q).await?;
    let rows = media::list(&state.db, &q).await?;
    let mut items = Vec::with_capacity(rows.len());
    for r in &rows {
        items.push(admin_item(&state, r).await?);
    }
    Ok(Json(ListResponse { items, page, total }))
}

pub async fn get_one(
    AdminSession(_): AdminSession,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<AdminItem>, ApiError> {
    item_json(&state, &id).await
}

#[derive(Deserialize)]
struct PatchBody {
    title: Option<String>,
    description: Option<String>,
    public: Option<bool>,
    tags: Option<Vec<String>>,
}

pub async fn patch(
    AdminSession(session): AdminSession,
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<AdminItem>, ApiError> {
    require_csrf(&session, &headers, None)?;
    let b: PatchBody = parse_body(&body)?;
    let title = match &b.title {
        Some(t) => {
            let t = t.trim();
            if t.is_empty() || t.chars().count() > 200 {
                return Err(ApiError::bad("title must be 1 to 200 characters"));
            }
            Some(t.to_string())
        }
        None => None,
    };
    if let Some(d) = &b.description {
        if d.chars().count() > 4000 {
            return Err(ApiError::bad("description is too long"));
        }
    }
    let mut names: Vec<String> = Vec::new();
    for raw in b.tags.iter().flatten() {
        let n = clean_name(raw)?;
        if !names.iter().any(|x| x.eq_ignore_ascii_case(&n)) {
            names.push(n);
        }
    }
    let current = media::get(&state.db, &id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    if title.is_some() || b.description.is_some() {
        media::update_details(
            &state.db,
            &id,
            title.as_deref().unwrap_or(&current.title),
            b.description.as_deref().unwrap_or(&current.description),
        )
        .await?;
    }
    if let Some(p) = b.public {
        media::set_public(&state.db, std::slice::from_ref(&id), p).await?;
    }
    if b.tags.is_some() {
        for t in tags::for_media(&state.db, &id).await? {
            if !names.iter().any(|n| n.eq_ignore_ascii_case(&t.name)) {
                tags::detach(&state.db, &id, t.id).await?;
            }
        }
        for n in &names {
            tags::attach_by_name(&state.db, std::slice::from_ref(&id), n).await?;
        }
    }
    item_json(&state, &id).await
}

#[derive(Deserialize)]
pub struct DeleteQuery {
    #[serde(default)]
    delete_file: bool,
}

pub async fn delete(
    AdminSession(session): AdminSession,
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Query<DeleteQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_csrf(&session, &headers, None)?;
    let m = media::get(&state.db, &id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    delete_media(&state, &m, q.delete_file).await?;
    Ok(Json(json!({"ok": true})))
}

fn canon(p: &FsPath) -> Option<PathBuf> {
    std::fs::canonicalize(p).ok()
}

fn upload_root(state: &AppState) -> Option<PathBuf> {
    let folder = state.settings.get().upload_folder.clone();
    canon(&state.config.data_dir.join(safe_folder(&folder)))
}

fn scan_roots(state: &AppState) -> Vec<PathBuf> {
    state
        .config
        .video_dirs
        .iter()
        .chain(state.config.image_dirs.iter())
        .filter_map(|d| canon(d))
        .collect()
}

/// Removes derived files and the row (cascading tags, views, jobs, renditions).
/// The source file is deleted only when it is an upload inside the upload
/// folder, or `delete_file` is set and it lies inside a configured scan root.
/// Otherwise a tombstone keeps the scanner from re-adding it.
pub async fn delete_media(
    state: &AppState,
    media: &Media,
    delete_file: bool,
) -> anyhow::Result<()> {
    DerivedPaths::new(&state.config.data_dir, &media.id).remove_all()?;
    let file = canon(FsPath::new(&media.path));
    let in_upload = || match (&file, upload_root(state)) {
        (Some(f), Some(r)) => f.starts_with(r),
        _ => false,
    };
    let in_scan = || match &file {
        Some(f) => scan_roots(state).iter().any(|r| f.starts_with(r)),
        None => false,
    };
    let remove = if media.source == Source::Upload && in_upload() {
        tracing::info!(id = %media.id, "delete: removing uploaded file");
        true
    } else if delete_file && in_scan() {
        tracing::info!(id = %media.id, "delete: removing scanned file on request");
        true
    } else {
        false
    };
    if remove {
        if let Some(f) = &file {
            std::fs::remove_file(f)?;
        }
    }
    media::delete(&state.db, &media.id).await?;
    if !remove {
        tracing::info!(id = %media.id, "delete: file kept, tombstone added");
        media::add_tombstone(&state.db, &media.id, &media.path).await?;
    }
    Ok(())
}

fn valid_folder(folder: &str) -> Result<String, ApiError> {
    let f = folder.strip_suffix('/').unwrap_or(folder);
    let ok = !f.is_empty()
        && !f.contains('\\')
        && !f.contains('\0')
        && !f.starts_with('/')
        && f.split('/').all(|c| !c.is_empty() && c != "." && c != "..");
    if ok {
        Ok(f.to_string())
    } else {
        Err(ApiError::bad("invalid folder"))
    }
}

/// Moves the source file to `folder` (relative) under its root and updates the row.
pub async fn move_media(state: &AppState, media: &Media, folder: &str) -> Result<(), ApiError> {
    let folder = valid_folder(folder)?;
    let current =
        canon(FsPath::new(&media.path)).ok_or_else(|| ApiError::bad("source file is missing"))?;
    let root = if media.source == Source::Upload {
        upload_root(state)
    } else {
        scan_roots(state)
            .into_iter()
            .find(|r| current.starts_with(r))
    }
    .ok_or_else(|| ApiError::bad("file is not inside a movable root"))?;
    let dir = root.join(&folder);
    // Components are validated, so this only fails on filesystem trouble.
    if !dir.components().all(|c| !matches!(c, Component::ParentDir)) {
        return Err(ApiError::bad("invalid folder"));
    }
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(anyhow::Error::from)?;
    // A symlinked folder component must not lead out of the root.
    let dir = canon(&dir).filter(|d| d.starts_with(&root));
    let dir = dir.ok_or_else(|| ApiError::bad("invalid folder"))?;
    let name = current
        .file_name()
        .ok_or_else(|| ApiError::bad("source file has no name"))?;
    let target = dir.join(name);
    if target == current {
        return Ok(());
    }
    if tokio::fs::try_exists(&target)
        .await
        .map_err(anyhow::Error::from)?
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "a file with that name already exists there",
        ));
    }
    tokio::fs::rename(&current, &target)
        .await
        .map_err(anyhow::Error::from)?;
    if let Err(e) = media::relocate(&state.db, &media.id, &target.to_string_lossy(), &folder).await
    {
        let _ = tokio::fs::rename(&target, &current).await;
        return Err(e.into());
    }
    Ok(())
}

#[derive(Deserialize)]
struct BulkBody {
    action: String,
    ids: Vec<String>,
    tag: Option<String>,
    folder: Option<String>,
    #[serde(default)]
    delete_file: bool,
}

#[derive(Serialize)]
struct Failure {
    id: String,
    error: String,
}

#[derive(Serialize)]
pub struct BulkResponse {
    ok: usize,
    failed: Vec<Failure>,
}

pub async fn bulk(
    AdminSession(session): AdminSession,
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<BulkResponse>, ApiError> {
    require_csrf(&session, &headers, None)?;
    let b: BulkBody = parse_body(&body)?;
    let tag = match b.action.as_str() {
        "add_tag" | "remove_tag" => {
            let t = b
                .tag
                .as_deref()
                .ok_or_else(|| ApiError::bad("tag is required"))?;
            Some(clean_name(t)?)
        }
        _ => None,
    };
    if b.action == "move" && b.folder.is_none() {
        return Err(ApiError::bad("folder is required"));
    }
    if !matches!(
        b.action.as_str(),
        "public"
            | "link_only"
            | "add_tag"
            | "remove_tag"
            | "move"
            | "transcode"
            | "remove_renditions"
            | "delete"
    ) {
        return Err(ApiError::bad("unknown action"));
    }
    let mut ok = 0;
    let mut failed = Vec::new();
    for id in &b.ids {
        let result: Result<(), ApiError> = async {
            let m = media::get(&state.db, id)
                .await?
                .ok_or_else(ApiError::not_found)?;
            let one = std::slice::from_ref(id);
            match b.action.as_str() {
                "public" => media::set_public(&state.db, one, true).await?,
                "link_only" => media::set_public(&state.db, one, false).await?,
                "add_tag" => {
                    tags::attach_by_name(&state.db, one, tag.as_deref().unwrap_or("")).await?;
                }
                "remove_tag" => {
                    let name = tag.as_deref().unwrap_or("");
                    let found = tags::for_media(&state.db, id)
                        .await?
                        .into_iter()
                        .find(|t: &Tag| t.name.eq_ignore_ascii_case(name));
                    match found {
                        Some(t) => tags::detach(&state.db, id, t.id).await?,
                        None => return Err(ApiError::bad("tag not on this item")),
                    }
                }
                "move" => move_media(&state, &m, b.folder.as_deref().unwrap_or("")).await?,
                "delete" => delete_media(&state, &m, b.delete_file).await?,
                _ => return Err(ApiError::bad("not available yet")),
            }
            Ok(())
        }
        .await;
        match result {
            Ok(()) => ok += 1,
            Err(e) => failed.push(Failure {
                id: id.clone(),
                error: e.message,
            }),
        }
    }
    Ok(Json(BulkResponse { ok, failed }))
}
