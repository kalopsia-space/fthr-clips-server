use chrono::Utc;

use super::{Db, Result};

/// Record a view; returns true if this `ip_hash` had not viewed the media before.
pub async fn record(db: &Db, media_id: &str, ip_hash: &str) -> Result<bool> {
    let result =
        sqlx::query("INSERT OR IGNORE INTO view (media_id, ip_hash, first_seen) VALUES (?, ?, ?)")
            .bind(media_id)
            .bind(ip_hash)
            .bind(Utc::now())
            .execute(&db.0)
            .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn count(db: &Db, media_id: &str) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT COUNT(*) FROM view WHERE media_id = ?")
            .bind(media_id)
            .fetch_one(&db.0)
            .await?,
    )
}
