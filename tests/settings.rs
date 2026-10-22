use std::collections::HashMap;
use std::net::IpAddr;

use axum::http::{HeaderMap, HeaderValue};
use fthr_clips_server::config::{Config, TranscodeMode};
use fthr_clips_server::db::media::Sort;
use fthr_clips_server::db::Db;
use fthr_clips_server::settings::{public_base, Settings, SettingsStore};
use serde_json::json;

fn config(env: &[(&str, &str)]) -> Config {
    let vars: HashMap<String, String> = env
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    Config::from_env_map(&vars, None).unwrap()
}

async fn open(dir: &tempfile::TempDir) -> Db {
    Db::open(&dir.path().join("library.db")).await.unwrap()
}

fn headers(host: &str, proto: Option<&str>) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert("host", HeaderValue::from_str(host).unwrap());
    if let Some(p) = proto {
        h.insert("x-forwarded-proto", HeaderValue::from_str(p).unwrap());
    }
    h
}

#[tokio::test]
async fn settings_defaults_come_from_config() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = open(&dir).await;
    let cfg = config(&[
        ("SITE_TITLE", "My Clips"),
        ("SCAN_INTERVAL_MINUTES", "7"),
        ("WEBHOOK_URL", "http://example.com/hook"),
    ]);
    let store = SettingsStore::load(&db, &cfg).await.unwrap();
    let s = store.get();
    assert_eq!(s.site_title, "My Clips");
    assert_eq!(s.scan_interval_minutes, 7);
    assert_eq!(s.default_sort, Sort::Newest);
    assert!(s.upload_default_public);
    assert!(!s.library_requires_login);
    assert_eq!(s.upload_folder, "uploads");
    assert_eq!(s.ladder, vec![720]);
    assert!(s.webhooks_enabled);
    assert!(s.video_exts.contains(&"mp4".to_string()));
    assert!(s.image_exts.contains(&"png".to_string()));
    let off = SettingsStore::load(&db, &config(&[])).await.unwrap();
    assert!(!off.get().webhooks_enabled);
    assert_eq!(store.instance_secret().len(), 32);
}

#[tokio::test]
async fn settings_update_persists_and_overrides() {
    let dir = tempfile::TempDir::new().unwrap();
    let cfg = config(&[("SITE_TITLE", "Before")]);
    let secret;
    {
        let db = open(&dir).await;
        let store = SettingsStore::load(&db, &cfg).await.unwrap();
        secret = store.instance_secret();
        let new = store
            .update(
                &db,
                json!({"site_title": "After", "default_sort": "most_viewed", "transcoding": "off"}),
            )
            .await
            .unwrap();
        assert_eq!(new.site_title, "After");
        assert_eq!(store.get().default_sort, Sort::MostViewed);

        let err = store.update(&db, json!({"bogus": 1})).await.unwrap_err();
        assert!(err.to_string().contains("bogus"));
        let err = store
            .update(&db, json!({"scan_interval_minutes": "x"}))
            .await
            .unwrap_err();
        assert!(!err.to_string().is_empty());
        assert_eq!(store.get().site_title, "After");
    }
    let db = open(&dir).await;
    let store = SettingsStore::load(&db, &cfg).await.unwrap();
    let s = store.get();
    assert_eq!(s.site_title, "After");
    assert_eq!(s.default_sort, Sort::MostViewed);
    assert_eq!(s.transcoding, TranscodeMode::Off);
    assert_eq!(store.instance_secret(), secret);
}

fn settings_with_base(base: Option<&str>) -> Settings {
    let dir_cfg = config(&[]);
    let mut s = Settings::from_config(&dir_cfg);
    s.public_base_url = base.map(String::from);
    s
}

#[test]
fn public_base_prefers_config() {
    let s = settings_with_base(Some("https://clips.example.com/"));
    let h = headers("other.local", Some("https"));
    let peer: IpAddr = "10.0.0.1".parse().unwrap();
    assert_eq!(public_base(&s, &h, peer, &[]), "https://clips.example.com");
    let s = settings_with_base(None);
    assert_eq!(
        public_base(&s, &headers("a.b:8080", None), peer, &[]),
        "http://a.b:8080"
    );
}

#[test]
fn public_base_ignores_forwarded_proto_from_untrusted_peer() {
    let s = settings_with_base(None);
    let h = headers("clips.example.com", Some("https"));
    let trusted = ["10.0.0.0/8".parse().unwrap()];
    let untrusted: IpAddr = "203.0.113.5".parse().unwrap();
    let proxy: IpAddr = "10.1.2.3".parse().unwrap();
    assert_eq!(
        public_base(&s, &h, untrusted, &trusted),
        "http://clips.example.com"
    );
    assert_eq!(
        public_base(&s, &h, proxy, &trusted),
        "https://clips.example.com"
    );
}

#[tokio::test]
async fn concurrent_updates_to_different_keys_both_survive() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = open(&dir).await;
    let store = std::sync::Arc::new(SettingsStore::load(&db, &config(&[])).await.unwrap());
    let mut tasks = Vec::new();
    for i in 0..20 {
        let (store, db) = (store.clone(), db.clone());
        tasks.push(tokio::spawn(async move {
            let patch = if i % 2 == 0 {
                json!({"site_title": format!("t{i}")})
            } else {
                json!({"upload_folder": format!("f{i}")})
            };
            store.update(&db, patch).await.unwrap();
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }
    let s = store.get();
    assert!(s.site_title.starts_with('t'));
    assert!(s.upload_folder.starts_with('f'));
}

#[tokio::test]
async fn update_equal_to_env_value_still_persists() {
    let dir = tempfile::TempDir::new().unwrap();
    {
        let db = open(&dir).await;
        let store = SettingsStore::load(&db, &config(&[("SITE_TITLE", "X")]))
            .await
            .unwrap();
        store.update(&db, json!({"site_title": "X"})).await.unwrap();
    }
    let db = open(&dir).await;
    let store = SettingsStore::load(&db, &config(&[("SITE_TITLE", "Y")]))
        .await
        .unwrap();
    assert_eq!(store.get().site_title, "X");
}

#[tokio::test]
async fn invalid_stored_row_falls_back_to_default() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = open(&dir).await;
    fthr_clips_server::db::settings::set_json(&db, "scan_interval_minutes", &json!("nope"))
        .await
        .unwrap();
    fthr_clips_server::db::settings::set_json(&db, "site_title", &json!("Kept"))
        .await
        .unwrap();
    let cfg = config(&[("SCAN_INTERVAL_MINUTES", "9")]);
    let s = SettingsStore::load(&db, &cfg).await.unwrap().get();
    assert_eq!(s.scan_interval_minutes, 9);
    assert_eq!(s.site_title, "Kept");
}
