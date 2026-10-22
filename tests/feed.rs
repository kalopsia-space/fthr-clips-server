mod common;
use common::{login, real_png, tiny_mp4, TestServer};
use fthr_clips_server::db::media;
use serde_json::{json, Value};

const ENV: &[(&str, &str)] = &[("UPLOAD_TOKEN", "t"), ("ADMIN_PASSWORD", "pw")];

async fn up(s: &TestServer, name: &str, mut bytes: Vec<u8>) -> (String, String) {
    bytes.extend_from_slice(name.as_bytes());
    let r = s.upload(name, bytes, Some("t")).await;
    assert_eq!(r.status(), 200);
    let j: Value = r.json().await.unwrap();
    let slug = j["url"].as_str().unwrap().rsplit('/').next().unwrap();
    let m = media::get_by_slug(s.db(), slug).await.unwrap().unwrap();
    (slug.to_string(), m.id)
}

async fn feed(s: &TestServer) -> rss::Channel {
    let r = s.client.get(s.url("/feed.rss")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.headers()["content-type"],
        "application/rss+xml; charset=utf-8"
    );
    let body = r.text().await.unwrap();
    rss::Channel::read_from(body.as_bytes()).unwrap()
}

#[tokio::test]
async fn feed_lists_public_only_with_enclosure() {
    let s = TestServer::spawn(ENV).await;
    let (slug, _) = up(&s, "alphaclip.mp4", tiny_mp4()).await;
    let (_, hidden) = up(&s, "bravoclip.mp4", tiny_mp4()).await;
    media::set_public(s.db(), &[hidden], false).await.unwrap();
    let ch = feed(&s).await;
    assert_eq!(ch.description(), "Public clips");
    assert!(ch.link().starts_with("http://"));
    assert_eq!(ch.items().len(), 1);
    let it = &ch.items()[0];
    assert_eq!(it.title(), Some("alphaclip"));
    let link = it.link().unwrap();
    assert!(link.starts_with("http://") && link.ends_with(&format!("/w/{slug}")));
    assert!(it.guid().unwrap().is_permalink());
    assert_eq!(it.guid().unwrap().value(), link);
    assert!(it.pub_date().is_some());
    let enc = it.enclosure().unwrap();
    assert!(enc.url().starts_with("http://"));
    assert!(enc.url().ends_with("/original.mp4"));
    assert_eq!(enc.mime_type(), "video/mp4");
    assert!(enc.length().parse::<u64>().unwrap() > 0);
}

#[tokio::test]
async fn feed_is_gated_when_library_requires_login() {
    let s = TestServer::spawn(ENV).await;
    up(&s, "a.mp4", tiny_mp4()).await;
    let (c, csrf) = login(&s, "pw").await;
    let r = c
        .put(s.url("/admin/api/settings"))
        .header("x-csrf-token", &csrf)
        .json(&json!({"library_requires_login": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let r = s.client.get(s.url("/feed.rss")).send().await.unwrap();
    assert_eq!(r.status(), 401);
    let r = c.get(s.url("/feed.rss")).send().await.unwrap();
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn feed_includes_images_with_thumbnail_extension() {
    let s = TestServer::spawn(ENV).await;
    let (slug, id) = up(&s, "pic.png", real_png()).await;
    for _ in 0..100 {
        if media::get_by_slug(s.db(), &slug)
            .await
            .unwrap()
            .unwrap()
            .has_thumb
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let _ = id;
    let ch = feed(&s).await;
    assert_eq!(ch.items().len(), 1);
    let it = &ch.items()[0];
    assert!(it.link().unwrap().ends_with(&format!("/i/{slug}")));
    assert_eq!(it.enclosure().unwrap().mime_type(), "image/png");
    let thumb = &it.extensions()["media"]["thumbnail"][0];
    let url = thumb.attrs()["url"].as_str();
    assert!(url.starts_with("http://") && url.ends_with("/thumb.jpg"));
}

#[tokio::test]
async fn feed_omits_private_and_unavailable() {
    let s = TestServer::spawn(ENV).await;
    let (_, a) = up(&s, "gone.mp4", tiny_mp4()).await;
    let (_, b) = up(&s, "priv.mp4", tiny_mp4()).await;
    up(&s, "okclip.mp4", tiny_mp4()).await;
    media::set_public(s.db(), &[b], false).await.unwrap();
    sqlx::query("UPDATE media SET available = 0 WHERE id = ?")
        .bind(&a)
        .execute(&s.db().0)
        .await
        .unwrap();
    let body = s
        .client
        .get(s.url("/feed.rss"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(body.contains("xmlns:media=\"http://search.yahoo.com/mrss/\""));
    assert!(body.contains("okclip"));
    assert!(!body.contains("gone") && !body.contains("priv"));
}
