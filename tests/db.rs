use std::time::Duration;

use chrono::TimeZone;

use fthr_clips_server::db::media::{ListQuery, NewMedia, Sort};
use fthr_clips_server::db::{self, Db};
use fthr_clips_server::media::kind::MediaKind;

fn query(public_only: bool) -> ListQuery {
    ListQuery {
        kind: None,
        public_only,
        q: None,
        tag: None,
        sort: Sort::Newest,
        page: 1,
        per_page: 50,
    }
}

#[tokio::test]
async fn migrates_and_round_trips_media() {
    let db = Db::open_memory().await.unwrap();
    db::media::insert(
        &db,
        &NewMedia::test("a1", "slugA", MediaKind::Video, "/v/a.mp4"),
    )
    .await
    .unwrap();
    let m = db::media::get(&db, "a1").await.unwrap().unwrap();
    assert_eq!(m.slug, "slugA");
    assert_eq!(m.kind, MediaKind::Video);
    assert_eq!(m.title, "a");
    assert_eq!(m.extension, "mp4");
    assert!(m.available);
    assert!(!m.public);
    assert_eq!(
        db::media::get_by_slug(&db, "slugA")
            .await
            .unwrap()
            .unwrap()
            .id,
        "a1"
    );
    assert!(db::media::slug_exists(&db, "slugA").await.unwrap());
    assert!(!db::media::slug_exists(&db, "nope").await.unwrap());
}

#[tokio::test]
async fn list_filters_public_and_tag_and_search() {
    let db = Db::open_memory().await.unwrap();
    let mut first = NewMedia::test("m1", "s1", MediaKind::Video, "/v/first.mp4");
    first.public = true;
    let mut second = NewMedia::test("m2", "s2", MediaKind::Video, "/v/second.mp4");
    second.public = true;
    let third = NewMedia::test("m3", "s3", MediaKind::Image, "/i/third.png");
    for m in [&first, &second, &third] {
        db::media::insert(&db, m).await.unwrap();
    }
    db::tags::attach_by_name(&db, &["m1".to_string()], "x")
        .await
        .unwrap();

    assert_eq!(db::media::list(&db, &query(true)).await.unwrap().len(), 2);
    assert_eq!(db::media::list(&db, &query(false)).await.unwrap().len(), 3);

    let tagged = db::media::list(
        &db,
        &ListQuery {
            tag: Some("x".into()),
            ..query(false)
        },
    )
    .await
    .unwrap();
    assert_eq!(tagged.len(), 1);
    assert_eq!(tagged[0].media.id, "m1");
    assert_eq!(tagged[0].tags[0].name, "x");

    let found = db::media::list(
        &db,
        &ListQuery {
            q: Some("Second".into()),
            ..query(false)
        },
    )
    .await
    .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].media.id, "m2");

    // Unavailable rows are hidden from public listings only.
    db::media::set_available(&db, "m1", false).await.unwrap();
    assert_eq!(db::media::list(&db, &query(true)).await.unwrap().len(), 1);
    assert_eq!(db::media::list(&db, &query(false)).await.unwrap().len(), 3);
    assert_eq!(db::media::count(&db, None, true).await.unwrap(), 1);
    assert_eq!(
        db::media::count(&db, Some(MediaKind::Image), false)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn list_sorts_and_paginates() {
    let db = Db::open_memory().await.unwrap();
    for (id, title) in [("a", "banana"), ("b", "Apple"), ("c", "cherry")] {
        let mut m = NewMedia::test(id, id, MediaKind::Video, "/v/x.mp4");
        m.title = title.into();
        db::media::insert(&db, &m).await.unwrap();
    }
    db::views::record(&db, "c", "h1").await.unwrap();
    db::views::record(&db, "c", "h2").await.unwrap();
    db::views::record(&db, "a", "h1").await.unwrap();

    let ids = |v: Vec<db::media::MediaWithViews>| -> Vec<String> {
        v.into_iter().map(|m| m.media.id).collect()
    };
    let sorted = |sort| ListQuery {
        sort,
        ..query(false)
    };
    assert_eq!(
        ids(db::media::list(&db, &sorted(Sort::Title)).await.unwrap()),
        ["b", "a", "c"]
    );
    let viewed = db::media::list(&db, &sorted(Sort::MostViewed))
        .await
        .unwrap();
    assert_eq!(viewed[0].media.id, "c");
    assert_eq!(viewed[0].views, 2);
    assert_eq!(viewed[1].media.id, "a");

    let page2 = db::media::list(
        &db,
        &ListQuery {
            sort: Sort::Title,
            page: 2,
            per_page: 2,
            ..query(false)
        },
    )
    .await
    .unwrap();
    assert_eq!(ids(page2), ["c"]);
}

#[tokio::test]
async fn views_dedupe_by_ip_hash() {
    let db = Db::open_memory().await.unwrap();
    db::media::insert(
        &db,
        &NewMedia::test("a1", "s", MediaKind::Video, "/v/a.mp4"),
    )
    .await
    .unwrap();
    assert!(db::views::record(&db, "a1", "h").await.unwrap());
    assert!(!db::views::record(&db, "a1", "h").await.unwrap());
    assert_eq!(db::views::count(&db, "a1").await.unwrap(), 1);
}

#[tokio::test]
async fn sessions_expire() {
    let db = Db::open_memory().await.unwrap();
    let dead = db::sessions::create(&db, Duration::ZERO).await.unwrap();
    assert!(db::sessions::get_valid(&db, &dead.id)
        .await
        .unwrap()
        .is_none());
    let live = db::sessions::create(&db, Duration::from_secs(60))
        .await
        .unwrap();
    assert_eq!(
        db::sessions::get_valid(&db, &live.id)
            .await
            .unwrap()
            .unwrap()
            .csrf,
        live.csrf
    );
    assert_eq!(db::sessions::purge_expired(&db).await.unwrap(), 1);
}

#[tokio::test]
async fn settings_and_tokens_round_trip() {
    let db = Db::open_memory().await.unwrap();
    db::settings::set_json(&db, "k", &serde_json::json!({"a": 1}))
        .await
        .unwrap();
    db::settings::set_json(&db, "k", &serde_json::json!(2))
        .await
        .unwrap();
    assert_eq!(
        db::settings::get_json(&db, "k").await.unwrap(),
        Some(serde_json::json!(2))
    );
    assert_eq!(db::settings::all(&db).await.unwrap().len(), 1);

    let t = db::tokens::create(&db, "cli", "hash", "abc").await.unwrap();
    assert!(db::tokens::find_active_by_hash(&db, "hash")
        .await
        .unwrap()
        .is_some());
    db::tokens::revoke(&db, t.id).await.unwrap();
    assert!(db::tokens::find_active_by_hash(&db, "hash")
        .await
        .unwrap()
        .is_none());
    assert_eq!(db::tokens::list(&db).await.unwrap().len(), 1);
}

#[tokio::test]
async fn open_creates_parent_dir_and_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("nested/library.db");
    Db::open(&path).await.unwrap();
    assert!(path.exists());
}

