mod common;
use common::{tiny_mp4, TestServer};
use fthr_clips_server::db::media;

const OLD: &str = "abcdefghijklmnopqrstuvwx";

fn seed(dir: &std::path::Path, id: &str, name: &str, bytes: &[u8]) {
    std::fs::write(dir.join(id), bytes).unwrap();
    std::fs::write(
        dir.join(format!("{id}.json")),
        format!(r#"{{"name":"{name}","content_type":"video/mp4","listed":true}}"#),
    )
    .unwrap();
}

fn seed_raw(dir: &std::path::Path, id: &str, json: &str, bytes: &[u8]) {
    std::fs::write(dir.join(id), bytes).unwrap();
    std::fs::write(dir.join(format!("{id}.json")), json).unwrap();
}

async fn server_with(id: &str, name: &str, bytes: &[u8]) -> TestServer {
    let dir = tempfile::TempDir::new().unwrap();
    seed(dir.path(), id, name, bytes);
    TestServer::spawn_with_dir(dir, &[]).await
}

#[tokio::test]
async fn import_moves_files_and_redirects() {
    let dir = tempfile::TempDir::new().unwrap();
    seed(dir.path(), OLD, "Ace.mp4", &tiny_mp4());
    let mtime: chrono::DateTime<chrono::Utc> = std::fs::metadata(dir.path().join(OLD))
        .unwrap()
        .modified()
        .unwrap()
        .into();
    let s = TestServer::spawn_with_dir(dir, &[]).await;
    let r = s
        .client
        .get(s.url(&format!("/files/{OLD}")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 301);
    assert_eq!(r.headers()["cache-control"], "public, max-age=86400");
    let loc = r.headers()["location"].to_str().unwrap().to_string();
    assert!(loc.starts_with("/w/"), "{loc}");
    assert!(s.data_dir.path().join("uploads/Ace.mp4").exists());
    assert!(!s.data_dir.path().join(OLD).exists());
    assert!(!s.data_dir.path().join(format!("{OLD}.json")).exists());
    let slug = loc.trim_start_matches("/w/");
    let m = media::get_by_slug(s.db(), slug).await.unwrap().unwrap();
    assert!(m.public);
    assert_eq!(m.created_at.timestamp(), mtime.timestamp());
    assert_eq!(m.title, "Ace");
    let page = s.client.get(s.url(&loc)).send().await.unwrap();
    assert_eq!(page.status(), 200);
}

#[tokio::test]
async fn import_is_idempotent() {
    let s = server_with(OLD, "Ace.mp4", &tiny_mp4()).await;
    let before = s
        .client
        .get(s.url(&format!("/files/{OLD}")))
        .send()
        .await
        .unwrap();
    let loc = before.headers()["location"].to_str().unwrap().to_string();
    // The same pair shows up again (e.g. restored volume): nothing new is imported.
    seed(s.data_dir.path(), OLD, "Ace.mp4", &tiny_mp4());
    let report = fthr_clips_server::legacy::import(&s.state).await.unwrap();
    assert_eq!(report.imported, 0);
    let rows = media::list(
        s.db(),
        &media::ListQuery {
            kind: None,
            public_only: false,
            q: None,
            tag: None,
            sort: media::Sort::Newest,
            page: 1,
            per_page: 50,
        },
    )
    .await
    .unwrap();
    assert_eq!(rows.len(), 1);
    let after = s
        .client
        .get(s.url(&format!("/files/{OLD}")))
        .send()
        .await
        .unwrap();
    assert_eq!(after.headers()["location"].to_str().unwrap(), loc);
}

#[tokio::test]
async fn import_skips_unknown_type_and_leaves_files() {
    let s = server_with(OLD, "blob.bin", b"whatever bytes here").await;
    assert!(s.data_dir.path().join(OLD).exists());
    assert!(s.data_dir.path().join(format!("{OLD}.json")).exists());
    let r = s
        .client
        .get(s.url(&format!("/files/{OLD}")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    // Handled-but-unimportable: not reprocessed, not re-warned.
    assert!(media::legacy_exists(s.db(), OLD).await.unwrap());
    let again = fthr_clips_server::legacy::import(&s.state).await.unwrap();
    assert_eq!((again.imported, again.skipped), (0, 0));
    assert!(s.data_dir.path().join(OLD).exists());
}

#[tokio::test]
async fn files_route_rejects_bad_ids() {
    let s = TestServer::spawn(&[]).await;
    for p in [
        "/files/..%2fx",
        "/files/short",
        "/files/abcdefghijklmnopqrstuvwx",
    ] {
        let r = s.client.get(s.url(p)).send().await.unwrap();
        assert_eq!(r.status(), 404, "{p}");
    }
}

#[tokio::test]
async fn import_drops_duplicate_content_under_new_old_id() {
    let other = "ABCDEFGHIJKLMNOPQRSTUVWX";
    let dir = tempfile::TempDir::new().unwrap();
    seed(dir.path(), OLD, "Ace.mp4", &tiny_mp4());
    seed(dir.path(), other, "Copy.mp4", &tiny_mp4());
    let s = TestServer::spawn_with_dir(dir, &[]).await;
    let mut locs = Vec::new();
    for id in [OLD, other] {
        let r = s
            .client
            .get(s.url(&format!("/files/{id}")))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 301);
        locs.push(r.headers()["location"].to_str().unwrap().to_string());
        assert!(!s.data_dir.path().join(id).exists());
        assert!(!s.data_dir.path().join(format!("{id}.json")).exists());
    }
    assert_eq!(locs[0], locs[1]);
    let n = std::fs::read_dir(s.data_dir.path().join("uploads"))
        .unwrap()
        .count();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn sidecar_defaults_and_missing_name() {
    let dir = tempfile::TempDir::new().unwrap();
    seed_raw(
        dir.path(),
        OLD,
        r#"{"content_type":"video/mp4"}"#,
        &tiny_mp4(),
    );
    let s = TestServer::spawn_with_dir(dir, &[]).await;
    let r = s
        .client
        .get(s.url(&format!("/files/{OLD}")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 301);
    let loc = r.headers()["location"].to_str().unwrap();
    let m = media::get_by_slug(s.db(), loc.trim_start_matches("/w/"))
        .await
        .unwrap()
        .unwrap();
    assert!(m.public, "listed defaults to true");
    assert_eq!(m.title, OLD);
}

#[tokio::test]
async fn orphan_from_crashed_run_is_adopted() {
    let dir = tempfile::TempDir::new().unwrap();
    seed(dir.path(), OLD, "Ace.mp4", &tiny_mp4());
    std::fs::create_dir(dir.path().join("uploads")).unwrap();
    std::fs::write(dir.path().join("uploads/Ace.mp4"), tiny_mp4()).unwrap();
    std::fs::write(dir.path().join("uploads/.incoming-legacy-dead"), b"x").unwrap();
    let s = TestServer::spawn_with_dir(dir, &[]).await;
    let mut names: Vec<_> = std::fs::read_dir(s.data_dir.path().join("uploads"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, vec!["Ace.mp4".to_string()]);
    let r = s
        .client
        .get(s.url(&format!("/files/{OLD}")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 301);
}
