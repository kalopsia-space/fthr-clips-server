use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use axum::{routing::get, Router};
use tokio_util::sync::CancellationToken;

use crate::config::Config;
use crate::db::Db;
use crate::ffmpeg::{encoders, encoders::Encoder, Ffmpeg};
use crate::jobs::{new_event_bus, EventBus, ScanSummary};
use crate::settings::SettingsStore;
use crate::web;
use crate::web::security::{security_headers, RateLimiter};

/// Whether encoder detection has run: `Pending` means not finished yet,
/// `Unavailable` means it finished and nothing works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderState {
    Pending,
    Detected(Encoder),
    Unavailable,
}

impl EncoderState {
    pub fn name(&self) -> Option<&'static str> {
        match self {
            EncoderState::Detected(e) => Some(e.name()),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: Db,
    pub settings: Arc<SettingsStore>,
    pub login_limiter: Arc<RateLimiter>,
    pub upload_limiter: Arc<RateLimiter>,
    pub ffmpeg: Arc<Ffmpeg>,
    /// Result of background encoder detection (see [`EncoderState`]).
    pub encoder: Arc<tokio::sync::RwLock<EncoderState>>,
    pub events: EventBus,
    /// Cancellation tokens of running jobs, keyed by job id.
    pub running: Arc<Mutex<HashMap<i64, CancellationToken>>>,
    /// Written by the library scanner; `None` until the first scan.
    pub last_scan: Arc<tokio::sync::RwLock<Option<ScanSummary>>>,
    /// Single-flight control for the library scanner.
    pub scan: Arc<crate::media::scan::ScanControl>,
}

impl AppState {
    /// Detect the encoder in the background; the state is `Pending` meanwhile.
    pub fn spawn_encoder_detection(&self, choice: crate::config::EncoderChoice) {
        let (ffmpeg, slot) = (self.ffmpeg.clone(), self.encoder.clone());
        tokio::spawn(async move {
            *slot.write().await = EncoderState::Pending;
            let found = encoders::detect(&ffmpeg, choice).await;
            match found {
                Some(e) => tracing::info!("video encoder: {}", e.name()),
                None => tracing::info!("no working video encoder found"),
            }
            *slot.write().await = found.map_or(EncoderState::Unavailable, EncoderState::Detected);
        });
    }

    pub fn new(config: Config, db: Db, settings: Arc<SettingsStore>) -> Self {
        let ffmpeg = Arc::new(Ffmpeg::locate(&config));
        Self {
            config: Arc::new(config),
            db,
            settings,
            login_limiter: Arc::new(RateLimiter::new(10, Duration::from_secs(60))),
            upload_limiter: Arc::new(RateLimiter::new(30, Duration::from_secs(60))),
            ffmpeg,
            encoder: Arc::new(tokio::sync::RwLock::new(EncoderState::Pending)),
            events: new_event_bus(),
            running: Arc::new(Mutex::new(HashMap::new())),
            last_scan: Arc::new(tokio::sync::RwLock::new(None)),
            scan: Arc::new(crate::media::scan::ScanControl::default()),
        }
    }
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(web::healthz))
        .route(
            "/media/{slug}/{file}",
            get(web::media_files::get_media_file),
        )
        .route("/", get(web::library::videos))
        .route("/images", get(web::library::images))
        .route("/api/library", get(web::library::api_library))
        .route("/feed.rss", get(web::feed::feed))
        .route("/w/{slug}", get(web::clip::video_page))
        .route("/i/{slug}", get(web::clip::image_page))
        .route("/static/{*path}", get(web::assets::serve_static))
        .merge(web::admin::routes())
        .merge(web::upload_routes(state.config.max_upload_bytes))
        // Later tasks must add routes and fallbacks ABOVE this line: `layer`
        // only wraps what is registered before it.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            security_headers,
        ))
        .with_state(state)
}

pub async fn serve(config: Config) -> anyhow::Result<()> {
    let addr = format!("{}:{}", config.bind_host, config.port);
    let db = Db::open(&config.data_dir.join("library.db")).await?;
    let settings = Arc::new(SettingsStore::load(&db, &config).await?);
    let state = AppState::new(config, db, settings);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    tracing::info!("listening on {addr}");
    match &state.ffmpeg.version {
        Some(v) => tracing::info!("ffmpeg {v}"),
        None => tracing::warn!("ffmpeg not found: previews and transcoding are disabled"),
    }
    state.spawn_encoder_detection(state.settings.get().encoder);
    crate::jobs::spawn_pools(&state).await;
    crate::media::scan::spawn_background(&state);
    axum::serve(
        listener,
        build_router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .context("server error")
}
