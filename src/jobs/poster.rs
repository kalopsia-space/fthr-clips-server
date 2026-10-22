//! ffmpeg-backed jobs: `probe`, `poster`, `preview`. Every output is written to
//! a `*.tmp.*` file and renamed into place.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context};
use tokio_util::sync::CancellationToken;

use crate::app::AppState;
use crate::db::jobs::{self, Job};
use crate::db::media::{self, Media, ProbeFields};
use crate::media::paths::DerivedPaths;

async fn load(state: &AppState, job: &Job) -> anyhow::Result<Media> {
    media::get(&state.db, &job.media_id)
        .await?
        .ok_or_else(|| anyhow!("media {} no longer exists", job.media_id))
}

fn args(parts: &[&dyn AsRef<std::ffi::OsStr>]) -> Vec<OsString> {
    parts.iter().map(|p| p.as_ref().to_os_string()).collect()
}

fn secs(s: f64) -> String {
    format!("{s:.3}")
}

pub async fn probe(state: &AppState, job: &Job) -> anyhow::Result<()> {
    let m = load(state, job).await?;
    let info = state.ffmpeg.probe(Path::new(&m.path)).await?;
    let creation = info.creation_time;
    media::set_probe(&state.db, &m.id, &ProbeFields::from(info)).await?;
    if m.recorded_at.is_none() {
        if let Some(at) = creation {
            media::set_recorded_at(&state.db, &m.id, at).await?;
        }
    }
    // Poster and preview need the duration probed above.
    jobs::enqueue(&state.db, &m.id, super::POSTER).await?;
    jobs::enqueue(&state.db, &m.id, super::PREVIEW).await?;
    Ok(())
}

async fn prepare(state: &AppState, m: &Media) -> anyhow::Result<DerivedPaths> {
    let derived = DerivedPaths::new(&state.config.data_dir, &m.id);
    tokio::fs::create_dir_all(derived.dir())
        .await
        .context("creating derived dir")?;
    Ok(derived)
}

async fn commit(tmp: &Path, dest: &Path) -> anyhow::Result<()> {
    tokio::fs::rename(tmp, dest)
        .await
        .with_context(|| format!("renaming into {}", dest.display()))
}

pub async fn poster(state: &AppState, job: &Job, cancel: CancellationToken) -> anyhow::Result<()> {
    let m = load(state, job).await?;
    let derived = prepare(state, &m).await?;
    let tmp: PathBuf = derived.dir().join("poster.tmp.jpg");
    let seek = m.duration_ms.map_or(0.0, |d| d as f64 / 1000.0 * 0.1);
    let run = |ss: f64| {
        let a = args(&[
            &"-ss",
            &secs(ss),
            &"-i",
            &m.path,
            &"-frames:v",
            &"1",
            &"-vf",
            &"scale=-2:720",
            &tmp,
        ]);
        let (ffmpeg, cancel) = (state.ffmpeg.clone(), cancel.clone());
        async move {
            ffmpeg
                .run(&a, None, Duration::from_secs(60), Some(cancel), |_| {})
                .await
        }
    };
    let mut result = run(seek).await;
    if result.is_err() && seek > 0.0 && !cancel.is_cancelled() {
        result = run(0.0).await;
    }
    if let Err(e) = result {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e);
    }
    commit(&tmp, &derived.poster()).await?;
    media::set_flags(&state.db, &m.id, Some(true), None, None).await?;
    media::set_processing(&state.db, &m.id, "ready", None).await?;
    Ok(())
}

pub async fn preview(state: &AppState, job: &Job, cancel: CancellationToken) -> anyhow::Result<()> {
    if !state.ffmpeg.has_x264().await {
        tracing::info!(job = job.id, "libx264 unavailable: skipping preview");
        return Ok(());
    }
    let m = load(state, job).await?;
    let derived = prepare(state, &m).await?;
    let tmp: PathBuf = derived.dir().join("preview.tmp.mp4");
    let start = m
        .duration_ms
        .map_or(0.0, |d| (d as f64 / 1000.0 / 2.0 - 2.0).max(0.0));
    let a = args(&[
        &"-ss",
        &secs(start),
        &"-t",
        &"4",
        &"-i",
        &m.path,
        &"-vf",
        &"scale=-2:360",
        &"-an",
        &"-c:v",
        &"libx264",
        &"-preset",
        &"veryfast",
        &"-crf",
        &"30",
        &"-movflags",
        &"+faststart",
        &tmp,
    ]);
    let r = state
        .ffmpeg
        .run(&a, None, Duration::from_secs(180), Some(cancel), |_| {})
        .await;
    if let Err(e) = r {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e);
    }
    commit(&tmp, &derived.preview()).await?;
    media::set_flags(&state.db, &m.id, None, Some(true), None).await?;
    Ok(())
}
