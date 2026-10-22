mod common;

use common::{login, tiny_mp4, TestServer};
use serde_json::{json, Value};

const PW: &str = "pw";

async fn server() -> TestServer {
    TestServer::spawn(&[("ADMIN_PASSWORD", PW), ("UPLOAD_TOKEN", "t")]).await
}

async fn put(s: &TestServer, c: &reqwest::Client, csrf: &str, body: Value) -> reqwest::Response {
    c.put(s.url("/admin/api/settings"))
        .header("x-csrf-token", csrf)
        .json(&body)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn put_settings_persists_across_restart() {
    let s = server().await;
    let (c, csrf) = login(&s, PW).await;
    let r = put(&s, &c, &csrf, json!({"library_requires_login": true})).await;
    assert_eq!(r.status(), 200);
    let j: Value = r.json().await.unwrap();
    assert_eq!(j["library_requires_login"], true);
    assert!(j["effective_encoder"].is_null());

    let dir = s.data_dir.path().to_string_lossy().into_owned();
    let s2 = TestServer::spawn(&[("ADMIN_PASSWORD", PW), ("DATA_DIR", &dir)]).await;
    let r = s2.client.get(s2.url("/")).send().await.unwrap();
    assert_eq!(r.status(), 303);
    assert!(r.headers()["location"]
        .to_str()
        .unwrap()
        .starts_with("/admin/login"));
}

#[tokio::test]
async fn library_login_gate_leaves_share_links_open() {
    let s = server().await;
    let r = s.upload("a.mp4", tiny_mp4(), Some("t")).await;
    let j: Value = r.json().await.unwrap();
    let url = j["url"].as_str().unwrap();
    let slug = url.rsplit('/').next().unwrap();
    let (c, csrf) = login(&s, PW).await;
    assert_eq!(
        put(&s, &c, &csrf, json!({"library_requires_login": true}))
            .await
            .status(),
        200
    );
    let anon = &s.client;
    assert_eq!(
        anon.get(s.url(&format!("/w/{slug}")))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(anon.get(s.url("/")).send().await.unwrap().status(), 303);
    assert_eq!(
        anon.get(s.url("/images")).send().await.unwrap().status(),
        303
    );
    // signed-in admin still sees the library
    assert_eq!(c.get(s.url("/")).send().await.unwrap().status(), 200);
}

#[tokio::test]
async fn api_library_is_401_when_gated() {
    let s = server().await;
    let (c, csrf) = login(&s, PW).await;
    put(&s, &c, &csrf, json!({"library_requires_login": true})).await;
    let r = s.client.get(s.url("/api/library")).send().await.unwrap();
    assert_eq!(r.status(), 401);
    assert_eq!(r.json::<Value>().await.unwrap()["error"], "unauthorized");
    assert_eq!(
        c.get(s.url("/api/library")).send().await.unwrap().status(),
        200
    );
}

#[tokio::test]
async fn sharing_form_post_updates_site_title() {
    let s = server().await;
    let (c, csrf) = login(&s, PW).await;
    let page = c.get(s.url("/admin/sharing")).send().await.unwrap();
    assert_eq!(page.status(), 200);
    assert!(page.text().await.unwrap().contains("name=\"csrf\""));
    let r = c
        .post(s.url("/admin/sharing"))
        .form(&[
            ("csrf", csrf.as_str()),
            ("site_title", "Zebra Clips"),
            ("public_base_url", ""),
            ("default_sort", "oldest"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 303);
    assert_eq!(r.headers()["location"], "/admin/sharing?saved=1");
    let home = s.client.get(s.url("/")).send().await.unwrap();
    let body = home.text().await.unwrap();
    let title = body
        .split("<title>")
        .nth(1)
        .unwrap()
        .split("</title>")
        .next()
        .unwrap();
    assert!(title.contains("Zebra Clips"), "{title}");
    let g = c
        .get(s.url("/admin/api/settings"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(g["default_sort"], "oldest");
    assert_eq!(g["library_requires_login"], false);
}

#[tokio::test]
async fn form_post_requires_csrf_and_reports_errors() {
    let s = server().await;
    let (c, csrf) = login(&s, PW).await;
    let r = c
        .post(s.url("/admin/transcoding"))
        .form(&[("csrf", "bad"), ("transcode_workers", "2")])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let r = c
        .post(s.url("/admin/transcoding"))
        .form(&[
            ("csrf", csrf.as_str()),
            ("transcoding", "on"),
            ("encoder", "auto"),
            ("transcode_workers", "0"),
            ("transcode_timeout_minutes", "30"),
            ("ladder", "720"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 303);
    let loc = r.headers()["location"].to_str().unwrap().to_string();
    assert!(loc.starts_with("/admin/transcoding?error="), "{loc}");
    let page = c
        .get(s.url(&loc))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(page.contains("workers"));
    // valid post with two ladder rungs
    let r = c
        .post(s.url("/admin/transcoding"))
        .form(&[
            ("csrf", csrf.as_str()),
            ("transcoding", "on"),
            ("encoder", "x264"),
            ("transcode_workers", "2"),
            ("transcode_timeout_minutes", "30"),
            ("ladder", "480"),
            ("ladder", "1080"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(r.headers()["location"], "/admin/transcoding?saved=1");
    let g = c
        .get(s.url("/admin/api/settings"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(g["ladder"], json!([480, 1080]));
    assert_eq!(g["encoder"], "x264");
}

#[tokio::test]
async fn all_tabs_render_and_checkbox_absent_means_false() {
    let s = server().await;
    let (c, csrf) = login(&s, PW).await;
    for t in ["library", "transcoding", "sharing", "integrations"] {
        let r = c.get(s.url(&format!("/admin/{t}"))).send().await.unwrap();
        assert_eq!(r.status(), 200, "{t}");
    }
    put(
        &s,
        &c,
        &csrf,
        json!({"webhooks_enabled": true, "upload_default_public": true}),
    )
    .await;
    let r = c
        .post(s.url("/admin/integrations"))
        .form(&[
            ("csrf", csrf.as_str()),
            ("discord_webhook_url", ""),
            ("webhook_url", ""),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 303);
    let g = c
        .get(s.url("/admin/api/settings"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(g["webhooks_enabled"], false);
    assert_eq!(g["upload_default_public"], true);
}

#[tokio::test]
async fn put_rejects_invalid_values() {
    let s = server().await;
    let (c, csrf) = login(&s, PW).await;
    for body in [
        json!({"ladder": [999]}),
        json!({"transcode_workers": 0}),
        json!({"upload_folder": "../x"}),
        json!({"bogus": 1}),
        json!({"public_base_url": "ftp://x"}),
    ] {
        let r = put(&s, &c, &csrf, body.clone()).await;
        assert_eq!(r.status(), 400, "{body}");
        assert!(r.json::<Value>().await.unwrap()["error"].is_string());
    }
    let r = c
        .put(s.url("/admin/api/settings"))
        .json(&json!({"site_title": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let r = s
        .client
        .get(s.url("/admin/api/settings"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
}
