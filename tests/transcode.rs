mod common;

use std::time::Duration;

use common::{fixture_mp4, login, TestServer};
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

async fn wait_for<F, Fut>(secs: u64, mut f: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..secs * 5 {
        if f().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

async fn status(s: &TestServer, path: &str) -> u16 {
    s.client
        .get(s.url(path))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn admin_item(s: &TestServer, c: &reqwest::Client, id: &str) -> Value {
    c.get(s.url(&format!("/admin/api/media/{id}")))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn compat_mode_skips_playable_mp4() {
    let Some(fx) = fixture_mp4() else {
        eprintln!("skipped: ffmpeg not found");
        return;
    };
    let s = server(&[("TRANSCODING", "compat")]).await;
    let (c, _) = login(&s, PW).await;
    let id = upload_id(&s, "ok.mp4", std::fs::read(fx).unwrap()).await;
    // Poster and preview are enqueued right after probe; wait for the poster.
    assert!(
        wait_for(30, || async {
            media::get(s.db(), &id).await.unwrap().unwrap().has_poster
        })
        .await
    );
    tokio::time::sleep(Duration::from_secs(1)).await;
    let item = admin_item(&s, &c, &id).await;
    assert_eq!(item["renditions"].as_array().unwrap().len(), 0);
    let kinds: Vec<&str> = item["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["kind"].as_str().unwrap())
        .collect();
    assert!(
        kinds.iter().all(|k| !k.starts_with("transcode")),
        "unexpected jobs: {kinds:?}"
    );
}

#[tokio::test]
async fn compat_mode_transcodes_mkv() {
    let Some(fx) = fixture_mp4() else {
        eprintln!("skipped: ffmpeg not found");
        return;
    };
    let dir = tempfile::TempDir::new().unwrap();
    let mkv = dir.path().join("tiny.mkv");
    let ok = std::process::Command::new("ffmpeg")
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(&fx)
        .args(["-c", "copy", "-y"])
        .arg(&mkv)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(ok, "remux failed");
    let s = server(&[("TRANSCODING", "compat")]).await;
    let id = upload_id(&s, "clip.mkv", std::fs::read(&mkv).unwrap()).await;
    let slug = media::get(s.db(), &id).await.unwrap().unwrap().slug;
    assert!(
        wait_for(90, || async {
            status(&s, &format!("/media/{slug}/720p.mp4")).await == 200
        })
        .await,
        "720p rendition never appeared"
    );
    let html = s
        .client
        .get(s.url(&format!("/w/{slug}")))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let og = html
        .lines()
        .find(|l| l.contains("og:video\""))
        .expect("og:video tag");
    assert!(og.contains("/720p.mp4\""), "og:video: {og}");
    assert!(
        !html.contains("original.mkv\" type"),
        "mkv listed as source"
    );
}

#[tokio::test]
async fn manual_transcode_and_remove() {
    let Some(fx) = fixture_mp4() else {
        eprintln!("skipped: ffmpeg not found");
        return;
    };
    let s = server(&[("TRANSCODING", "off")]).await;
    let (c, csrf) = login(&s, PW).await;
    let id = upload_id(&s, "ok.mp4", std::fs::read(fx).unwrap()).await;
    let slug = media::get(s.db(), &id).await.unwrap().unwrap().slug;
    assert!(
        wait_for(30, || async {
            media::get(s.db(), &id)
                .await
                .unwrap()
                .unwrap()
                .duration_ms
                .is_some()
        })
        .await
    );
    let bad = c
        .post(s.url(&format!("/admin/api/media/{id}/transcode")))
        .header(H, &csrf)
        .json(&json!({"heights": [123]}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
    let r = c
        .post(s.url(&format!("/admin/api/media/{id}/transcode")))
        .header(H, &csrf)
        .json(&json!({"heights": [480]}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert!(
        wait_for(90, || async {
            status(&s, &format!("/media/{slug}/480p.mp4")).await == 200
        })
        .await,
        "480p rendition never appeared"
    );
    let item = admin_item(&s, &c, &id).await;
    assert_eq!(item["renditions"][0]["state"], "ready");
    let anon = s
        .client
        .get(s.url(&format!("/w/{slug}")))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(anon.contains("data-rung=\"480\"") && !anon.contains("data-transcode"));
    let admin = c
        .get(s.url(&format!("/w/{slug}")))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(admin.contains("data-remove-rendition=\"480\""));
    let r = c
        .delete(s.url(&format!("/admin/api/media/{id}/renditions/480")))
        .header(H, &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(status(&s, &format!("/media/{slug}/480p.mp4")).await, 404);
    let item = admin_item(&s, &c, &id).await;
    assert_eq!(item["renditions"].as_array().unwrap().len(), 0);
}
