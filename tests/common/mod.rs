#![allow(dead_code)]
use std::collections::HashMap;
use std::sync::Arc;

use fthr_clips_server::app::{build_router, AppState};
use fthr_clips_server::config::Config;
use fthr_clips_server::db::Db;
use fthr_clips_server::settings::SettingsStore;

pub struct TestServer {
    pub base: String,
    pub client: reqwest::Client,
    pub data_dir: tempfile::TempDir,
}

impl TestServer {
    pub async fn spawn(env: &[(&str, &str)]) -> TestServer {
        let data_dir = tempfile::TempDir::new().unwrap();
        let mut vars: HashMap<String, String> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        if !vars.contains_key("DATA_DIR") && !vars.contains_key("STORAGE_DIR") {
            vars.insert(
                "DATA_DIR".into(),
                data_dir.path().to_string_lossy().into_owned(),
            );
        }
        let config = Config::from_env_map(&vars, None).unwrap();
        let db = Db::open(&config.data_dir.join("library.db")).await.unwrap();
        let settings = Arc::new(SettingsStore::load(&db, &config).await.unwrap());
        let state = AppState::new(config, db, settings);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = build_router(state);
        tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap();
        });
        TestServer {
            base,
            client: reqwest::Client::new(),
            data_dir,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }
}