fn ids(v: Vec<db::media::MediaWithViews>) -> Vec<String> {
    v.into_iter().map(|m| m.media.id).collect()
}

async fn add(db: &Db, id: &str, title: &str) {
    let mut m = NewMedia::test(id, id, MediaKind::Video, &format!("/v/{id}.mp4"));
    m.title = title.into();
    db::media::insert(db, &m).await.unwrap();
}

#[tokio::test]
async fn media_mutations_and_lookups() {
    let db = Db::open_memory().await.unwrap();
    add(&db, "a", "A").await;
    add(&db, "b", "B").await;

    assert_eq!(
        db::media::get_by_path(&db, "/v/a.mp4")
            .await
            .unwrap()
            .unwrap()
            .id,
        "a"
    );
    assert!(db::media::get_by_path(&db, "/nope")
        .await
        .unwrap()
        .is_none());
    assert_eq!(db::media::all_ids(&db).await.unwrap(), ["a", "b"]);

    db::media::set_available(&db, "a", false).await.unwrap();
    db::media::relink(&db, "a", "/moved/a.mp4").await.unwrap();
    let m = db::media::get(&db, "a").await.unwrap().unwrap();
    assert_eq!(m.path, "/moved/a.mp4");
    assert!(m.available);

    db::media::update_details(&db, "a", "New", "Desc")
        .await
        .unwrap();
    let m = db::media::get(&db, "a").await.unwrap().unwrap();
    assert_eq!((m.title.as_str(), m.description.as_str()), ("New", "Desc"));

    db::media::set_public(&db, &["a".into(), "b".into()], true)
        .await
        .unwrap();
    assert_eq!(db::media::count(&db, None, true).await.unwrap(), 2);
    db::media::set_public(&db, &["b".into()], false)
        .await
        .unwrap();
    assert_eq!(db::media::count(&db, None, true).await.unwrap(), 1);

    let probe = db::media::ProbeFields {
        duration_ms: Some(1500),
        width: Some(1920),
        height: Some(1080),
        codec_video: Some("h264".into()),
        codec_audio: Some("aac".into()),
        container: Some("mp4".into()),
        fps: Some(29.97),
        bitrate: Some(5000),
        probe_json: Some("{}".into()),
    };
    db::media::set_probe(&db, "a", &probe).await.unwrap();
    let m = db::media::get(&db, "a").await.unwrap().unwrap();
    assert_eq!(m.duration_ms, Some(1500));
    assert_eq!((m.width, m.height), (Some(1920), Some(1080)));
    assert_eq!(m.codec_video.as_deref(), Some("h264"));
    assert_eq!(m.codec_audio.as_deref(), Some("aac"));
    assert_eq!(m.container.as_deref(), Some("mp4"));
    assert_eq!(m.fps, Some(29.97));
    assert_eq!(m.bitrate, Some(5000));
    assert_eq!(m.probe_json.as_deref(), Some("{}"));

    db::media::set_flags(&db, "a", Some(true), None, Some(true))
        .await
        .unwrap();
    let m = db::media::get(&db, "a").await.unwrap().unwrap();
    assert!(m.has_poster && !m.has_preview && m.has_thumb);
    db::media::set_flags(&db, "a", None, Some(true), Some(false))
        .await
        .unwrap();
    let m = db::media::get(&db, "a").await.unwrap().unwrap();
    assert!(m.has_poster && m.has_preview && !m.has_thumb);

    assert_eq!(
        db::media::get(&db, "a")
            .await
            .unwrap()
            .unwrap()
            .processing_state,
        "pending"
    );
    db::media::set_processing(&db, "a", "failed", Some("boom"))
        .await
        .unwrap();
    let m = db::media::get(&db, "a").await.unwrap().unwrap();
    assert_eq!(m.processing_state, "failed");
    assert_eq!(m.processing_error.as_deref(), Some("boom"));
}

