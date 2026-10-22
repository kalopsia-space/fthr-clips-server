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
/// exists, nothing is inserted and its id is returned. Atomic: a single
/// INSERT guarded by the partial unique index `idx_job_active`.
pub async fn enqueue(db: &Db, media_id: &str, kind: &str) -> Result<i64> {
    let now = Utc::now();
    // The active job could finish between the insert and the select; retry then.
    for _ in 0..5 {
        let inserted: Option<i64> = sqlx::query_scalar(
            "INSERT INTO job (media_id, kind, created_at, updated_at) \
             VALUES (?, ?, ?, ?) ON CONFLICT DO NOTHING RETURNING id",
        )
        .bind(media_id)
        .bind(kind)
        .bind(now)
        .bind(now)
        .fetch_optional(&db.0)
        .await?;
        if let Some(id) = inserted {
            return Ok(id);
        }
        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM job WHERE media_id = ? AND kind = ? \
             AND state IN ('queued', 'running') ORDER BY id LIMIT 1",
        )
        .bind(media_id)
        .bind(kind)
        .fetch_optional(&db.0)
        .await?;
        if let Some(id) = existing {
            return Ok(id);
        }
    }
    anyhow::bail!("could not enqueue {kind} job for {media_id}")
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

/// Mark a running job done. Returns false when it was no longer running
/// (cancelled meanwhile), in which case nothing changes.
pub async fn finish(db: &Db, id: i64) -> Result<bool> {
    let r = sqlx::query(
        "UPDATE job SET state = 'done', progress = 1, error = NULL, updated_at = ? \
         WHERE id = ? AND state = 'running'",
    )
    .bind(Utc::now())
    .bind(id)
    .execute(&db.0)
    .await?;
    Ok(r.rows_affected() > 0)
}

/// Mark a running job failed; false when it was no longer running.
pub async fn fail(db: &Db, id: i64, error: &str) -> Result<bool> {
    let r = sqlx::query(
        "UPDATE job SET state = 'failed', error = ?, updated_at = ? \
         WHERE id = ? AND state = 'running'",
    )
    .bind(error)
    .bind(Utc::now())
    .bind(id)
    .execute(&db.0)
    .await?;
    Ok(r.rows_affected() > 0)
}

/// Requeue jobs left `running` by a crash or restart. Returns how many.
pub async fn reset_stale_running(db: &Db) -> Result<u64> {
    let r = sqlx::query(
        "UPDATE job SET state = 'queued', progress = 0, error = 'interrupted', updated_at = ? \
         WHERE state = 'running'",
    )
    .bind(Utc::now())
    .execute(&db.0)
    .await?;
    Ok(r.rows_affected())
}

pub async fn cancel(db: &Db, id: i64) -> Result<()> {
    set_state(db, id, "cancelled", None, &["queued", "running"]).await
}

/// Requeue a failed or cancelled job. Returns `Ok(true)` if it was requeued,
/// `Ok(false)` if nothing changed (wrong state, or another queued/running job
/// for the same `(media_id, kind)` already exists).
pub async fn retry(db: &Db, id: i64) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE job SET state = 'queued', progress = 0, error = NULL, updated_at = ? \
         WHERE id = ? AND state IN ('failed', 'cancelled') \
         AND NOT EXISTS (SELECT 1 FROM job j2 WHERE j2.media_id = job.media_id \
           AND j2.kind = job.kind AND j2.state IN ('queued', 'running'))",
    )
    .bind(Utc::now())
    .bind(id)
    .execute(&db.0)
    .await?;
    Ok(result.rows_affected() > 0)
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

/// Renditions of a media item, tallest first (`height DESC`).
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

/// A job row joined with its media title (empty when the media is gone).
#[derive(Debug, Clone, FromRow)]
pub struct JobWithTitle {
    #[sqlx(flatten)]
    pub job: Job,
    pub media_title: Option<String>,
}

const WITH_TITLE: &str = "SELECT j.*, m.title AS media_title FROM job j \
     LEFT JOIN media m ON m.id = j.media_id";

pub async fn list_active_titled(db: &Db) -> Result<Vec<JobWithTitle>> {
    Ok(sqlx::query_as(&format!(
        "{WITH_TITLE} WHERE j.state IN ('queued', 'running') ORDER BY j.id"
    ))
    .fetch_all(&db.0)
    .await?)
}

/// Most recently updated jobs that are no longer active.
pub async fn list_recent(db: &Db, limit: i64) -> Result<Vec<JobWithTitle>> {
    Ok(sqlx::query_as(&format!(
        "{WITH_TITLE} WHERE j.state NOT IN ('queued', 'running') \
         ORDER BY j.updated_at DESC, j.id DESC LIMIT ?"
    ))
    .bind(limit)
    .fetch_all(&db.0)
    .await?)
}

pub async fn get(db: &Db, id: i64) -> Result<Option<Job>> {
    Ok(sqlx::query_as("SELECT * FROM job WHERE id = ?")
        .bind(id)
        .fetch_optional(&db.0)
        .await?)
}

/// Put a running job back in the queue after a failed attempt, keeping the error.
pub async fn requeue(db: &Db, id: i64, error: &str) -> Result<()> {
    sqlx::query(
        "UPDATE job SET state = 'queued', progress = 0, error = ?, updated_at = ? \
         WHERE id = ? AND state = 'running'",
    )
    .bind(error)
    .bind(Utc::now())
    .bind(id)
    .execute(&db.0)
    .await?;
    Ok(())
}

/// Update progress of an existing rendition row (never creates one).
pub async fn rendition_progress(db: &Db, media_id: &str, height: i64, progress: f64) -> Result<()> {
    sqlx::query(
        "UPDATE rendition SET progress = ? WHERE media_id = ? AND height = ? AND state = 'running'",
    )
    .bind(progress)
    .bind(media_id)
    .bind(height)
    .execute(&db.0)
    .await?;
    Ok(())
}

/// Mark an existing rendition failed (never creates one).
pub async fn rendition_fail(db: &Db, media_id: &str, height: i64, error: &str) -> Result<()> {
    sqlx::query(
        "UPDATE rendition SET state = 'failed', error = ? WHERE media_id = ? AND height = ?",
    )
    .bind(error)
    .bind(media_id)
    .bind(height)
    .execute(&db.0)
    .await?;
    Ok(())
}

/// Cancel every queued or running job of `kind` for a media item; returns their ids.
pub async fn cancel_active_kind(db: &Db, media_id: &str, kind: &str) -> Result<Vec<i64>> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "UPDATE job SET state = 'cancelled', updated_at = ? WHERE media_id = ? AND kind = ? \
         AND state IN ('queued', 'running') RETURNING id",
    )
    .bind(Utc::now())
    .bind(media_id)
    .bind(kind)
    .fetch_all(&db.0)
    .await?;
    Ok(ids)
}
