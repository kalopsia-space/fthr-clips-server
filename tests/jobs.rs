mod common;

use std::time::Duration;

use common::{fixture_mp4, login, real_png, tiny_mp4, TestServer};
use fthr_clips_server::db::media;
use serde_json::{json, Value};

const PW: &str = "pw";
const H: &str = "x-csrf-token";

async fn server(extra: &[(&str, &str)]) -> TestServer {
    let mut env = vec![("ADMIN_PASSWORD", PW), ("UPLOAD_TOKEN", "t")];
    env.extend_from_slice(extra);
    TestServer::spawn(&env).await
}

async fn upload_id(s: &TestServer, name: &str, bytes: Vec<u8>) -> String {
    let r = s.upload(name, bytes, Some("t")).await;
    assert_eq!(r.status(), 200);
    r.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn wait_for<F, Fut>(mut f: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..150 {
        if f().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

#[tokio::test]
async fn poster_and_probe_run_after_upload() {
    let Some(fx) = fixture_mp4() else {
        eprintln!("skipped: ffmpeg not found");
        return;
    };
    let s = server(&[]).await;
    let id = upload_id(&s, "real.mp4", std::fs::read(fx).unwrap()).await;
    let ok = wait_for(|| async {
        let m = media::get(s.db(), &id).await.unwrap().unwrap();
        m.has_poster && m.duration_ms.is_some()
    })
    .await;
    assert!(ok, "poster/probe did not finish in 30 s");
    let m = media::get(s.db(), &id).await.unwrap().unwrap();
    assert_eq!(m.processing_state, "ready");
    let r = s
        .client
        .get(s.url(&format!("/media/{}/poster.jpg", m.slug)))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert!(
        wait_for(|| async { media::get(s.db(), &id).await.unwrap().unwrap().has_preview }).await
    );
}

#[tokio::test]
async fn thumb_runs_without_ffmpeg() {
    let s = server(&[
        ("FFMPEG_PATH", "/nonexistent"),
        ("FFPROBE_PATH", "/nonexistent"),
    ])
    .await;
    let id = upload_id(&s, "pic.png", real_png()).await;
    let ok = wait_for(|| async { media::get(s.db(), &id).await.unwrap().unwrap().has_thumb }).await;
    assert!(ok, "thumb not generated");
    let m = media::get(s.db(), &id).await.unwrap().unwrap();
    assert_eq!((m.width, m.height), (Some(1600), Some(900)));
    let r = s
        .client
        .get(s.url(&format!("/media/{}/thumb.jpg", m.slug)))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let bytes = r.bytes().await.unwrap();
    let img = image::load_from_memory(&bytes).unwrap();
    assert_eq!(img.width(), 720);
}

#[tokio::test]
async fn jobs_stay_queued_without_ffmpeg_and_pages_render() {
    let s = server(&[
        ("FFMPEG_PATH", "/nonexistent"),
        ("FFPROBE_PATH", "/nonexistent"),
    ])
    .await;
    let id = upload_id(&s, "clip.mp4", tiny_mp4()).await;
    let slug = media::get(s.db(), &id).await.unwrap().unwrap().slug;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    for path in ["/".to_string(), format!("/w/{slug}")] {
        let r = s.client.get(s.url(&path)).send().await.unwrap();
        assert_eq!(r.status(), 200, "{path}");
        assert!(
            r.text().await.unwrap().contains("poster-placeholder.svg"),
            "{path}"
        );
    }
    let (c, _) = login(&s, PW).await;
    let j: Value = c
        .get(s.url("/admin/api/jobs"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let active = j["active"].as_array().unwrap();
    assert!(
        active.iter().any(|x| x["kind"] == "probe"
            && x["state"] == "queued"
            && x["media_id"] == id.as_str()),
        "{j}"
    );
}

#[tokio::test]
async fn cancel_and_retry_change_state() {
    let s = server(&[
        ("FFMPEG_PATH", "/nonexistent"),
        ("FFPROBE_PATH", "/nonexistent"),
    ])
    .await;
    let id = upload_id(&s, "clip.mp4", tiny_mp4()).await;
    let (c, csrf) = login(&s, PW).await;
    let jobs = || async {
        c.get(s.url("/admin/api/jobs"))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap()
    };
    let j = jobs().await;
    let job = j["active"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["media_id"] == id.as_str())
        .unwrap()
        .clone();
    let jid = job["id"].as_i64().unwrap();
    assert!(job["media_title"].is_string());

    let r = c
        .post(s.url(&format!("/admin/api/jobs/{jid}/cancel")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let r = c
        .post(s.url(&format!("/admin/api/jobs/{jid}/cancel")))
        .header(H, &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let j = jobs().await;
    assert!(j["active"]
        .as_array()
        .unwrap()
        .iter()
        .all(|x| x["id"] != jid));
    let rec = j["recent"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == jid)
        .unwrap();
    assert_eq!(rec["state"], "cancelled");

    let r = c
        .post(s.url(&format!("/admin/api/jobs/{jid}/retry")))
        .header(H, &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"requeued": true}));
    let j = jobs().await;
    let act = j["active"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == jid)
        .unwrap();
    assert_eq!(act["state"], "queued");
}

#[tokio::test]
async fn stats_endpoint_shape() {
    let s = server(&[
        ("FFMPEG_PATH", "/nonexistent"),
        ("FFPROBE_PATH", "/nonexistent"),
    ])
    .await;
    upload_id(&s, "clip.mp4", tiny_mp4()).await;
    let r = s
        .client
        .get(s.url("/admin/api/stats"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    let (c, _) = login(&s, PW).await;
    let j: Value = c
        .get(s.url("/admin/api/stats"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(j["media"], json!({"videos": 1, "images": 0}));
    assert_eq!(j["ffmpeg"]["available"], false);
    assert!(j["ffmpeg"]["version"].is_null());
    assert!(j["encoder"].is_null());
    assert!(j["last_scan"].is_null());
    assert_eq!(j["workers"]["light"], 2);
    assert!(j["workers"]["heavy"].is_number());
    assert!(
        j["disk"].is_null()
            || (j["disk"]["total"].as_u64().unwrap() > 0 && j["disk"]["path"].is_string())
    );
}

#[tokio::test]
async fn stale_running_jobs_are_requeued() {
    use fthr_clips_server::db::jobs;
    let s = server(&[
        ("FFMPEG_PATH", "/nonexistent"),
        ("FFPROBE_PATH", "/nonexistent"),
    ])
    .await;
    let id = upload_id(&s, "clip.mp4", tiny_mp4()).await;
    let job = jobs::claim_next(s.db(), &["probe"]).await.unwrap().unwrap();
    assert_eq!(job.state, "running");
    assert_eq!(jobs::reset_stale_running(s.db()).await.unwrap(), 1);
    let j = jobs::get(s.db(), job.id).await.unwrap().unwrap();
    assert_eq!(
        (j.state.as_str(), j.error.as_deref(), j.attempts),
        ("queued", Some("interrupted"), 1)
    );
    assert_eq!(j.media_id, id);
    // Guarded finish/fail do nothing on a non-running job.
    assert!(!jobs::finish(s.db(), job.id).await.unwrap());
    assert!(!jobs::fail(s.db(), job.id, "x").await.unwrap());
}
