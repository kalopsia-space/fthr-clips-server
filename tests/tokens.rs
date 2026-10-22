mod common;

use common::{login, tiny_mp4, TestServer};
use serde_json::{json, Value};

const ENV: &[(&str, &str)] = &[("ADMIN_PASSWORD", "pw"), ("UPLOAD_TOKEN", "envtok")];

async fn create(c: &reqwest::Client, s: &TestServer, csrf: &str, name: &str) -> Value {
    let r = c
        .post(s.url("/admin/api/tokens"))
        .header("X-CSRF-Token", csrf)
        .json(&json!({ "name": name }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

#[tokio::test]
async fn created_token_uploads_and_env_token_still_works() {
    let s = TestServer::spawn(ENV).await;
    let (c, csrf) = login(&s, "pw").await;
    let v = create(&c, &s, &csrf, "  laptop ").await;
    let tok = v["token"].as_str().unwrap();
    assert!(tok.starts_with("fthr_") && tok.len() == 37);
    assert_eq!(v["name"], "laptop");
    assert_eq!(v["prefix"], &tok[..12]);
    assert_eq!(s.upload("a.mp4", tiny_mp4(), Some(tok)).await.status(), 200);
    assert_eq!(
        s.upload("b.mp4", tiny_mp4(), Some("envtok")).await.status(),
        200
    );
    assert_eq!(
        s.upload("c.mp4", tiny_mp4(), Some("fthr_nope"))
            .await
            .status(),
        401
    );
}

#[tokio::test]
async fn revoked_token_is_401() {
    let s = TestServer::spawn(ENV).await;
    let (c, csrf) = login(&s, "pw").await;
    let v = create(&c, &s, &csrf, "x").await;
    let tok = v["token"].as_str().unwrap();
    let id = v["id"].as_i64().unwrap();
    let url = s.url(&format!("/admin/api/tokens/{id}"));
    let r = c.delete(&url).send().await.unwrap();
    assert_eq!(r.status(), 403);
    let r = c
        .delete(&url)
        .header("X-CSRF-Token", &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(s.upload("a.mp4", tiny_mp4(), Some(tok)).await.status(), 401);
    let r = c
        .delete(s.url("/admin/api/tokens/99999"))
        .header("X-CSRF-Token", &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let list: Value = c
        .get(s.url("/admin/api/tokens"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list[0]["revoked_at"].is_string());
}

#[tokio::test]
async fn last_used_updates_after_upload() {
    let s = TestServer::spawn(ENV).await;
    let (c, csrf) = login(&s, "pw").await;
    let v = create(&c, &s, &csrf, "x").await;
    let list = |c: reqwest::Client| {
        let u = s.url("/admin/api/tokens");
        async move {
            c.get(u)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };
    assert!(list(c.clone()).await[0]["last_used_at"].is_null());
    let tok = v["token"].as_str().unwrap();
    assert_eq!(s.upload("a.mp4", tiny_mp4(), Some(tok)).await.status(), 200);
    assert!(list(c.clone()).await[0]["last_used_at"].is_string());
}

#[tokio::test]
async fn token_list_never_exposes_hash() {
    let s = TestServer::spawn(ENV).await;
    let (c, csrf) = login(&s, "pw").await;
    create(&c, &s, &csrf, "x").await;
    let body = c
        .get(s.url("/admin/api/tokens"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!body.contains("token_hash") && !body.contains("hash"));
    let v: Value = serde_json::from_str(&body).unwrap();
    for (_, val) in v[0].as_object().unwrap() {
        if let Some(t) = val.as_str() {
            assert!(!(t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit())));
        }
    }
    // Name validation.
    for bad in ["", "   ", &"n".repeat(61)] {
        let r = c
            .post(s.url("/admin/api/tokens"))
            .header("X-CSRF-Token", &csrf)
            .json(&json!({ "name": bad }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 400);
    }
}

#[tokio::test]
async fn tokens_page_is_gated() {
    let s = TestServer::spawn(ENV).await;
    let r = s.client.get(s.url("/admin/tokens")).send().await.unwrap();
    assert_eq!(r.status(), 303);
    let r = s
        .client
        .get(s.url("/admin/api/tokens"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    let (c, csrf) = login(&s, "pw").await;
    create(&c, &s, &csrf, "my<b>tok").await;
    let r = c.get(s.url("/admin/tokens")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let html = r.text().await.unwrap();
    assert!(html.contains("my&#60;b&#62;tok") || html.contains("my&lt;b&gt;tok"));
    assert!(!html.contains("my<b>tok"));
}
