use chrono::{DateTime, Utc};
use sqlx::{FromRow, QueryBuilder, Row, Sqlite};

use super::tags::{self, Tag};
use super::{Db, Result};
use crate::media::kind::{MediaKind, Source};

#[derive(Debug, Clone, FromRow)]
pub struct Media {
    pub id: String,
    pub slug: String,
    pub kind: MediaKind,
    pub path: String,
    pub source: Source,
    pub folder: String,
    pub extension: String,
    pub size: i64,
    pub available: bool,
    pub title: String,
    pub description: String,
    pub public: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub recorded_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub codec_video: Option<String>,
    pub codec_audio: Option<String>,
    pub container: Option<String>,
    pub fps: Option<f64>,
    pub bitrate: Option<i64>,
    pub probe_json: Option<String>,
    pub has_poster: bool,
    pub has_preview: bool,
    pub has_thumb: bool,
    /// "pending" | "ready" | "failed"
    pub processing_state: String,
    pub processing_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewMedia {
    pub id: String,
    pub slug: String,
    pub kind: MediaKind,
    pub path: String,
    pub source: Source,
    pub folder: String,
    pub extension: String,
    pub size: i64,
    pub title: String,
    pub public: bool,
    pub recorded_at: Option<DateTime<Utc>>,
}

impl NewMedia {
    /// Test-only constructor (integration tests cannot see `cfg(test)`).
    /// Upload source, empty folder, size 0, private, title = file stem.
    pub fn test(id: &str, slug: &str, kind: MediaKind, path: &str) -> NewMedia {
        let p = std::path::Path::new(path);
        let part = |s: Option<&std::ffi::OsStr>| {
            s.map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        NewMedia {
            id: id.to_string(),
            slug: slug.to_string(),
            kind,
            path: path.to_string(),
            source: Source::Upload,
            folder: String::new(),
            extension: part(p.extension()),
            size: 0,
            title: part(p.file_stem()),
            public: false,
            recorded_at: None,
        }
    }
}

/// Fields written by a probe of the media file.
#[derive(Debug, Clone, Default)]
pub struct ProbeFields {
    pub duration_ms: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub codec_video: Option<String>,
    pub codec_audio: Option<String>,
    pub container: Option<String>,
    pub fps: Option<f64>,
    pub bitrate: Option<i64>,
    pub probe_json: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    Newest,
    Oldest,
    MostViewed,
    Title,
    Longest,
}

#[derive(Debug, Clone)]
pub struct ListQuery {
    pub kind: Option<MediaKind>,
    pub public_only: bool,
    pub q: Option<String>,
    pub tag: Option<String>,
    pub sort: Sort,
    /// 1-based.
    pub page: u32,
    pub per_page: u32,
}

#[derive(Debug, Clone)]
pub struct MediaWithViews {
    pub media: Media,
    pub views: i64,
    pub tags: Vec<Tag>,
}

pub async fn insert(db: &Db, m: &NewMedia) -> Result<()> {
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO media (id, slug, kind, path, source, folder, extension, size, title, \
         public, created_at, updated_at, recorded_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&m.id)
    .bind(&m.slug)
    .bind(m.kind)
    .bind(&m.path)
    .bind(m.source)
    .bind(&m.folder)
    .bind(&m.extension)
    .bind(m.size)
    .bind(&m.title)
    .bind(m.public)
    .bind(now)
    .bind(now)
    .bind(m.recorded_at)
    .execute(&db.0)
    .await?;
    Ok(())
}

async fn get_where(db: &Db, column: &str, value: &str) -> Result<Option<Media>> {
    let sql = format!("SELECT * FROM media WHERE {column} = ?");
    Ok(sqlx::query_as::<_, Media>(&sql)
        .bind(value)
        .fetch_optional(&db.0)
        .await?)
}

pub async fn get(db: &Db, id: &str) -> Result<Option<Media>> {
    get_where(db, "id", id).await
}

pub async fn get_by_slug(db: &Db, slug: &str) -> Result<Option<Media>> {
    get_where(db, "slug", slug).await
}

pub async fn get_by_path(db: &Db, path: &str) -> Result<Option<Media>> {
    get_where(db, "path", path).await
}

pub async fn slug_exists(db: &Db, slug: &str) -> Result<bool> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media WHERE slug = ?")
        .bind(slug)
        .fetch_one(&db.0)
        .await?;
    Ok(n > 0)
}

pub async fn set_available(db: &Db, id: &str, available: bool) -> Result<()> {
    sqlx::query("UPDATE media SET available = ?, updated_at = ? WHERE id = ?")
        .bind(available)
        .bind(Utc::now())
        .bind(id)
        .execute(&db.0)
        .await?;
    Ok(())
}

/// Point a media row at a new path (file moved or restored) and mark it available.
pub async fn relink(db: &Db, id: &str, new_path: &str) -> Result<()> {
    sqlx::query("UPDATE media SET path = ?, available = 1, updated_at = ? WHERE id = ?")
        .bind(new_path)
        .bind(Utc::now())
        .bind(id)
        .execute(&db.0)
        .await?;
    Ok(())
}

pub async fn update_details(db: &Db, id: &str, title: &str, description: &str) -> Result<()> {
    sqlx::query("UPDATE media SET title = ?, description = ?, updated_at = ? WHERE id = ?")
        .bind(title)
        .bind(description)
        .bind(Utc::now())
        .bind(id)
        .execute(&db.0)
        .await?;
    Ok(())
}

pub async fn set_public(db: &Db, ids: &[String], public: bool) -> Result<()> {
    let mut tx = db.0.begin().await?;
    for id in ids {
        sqlx::query("UPDATE media SET public = ?, updated_at = ? WHERE id = ?")
            .bind(public)
            .bind(Utc::now())
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn set_probe(db: &Db, id: &str, p: &ProbeFields) -> Result<()> {
    sqlx::query(
        "UPDATE media SET duration_ms = ?, width = ?, height = ?, codec_video = ?, \
         codec_audio = ?, container = ?, fps = ?, bitrate = ?, probe_json = ?, \
         updated_at = ? WHERE id = ?",
    )
    .bind(p.duration_ms)
    .bind(p.width)
    .bind(p.height)
    .bind(&p.codec_video)
    .bind(&p.codec_audio)
    .bind(&p.container)
    .bind(p.fps)
    .bind(p.bitrate)
    .bind(&p.probe_json)
    .bind(Utc::now())
    .bind(id)
    .execute(&db.0)
    .await?;
    Ok(())
}

/// Update only the flags that are `Some`.
pub async fn set_flags(
    db: &Db,
    id: &str,
    has_poster: Option<bool>,
    has_preview: Option<bool>,
    has_thumb: Option<bool>,
) -> Result<()> {
    sqlx::query(
        "UPDATE media SET has_poster = COALESCE(?, has_poster), \
         has_preview = COALESCE(?, has_preview), has_thumb = COALESCE(?, has_thumb), \
         updated_at = ? WHERE id = ?",
    )
    .bind(has_poster)
    .bind(has_preview)
    .bind(has_thumb)
    .bind(Utc::now())
    .bind(id)
    .execute(&db.0)
    .await?;
    Ok(())
}

pub async fn set_processing(db: &Db, id: &str, state: &str, error: Option<&str>) -> Result<()> {
    sqlx::query(
        "UPDATE media SET processing_state = ?, processing_error = ?, updated_at = ? WHERE id = ?",
    )
    .bind(state)
    .bind(error)
    .bind(Utc::now())
    .bind(id)
    .execute(&db.0)
    .await?;
    Ok(())
}

pub async fn delete(db: &Db, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM media WHERE id = ?")
        .bind(id)
        .execute(&db.0)
        .await?;
    Ok(())
}

const SORT_NEWEST: &str = "COALESCE(m.recorded_at, m.created_at)";

fn push_filters<'a>(qb: &mut QueryBuilder<'a, Sqlite>, q: &'a ListQuery) {
    qb.push(" WHERE 1 = 1");
    if let Some(kind) = q.kind {
        qb.push(" AND m.kind = ").push_bind(kind);
    }
    if q.public_only {
        qb.push(" AND m.public = 1 AND m.available = 1");
    }
    if let Some(tag) = &q.tag {
        qb.push(
            " AND m.id IN (SELECT mt.media_id FROM media_tag mt \
             JOIN tag t ON t.id = mt.tag_id WHERE t.name = ",
        )
        .push_bind(tag.as_str())
        .push(")");
    }
    if let Some(text) = q.q.as_deref().filter(|s| !s.trim().is_empty()) {
        let like = format!("%{}%", escape_like(&text.trim().to_lowercase()));
        qb.push(" AND (LOWER(m.title) LIKE ")
            .push_bind(like.clone())
            .push(" ESCAPE '\\' OR LOWER(m.description) LIKE ")
            .push_bind(like.clone())
            .push(
                " ESCAPE '\\' OR m.id IN (SELECT mt.media_id FROM media_tag mt \
                 JOIN tag t ON t.id = mt.tag_id WHERE LOWER(t.name) LIKE ",
            )
            .push_bind(like)
            .push(" ESCAPE '\\'))");
    }
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub async fn list(db: &Db, q: &ListQuery) -> Result<Vec<MediaWithViews>> {
    let mut qb = QueryBuilder::<Sqlite>::new(
        "SELECT m.*, (SELECT COUNT(*) FROM view v WHERE v.media_id = m.id) AS views FROM media m",
    );
    push_filters(&mut qb, q);
    let order = match q.sort {
        Sort::Newest => format!(" ORDER BY {SORT_NEWEST} DESC, m.id"),
        Sort::Oldest => format!(" ORDER BY {SORT_NEWEST} ASC, m.id"),
        Sort::MostViewed => format!(" ORDER BY views DESC, {SORT_NEWEST} DESC, m.id"),
        Sort::Title => " ORDER BY m.title COLLATE NOCASE ASC, m.id".to_string(),
        Sort::Longest => " ORDER BY m.duration_ms DESC NULLS LAST, m.id".to_string(),
    };
    qb.push(order);
    let offset = i64::from(q.page.max(1) - 1) * i64::from(q.per_page);
    qb.push(" LIMIT ")
        .push_bind(i64::from(q.per_page))
        .push(" OFFSET ")
        .push_bind(offset);

    let rows = qb.build().fetch_all(&db.0).await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let media = Media::from_row(row)?;
        let views: i64 = row.try_get("views")?;
        let tags = tags::for_media(db, &media.id).await?;
        out.push(MediaWithViews { media, views, tags });
    }
    Ok(out)
}

pub async fn count(db: &Db, kind: Option<MediaKind>, public_only: bool) -> Result<i64> {
    let mut qb = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM media m WHERE 1 = 1");
    if let Some(kind) = kind {
        qb.push(" AND m.kind = ").push_bind(kind);
    }
    if public_only {
        qb.push(" AND m.public = 1 AND m.available = 1");
    }
    Ok(qb.build_query_scalar().fetch_one(&db.0).await?)
}

/// Number of rows matching the filters of `q` (ignores sort and paging).
pub async fn count_matching(db: &Db, q: &ListQuery) -> Result<i64> {
    let mut qb = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM media m");
    push_filters(&mut qb, q);
    Ok(qb.build_query_scalar().fetch_one(&db.0).await?)
}

pub async fn all_ids(db: &Db) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar("SELECT id FROM media ORDER BY id")
        .fetch_all(&db.0)
        .await?)
}
