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
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            data_dir,
            db,
        }
    }

    /// A client that keeps cookies and does not follow redirects.
    pub fn client_with_cookies(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
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

/// Logs in with the given password; returns a cookie-keeping client and the CSRF token.
pub async fn login(server: &TestServer, password: &str) -> (reqwest::Client, String) {
    let c = server.client_with_cookies();
    let r = c
        .post(server.url("/admin/login"))
        .json(&serde_json::json!({ "password": password }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let csrf = r.json::<serde_json::Value>().await.unwrap()["csrf"]
        .as_str()
        .unwrap()
        .to_string();
    (c, csrf)
}

/// Generates (once) a 2 s 320x240 h264/aac mp4 in the temp dir. `None` when ffmpeg is not on PATH.
pub fn fixture_mp4() -> Option<std::path::PathBuf> {
    use std::sync::OnceLock;
    static FIXTURE: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            let dir = std::env::temp_dir().join("fthr-clips-fixtures");
            std::fs::create_dir_all(&dir).ok()?;
            let final_path = dir.join("tiny.mp4");
            if final_path.exists() {
                return Some(final_path);
            }
            // Write to a per-process name then rename, so parallel test binaries never see a partial file.
            let tmp = dir.join(format!("tiny-{}.mp4", std::process::id()));
            let ok = std::process::Command::new("ffmpeg")
                .args(["-nostdin", "-hide_banner", "-loglevel", "error"])
                .args([
                    "-f",
                    "lavfi",
                    "-i",
                    "testsrc=duration=2:size=320x240:rate=30",
                ])
                .args(["-f", "lavfi", "-i", "sine=duration=2"])
                .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"])
                .args(["-shortest", "-y"])
                .arg(&tmp)
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok {
                return None;
            }
            std::fs::rename(&tmp, &final_path).ok()?;
            Some(final_path)
        })
        .clone()
}
