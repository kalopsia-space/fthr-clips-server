use chrono::{DateTime, Utc};
use sqlx::FromRow;

use super::{Db, Result};

#[derive(Debug, Clone, FromRow)]
pub struct UploadToken {
    pub id: i64,
    pub name: String,
    pub prefix: String,
    pub token_hash: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

pub async fn create(db: &Db, name: &str, token_hash: &str, prefix: &str) -> Result<UploadToken> {
    Ok(sqlx::query_as(
        "INSERT INTO upload_token (name, prefix, token_hash, created_at) \
         VALUES (?, ?, ?, ?) RETURNING *",
    )
    .bind(name)
    .bind(prefix)
    .bind(token_hash)
    .bind(Utc::now())
    .fetch_one(&db.0)
    .await?)
}

pub async fn find_active_by_hash(db: &Db, hash: &str) -> Result<Option<UploadToken>> {
    Ok(
        sqlx::query_as("SELECT * FROM upload_token WHERE token_hash = ? AND revoked_at IS NULL")
            .bind(hash)
            .fetch_optional(&db.0)
            .await?,
    )
}

pub async fn touch(db: &Db, id: i64) -> Result<()> {
    sqlx::query("UPDATE upload_token SET last_used_at = ? WHERE id = ?")
        .bind(Utc::now())
        .bind(id)
        .execute(&db.0)
        .await?;
    Ok(())
}

pub async fn revoke(db: &Db, id: i64) -> Result<()> {
    sqlx::query("UPDATE upload_token SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
        .bind(Utc::now())
        .bind(id)
        .execute(&db.0)
        .await?;
    Ok(())
}

pub async fn list(db: &Db) -> Result<Vec<UploadToken>> {
    Ok(sqlx::query_as("SELECT * FROM upload_token ORDER BY id")
        .fetch_all(&db.0)
        .await?)
}
