mod common;
use common::{tiny_mp4, tiny_png, TestServer};

const ENV: &[(&str, &str)] = &[("UPLOAD_TOKEN", "t")];
const NAME: &str = "Ace 2026-10-22 18-03-11.mp4";

fn upload_files(s: &TestServer) -> Vec<String> {
    match std::fs::read_dir(s.data_dir.path().join("uploads")) {
        Ok(rd) => rd
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => vec![],
    }
}

#[tokio::test]
async fn head_requires_bearer() {
    let s = TestServer::spawn(ENV).await;
    let r = s.client.head(s.url("/upload")).send().await.unwrap();
    assert_eq!(r.status(), 401);
    let r = s
        .client
        .head(s.url("/upload"))
        .bearer_auth("wrong")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    let r = s
        .client
        .head(s.url("/upload"))
        .bearer_auth("t")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn post_without_token_is_401() {
    let s = TestServer::spawn(ENV).await;
    let r = s.upload(NAME, tiny_mp4(), None).await;
    assert_eq!(r.status(), 401);
    assert!(upload_files(&s).is_empty());
}

#[tokio::test]
async fn upload_round_trip_returns_share_url() {
    let s = TestServer::spawn(ENV).await;
    let body = tiny_mp4();
    let r = s.upload(NAME, body.clone(), Some("t")).await;
    assert_eq!(r.status(), 200);
    let j: serde_json::Value = r.json().await.unwrap();
    let url = j["url"].as_str().unwrap();
    let prefix = format!("{}/w/", s.base);
    assert!(url.starts_with(&prefix), "{url}");
    let slug = &url[prefix.len()..];
    assert_eq!(slug.len(), 11);
    assert!(slug.chars().all(|c| c.is_ascii_alphanumeric()));
    assert_eq!(j["listed"], true);
    assert_eq!(j["bytes"], body.len());
    assert_eq!(j["id"].as_str().unwrap().len(), 32);
    assert_eq!(upload_files(&s), vec![NAME.to_string()]);
    let r2 = s.upload(NAME, body, Some("t")).await;
    let j2: serde_json::Value = r2.json().await.unwrap();
    assert_eq!(j2["url"], j["url"]);
}

#[tokio::test]
async fn duplicate_upload_returns_existing_url() {
    let s = TestServer::spawn(ENV).await;
    let a: serde_json::Value = s
        .upload("a.mp4", tiny_mp4(), Some("t"))
        .await
        .json()
        .await
        .unwrap();
    let b: serde_json::Value = s
        .upload("other name.mp4", tiny_mp4(), Some("t"))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(a["url"], b["url"]);
    assert_eq!(upload_files(&s), vec!["a.mp4".to_string()]);
}

#[tokio::test]
async fn oversize_upload_is_413_and_cleans_up() {
    let s = TestServer::spawn(&[("UPLOAD_TOKEN", "t"), ("MAX_UPLOAD_BYTES", "600")]).await;
    let r = s.upload(NAME, tiny_mp4(), Some("t")).await;
    assert_eq!(r.status(), 413);
    assert!(upload_files(&s).is_empty());
}

#[tokio::test]
async fn unknown_type_is_415_and_cleans_up() {
    let s = TestServer::spawn(ENV).await;
    let mut body = vec![0u8; 1024];
    body[..2].copy_from_slice(b"MZ");
    let r = s.upload("clip.exe", body, Some("t")).await;
    assert_eq!(r.status(), 415);
    assert!(upload_files(&s).is_empty());
}

#[tokio::test]
async fn no_extension_is_415() {
    let s = TestServer::spawn(ENV).await;
    let r = s.upload("clip", tiny_mp4(), Some("t")).await;
    assert_eq!(r.status(), 415);
    assert!(upload_files(&s).is_empty());
}

#[tokio::test]
async fn upload_rate_limit_is_429() {
    let s = TestServer::spawn(&[("UPLOAD_TOKEN", "t"), ("MAX_UPLOAD_BYTES", "100000")]).await;
    for i in 0..31u32 {
        let mut b = tiny_mp4();
        b[100..104].copy_from_slice(&i.to_le_bytes());
        let r = s.upload(&format!("c{i}.mp4"), b, Some("t")).await;
        if i < 30 {
            assert_eq!(r.status(), 200, "upload {i}");
        } else {
            assert_eq!(r.status(), 429);
        }
    }
}

#[tokio::test]
async fn public_base_url_env_is_used() {
    let s = TestServer::spawn(&[
        ("UPLOAD_TOKEN", "t"),
        ("PUBLIC_BASE_URL", "https://clips.example"),
    ])
    .await;
    let j: serde_json::Value = s
        .upload(NAME, tiny_mp4(), Some("t"))
        .await
        .json()
        .await
        .unwrap();
    assert!(j["url"]
        .as_str()
        .unwrap()
        .starts_with("https://clips.example/w/"));
}

#[tokio::test]
async fn image_upload_gets_i_slug() {
    let s = TestServer::spawn(ENV).await;
    let j: serde_json::Value = s
        .upload("shot.png", tiny_png(), Some("t"))
        .await
        .json()
        .await
        .unwrap();
    assert!(j["url"].as_str().unwrap().contains("/i/"));
}

fn upload_sizes(s: &TestServer) -> Vec<u64> {
    let mut v: Vec<u64> = std::fs::read_dir(s.data_dir.path().join("uploads"))
        .unwrap()
        .map(|e| e.unwrap().metadata().unwrap().len())
        .collect();
    v.sort();
    v
}

#[tokio::test]
async fn concurrent_same_name_uploads_do_not_clobber() {
    let s = TestServer::spawn(ENV).await;
    let a = tiny_mp4();
    let mut b = tiny_mp4();
    b.extend_from_slice(&[7u8; 1024]);
    let (ra, rb) = tokio::join!(
        s.upload("x.mp4", a, Some("t")),
        s.upload("x.mp4", b, Some("t"))
    );
    assert_eq!(ra.status(), 200);
    assert_eq!(rb.status(), 200);
    let ja: serde_json::Value = ra.json().await.unwrap();
    let jb: serde_json::Value = rb.json().await.unwrap();
    assert_ne!(ja["id"], jb["id"]);
    assert_eq!(ja["bytes"], 1024);
    assert_eq!(jb["bytes"], 2048);
    assert_eq!(upload_files(&s).len(), 2);
    assert_eq!(upload_sizes(&s), vec![1024, 2048]);
}

#[tokio::test]
async fn non_ascii_name_keeps_extension() {
    let s = TestServer::spawn(ENV).await;
    let r = s.upload("é.mp4", tiny_mp4(), Some("t")).await;
    assert_eq!(r.status(), 200);
    let files = upload_files(&s);
    assert_eq!(files.len(), 1);
    assert!(
        files[0].ends_with(".mp4") && files[0].len() > 4,
        "{files:?}"
    );
}

#[tokio::test]
async fn large_oversize_aborts_stream_and_cleans_up() {
    let s = TestServer::spawn(&[("UPLOAD_TOKEN", "t"), ("MAX_UPLOAD_BYTES", "20000")]).await;
    let mut body = tiny_mp4();
    body.resize(64 * 1024, 1);
    let r = s.upload(NAME, body, Some("t")).await;
    assert_eq!(r.status(), 413);
    assert!(upload_files(&s).is_empty());
}

#[tokio::test]
async fn traversal_filename_stays_in_uploads() {
    let s = TestServer::spawn(ENV).await;
    let r = s.upload("../../evil.mp4", tiny_mp4(), Some("t")).await;
    assert_eq!(r.status(), 200);
    assert_eq!(upload_files(&s), vec!["evil.mp4".to_string()]);
    assert!(!s.data_dir.path().join("evil.mp4").exists());
}

#[tokio::test]
async fn failed_auth_is_throttled_but_valid_token_still_works() {
    let s = TestServer::spawn(ENV).await;
    let mut last = 0;
    for _ in 0..21 {
        last = s
            .client
            .head(s.url("/upload"))
            .bearer_auth("bad")
            .send()
            .await
            .unwrap()
            .status()
            .as_u16();
    }
    assert_eq!(last, 429);
    let r = s
        .client
        .head(s.url("/upload"))
        .bearer_auth("t")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}
