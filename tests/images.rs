mod common;

use common::{login, real_png_sized, tiny_mp4, TestServer};
use fthr_clips_server::db::media;
use serde_json::Value;

const ENV: &[(&str, &str)] = &[("UPLOAD_TOKEN", "t"), ("ADMIN_PASSWORD", "pw")];

async fn up_png(s: &TestServer, name: &str, w: u32, h: u32) -> (String, String) {
    let r = s.upload(name, real_png_sized(w, h), Some("t")).await;
    assert_eq!(r.status(), 200);
    let j: Value = r.json().await.unwrap();
    let slug = j["url"].as_str().unwrap().rsplit('/').next().unwrap();
    let m = media::get_by_slug(s.db(), slug).await.unwrap().unwrap();
    (slug.to_string(), m.id)
}

async fn get_text(s: &TestServer, path: &str) -> (u16, String) {
    let r = s.client.get(s.url(path)).send().await.unwrap();
    (r.status().as_u16(), r.text().await.unwrap())
}

fn meta(body: &str, key: &str, name: &str) -> Option<String> {
    let needle = format!("{key}=\"{name}\" content=\"");
    let i = body.find(&needle)? + needle.len();
    Some(body[i..i + body[i..].find('"')?].to_string())
}

#[tokio::test]
async fn images_page_lists_only_images_that_are_public() {
    let s = TestServer::spawn(ENV).await;
    let (_, a) = up_png(&s, "shotalpha.png", 64, 48).await;
    let (hidden_slug, b) = up_png(&s, "shotbravo.png", 65, 48).await;
    media::set_public(s.db(), &[b], false).await.unwrap();
    media::set_flags(s.db(), &a, None, None, Some(true))
        .await
        .unwrap();
    let r = s.upload("clipvid.mp4", tiny_mp4(), Some("t")).await;
    assert_eq!(r.status(), 200);

    let (st, body) = get_text(&s, "/images").await;
    assert_eq!(st, 200);
    assert!(body.contains("Screenshots"));
    assert!(body.contains("1 image"), "{body}");
    assert!(body.contains("shotalpha"));
    assert!(!body.contains("shotbravo"));
    assert!(!body.contains("clipvid"));
    assert!(body.contains("/thumb.jpg"));
    assert!(body.contains(">PNG<"));
    assert!(!body.contains("pill dur"));
    assert!(!body.contains("data-preview"));
    assert!(body.contains("action=\"/images\""));
    // Non-public image stays viewable by slug.
    assert_eq!(get_text(&s, &format!("/i/{hidden_slug}")).await.0, 200);
}

#[tokio::test]
async fn image_page_has_og_image_of_original() {
    let s = TestServer::spawn(ENV).await;
    let (slug, id) = up_png(&s, "shotog.png", 64, 48).await;
    media::set_dimensions(s.db(), &id, 64, 48).await.unwrap();
    let (st, body) = get_text(&s, &format!("/i/{slug}")).await;
    assert_eq!(st, 200);
    let p = |n| meta(&body, "property", n);
    assert_eq!(
        p("og:image").unwrap(),
        format!("{}/media/{slug}/original.png", s.base)
    );
    assert_eq!(p("og:type").unwrap(), "website");
    assert_eq!(p("og:image:width").unwrap(), "64");
    assert_eq!(p("og:image:height").unwrap(), "48");
    assert_eq!(p("og:image:type").unwrap(), "image/png");
    assert_eq!(p("og:url").unwrap(), format!("{}/i/{slug}", s.base));
    assert_eq!(
        meta(&body, "name", "twitter:card").unwrap(),
        "summary_large_image"
    );
    assert!(!body.contains("og:video"));
    assert!(body.contains("class=\"lightbox\""));
    assert!(body.contains("64×48"));
    assert!(body.contains("?download=1"));
    assert!(!body.contains(" onclick="));
    assert!(
        !body.contains("data-visibility"),
        "anonymous: no admin panels"
    );
    let r = s
        .client
        .get(s.url(&format!("/i/{slug}")))
        .send()
        .await
        .unwrap();
    assert!(!r.headers().contains_key("x-frame-options"));
}

#[tokio::test]
async fn image_page_counts_views() {
    let s = TestServer::spawn(ENV).await;
    let (slug, id) = up_png(&s, "shotviews.png", 64, 48).await;
    get_text(&s, &format!("/i/{slug}")).await;
    get_text(&s, &format!("/i/{slug}")).await;
    // Same visitor within the dedupe window counts once, exactly like clips.
    assert_eq!(media::view_count(s.db(), &id).await.unwrap(), 1);
    let (_, body) = get_text(&s, &format!("/i/{slug}")).await;
    assert!(body.contains("1 view"));
}

#[tokio::test]
async fn image_page_404_for_video_slug_and_vice_versa() {
    let s = TestServer::spawn(ENV).await;
    let r = s.upload("clipvid.mp4", tiny_mp4(), Some("t")).await;
    let j: Value = r.json().await.unwrap();
    let vslug = j["url"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_string();
    let (islug, _) = up_png(&s, "shotkind.png", 64, 48).await;
    assert_eq!(get_text(&s, &format!("/i/{vslug}")).await.0, 404);
    assert_eq!(get_text(&s, &format!("/w/{islug}")).await.0, 404);
    assert_eq!(get_text(&s, &format!("/i/{islug}")).await.0, 200);
    assert_eq!(get_text(&s, &format!("/w/{vslug}")).await.0, 200);
}

#[tokio::test]
async fn images_page_tag_links_point_at_images() {
    let s = TestServer::spawn(ENV).await;
    let (_, id) = up_png(&s, "shottag.png", 64, 48).await;
    fthr_clips_server::db::tags::attach_by_name(s.db(), &[id], "moody")
        .await
        .unwrap();
    let (_, body) = get_text(&s, "/images").await;
    assert!(body.contains("href=\"/images?tag=moody\""), "{body}");
    assert!(!body.contains("href=\"/?tag=moody\""));
    let (_, body) = get_text(&s, "/").await;
    assert!(body.contains("href=\"/?tag=moody\"") || !body.contains("moody"));
}

#[tokio::test]
async fn api_image_items_use_thumb_and_no_preview() {
    let s = TestServer::spawn(ENV).await;
    let (slug, id) = up_png(&s, "shotapi.png", 64, 48).await;
    media::set_flags(s.db(), &id, None, None, Some(true))
        .await
        .unwrap();
    let j: Value = s
        .client
        .get(s.url("/api/library?kind=image"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        j["items"][0]["poster_url"],
        format!("{}/media/{slug}/thumb.jpg", s.base)
    );
    assert!(j["items"][0]["preview_url"].is_null());
}

#[tokio::test]
async fn signed_in_image_page_is_editable() {
    let s = TestServer::spawn(ENV).await;
    let (slug, _) = up_png(&s, "shotedit.png", 64, 48).await;
    let (c, _) = login(&s, "pw").await;
    let body = c
        .get(s.url(&format!("/i/{slug}")))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(body.contains("id=\"details-form\""));
    assert!(body.contains("data-visibility"));
    assert!(body.contains("data-delete"));
    assert!(body.contains("/static/admin.js"));
}