#[tokio::test]
async fn delete_cascades_to_dependent_rows() {
    let db = Db::open_memory().await.unwrap();
    add(&db, "a", "A").await;
    db::tags::attach_by_name(&db, &["a".into()], "t")
        .await
        .unwrap();
    db::views::record(&db, "a", "h").await.unwrap();
    db::jobs::enqueue(&db, "a", "probe").await.unwrap();
    db::jobs::rendition_upsert(&db, &rendition("a", 720, "queued"))
        .await
        .unwrap();

    db::media::delete(&db, "a").await.unwrap();
    assert!(db::media::get(&db, "a").await.unwrap().is_none());
    for table in ["media_tag", "view", "job", "rendition"] {
        let n: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&db.0)
            .await
            .unwrap();
        assert_eq!(n, 0, "{table} not cascaded");
    }
    // The tag itself survives.
    assert_eq!(db::tags::list_all(&db).await.unwrap().len(), 1);
}

#[tokio::test]
async fn list_oldest_and_longest_sorts() {
    let db = Db::open_memory().await.unwrap();
    for (id, year, dur) in [
        ("a", 2021, Some(10)),
        ("b", 2020, Some(30)),
        ("c", 2022, None),
    ] {
        let mut m = NewMedia::test(id, id, MediaKind::Video, "/v/x.mp4");
        m.recorded_at = Some(chrono::Utc.with_ymd_and_hms(year, 1, 1, 0, 0, 0).unwrap());
        db::media::insert(&db, &m).await.unwrap();
        let probe = db::media::ProbeFields {
            duration_ms: dur,
            ..Default::default()
        };
        db::media::set_probe(&db, id, &probe).await.unwrap();
    }
    let sorted = |sort| ListQuery {
        sort,
        ..query(false)
    };
    assert_eq!(
        ids(db::media::list(&db, &sorted(Sort::Oldest)).await.unwrap()),
        ["b", "a", "c"]
    );
    assert_eq!(
        ids(db::media::list(&db, &sorted(Sort::Newest)).await.unwrap()),
        ["c", "a", "b"]
    );
    assert_eq!(
        ids(db::media::list(&db, &sorted(Sort::Longest)).await.unwrap()),
        ["b", "a", "c"]
    );
}

