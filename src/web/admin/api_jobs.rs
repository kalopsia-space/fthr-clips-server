//! Job queue, stats and event stream: `/admin/api/{jobs,stats,events}`.

use std::convert::Infallible;
use std::path::Path as FsPath;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::Json;
use chrono::{DateTime, Utc};
use futures::{Stream, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use tokio_stream::wrappers::BroadcastStream;

use super::api_media::ApiError;
use super::{require_csrf, AdminSession};
use crate::app::AppState;
use crate::db::jobs::{self, JobWithTitle};
use crate::db::media;
use crate::jobs::LIGHT_WORKERS;
use crate::media::kind::MediaKind;

#[derive(Serialize)]
pub struct JobView {
    id: i64,
    media_id: String,
    media_title: Option<String>,
    kind: String,
    state: String,
    attempts: i64,
    progress: f64,
    error: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<JobWithTitle> for JobView {
    fn from(r: JobWithTitle) -> Self {
        let j = r.job;
        Self {
            id: j.id,
            media_id: j.media_id,
            media_title: r.media_title,
            kind: j.kind,
            state: j.state,
            attempts: j.attempts,
            progress: j.progress,
            error: j.error,
            created_at: j.created_at,
            updated_at: j.updated_at,
        }
    }
}

pub async fn list(
    AdminSession(_): AdminSession,
    State(state): State<AppState>,
) -> Result<Json<Value>, ApiError> {
    let active: Vec<JobView> = jobs::list_active_titled(&state.db)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    let recent: Vec<JobView> = jobs::list_recent(&state.db, 50)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(json!({ "active": active, "recent": recent })))
}

pub async fn cancel(
    AdminSession(session): AdminSession,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_csrf(&session, &headers, None)?;
    let job = jobs::get(&state.db, id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let cancellable = matches!(job.state.as_str(), "queued" | "running");
    if cancellable {
        jobs::cancel(&state.db, id).await?;
        let token = state
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .cloned();
        if let Some(t) = token {
            t.cancel();
        }
        if let Some(h) = job
            .kind
            .strip_prefix("transcode:")
            .and_then(|h| h.parse::<u32>().ok())
        {
            jobs::rendition_fail(&state.db, &job.media_id, i64::from(h), "cancelled").await?;
            let tmp = crate::media::paths::DerivedPaths::new(&state.config.data_dir, &job.media_id)
                .rendition_tmp(h, job.id);
            let _ = tokio::fs::remove_file(tmp).await;
        }
        crate::jobs::worker::emit(&state, &job, "cancelled", job.progress, None);
    }
    Ok(Json(json!({ "cancelled": cancellable })))
}

pub async fn retry(
    AdminSession(session): AdminSession,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_csrf(&session, &headers, None)?;
    let job = jobs::get(&state.db, id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let requeued = jobs::retry(&state.db, id).await?;
    if requeued {
        crate::jobs::worker::emit(&state, &job, "queued", 0.0, None);
    }
    Ok(Json(json!({ "requeued": requeued })))
}

/// Used and total bytes of the disk holding `dir` (longest mount-point prefix).
fn disk_usage(dir: &FsPath) -> Option<(u64, u64)> {
    let dir = std::fs::canonicalize(dir).ok()?;
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let disk = disks
        .list()
        .iter()
        .filter(|d| dir.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())?;
    let total = disk.total_space();
    Some((total.saturating_sub(disk.available_space()), total))
}

pub async fn stats(
    AdminSession(_): AdminSession,
    State(state): State<AppState>,
) -> Result<Json<Value>, ApiError> {
    let videos = media::count(&state.db, Some(MediaKind::Video), false).await?;
    let images = media::count(&state.db, Some(MediaKind::Image), false).await?;
    let data_dir = state.config.data_dir.clone();
    let path = data_dir.to_string_lossy().into_owned();
    let disk = tokio::task::spawn_blocking(move || disk_usage(&data_dir))
        .await
        .ok()
        .flatten()
        .map(|(used, total)| json!({ "used": used, "total": total, "path": path }));
    let last_scan = state.last_scan.read().await.clone();
    let encoder = state.encoder.read().await.name();
    Ok(Json(json!({
        "media": { "videos": videos, "images": images },
        "disk": disk,
        "last_scan": last_scan,
        "ffmpeg": { "available": state.ffmpeg.available(), "version": state.ffmpeg.version },
        "encoder": encoder,
        "workers": { "light": LIGHT_WORKERS, "heavy": state.settings.get().transcode_workers },
    })))
}

pub async fn events(
    AdminSession(_): AdminSession,
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let stream = BroadcastStream::new(state.events.subscribe()).filter_map(|r| async move {
        match r {
            Ok(ev) => Some(Ok(SseEvent::default().event(ev.name()).data(ev.json()))),
            Err(e) => {
                tracing::debug!("event stream receiver lagged: {e}");
                None
            }
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

/// `POST /admin/api/scan`: run a library scan now (or join the one in flight).
pub async fn scan(
    AdminSession(session): AdminSession,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<crate::media::scan::ScanReport>, ApiError> {
    require_csrf(&session, &headers, None)?;
    let report = crate::media::scan::request_scan(&state).await?;
    Ok(Json(report))
}
