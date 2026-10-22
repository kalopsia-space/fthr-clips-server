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
    db: Db,
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
        let state = AppState::new(config, db.clone(), settings);
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
            db,
        }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    pub async fn upload(
        &self,
        name: &str,
        bytes: Vec<u8>,
        token: Option<&str>,
    ) -> reqwest::Response {
        let part = reqwest::multipart::Part::bytes(bytes).file_name(name.to_string());
        let form = reqwest::multipart::Form::new().part("clip", part);
        let mut req = self.client.post(self.url("/upload")).multipart(form);
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        req.send().await.unwrap()
    }
}

pub fn tiny_mp4() -> Vec<u8> {
    let mut v = vec![0u8; 1024];
    let head = b"\0\0\0\x18ftypisom\0\0\0\0isomiso2mp41";
    v[..head.len()].copy_from_slice(head);
    v
}

pub fn tiny_png() -> Vec<u8> {
    let mut v = vec![0u8; 1024];
    let head = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
    v[..head.len()].copy_from_slice(head);
    v
}
