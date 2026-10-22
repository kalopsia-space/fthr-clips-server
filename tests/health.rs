mod common;
#[tokio::test]
async fn healthz_returns_ok() {
    let s = common::TestServer::spawn(&[("UPLOAD_TOKEN", "t")]).await;
    let r = s.client.get(s.url("/healthz")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.json::<serde_json::Value>().await.unwrap(),
        serde_json::json!({"status":"ok"})
    );
}
#[test]
fn config_storage_dir_alias_and_defaults() {
    let mut v = std::collections::HashMap::new();
    v.insert("STORAGE_DIR".to_string(), "/tmp/x".to_string());
    let c = fthr_clips_server::config::Config::from_env_map(&v, None).unwrap();
    assert_eq!(c.data_dir, std::path::PathBuf::from("/tmp/x"));
    assert_eq!(c.bind_host, "127.0.0.1");
    assert_eq!(c.port, 8080);
    assert_eq!(c.max_upload_bytes, 524_288_000);
    assert!(matches!(
        c.transcoding,
        fthr_clips_server::config::TranscodeMode::Compat
    ));
}
#[test]
fn env_storage_dir_overrides_toml_data_dir() {
    let mut v = std::collections::HashMap::new();
    v.insert("STORAGE_DIR".to_string(), "/env/dir".to_string());
    let c = fthr_clips_server::config::Config::from_env_map(&v, Some("data_dir = \"/toml/dir\""))
        .unwrap();
    assert_eq!(c.data_dir, std::path::PathBuf::from("/env/dir"));
    let c = fthr_clips_server::config::Config::from_env_map(
        &Default::default(),
        Some("storage_dir = \"/toml/s\""),
    )
    .unwrap();
    assert_eq!(c.data_dir, std::path::PathBuf::from("/toml/s"));
}
