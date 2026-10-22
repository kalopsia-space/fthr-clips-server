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

async fn server_with(id: &str, name: &str, bytes: &[u8]) -> TestServer {
    let dir = tempfile::TempDir::new().unwrap();
    seed(dir.path(), id, name, bytes);
    TestServer::spawn_with_dir(dir, &[]).await
}

#[tokio::test]
async fn import_moves_files_and_redirects() {
    let s = server_with(OLD, "Ace.mp4", &tiny_mp4()).await;
    let r = s
        .client
        .get(s.url(&format!("/files/{OLD}")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 301);
    let loc = r.headers()["location"].to_str().unwrap().to_string();
    assert!(loc.starts_with("/w/"), "{loc}");
    assert!(s.data_dir.path().join("uploads/Ace.mp4").exists());
    assert!(!s.data_dir.path().join(OLD).exists());
    assert!(!s.data_dir.path().join(format!("{OLD}.json")).exists());
    let slug = loc.trim_start_matches("/w/");
    let m = media::get_by_slug(s.db(), slug).await.unwrap().unwrap();
    assert!(m.public);
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
