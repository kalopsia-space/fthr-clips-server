//! Admin-editable settings: Config-derived defaults overridden by `setting` rows.

use std::net::IpAddr;
use std::sync::{Arc, RwLock};

use anyhow::{anyhow, bail, Context};
use axum::http::HeaderMap;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::config::{Config, EncoderChoice, TranscodeMode};
use crate::db::media::Sort;
use crate::db::{settings as store, Db};
use crate::media::kind::{IMAGE_EXTS, VIDEO_EXTS};

const SECRET_KEY: &str = "instance_secret";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub site_title: String,
    pub public_base_url: Option<String>,
    pub library_requires_login: bool,
    pub default_sort: Sort,
    pub upload_default_public: bool,
    pub upload_folder: String,
    pub scan_interval_minutes: u64,
    pub video_exts: Vec<String>,
    pub image_exts: Vec<String>,
    pub transcoding: TranscodeMode,
    pub encoder: EncoderChoice,
    pub ladder: Vec<u32>,
    pub transcode_workers: usize,
    pub transcode_timeout_minutes: u64,
    pub discord_webhook_url: Option<String>,
    pub webhook_url: Option<String>,
    pub webhooks_enabled: bool,
}

impl Settings {
    pub fn from_config(c: &Config) -> Settings {
        Settings {
            site_title: c.site_title.clone(),
            public_base_url: c.public_base_url.clone(),
            library_requires_login: false,
            default_sort: Sort::Newest,
            upload_default_public: true,
            upload_folder: "uploads".into(),
            scan_interval_minutes: c.scan_interval_minutes,
            video_exts: VIDEO_EXTS.iter().map(|s| s.to_string()).collect(),
            image_exts: IMAGE_EXTS.iter().map(|s| s.to_string()).collect(),
            transcoding: c.transcoding,
            encoder: c.encoder,
            ladder: vec![720],
            transcode_workers: c.transcode_workers,
            transcode_timeout_minutes: c.transcode_timeout_minutes,
            discord_webhook_url: c.discord_webhook_url.clone(),
            webhook_url: c.webhook_url.clone(),
            webhooks_enabled: c.discord_webhook_url.is_some() || c.webhook_url.is_some(),
        }
    }

    fn to_map(&self) -> Map<String, Value> {
        match serde_json::to_value(self) {
            Ok(Value::Object(m)) => m,
            _ => Map::new(),
        }
    }
}

pub struct SettingsStore {
    current: RwLock<Arc<Settings>>,
    secret: [u8; 32],
    update_lock: tokio::sync::Mutex<()>,
}

impl SettingsStore {
    pub async fn load(db: &Db, config: &Config) -> anyhow::Result<SettingsStore> {
        let mut merged = Settings::from_config(config).to_map();
        let rows = store::all(db).await?;
        for (k, v) in &rows {
            let Some(old) = merged.get(k).cloned() else {
                continue;
            };
            merged.insert(k.clone(), v.clone());
            if serde_json::from_value::<Settings>(Value::Object(merged.clone())).is_err() {
                tracing::warn!(key = %k, "ignoring invalid stored setting");
                merged.insert(k.clone(), old);
            }
        }
        let settings: Settings =
            serde_json::from_value(Value::Object(merged)).context("invalid default settings")?;

        let secret = match rows.get(SECRET_KEY).and_then(Value::as_str) {
            Some(h) => hex::decode(h)
                .ok()
                .and_then(|b| <[u8; 32]>::try_from(b).ok())
                .ok_or_else(|| anyhow!("stored instance_secret is malformed"))?,
            None => {
                let mut s = [0u8; 32];
                rand::rng().fill_bytes(&mut s);
                store::set_json(db, SECRET_KEY, &Value::String(hex::encode(s))).await?;
                s
            }
        };
        Ok(SettingsStore {
            current: RwLock::new(Arc::new(settings)),
            secret,
            update_lock: tokio::sync::Mutex::new(()),
        })
    }

    pub fn get(&self) -> Arc<Settings> {
        self.current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn instance_secret(&self) -> [u8; 32] {
        self.secret
    }

    pub async fn update(&self, db: &Db, patch: Value) -> anyhow::Result<Arc<Settings>> {
        let Value::Object(patch) = patch else {
            bail!("settings patch must be a JSON object");
        };
        let _guard = self.update_lock.lock().await;
        let mut merged = self.get().to_map();
        let unknown: Vec<&str> = patch
            .keys()
            .filter(|k| !merged.contains_key(*k))
            .map(String::as_str)
            .collect();
        if !unknown.is_empty() {
            bail!("unknown settings: {}", unknown.join(", "));
        }
        let mut changed = Vec::new();
        for (k, v) in patch {
            merged.insert(k.clone(), v);
            changed.push(k);
        }
        let new: Settings =
            serde_json::from_value(Value::Object(merged.clone())).context("invalid settings")?;
        for k in &changed {
            store::set_json(db, k, &merged[k]).await?;
        }
        let new = Arc::new(new);
        *self.current.write().unwrap_or_else(|e| e.into_inner()) = new.clone();
        Ok(new)
    }
}

pub fn public_base(
    settings: &Settings,
    headers: &HeaderMap,
    peer: IpAddr,
    trusted: &[ipnet::IpNet],
) -> String {
    if let Some(base) = settings
        .public_base_url
        .as_deref()
        .map(|b| b.trim_end_matches('/'))
        .filter(|b| !b.is_empty())
    {
        return base.to_string();
    }
    let scheme = if crate::web::security::is_trusted_peer(peer, trusted) {
        headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(|v| v.trim().to_ascii_lowercase())
            .filter(|v| v == "https" || v == "http")
            .unwrap_or_else(|| "http".into())
    } else {
        "http".into()
    };
    let host = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .filter(|h| !h.is_empty())
        .unwrap_or("localhost");
    format!("{scheme}://{host}")
}
