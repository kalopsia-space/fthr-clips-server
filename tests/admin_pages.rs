mod common;

use common::{login, tiny_mp4, TestServer};
use fthr_clips_server::db::media;
use serde_json::{json, Value};

const PW: &str = "pw";

async fn server() -> TestServer {
    TestServer::spawn(&[("ADMIN_PASSWORD", PW), ("UPLOAD_TOKEN", "t")]).await
}

async fn upload(s: &TestServer, n: u8) -> (String, String) {
    let mut body = tiny_mp4();
    body[100] = n;
    let r = s.upload(&format!("clip{n}.mp4"), body, Some("t")).await;
    assert_eq!(r.status(), 200);
    let id = r.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let m = media::get(s.db(), &id).await.unwrap().unwrap();
    (id, m.slug)
}

async fn make_private(s: &TestServer, c: &reqwest::Client, csrf: &str, id: &str) {
    let r = c
        .patch(s.url(&format!("/admin/api/media/{id}")))
        .header("x-csrf-token", csrf)
        .json(&json!({"public": false, "title": "Secret clip", "tags": ["alpha"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn admin_files_page_lists_private_items() {
    let s = server().await;
    let (id, slug) = upload(&s, 1).await;
    let (c, csrf) = login(&s, PW).await;
    make_private(&s, &c, &csrf, &id).await;
    let r = c.get(s.url("/admin/files")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let body = r.text().await.unwrap();
    assert!(body.contains("Secret clip"));
    assert!(body.contains(&format!("data-id=\"{id}\"")));
    assert!(body.contains(&format!("data-slug=\"{slug}\"")));
    assert!(body.contains("Link only"));
    assert!(body.contains("alpha"));
    assert!(body.contains("id=\"stat-media\""));
    assert!(body.contains("id=\"queue\""));
    assert!(body.contains("data-csrf=\""));
    assert!(body.contains("/static/admin.js"));
    // the public library must not list it
    let public = s
        .client
        .get(s.url("/"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!public.contains("Secret clip"));
}

#[tokio::test]
async fn admin_files_page_is_gated() {
    let s = server().await;
    for path in ["/admin/files", "/admin/tags", "/admin"] {
        let r = s.client.get(s.url(path)).send().await.unwrap();
        assert_eq!(r.status(), 303, "{path}");
        let loc = r.headers()["location"].to_str().unwrap();
        assert!(loc.starts_with("/admin/login"), "{path}: {loc}");
    }
}

#[tokio::test]
async fn admin_root_redirects_to_files() {
    let s = server().await;
    let (c, _) = login(&s, PW).await;
    let r = c.get(s.url("/admin")).send().await.unwrap();
    assert_eq!(r.status(), 303);
    assert_eq!(r.headers()["location"], "/admin/files");
}

#[tokio::test]
async fn tags_page_lists_tags() {
    let s = server().await;
    let (id, _) = upload(&s, 2).await;
    let (c, csrf) = login(&s, PW).await;
    make_private(&s, &c, &csrf, &id).await;
    let body = c
        .get(s.url("/admin/tags"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(body.contains("alpha"));
    assert!(body.contains("data-colour=\"#"));
    assert!(body.contains("data-csrf=\""));
}

#[tokio::test]
async fn clip_page_shows_editor_when_signed_in() {
    let s = server().await;
    let (_, slug) = upload(&s, 3).await;
    let anon = s
        .client
        .get(s.url(&format!("/w/{slug}")))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!anon.contains("data-csrf"));
    assert!(!anon.contains("Visibility"));
    assert!(!anon.contains("data-delete"));
    assert!(!anon.contains("admin.js"));

    let (c, _) = login(&s, PW).await;
    let body = c
        .get(s.url(&format!("/w/{slug}")))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(body.contains("data-csrf=\""));
    assert!(body.contains("Visibility"));
    assert!(body.contains("data-visibility=\"public\""));
    assert!(body.contains("data-visibility=\"link_only\""));
    assert!(body.contains("data-delete"));
    assert!(body.contains("/static/admin.js"));
}
