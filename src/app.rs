use std::sync::Arc;

use anyhow::Context;
use axum::{routing::get, Router};

use crate::config::Config;
use crate::db::Db;
use crate::settings::SettingsStore;
use crate::web;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: Db,
    pub settings: Arc<SettingsStore>,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(web::healthz))
        .with_state(state)
}

pub async fn serve(config: Config) -> anyhow::Result<()> {
    let addr = format!("{}:{}", config.bind_host, config.port);
    let db = Db::open(&config.data_dir.join("library.db")).await?;
    let settings = Arc::new(SettingsStore::load(&db, &config).await?);
    let state = AppState {
        config: Arc::new(config),
        db,
        settings,
    };
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    tracing::info!("listening on {addr}");
    axum::serve(listener, build_router(state))
        .await
        .context("server error")
}
