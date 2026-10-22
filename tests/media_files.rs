mod common;
use common::{tiny_mp4, tiny_png, TestServer};
use fthr_clips_server::db::media;

const ENV: &[(&str, &str)] = &[("UPLOAD_TOKEN", "t")];

async fn up(s: &TestServer, name: &str, bytes: Vec<u8>) -> String {
    let r = s.upload(name, bytes, Some("t")).await;
    assert_eq!(r.status(), 200);
    let j: serde_json::Value = r.json().await.unwrap();
    j["url"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn original_streams_with_range() {
    let s = TestServer::spawn(ENV).await;
    let slug = up(&s, "clip.mp4", tiny_mp4()).await;
    let r = s
        .client
        .get(s.url(&format!("/media/{slug}/original.mp4")))
        .header("Range", "bytes=0-9")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 206);
    assert_eq!(r.headers()["content-type"], "video/mp4");
    assert!(!r.headers().contains_key("x-frame-options"));
    assert!(r.headers().contains_key("content-range"));
    assert_eq!(r.bytes().await.unwrap().len(), 10);
}

#[tokio::test]
async fn range_past_end_is_416() {
    let s = TestServer::spawn(ENV).await;
    let slug = up(&s, "clip.mp4", tiny_mp4()).await;
    let r = s
        .client
        .get(s.url(&format!("/media/{slug}/original.mp4")))
        .header("Range", "bytes=999999999-")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 416);
    assert!(!r.headers().contains_key("cache-control"));
}

#[tokio::test]
async fn wrong_extension_is_404() {
    let s = TestServer::spawn(ENV).await;
    let slug = up(&s, "clip.mp4", tiny_mp4()).await;
    for f in [
        "original.mkv",
        "original.MP4",
        "original",
        "other.mp4",
        "../x",
        "480p.mp4",
        "720p.mp4",
    ] {
        let r = s
            .client
            .get(s.url(&format!("/media/{slug}/{f}")))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 404, "{f}");
    }
    let r = s
        .client
        .get(s.url("/media/nosuchslug/original.mp4"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn unavailable_media_is_404() {
    let s = TestServer::spawn(ENV).await;
    let slug = up(&s, "clip.mp4", tiny_mp4()).await;
    let m = media::get_by_slug(s.db(), &slug).await.unwrap().unwrap();
    media::set_available(s.db(), &m.id, false).await.unwrap();
    let r = s
        .client
        .get(s.url(&format!("/media/{slug}/original.mp4")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn link_only_media_is_still_served() {
    let s = TestServer::spawn(ENV).await;
    let slug = up(&s, "clip.mp4", tiny_mp4()).await;
    let m = media::get_by_slug(s.db(), &slug).await.unwrap().unwrap();
    media::set_public(s.db(), std::slice::from_ref(&m.id), false)
        .await
        .unwrap();
    let r = s
        .client
        .get(s.url(&format!("/media/{slug}/original.mp4")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["cache-control"], "private, max-age=0");
    assert_eq!(r.bytes().await.unwrap().len(), tiny_mp4().len());
}

#[tokio::test]
async fn content_disposition_has_original_name() {
    let s = TestServer::spawn(ENV).await;
    let slug = up(&s, "clip.mp4", tiny_mp4()).await;
    let m = media::get_by_slug(s.db(), &slug).await.unwrap().unwrap();
    // Uploads sanitise names, so link a file whose name needs escaping.
    let odd = s.data_dir.path().join("my \"q\" \\ \u{e9}.mp4");
    std::fs::write(&odd, tiny_mp4()).unwrap();
    media::relink(s.db(), &m.id, odd.to_str().unwrap())
        .await
        .unwrap();
    let r = s
        .client
        .get(s.url(&format!("/media/{slug}/original.mp4")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let cd = r.headers()["content-disposition"]
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cd.starts_with("inline; filename=\"my \\\"q\\\" \\\\ .mp4\""),
        "{cd}"
    );
    assert!(
        cd.contains("filename*=UTF-8''my%20%22q%22%20%5C%20%C3%A9.mp4"),
        "{cd}"
    );
    assert!(cd.is_ascii());
}

#[tokio::test]
async fn download_param_sets_attachment() {
    let s = TestServer::spawn(ENV).await;
    let slug = up(&s, "clip.mp4", tiny_mp4()).await;
    let r = s
        .client
        .get(s.url(&format!("/media/{slug}/original.mp4?download=1")))
        .send()
        .await
        .unwrap();
    let cd = r.headers()["content-disposition"]
        .to_str()
        .unwrap()
        .to_string();
    assert!(cd.starts_with("attachment; filename=\"clip.mp4\""), "{cd}");
}

#[tokio::test]
async fn missing_poster_is_404() {
    let s = TestServer::spawn(ENV).await;
    let slug = up(&s, "clip.mp4", tiny_mp4()).await;
    for f in ["poster.jpg", "preview.mp4", "thumb.jpg"] {
        let r = s
            .client
            .get(s.url(&format!("/media/{slug}/{f}")))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 404, "{f}");
    }
}

#[tokio::test]
async fn derived_file_is_served_with_cache_header() {
    let s = TestServer::spawn(ENV).await;
    let slug = up(&s, "pic.png", tiny_png()).await;
    let m = media::get_by_slug(s.db(), &slug).await.unwrap().unwrap();
    let d = fthr_clips_server::media::paths::DerivedPaths::new(
        &s.data_dir.path().canonicalize().unwrap(),
        &m.id,
    );
    std::fs::create_dir_all(d.dir()).unwrap();
    std::fs::write(d.thumb(), b"jpegbytes").unwrap();
    let r = s
        .client
        .get(s.url(&format!("/media/{slug}/thumb.jpg")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["content-type"], "image/jpeg");
    assert_eq!(r.headers()["cache-control"], "public, max-age=3600");
}

#[cfg(unix)]
#[cfg(unix)]
#[tokio::test]
async fn symlink_outside_roots_is_404() {
    let s = TestServer::spawn(ENV).await;
    let slug = up(&s, "clip.mp4", tiny_mp4()).await;
    let m = media::get_by_slug(s.db(), &slug).await.unwrap().unwrap();
    let outside = tempfile::TempDir::new().unwrap();
    let target = outside.path().join("secret.mp4");
    std::fs::write(&target, tiny_mp4()).unwrap();
    let link = s.data_dir.path().join("link.mp4");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    media::relink(s.db(), &m.id, link.to_str().unwrap())
        .await
        .unwrap();
    let r = s
        .client
        .get(s.url(&format!("/media/{slug}/original.mp4")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}
