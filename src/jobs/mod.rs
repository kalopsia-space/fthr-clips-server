//! Background job pipeline: queue rows live in the DB (`db::jobs`), workers
//! claim them, and progress is broadcast on the [`EventBus`].
//!
//! Ordering: only `probe` is enqueued for a new video. The probe job enqueues
//! `poster` and `preview` once the duration is known (Task 17 adds transcode
//! decisions at the same point). Images get a single `thumb` job.
//!
//! No ffmpeg: the server still starts. The light pool then claims only `thumb`
//! (pure Rust) and the heavy pool claims nothing, so ffmpeg-dependent jobs stay
//! `queued`. Availability is decided once, when the pools are spawned.

pub mod poster;
pub mod thumb;
pub mod transcode;
pub mod worker;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::app::AppState;
use crate::db::{self, Media};
use crate::media::kind::MediaKind;

pub const PROBE: &str = "probe";
pub const POSTER: &str = "poster";
pub const PREVIEW: &str = "preview";
pub const THUMB: &str = "thumb";
pub const TRANSCODE_LADDER: [u32; 3] = [480, 720, 1080];
pub const LIGHT_WORKERS: usize = 2;
pub const MAX_ATTEMPTS: i64 = 3;

pub fn transcode_kind(height: u32) -> String {
    format!("transcode:{height}")
}

#[derive(Debug, Clone, Serialize)]
pub struct JobEvent {
    pub job_id: i64,
    pub media_id: String,
    pub kind: String,
    pub state: String,
    pub progress: f64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanSummary {
    pub at: DateTime<Utc>,
    pub found: u32,
}

#[derive(Debug, Clone)]
pub enum Event {
    Job(JobEvent),
    Scan(ScanSummary),
}

impl Event {
    pub fn name(&self) -> &'static str {
        match self {
            Event::Job(_) => "job",
            Event::Scan(_) => "scan",
        }
    }

    pub fn json(&self) -> String {
        match self {
            Event::Job(e) => serde_json::to_string(e),
            Event::Scan(s) => serde_json::to_string(s),
        }
        .unwrap_or_else(|_| "{}".into())
    }
}

pub type EventBus = broadcast::Sender<Event>;

pub fn new_event_bus() -> EventBus {
    broadcast::channel(256).0
}

/// Queue the first job(s) for a freshly inserted media row. Errors are logged:
/// a failed enqueue must not fail the upload (the scanner can re-enqueue).
pub async fn enqueue_for_new_media(state: &AppState, media: &Media) {
    let kind = match media.kind {
        MediaKind::Video => PROBE,
        MediaKind::Image => THUMB,
    };
    if let Err(e) = db::jobs::enqueue(&state.db, &media.id, kind).await {
        tracing::warn!(media = %media.id, "enqueue {kind}: {e:#}");
    }
}

/// Start the light and heavy worker pools. Jobs left `running` by a previous
/// process are put back in the queue first.
pub async fn spawn_pools(state: &AppState) {
    match db::jobs::reset_stale_running(&state.db).await {
        Ok(0) => {}
        Ok(n) => tracing::info!("requeued {n} interrupted job(s)"),
        Err(e) => tracing::warn!("resetting interrupted jobs: {e:#}"),
    }
    let ffmpeg = state.ffmpeg.available();
    let light: Vec<String> = if ffmpeg {
        [PROBE, POSTER, PREVIEW, THUMB]
            .map(String::from)
            .into_iter()
            .collect()
    } else {
        vec![THUMB.to_string()]
    };
    worker::spawn(state.clone(), "light", light, LIGHT_WORKERS);
    let heavy: Vec<String> = if ffmpeg {
        TRANSCODE_LADDER
            .iter()
            .map(|h| transcode_kind(*h))
            .collect()
    } else {
        Vec::new()
    };
    worker::spawn(
        state.clone(),
        "heavy",
        heavy,
        state.settings.get().transcode_workers,
    );
}
