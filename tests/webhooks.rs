mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use common::{login, real_png, TestServer};
use serde_json::{json, Value};

type Sink = Arc<Mutex<Vec<(String, Value)>>>;

async fn sink_server() -> (u16, Sink) {
    let sink: Sink = Arc::default();
    let app = Router::new()
        .route(
            "/{name}",
            post(
                |State(s): State<Sink>, Path(name): Path<String>, Json(v): Json<Value>| async move {
                    s.lock().unwrap().push((name, v));
                },
            ),
        )
        .with_state(sink.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (port, sink)
}

async fn spawn(port: u16) -> TestServer {
    let generic = format!("http://127.0.0.1:{port}/generic");
    let discord = format!("http://127.0.0.1:{port}/discord");
    TestServer::spawn(&[
        ("ADMIN_PASSWORD", "pw"),
        ("UPLOAD_TOKEN", "t"),
        ("WEBHOOK_URL", &generic),
        ("DISCORD_WEBHOOK_URL", &discord),
        ("PUBLIC_BASE_URL", "http://example.test"),
    ])
    .await
}

async fn wait_len(sink: &Sink, n: usize) -> bool {
    for _ in 0..100 {
        if sink.lock().unwrap().len() >= n {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

async fn patch_public(s: &TestServer, c: &reqwest::Client, csrf: &str, id: &str, p: bool) {
    let r = c
        .patch(s.url(&format!("/admin/api/media/{id}")))
        .header("x-csrf-token", csrf)
        .json(&json!({ "public": p }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn webhooks_fire_once_on_first_public() {
    let (port, sink) = sink_server().await;
    let s = spawn(port).await;
    let r = s.upload("a.png", real_png(), Some("t")).await;
    assert_eq!(r.status(), 200);
    let id = r.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(wait_len(&sink, 2).await, "expected two webhook bodies");
    {
        let got = sink.lock().unwrap();
        let d = &got.iter().find(|(n, _)| n == "discord").unwrap().1;
        let url = d["embeds"][0]["url"].as_str().unwrap();
        assert!(url.starts_with("http://example.test"), "{url}");
        assert!(url.contains("/i/"), "{url}");
        assert_eq!(d["embeds"][0]["color"], 2155424);
        let g = &got.iter().find(|(n, _)| n == "generic").unwrap().1;
        assert!(g["slug"].is_string());
        assert_eq!(g["public"], true);
    }
    let (c, csrf) = login(&s, "pw").await;
    patch_public(&s, &c, &csrf, &id, false).await;
    patch_public(&s, &c, &csrf, &id, true).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let got = sink.lock().unwrap();
    assert_eq!(got.iter().filter(|(n, _)| n == "discord").count(), 1);
    assert_eq!(got.iter().filter(|(n, _)| n == "generic").count(), 1);
}

#[tokio::test]
async fn webhooks_not_sent_for_link_only_items() {
    let (port, sink) = sink_server().await;
    let s = spawn(port).await;
    let (c, csrf) = login(&s, "pw").await;
    let r = c
        .put(s.url("/admin/api/settings"))
        .header("x-csrf-token", &csrf)
        .json(&json!({ "upload_default_public": false }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let r = s.upload("b.png", real_png(), Some("t")).await;
    assert_eq!(r.status(), 200);
    let id = r.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    // wait for the thumb to exist
    for _ in 0..100 {
        let m = fthr_clips_server::db::media::get(s.db(), &id)
            .await
            .unwrap()
            .unwrap();
        if m.has_thumb {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(sink.lock().unwrap().is_empty());
}
