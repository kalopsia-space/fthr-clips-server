//! `transcode:{height}` jobs and the mode rules that decide which renditions
//! a freshly probed video gets. Output goes to `{h}p.tmp.mp4` and is renamed
//! into place; nothing is ever upscaled.

use std::ffi::OsString;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};
use tokio_util::sync::CancellationToken;

use super::{worker, TRANSCODE_LADDER};
use crate::app::AppState;
use crate::config::TranscodeMode;
use crate::db::jobs::{self, Job, Rendition};
use crate::db::media;
use crate::ffmpeg::encoders;
use crate::media::paths::DerivedPaths;
use crate::settings::Settings;

/// Rungs to render automatically for a probed video.
///
/// `Off`: none. `Compat`: none when the browser can play the original, else
/// the tallest rung not above the source (the smallest rung when the source is
/// shorter than all of them; `720` preference when the height is unknown).
/// `On`: every rung not above the source; when the source is shorter than every
/// rung, nothing if playable, else the smallest rung (scaled to the source).
pub fn plan_for(settings: &Settings, playable: bool, height: Option<u32>) -> Vec<u32> {
    let mut ladder: Vec<u32> = settings
        .ladder
        .iter()
        .copied()
        .filter(|h| TRANSCODE_LADDER.contains(h))
        .collect();
    ladder.sort_unstable();
    ladder.dedup();
    let Some(&smallest) = ladder.first() else {
        return Vec::new();
    };
    match settings.transcoding {
        TranscodeMode::Off => Vec::new(),
        TranscodeMode::Compat => {
            if playable {
                return Vec::new();
            }
            let limit = height.unwrap_or(720);
            let pick = ladder.iter().copied().rfind(|h| *h <= limit);
            vec![pick.unwrap_or(smallest)]
        }
        TranscodeMode::On => {
            let limit = height.unwrap_or(1080);
            let all: Vec<u32> = ladder.iter().copied().filter(|h| *h <= limit).collect();
            if all.is_empty() && !playable {
                vec![smallest]
            } else {
                all
            }
        }
    }
}

/// Queue a rendition: row `queued`, plus the job. Used by the probe job and the admin API.
pub async fn queue_rendition(state: &AppState, media_id: &str, height: u32) -> anyhow::Result<()> {
    jobs::rendition_upsert(
        &state.db,
        &Rendition {
            media_id: media_id.to_string(),
            height: i64::from(height),
            path: String::new(),
            size: 0,
            state: "queued".into(),
            progress: 0.0,
            error: None,
        },
    )
    .await?;
    jobs::enqueue(&state.db, media_id, &super::transcode_kind(height)).await?;
    Ok(())
}

async fn encoder(state: &AppState) -> anyhow::Result<encoders::Encoder> {
    if let Some(e) = *state.encoder.read().await {
        return Ok(e);
    }
    // Startup detection may not have finished (or tests never ran it).
    let found = encoders::detect(&state.ffmpeg, state.settings.get().encoder).await;
    let mut slot = state.encoder.write().await;
    if slot.is_none() {
        *slot = found;
    }
    (*slot).ok_or_else(|| anyhow!("no working video encoder found"))
}

fn height_of(kind: &str) -> anyhow::Result<u32> {
    kind.strip_prefix("transcode:")
        .and_then(|h| h.parse().ok())
        .ok_or_else(|| anyhow!("bad transcode job kind {kind}"))
}

