use std::time::Duration;

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
async fn jobs_enqueue_is_idempotent_and_claim_is_atomic() {
    let db = Db::open_memory().await.unwrap();
    db::media::insert(
        &db,
        &NewMedia::test("a1", "s", MediaKind::Video, "/v/a.mp4"),
    )
    .await
    .unwrap();
    let first = db::jobs::enqueue(&db, "a1", "probe").await.unwrap();
    let second = db::jobs::enqueue(&db, "a1", "probe").await.unwrap();
    assert_eq!(first, second);
    assert_eq!(db::jobs::list_for_media(&db, "a1").await.unwrap().len(), 1);

    assert!(db::jobs::claim_next(&db, &["thumb"])
        .await
        .unwrap()
        .is_none());
    let job = db::jobs::claim_next(&db, &["probe"])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.id, first);
    assert_eq!(job.state, "running");
    assert!(db::jobs::claim_next(&db, &["probe"])
        .await
        .unwrap()
        .is_none());

    // A running job still blocks a duplicate; a finished one does not.
    assert_eq!(db::jobs::enqueue(&db, "a1", "probe").await.unwrap(), first);
    db::jobs::finish(&db, first).await.unwrap();
    assert_ne!(db::jobs::enqueue(&db, "a1", "probe").await.unwrap(), first);
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
