mod common;

use common::{login, tiny_mp4, TestServer};
use fthr_clips_server::db::media::{self, NewMedia};
use fthr_clips_server::media::kind::{MediaKind, Source};
use serde_json::{json, Value};

const PW: &str = "pw";
const H: &str = "x-csrf-token";

async fn server() -> TestServer {
    TestServer::spawn(&[("ADMIN_PASSWORD", PW), ("UPLOAD_TOKEN", "t")]).await
}

/// Uploads a distinct clip; returns (id, slug).
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

async fn library_total(s: &TestServer) -> i64 {
    s.client
        .get(s.url("/api/library"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["total"]
        .as_i64()
        .unwrap()
}

#[tokio::test]
async fn patch_updates_title_and_tags() {
    let s = server().await;
    let (id, _) = upload(&s, 1).await;
    let (c, csrf) = login(&s, PW).await;
    let r = c
        .patch(s.url(&format!("/admin/api/media/{id}")))
        .header(H, &csrf)
        .json(&json!({"title": "  New title ", "description": "d", "tags": ["Funny", "funny", "Cats"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let j: Value = r.json().await.unwrap();
    assert_eq!(j["title"], "New title");
    assert_eq!(j["tags"].as_array().unwrap().len(), 2);
    assert!(j["path"].as_str().unwrap().ends_with(".mp4"));
    assert!(!j["path"].as_str().unwrap().contains('/'));

    let r = c
        .patch(s.url(&format!("/admin/api/media/{id}")))
        .header(H, &csrf)
        .json(&json!({"tags": []}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.json::<Value>().await.unwrap()["tags"], json!([]));

    let r = c
        .patch(s.url(&format!("/admin/api/media/{id}")))
        .header(H, &csrf)
        .json(&json!({"title": "  "}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn unlist_hides_from_public_library_then_relist_shows() {
    let s = server().await;
    let (id, _) = upload(&s, 2).await;
    let (c, csrf) = login(&s, PW).await;
    assert_eq!(library_total(&s).await, 1);
    let url = s.url(&format!("/admin/api/media/{id}"));
    let r = c
        .patch(&url)
        .header(H, &csrf)
        .json(&json!({"public": false}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.json::<Value>().await.unwrap()["public"], false);
    assert_eq!(library_total(&s).await, 0);
    let list: Value = c
        .get(s.url("/admin/api/media"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["total"], 1);
    assert_eq!(list["items"][0]["public"], false);
    c.patch(&url)
        .header(H, &csrf)
        .json(&json!({"public": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(library_total(&s).await, 1);
}

#[tokio::test]
async fn delete_upload_removes_file_and_404s() {
    let s = server().await;
    let (id, slug) = upload(&s, 3).await;
    let path = media::get(s.db(), &id).await.unwrap().unwrap().path;
    assert!(std::path::Path::new(&path).exists());
    let (c, csrf) = login(&s, PW).await;
    let r = c
        .delete(s.url(&format!("/admin/api/media/{id}")))
        .header(H, &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert!(!std::path::Path::new(&path).exists());
    let r = s
        .client
        .get(s.url(&format!("/w/{slug}")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    assert!(media::get(s.db(), &id).await.unwrap().is_none());
}

#[tokio::test]
async fn delete_scanned_file_keeps_file_without_flag() {
    let videos = tempfile::TempDir::new().unwrap();
    let outside = tempfile::TempDir::new().unwrap();
    let vd = videos.path().to_string_lossy().into_owned();
    let s = TestServer::spawn(&[("ADMIN_PASSWORD", PW), ("VIDEO_DIRS", &vd)]).await;
    let (c, csrf) = login(&s, PW).await;

    let mk = |dir: &std::path::Path, id: &str| {
        let p = dir.join(format!("{id}.mp4"));
        std::fs::write(&p, tiny_mp4()).unwrap();
        let mut m = NewMedia::test(
            id,
            &format!("slug{id}"),
            MediaKind::Video,
            &p.to_string_lossy(),
        );
        m.source = Source::Scan;
        (p, m)
    };
    let (p1, m1) = mk(videos.path(), "aaaa");
    let (p2, m2) = mk(videos.path(), "bbbb");
    let (p3, m3) = mk(outside.path(), "cccc");
    for m in [&m1, &m2, &m3] {
        media::insert(s.db(), m).await.unwrap();
    }
    let tomb = |id: &str| {
        let id = id.to_string();
        let db = s.db().clone();
        async move {
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM tombstone WHERE id = ?")
                .bind(id)
                .fetch_one(&db.0)
                .await
                .unwrap()
        }
    };

    let r = c
        .delete(s.url("/admin/api/media/aaaa"))
        .header(H, &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert!(p1.exists());
    assert_eq!(tomb("aaaa").await, 1);
    assert!(media::get(s.db(), "aaaa").await.unwrap().is_none());

    let r = c
        .delete(s.url("/admin/api/media/bbbb?delete_file=true"))
        .header(H, &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert!(!p2.exists());

    // Outside every configured root: never deleted, even with the flag.
    let r = c
        .delete(s.url("/admin/api/media/cccc?delete_file=true"))
        .header(H, &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert!(p3.exists());
    assert_eq!(tomb("cccc").await, 1);
}

#[tokio::test]
async fn bulk_add_tag_and_link_only() {
    let s = server().await;
    let (a, _) = upload(&s, 4).await;
    let (b, _) = upload(&s, 5).await;
    let (c, csrf) = login(&s, PW).await;
    let bulk = |body: Value| {
        let c = c.clone();
        let csrf = csrf.clone();
        let url = s.url("/admin/api/media/bulk");
        async move {
            c.post(url)
                .header(H, csrf)
                .json(&body)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };
    let j = bulk(json!({"action": "add_tag", "tag": "Best", "ids": [a, b, "nope"]})).await;
    assert_eq!(j["ok"], 2);
    assert_eq!(j["failed"], json!([{"id": "nope", "error": "not found"}]));
    let j = bulk(json!({"action": "link_only", "ids": [a, b]})).await;
    assert_eq!(j["ok"], 2);
    assert_eq!(library_total(&s).await, 0);
    let one: Value = c
        .get(s.url(&format!("/admin/api/media/{a}")))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(one["tags"][0]["name"], "Best");
    assert_eq!(one["public"], false);
    let j = bulk(json!({"action": "transcode", "ids": [a]})).await;
    assert_eq!(j["ok"], 0);
    assert_eq!(j["failed"][0]["error"], "not available yet");
}

#[tokio::test]
async fn tag_colour_validated_400() {
    let s = server().await;
    let (c, csrf) = login(&s, PW).await;
    let post = |body: Value| {
        c.post(s.url("/admin/api/tags"))
            .header(H, csrf.clone())
            .json(&body)
            .send()
    };
    assert_eq!(
        post(json!({"name": "a", "colour": "red"}))
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        post(json!({"name": "a", "colour": "#12345"}))
            .await
            .unwrap()
            .status(),
        400
    );
    let r = post(json!({"name": "a", "colour": "#12abEF"}))
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let id = r.json::<Value>().await.unwrap()["id"].as_i64().unwrap();
    assert_eq!(
        post(json!({"name": "A", "colour": "#000000"}))
            .await
            .unwrap()
            .status(),
        409
    );
    let r = c
        .patch(s.url(&format!("/admin/api/tags/{id}")))
        .header(H, &csrf)
        .json(&json!({"colour": "#zzzzzz"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    let r = c
        .patch(s.url(&format!("/admin/api/tags/{id}")))
        .header(H, &csrf)
        .json(&json!({"name": "b", "colour": "#000000"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let list: Value = c
        .get(s.url("/admin/api/tags"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list[0]["name"], "b");
    let r = c
        .delete(s.url(&format!("/admin/api/tags/{id}")))
        .header(H, &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn admin_api_requires_session_401() {
    let s = server().await;
    for (m, p) in [
        ("GET", "/admin/api/media"),
        ("GET", "/admin/api/tags"),
        ("PATCH", "/admin/api/media/x"),
        ("DELETE", "/admin/api/media/x"),
        ("POST", "/admin/api/media/bulk"),
        ("POST", "/admin/api/tags"),
    ] {
        let r = s
            .client
            .request(m.parse().unwrap(), s.url(p))
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 401, "{m} {p}");
    }
}

#[tokio::test]
async fn admin_api_requires_csrf_403() {
    let s = server().await;
    let (id, _) = upload(&s, 6).await;
    let (c, csrf) = login(&s, PW).await;
    for (m, p, body) in [
        (
            "PATCH",
            format!("/admin/api/media/{id}"),
            json!({"title": "x"}),
        ),
        ("DELETE", format!("/admin/api/media/{id}"), json!({})),
        (
            "POST",
            "/admin/api/media/bulk".to_string(),
            json!({"action": "public", "ids": [id]}),
        ),
        (
            "POST",
            "/admin/api/tags".to_string(),
            json!({"name": "q", "colour": "#000000"}),
        ),
        (
            "PATCH",
            "/admin/api/tags/1".to_string(),
            json!({"name": "q"}),
        ),
        ("DELETE", "/admin/api/tags/1".to_string(), json!({})),
    ] {
        for hdr in [None, Some("wrong")] {
            let mut req = c.request(m.parse().unwrap(), s.url(&p)).json(&body);
            if let Some(h) = hdr {
                req = req.header(H, h);
            }
            assert_eq!(req.send().await.unwrap().status(), 403, "{m} {p}");
        }
    }
    assert!(media::get(s.db(), &id).await.unwrap().is_some());
    let _ = csrf;
}

#[tokio::test]
async fn move_stays_inside_root() {
    let s = server().await;
    let (id, _) = upload(&s, 7).await;
    let (c, csrf) = login(&s, PW).await;
    let bulk = |folder: &'static str| {
        c.post(s.url("/admin/api/media/bulk"))
            .header(H, csrf.clone())
            .json(&json!({"action": "move", "folder": folder, "ids": [id]}))
            .send()
    };
    for bad in ["../x", "/abs", "a//b", "a\\b", ""] {
        let j: Value = bulk(bad).await.unwrap().json().await.unwrap();
        assert_eq!(j["ok"], 0, "{bad}");
    }
    let j: Value = bulk("sub/dir/").await.unwrap().json().await.unwrap();
    assert_eq!(j["ok"], 1, "{j}");
    let m = media::get(s.db(), &id).await.unwrap().unwrap();
    assert_eq!(m.folder, "sub/dir");
    assert!(std::path::Path::new(&m.path).exists());
    assert!(m.path.contains("/uploads/sub/dir/"));
}