pub async fn transcode(
    state: &AppState,
    job: &Job,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let h = height_of(&job.kind)?;
    let m = media::get(&state.db, &job.media_id)
        .await?
        .ok_or_else(|| anyhow!("media {} no longer exists", job.media_id))?;
    let derived = DerivedPaths::new(&state.config.data_dir, &m.id);
    tokio::fs::create_dir_all(derived.dir()).await?;
    let tmp = derived.dir().join(format!("{h}p.tmp.mp4"));
    let dest = derived.rendition(h);
    let hi = i64::from(h);

    jobs::rendition_upsert(
        &state.db,
        &Rendition {
            media_id: m.id.clone(),
            height: hi,
            path: String::new(),
            size: 0,
            state: "running".into(),
            progress: 0.0,
            error: None,
        },
    )
    .await?;

    let result = encode(state, job, &m, h, &tmp, cancel.clone()).await;
    match result {
        Ok(()) => {
            tokio::fs::rename(&tmp, &dest).await?;
            let size = tokio::fs::metadata(&dest).await?.len() as i64;
            jobs::rendition_ready(&state.db, &m.id, hi, &dest.to_string_lossy(), size).await?;
            // Removed while finishing: do not leave an orphan file behind.
            let still = jobs::renditions_for(&state.db, &m.id).await?;
            if !still.iter().any(|r| r.height == hi) {
                let _ = tokio::fs::remove_file(&dest).await;
            }
            Ok(())
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            if !cancel.is_cancelled() {
                let msg = format!("{e:#}");
                let tail: String = msg.chars().take(500).collect();
                if let Err(db) = jobs::rendition_fail(&state.db, &m.id, hi, &tail).await {
                    tracing::warn!("recording rendition failure: {db:#}");
                }
            }
            Err(e)
        }
    }
}

async fn encode(
    state: &AppState,
    job: &Job,
    m: &media::Media,
    h: u32,
    tmp: &Path,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let enc = encoder(state).await?;
    // Never upscale: the rung keeps its name but is scaled to the source height.
    let target = match m.height {
        Some(sh) if sh > 0 && (sh as u32) < h => ((sh as u32) & !1).max(2),
        _ => h,
    };
    let mut args: Vec<OsString> = vec!["-i".into(), (&m.path).into()];
    args.extend(enc.video_args(target, m.fps.unwrap_or(30.0)));
    args.push("-sn".into());
    args.push(tmp.into());

    let timeout = Duration::from_secs(state.settings.get().transcode_timeout_minutes * 60);
    let total = m
        .duration_ms
        .filter(|d| *d > 0)
        .map(|d| Duration::from_millis(d as u64));

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<f64>();
    let writer = {
        let (state, job, media_id) = (state.clone(), job.clone(), m.id.clone());
        tokio::spawn(async move {
            let (mut last, mut at) = (0.0_f64, Instant::now());
            while let Some(p) = rx.recv().await {
                if p - last < 0.02 && at.elapsed() < Duration::from_secs(1) {
                    continue;
                }
                (last, at) = (p, Instant::now());
                let _ = jobs::rendition_progress(&state.db, &media_id, i64::from(h), p).await;
                let _ = jobs::set_progress(&state.db, job.id, p).await;
                worker::emit(&state, &job, "running", p, None);
            }
        })
    };
    let r = state
        .ffmpeg
        .run(&args, total, timeout, Some(cancel), move |p| {
            let _ = tx.send(p);
        })
        .await;
    let _ = writer.await;
    if r.is_ok() && !tmp.exists() {
        bail!("ffmpeg produced no output");
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::collections::HashMap;

    fn settings(mode: &str, ladder: &[u32]) -> Settings {
        let vars: HashMap<String, String> = [
            ("DATA_DIR".to_string(), "/tmp/x".to_string()),
            ("TRANSCODING".to_string(), mode.to_string()),
        ]
        .into();
        let c = Config::from_env_map(&vars, None).unwrap();
        let mut s = Settings::from_config(&c);
        s.ladder = ladder.to_vec();
        s
    }

    #[test]
    fn plan_for_rules() {
        let l = [480, 720, 1080];
        let on = settings("on", &l);
        assert_eq!(plan_for(&on, true, Some(1080)), vec![480, 720, 1080]);
        assert_eq!(plan_for(&on, true, Some(720)), vec![480, 720]);
        assert_eq!(plan_for(&on, true, Some(240)), Vec::<u32>::new());
        assert_eq!(plan_for(&on, false, Some(240)), vec![480]);
        assert_eq!(plan_for(&on, true, None), vec![480, 720, 1080]);
        let compat = settings("compat", &l);
        assert_eq!(plan_for(&compat, true, Some(1080)), Vec::<u32>::new());
        assert_eq!(plan_for(&compat, false, Some(1080)), vec![1080]);
        assert_eq!(plan_for(&compat, false, Some(900)), vec![720]);
        assert_eq!(plan_for(&compat, false, Some(240)), vec![480]);
        assert_eq!(plan_for(&compat, false, None), vec![720]);
        assert_eq!(
            plan_for(&settings("off", &l), false, Some(1080)),
            Vec::<u32>::new()
        );
    }
}