#[tokio::test]
async fn search_matches_description_tag_and_escapes_like() {
    let db = Db::open_memory().await.unwrap();
    add(&db, "a", "100% real").await;
    add(&db, "b", "plain").await;
    add(&db, "c", "under_score").await;
    add(&db, "d", "underXscore").await;
    db::media::update_details(&db, "b", "plain", "has a Needle inside")
        .await
        .unwrap();
    db::tags::attach_by_name(&db, &["d".into()], "Gadget")
        .await
        .unwrap();

    let search = |q: &str| ListQuery {
        q: Some(q.into()),
        ..query(false)
    };
    assert_eq!(
        ids(db::media::list(&db, &search("needle")).await.unwrap()),
        ["b"]
    );
    assert_eq!(
        ids(db::media::list(&db, &search("gadg")).await.unwrap()),
        ["d"]
    );
    assert_eq!(
        ids(db::media::list(&db, &search("%")).await.unwrap()),
        ["a"]
    );
    assert_eq!(
        ids(db::media::list(&db, &search("r_s")).await.unwrap()),
        ["c"]
    );
}

#[tokio::test]
async fn tags_crud_and_attachment() {
    let db = Db::open_memory().await.unwrap();
    add(&db, "a", "A").await;
    let t = db::tags::create(&db, "one", "#111111").await.unwrap();
    assert_eq!((t.name.as_str(), t.colour.as_str()), ("one", "#111111"));
    db::tags::rename(&db, t.id, "uno").await.unwrap();
    db::tags::recolour(&db, t.id, "#222222").await.unwrap();
    let all = db::tags::list_all(&db).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(
        (all[0].name.as_str(), all[0].colour.as_str()),
        ("uno", "#222222")
    );

    db::tags::attach(&db, "a", t.id).await.unwrap();
    db::tags::attach(&db, "a", t.id).await.unwrap(); // idempotent
    assert_eq!(
        db::tags::for_media(&db, "a").await.unwrap(),
        all_with(&t, "uno", "#222222")
    );
    db::tags::detach(&db, "a", t.id).await.unwrap();
    assert!(db::tags::for_media(&db, "a").await.unwrap().is_empty());

    let made = db::tags::attach_by_name(&db, &["a".into()], "fresh")
        .await
        .unwrap();
    assert_eq!(made.colour, "#20e3a0");
    let again = db::tags::attach_by_name(&db, &["a".into()], "fresh")
        .await
        .unwrap();
    assert_eq!(made.id, again.id);

    db::tags::delete(&db, t.id).await.unwrap();
    assert_eq!(db::tags::list_all(&db).await.unwrap().len(), 1);
}

fn all_with(t: &db::Tag, name: &str, colour: &str) -> Vec<db::Tag> {
    vec![db::Tag {
        id: t.id,
        name: name.into(),
        colour: colour.into(),
    }]
}

fn rendition(media_id: &str, height: i64, state: &str) -> db::jobs::Rendition {
    db::jobs::Rendition {
        media_id: media_id.into(),
        height,
        path: String::new(),
        size: 0,
        state: state.into(),
        progress: 0.0,
        error: None,
    }
}

#[tokio::test]
async fn job_lifecycle() {
    let db = Db::open_memory().await.unwrap();
    add(&db, "a", "A").await;
    let id = db::jobs::enqueue(&db, "a", "probe").await.unwrap();
    assert_eq!(db::jobs::enqueue(&db, "a", "probe").await.unwrap(), id);
    assert_ne!(db::jobs::enqueue(&db, "a", "thumb").await.unwrap(), id);
    assert_eq!(db::jobs::list_active(&db).await.unwrap().len(), 2);

    assert!(db::jobs::claim_next(&db, &["other"])
        .await
        .unwrap()
        .is_none());
    let job = db::jobs::claim_next(&db, &["probe"])
        .await
        .unwrap()
        .unwrap();
    assert_eq!((job.id, job.state.as_str()), (id, "running"));
    db::jobs::set_progress(&db, id, 0.5).await.unwrap();
    assert_eq!(
        db::jobs::list_for_media(&db, "a").await.unwrap()[0].progress,
        0.5
    );

    db::jobs::fail(&db, id, "bad").await.unwrap();
    let j = &db::jobs::list_for_media(&db, "a").await.unwrap()[0];
    assert_eq!(
        (j.state.as_str(), j.error.as_deref()),
        ("failed", Some("bad"))
    );
    assert_eq!(db::jobs::list_active(&db).await.unwrap().len(), 1);

    // A failed job no longer blocks a fresh enqueue.
    let fresh = db::jobs::enqueue(&db, "a", "probe").await.unwrap();
    assert_ne!(fresh, id);
    db::jobs::cancel(&db, fresh).await.unwrap();
    let all = db::jobs::list_for_media(&db, "a").await.unwrap();
    assert_eq!(
        all.iter().find(|j| j.id == fresh).unwrap().state,
        "cancelled"
    );

    assert!(db::jobs::retry(&db, id).await.unwrap());
    let j = db::jobs::list_for_media(&db, "a").await.unwrap();
    let j = j.iter().find(|j| j.id == id).unwrap();
    assert_eq!((j.state.as_str(), j.error.as_deref()), ("queued", None));
    db::jobs::finish(&db, id).await.unwrap();
    assert_eq!(db::jobs::list_active(&db).await.unwrap().len(), 1);
}

