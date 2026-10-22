mod common;

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use common::{login, tiny_mp4, TestServer};
use fthr_clips_server::db::media;
use serde_json::Value;

const PW: &str = "pw";
const H: &str = "x-csrf-token";

async fn server(dir: &Path) -> TestServer {
    let d = dir.to_string_lossy().into_owned();
    TestServer::spawn(&[("ADMIN_PASSWORD", PW), ("VIDEO_DIRS", &d)]).await
}

async fn scan(s: &TestServer, c: &reqwest::Client, csrf: &str) -> Value {
    let r = c
        .post(s.url("/admin/api/scan"))
        .header(H, csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

/// File written long enough ago to pass the "recently modified" rule.
fn write_old(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
    let old = std::time::SystemTime::now() - Duration::from_secs(3600);
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(old)
        .unwrap();
}

async fn admin_items(c: &reqwest::Client, s: &TestServer) -> Vec<Value> {
    let v: Value = c
        .get(s.url("/admin/api/media"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    v["items"].as_array().unwrap().clone()
}

#[tokio::test]
async fn scan_requires_csrf() {
    let tmp = tempfile::TempDir::new().unwrap();
    let s = server(tmp.path()).await;
    let (c, _) = login(&s, PW).await;
    let r = c.post(s.url("/admin/api/scan")).send().await.unwrap();
    assert_eq!(r.status(), 403);
}

#[tokio::test]
async fn scan_picks_up_files_in_video_dir() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_old(&tmp.path().join("2026-10-21 10-00-00.mp4"), &tiny_mp4());
    let s = server(tmp.path()).await;
    let (c, csrf) = login(&s, PW).await;
    let r = scan(&s, &c, &csrf).await;
    assert_eq!(r["found"], 1);
    let items = admin_items(&c, &s).await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["public"], false);
    assert_eq!(items[0]["source"], "scan");
    let id = items[0]["id"].as_str().unwrap();
    let m = media::get(s.db(), id).await.unwrap().unwrap();
    assert!(m.recorded_at.is_some());
    assert_eq!(m.folder, "");
    // A second pass finds nothing new.
    assert_eq!(scan(&s, &c, &csrf).await["found"], 0);
}

#[tokio::test]
async fn growing_file_is_skipped_then_found() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("growing.mp4");
    std::fs::write(&path, tiny_mp4()).unwrap();
    let s = server(tmp.path()).await;
    let (c, csrf) = login(&s, PW).await;
    let p = path.clone();
    let writer = tokio::spawn(async move {
        for _ in 0..13 {
            let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
            f.write_all(&[0u8; 1024]).unwrap();
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    let r = scan(&s, &c, &csrf).await;
    assert!(r["skipped_growing"].as_u64().unwrap() >= 1, "{r}");
    assert_eq!(r["found"], 0);
    writer.await.unwrap();
    tokio::time::sleep(Duration::from_secs(11)).await;
    let r = scan(&s, &c, &csrf).await;
    assert_eq!(r["found"], 1, "{r}");
}

#[tokio::test]
async fn moved_file_is_relinked_not_duplicated() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_old(&tmp.path().join("a.mp4"), &tiny_mp4());
    let s = server(tmp.path()).await;
    let (c, csrf) = login(&s, PW).await;
    scan(&s, &c, &csrf).await;
    let before = admin_items(&c, &s).await;
    let id = before[0]["id"].as_str().unwrap().to_string();
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    std::fs::rename(tmp.path().join("a.mp4"), tmp.path().join("sub/a.mp4")).unwrap();
    let r = scan(&s, &c, &csrf).await;
    assert_eq!(r["relinked"], 1, "{r}");
    assert_eq!(r["found"], 0);
    let items = admin_items(&c, &s).await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], id.as_str());
    let m = media::get(s.db(), &id).await.unwrap().unwrap();
    assert!(m.path.ends_with("sub/a.mp4"), "{}", m.path);
    assert!(m.available);
}

#[tokio::test]
async fn missing_file_marked_unavailable_and_hidden() {
    let tmp = tempfile::TempDir::new().unwrap();
    let f = tmp.path().join("gone.mp4");
    write_old(&f, &tiny_mp4());
    let s = server(tmp.path()).await;
    let (c, csrf) = login(&s, PW).await;
    scan(&s, &c, &csrf).await;
    let item = admin_items(&c, &s).await.remove(0);
    let id = item["id"].as_str().unwrap().to_string();
    let m = media::get(s.db(), &id).await.unwrap().unwrap();
    media::set_public(s.db(), std::slice::from_ref(&id), true)
        .await
        .unwrap();
    assert_eq!(s.client.get(s.url("/")).send().await.unwrap().status(), 200);
    let page = s.client.get(s.url(&format!("/w/{}", m.slug)));
    assert_eq!(page.send().await.unwrap().status(), 200);

    std::fs::remove_file(&f).unwrap();
    let r = scan(&s, &c, &csrf).await;
    assert_eq!(r["missing"], 1, "{r}");
    let m = media::get(s.db(), &id).await.unwrap().unwrap();
    assert!(!m.available);
    let home = s.client.get(s.url("/")).send().await.unwrap();
    assert!(!home.text().await.unwrap().contains(&m.slug));
    let page = s.client.get(s.url(&format!("/w/{}", m.slug)));
    assert_eq!(page.send().await.unwrap().status(), 404);
}

#[tokio::test]
async fn missing_root_leaves_rows_alone() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("root");
    std::fs::create_dir(&root).unwrap();
    write_old(&root.join("a.mp4"), &tiny_mp4());
    let s = server(&root).await;
    let (c, csrf) = login(&s, PW).await;
    scan(&s, &c, &csrf).await;
    std::fs::remove_dir_all(&root).unwrap();
    let r = scan(&s, &c, &csrf).await;
    assert_eq!(r["missing"], 0, "{r}");
    let id = admin_items(&c, &s).await[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(media::get(s.db(), &id).await.unwrap().unwrap().available);
}

#[tokio::test]
async fn tombstoned_file_is_ignored() {
    let tmp = tempfile::TempDir::new().unwrap();
    let f = tmp.path().join("deleted.mp4");
    write_old(&f, &tiny_mp4());
    let canon = std::fs::canonicalize(&f).unwrap();
    let s = server(tmp.path()).await;
    let (c, csrf) = login(&s, PW).await;
    media::add_tombstone(s.db(), "deadbeef", &canon.to_string_lossy())
        .await
        .unwrap();
    let r = scan(&s, &c, &csrf).await;
    assert_eq!(r["found"], 0, "{r}");
    assert!(admin_items(&c, &s).await.is_empty());
}

#[tokio::test]
async fn hidden_and_unsupported_files_are_skipped() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_old(&tmp.path().join(".hidden.mp4"), &tiny_mp4());
    write_old(&tmp.path().join("notes.txt"), b"hello");
    let s = server(tmp.path()).await;
    let (c, csrf) = login(&s, PW).await;
    assert_eq!(scan(&s, &c, &csrf).await["found"], 0);
}
