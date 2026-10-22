mod common;
use common::{tiny_mp4, TestServer};
use fthr_clips_server::db::media::{self, ProbeFields};
use fthr_clips_server::db::tags;
use serde_json::Value;

const ENV: &[(&str, &str)] = &[("UPLOAD_TOKEN", "t")];

async fn up(s: &TestServer, name: &str) -> (String, String) {
    // Content-addressed ids: make each clip's bytes distinct.
    let mut bytes = tiny_mp4();
    bytes.extend_from_slice(name.as_bytes());
    let r = s.upload(name, bytes, Some("t")).await;
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

fn meta(body: &str, prop: &str) -> Option<String> {
    let needle = format!("property=\"{prop}\" content=\"");
    let i = body.find(&needle)? + needle.len();
    Some(body[i..i + body[i..].find('"')?].to_string())
}

#[tokio::test]
async fn library_lists_only_public() {
    let s = TestServer::spawn(ENV).await;
    let (_, _a) = up(&s, "alphaclip.mp4").await;
    let (_, b) = up(&s, "bravoclip.mp4").await;
    media::set_public(s.db(), &[b], false).await.unwrap();
    let (st, body) = get_text(&s, "/").await;
    assert_eq!(st, 200);
    assert!(body.contains("alphaclip"));
    assert!(!body.contains("bravoclip"));
    let j: Value = s
        .client
        .get(s.url("/api/library"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(j["total"], 1);
    assert_eq!(j["items"].as_array().unwrap().len(), 1);
    assert_eq!(j["items"][0]["title"], "alphaclip");
}

#[tokio::test]
async fn library_search_and_tag_filter() {
    let s = TestServer::spawn(ENV).await;
    let (_, a) = up(&s, "alphaclip.mp4").await;
    let (_, _b) = up(&s, "bravoclip.mp4").await;
    tags::attach_by_name(s.db(), &[a], "valorant")
        .await
        .unwrap();
    let (_, body) = get_text(&s, "/?q=alpha").await;
    assert!(body.contains("alphaclip") && !body.contains("bravoclip"));
    let (_, body) = get_text(&s, "/?tag=valorant").await;
    assert!(body.contains("alphaclip") && !body.contains("bravoclip"));
    let j: Value = s
        .client
        .get(s.url("/api/library?tag=valorant"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(j["total"], 1);
    assert_eq!(j["items"][0]["tags"][0]["name"], "valorant");
    let (_, body) = get_text(&s, "/?q=zzzz").await;
    assert!(!body.contains("alphaclip"));
}

#[tokio::test]
async fn clip_page_has_opengraph_tags() {
    let s = TestServer::spawn(ENV).await;
    let (slug, id) = up(&s, "alphaclip.mp4").await;
    media::set_flags(s.db(), &id, Some(true), None, None)
        .await
        .unwrap();
    let (st, body) = get_text(&s, &format!("/w/{slug}")).await;
    assert_eq!(st, 200);
    let base = s.base.clone();
    assert_eq!(
        meta(&body, "og:video").unwrap(),
        format!("{base}/media/{slug}/original.mp4")
    );
    assert_eq!(
        meta(&body, "og:image").unwrap(),
        format!("{base}/media/{slug}/poster.jpg")
    );
    assert_eq!(meta(&body, "og:url").unwrap(), format!("{base}/w/{slug}"));
    assert_eq!(meta(&body, "og:type").unwrap(), "video.other");
    assert_eq!(meta(&body, "og:video:type").unwrap(), "video/mp4");
    assert!(body.contains("name=\"twitter:card\" content=\"player\""));
}

#[tokio::test]
async fn clip_page_omits_og_video_when_not_playable() {
    let s = TestServer::spawn(ENV).await;
    let (slug, id) = up(&s, "alphaclip.mp4").await;
    let p = ProbeFields {
        container: Some("mkv".into()),
        codec_video: Some("h264".into()),
        ..Default::default()
    };
    media::set_probe(s.db(), &id, &p).await.unwrap();
    let (st, body) = get_text(&s, &format!("/w/{slug}")).await;
    assert_eq!(st, 200);
    assert!(meta(&body, "og:video").is_none());
    assert!(body.contains("<source src=\""));
    assert!(body.contains("/original."));
    assert!(body.contains("may not play in your browser"));
    assert!(body.contains("name=\"twitter:card\" content=\"summary_large_image\""));
    assert!(meta(&body, "og:image")
        .unwrap()
        .ends_with("/static/poster-placeholder.svg"));
}

#[tokio::test]
async fn clip_page_counts_distinct_views() {
    let s = TestServer::spawn(ENV).await;
    let (slug, _) = up(&s, "alphaclip.mp4").await;
    for _ in 0..2 {
        assert_eq!(get_text(&s, &format!("/w/{slug}")).await.0, 200);
    }
    let j: Value = s
        .client
        .get(s.url("/api/library"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(j["items"][0]["views"], 1);
    // The library listing itself must not count views.
    for _ in 0..2 {
        get_text(&s, "/").await;
    }
    let j: Value = s
        .client
        .get(s.url("/api/library"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(j["items"][0]["views"], 1);
}

#[tokio::test]
async fn clip_page_has_no_frame_options() {
    let s = TestServer::spawn(ENV).await;
    let (slug, _) = up(&s, "alphaclip.mp4").await;
    let r = s
        .client
        .get(s.url(&format!("/w/{slug}")))
        .send()
        .await
        .unwrap();
    assert!(!r.headers().contains_key("x-frame-options"));
    let r = s.client.get(s.url("/")).send().await.unwrap();
    assert!(r.headers().contains_key("x-frame-options"));
}

#[tokio::test]
async fn unknown_slug_is_404() {
    let s = TestServer::spawn(ENV).await;
    assert_eq!(get_text(&s, "/w/nope123").await.0, 404);
    assert_eq!(get_text(&s, "/i/nope123").await.0, 404);
    // A video slug is not an image page.
    let (slug, _) = up(&s, "alphaclip.mp4").await;
    assert_eq!(get_text(&s, &format!("/i/{slug}")).await.0, 404);
}

#[tokio::test]
async fn private_clip_page_reachable_by_link() {
    let s = TestServer::spawn(ENV).await;
    let (slug, id) = up(&s, "alphaclip.mp4").await;
    media::set_public(s.db(), &[id], false).await.unwrap();
    // Unlisted links still work: private means not listed, not secret.
    assert_eq!(get_text(&s, &format!("/w/{slug}")).await.0, 200);
}

#[tokio::test]
async fn static_assets_served() {
    let s = TestServer::spawn(ENV).await;
    let r = s.client.get(s.url("/static/app.css")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert!(r.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/css"));
    assert!(r.headers()["cache-control"]
        .to_str()
        .unwrap()
        .contains("immutable"));
    let css = r.text().await.unwrap();
    assert!(!css.contains("http"));
    assert_eq!(get_text(&s, "/static/app.js").await.0, 200);
    assert_eq!(get_text(&s, "/static/icons.svg").await.0, 200);
    assert_eq!(get_text(&s, "/static/nope.css").await.0, 404);
    // The library page links its assets with a cache-buster and has no inline script/style.
    let (_, body) = get_text(&s, "/").await;
    assert!(body.contains("/static/app.css?v="));
    assert!(!body.contains("<script>") && !body.contains("style=\"") && !body.contains("onclick"));
}

#[tokio::test]
async fn playback_sources_order_and_quality_select() {
    use fthr_clips_server::db::jobs::{self, Rendition};
    use fthr_clips_server::web::clip::{og_video, playback_sources};
    let s = TestServer::spawn(ENV).await;
    let (slug, id) = up(&s, "alphaclip.mp4").await;
    for (h, state) in [(480, "ready"), (720, "ready"), (1080, "failed")] {
        let r = Rendition {
            media_id: id.clone(),
            height: h,
            path: String::new(),
            size: 1,
            state: state.into(),
            progress: 1.0,
            error: None,
        };
        jobs::rendition_upsert(s.db(), &r).await.unwrap();
    }
    let m = media::get_by_slug(s.db(), &slug).await.unwrap().unwrap();
    let rends = jobs::renditions_for(s.db(), &id).await.unwrap();
    let src = playback_sources(&m, &rends);
    let labels: Vec<&str> = src.iter().map(|x| x.label.as_str()).collect();
    assert_eq!(labels, ["Source", "720p", "480p"]);
    assert_eq!(
        og_video(&src).unwrap().url,
        format!("/media/{slug}/original.mp4")
    );
    assert!(og_video(&[]).is_none());
    let (_, body) = get_text(&s, &format!("/w/{slug}?t=12")).await;
    assert!(body.contains("data-quality"));
    assert!(body.contains("data-start=\"12\""));
    assert_eq!(body.matches("<source ").count(), 3);

    // When the original is not playable the best rendition leads.
    let p = ProbeFields {
        container: Some("mkv".into()),
        codec_video: Some("hevc".into()),
        ..Default::default()
    };
    media::set_probe(s.db(), &id, &p).await.unwrap();
    let m = media::get_by_slug(s.db(), &slug).await.unwrap().unwrap();
    let src = playback_sources(&m, &rends);
    assert_eq!(src[0].label, "720p");
    let (_, body) = get_text(&s, &format!("/w/{slug}")).await;
    assert_eq!(
        meta(&body, "og:video").unwrap(),
        format!("{}/media/{slug}/720p.mp4", s.base)
    );
}

#[tokio::test]
async fn images_library_and_page() {
    let s = TestServer::spawn(ENV).await;
    let mut png = common::tiny_png();
    png.extend_from_slice(b"shot");
    let r = s.upload("shotone.png", png, Some("t")).await;
    assert_eq!(r.status(), 200);
    let j: Value = r.json().await.unwrap();
    let url = j["url"].as_str().unwrap().to_string();
    let slug = url.rsplit('/').next().unwrap();
    assert!(url.contains("/i/"));
    let (st, body) = get_text(&s, "/images").await;
    assert_eq!(st, 200);
    assert!(body.contains("shotone") && body.contains("Screenshots"));
    let (_, body) = get_text(&s, "/").await;
    assert!(!body.contains("shotone"));
    let (st, body) = get_text(&s, &format!("/i/{slug}")).await;
    assert_eq!(st, 200);
    assert!(meta(&body, "og:image").is_some());
    assert!(meta(&body, "og:video").is_none());
    assert!(!body.contains("<video"));
    let j: Value = s
        .client
        .get(s.url("/api/library?kind=image"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(j["total"], 1);
    assert_eq!(j["items"][0]["kind"], "image");
}

#[tokio::test]
async fn library_chips_exclude_link_only_tags() {
    let s = TestServer::spawn(ENV).await;
    let (_, a) = up(&s, "alphaclip.mp4").await;
    let (_, b) = up(&s, "bravoclip.mp4").await;
    media::set_public(s.db(), std::slice::from_ref(&b), false)
        .await
        .unwrap();
    let shown = tags::create(s.db(), "shownTag", "#20e3a0").await.unwrap();
    let hidden = tags::create(s.db(), "hiddenTag", "#20e3a0").await.unwrap();
    tags::attach(s.db(), &a, shown.id).await.unwrap();
    tags::attach(s.db(), &b, hidden.id).await.unwrap();
    let (st, body) = get_text(&s, "/").await;
    assert_eq!(st, 200);
    assert!(body.contains("shownTag"));
    assert!(!body.contains("hiddenTag"));
}
