use chrono::{DateTime, Utc};
use sqlx::{FromRow, QueryBuilder, Sqlite};

use super::{Db, Result};

#[derive(Debug, Clone, FromRow)]
pub struct Job {
    pub id: i64,
    pub media_id: String,
    pub kind: String,
    /// "queued" | "running" | "done" | "failed" | "cancelled"
    pub state: String,
    pub attempts: i64,
    pub progress: f64,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct Rendition {
    pub media_id: String,
    pub height: i64,
    pub path: String,
    pub size: i64,
    pub state: String,
    pub progress: f64,
    pub error: Option<String>,
}

/// Queue a job. If a queued or running job for the same `(media_id, kind)`
/// exists, nothing is inserted and its id is returned.
pub async fn enqueue(db: &Db, media_id: &str, kind: &str) -> Result<i64> {
    let now = Utc::now();
    let mut tx = db.0.begin().await?;
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM job WHERE media_id = ? AND kind = ? \
         AND state IN ('queued', 'running') ORDER BY id LIMIT 1",
    )
    .bind(media_id)
    .bind(kind)
    .fetch_optional(&mut *tx)
    .await?;
    let id = match existing {
        Some(id) => id,
        None => {
            sqlx::query_scalar(
                "INSERT INTO job (media_id, kind, created_at, updated_at) \
                 VALUES (?, ?, ?, ?) RETURNING id",
            )
            .bind(media_id)
            .bind(kind)
            .bind(now)
            .bind(now)
            .fetch_one(&mut *tx)
            .await?
        }
    };
    tx.commit().await?;
    Ok(id)
}

/// Atomically move the oldest queued job of one of `kinds` to running.
pub async fn claim_next(db: &Db, kinds: &[&str]) -> Result<Option<Job>> {
    if kinds.is_empty() {
        return Ok(None);
    }
    let mut qb = QueryBuilder::<Sqlite>::new("UPDATE job SET state = 'running', updated_at = ");
    qb.push_bind(Utc::now());
    qb.push(", attempts = attempts + 1 WHERE id = (SELECT id FROM job WHERE state = 'queued' AND kind IN (");
    let mut sep = qb.separated(", ");
    for kind in kinds {
        sep.push_bind(*kind);
    }
    qb.push(") ORDER BY id LIMIT 1) RETURNING *");
    Ok(qb.build_query_as::<Job>().fetch_optional(&db.0).await?)
}

async fn set_state(
    db: &Db,
    id: i64,
    state: &str,
    error: Option<&str>,
    only_from: &[&str],
) -> Result<()> {
    let mut qb = QueryBuilder::<Sqlite>::new("UPDATE job SET state = ");
    qb.push_bind(state)
        .push(", error = ")
        .push_bind(error)
        .push(", updated_at = ")
        .push_bind(Utc::now())
        .push(" WHERE id = ")
        .push_bind(id);
    if !only_from.is_empty() {
        qb.push(" AND state IN (");
        let mut sep = qb.separated(", ");
        for s in only_from {
            sep.push_bind(*s);
        }
        qb.push(")");
    }
    qb.build().execute(&db.0).await?;
    Ok(())
}

pub async fn set_progress(db: &Db, id: i64, progress: f64) -> Result<()> {
    sqlx::query("UPDATE job SET progress = ?, updated_at = ? WHERE id = ?")
        .bind(progress)
        .bind(Utc::now())
        .bind(id)
        .execute(&db.0)
        .await?;
    Ok(())
}

pub async fn finish(db: &Db, id: i64) -> Result<()> {
    sqlx::query(
        "UPDATE job SET state = 'done', progress = 1, error = NULL, updated_at = ? WHERE id = ?",
    )
    .bind(Utc::now())
    .bind(id)
    .execute(&db.0)
    .await?;
    Ok(())
}

pub async fn fail(db: &Db, id: i64, error: &str) -> Result<()> {
    set_state(db, id, "failed", Some(error), &[]).await
}

pub async fn cancel(db: &Db, id: i64) -> Result<()> {
    set_state(db, id, "cancelled", None, &["queued", "running"]).await
}

/// Requeue a failed job.
pub async fn retry(db: &Db, id: i64) -> Result<()> {
    sqlx::query(
        "UPDATE job SET state = 'queued', progress = 0, error = NULL, updated_at = ? \
         WHERE id = ? AND state = 'failed'",
    )
    .bind(Utc::now())
    .bind(id)
    .execute(&db.0)
    .await?;
    Ok(())
}

pub async fn list_active(db: &Db) -> Result<Vec<Job>> {
    Ok(
        sqlx::query_as("SELECT * FROM job WHERE state IN ('queued', 'running') ORDER BY id")
            .fetch_all(&db.0)
            .await?,
    )
}

pub async fn list_for_media(db: &Db, media_id: &str) -> Result<Vec<Job>> {
    Ok(
        sqlx::query_as("SELECT * FROM job WHERE media_id = ? ORDER BY id")
            .bind(media_id)
            .fetch_all(&db.0)
            .await?,
    )
}

pub async fn rendition_upsert(db: &Db, r: &Rendition) -> Result<()> {
    sqlx::query(
        "INSERT INTO rendition (media_id, height, path, size, state, progress, error) \
         VALUES (?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(media_id, height) DO UPDATE SET path = excluded.path, \
         size = excluded.size, state = excluded.state, progress = excluded.progress, \
         error = excluded.error",
    )
    .bind(&r.media_id)
    .bind(r.height)
    .bind(&r.path)
    .bind(r.size)
    .bind(&r.state)
    .bind(r.progress)
    .bind(&r.error)
    .execute(&db.0)
    .await?;
    Ok(())
}

pub async fn renditions_for(db: &Db, media_id: &str) -> Result<Vec<Rendition>> {
    Ok(
        sqlx::query_as("SELECT * FROM rendition WHERE media_id = ? ORDER BY height DESC")
            .bind(media_id)
            .fetch_all(&db.0)
            .await?,
    )
}

pub async fn rendition_delete(db: &Db, media_id: &str, height: i64) -> Result<()> {
    sqlx::query("DELETE FROM rendition WHERE media_id = ? AND height = ?")
        .bind(media_id)
        .bind(height)
        .execute(&db.0)
        .await?;
    Ok(())
}

pub async fn rendition_ready(
    db: &Db,
    media_id: &str,
    height: i64,
    path: &str,
    size: i64,
) -> Result<()> {
    sqlx::query(
        "UPDATE rendition SET state = 'ready', path = ?, size = ?, progress = 1, error = NULL \
         WHERE media_id = ? AND height = ?",
    )
    .bind(path)
    .bind(size)
    .bind(media_id)
    .bind(height)
    .execute(&db.0)
    .await?;
    Ok(())
}
