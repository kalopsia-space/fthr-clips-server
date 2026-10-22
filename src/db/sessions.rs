use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::FromRow;

use super::{Db, Result};

#[derive(Debug, Clone, FromRow)]
pub struct Session {
    pub id: String,
    pub csrf: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

fn random_token() -> String {
    hex::encode(rand::random::<[u8; 32]>())
}

pub async fn create(db: &Db, ttl: Duration) -> Result<Session> {
    let now = Utc::now();
    let session = Session {
        id: random_token(),
        csrf: random_token(),
        created_at: now,
        expires_at: now + chrono::Duration::from_std(ttl)?,
    };
    sqlx::query("INSERT INTO session (id, csrf, created_at, expires_at) VALUES (?, ?, ?, ?)")
        .bind(&session.id)
        .bind(&session.csrf)
        .bind(session.created_at)
        .bind(session.expires_at)
        .execute(&db.0)
        .await?;
    Ok(session)
}

/// Return the session if it has not expired, sliding its expiry forward by
/// its original lifetime (`expires_at - created_at`).
pub async fn get_valid(db: &Db, id: &str) -> Result<Option<Session>> {
    let found: Option<Session> = sqlx::query_as("SELECT * FROM session WHERE id = ?")
        .bind(id)
        .fetch_optional(&db.0)
        .await?;
    let Some(mut session) = found else {
        return Ok(None);
    };
    let now = Utc::now();
    if session.expires_at <= now {
        return Ok(None);
    }
    session.expires_at = now + (session.expires_at - session.created_at);
    sqlx::query("UPDATE session SET expires_at = ? WHERE id = ?")
        .bind(session.expires_at)
        .bind(id)
        .execute(&db.0)
        .await?;
    Ok(Some(session))
}

pub async fn delete(db: &Db, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM session WHERE id = ?")
        .bind(id)
        .execute(&db.0)
        .await?;
    Ok(())
}

pub async fn purge_expired(db: &Db) -> Result<u64> {
    let result = sqlx::query("DELETE FROM session WHERE expires_at <= ?")
        .bind(Utc::now())
        .execute(&db.0)
        .await?;
    Ok(result.rows_affected())
}