#[tokio::test]
async fn claim_next_is_atomic_under_concurrency() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = Db::open(&dir.path().join("library.db")).await.unwrap();
    add(&db, "a", "A").await;
    db::jobs::enqueue(&db, "a", "probe").await.unwrap();

    let mut tasks = Vec::new();
    for _ in 0..8 {
        let db = db.clone();
        tasks.push(tokio::spawn(async move {
            db::jobs::claim_next(&db, &["probe"]).await.unwrap()
        }));
    }
    let mut claimed = 0;
    for t in tasks {
        if t.await.unwrap().is_some() {
            claimed += 1;
        }
    }
    assert_eq!(claimed, 1);
}

#[tokio::test]
async fn enqueue_is_idempotent_under_concurrency() {
    let dir = tempfile::TempDir::new().unwrap();
    let db = Db::open(&dir.path().join("library.db")).await.unwrap();
    add(&db, "a", "A").await;
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let db = db.clone();
        tasks.push(tokio::spawn(async move {
            db::jobs::enqueue(&db, "a", "probe").await.unwrap()
        }));
    }
    let mut got = std::collections::HashSet::new();
    for t in tasks {
        got.insert(t.await.unwrap());
    }
    assert_eq!(got.len(), 1);
    assert_eq!(db::jobs::list_for_media(&db, "a").await.unwrap().len(), 1);
}

#[tokio::test]
async fn renditions_upsert_ready_delete() {
    let db = Db::open_memory().await.unwrap();
    add(&db, "a", "A").await;
    db::jobs::rendition_upsert(&db, &rendition("a", 720, "queued"))
        .await
        .unwrap();
    db::jobs::rendition_upsert(&db, &rendition("a", 480, "queued"))
        .await
        .unwrap();
    // Conflict path: same (media, height) updates in place.
    let mut r = rendition("a", 720, "running");
    r.progress = 0.4;
    db::jobs::rendition_upsert(&db, &r).await.unwrap();
    let all = db::jobs::renditions_for(&db, "a").await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(
        (all[0].height, all[0].state.as_str(), all[0].progress),
        (720, "running", 0.4)
    );

    db::jobs::rendition_ready(&db, "a", 720, "/r/720.mp4", 99)
        .await
        .unwrap();
    let all = db::jobs::renditions_for(&db, "a").await.unwrap();
    assert_eq!(
        (all[0].state.as_str(), all[0].path.as_str(), all[0].size),
        ("ready", "/r/720.mp4", 99)
    );
    db::jobs::rendition_delete(&db, "a", 720).await.unwrap();
    let all = db::jobs::renditions_for(&db, "a").await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].height, 480);
}

#[tokio::test]
async fn retry_refuses_when_an_active_job_exists() {
    let db = Db::open_memory().await.unwrap();
    add(&db, "a", "A").await;
    let a = db::jobs::enqueue(&db, "a", "probe").await.unwrap();
    db::jobs::claim_next(&db, &["probe"]).await.unwrap();
    db::jobs::fail(&db, a, "bad").await.unwrap();
    let b = db::jobs::enqueue(&db, "a", "probe").await.unwrap();
    assert_ne!(a, b);

    assert!(!db::jobs::retry(&db, a).await.unwrap());
    let jobs = db::jobs::list_for_media(&db, "a").await.unwrap();
    assert_eq!(jobs.iter().find(|j| j.id == a).unwrap().state, "failed");

    db::jobs::cancel(&db, b).await.unwrap();
    assert!(db::jobs::retry(&db, a).await.unwrap());
    let jobs = db::jobs::list_for_media(&db, "a").await.unwrap();
    assert_eq!(jobs.iter().find(|j| j.id == a).unwrap().state, "queued");
    // Not retryable once active.
    assert!(!db::jobs::retry(&db, a).await.unwrap());
}
