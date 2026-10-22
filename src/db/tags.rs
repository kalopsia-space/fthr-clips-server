use serde::Serialize;
use sqlx::FromRow;

use super::{Db, Result};

pub const DEFAULT_COLOUR: &str = "#20e3a0";

#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize)]
pub struct Tag {
    pub id: i64,
    pub name: String,
    pub colour: String,
}

pub async fn create(db: &Db, name: &str, colour: &str) -> Result<Tag> {
    Ok(
        sqlx::query_as("INSERT INTO tag (name, colour) VALUES (?, ?) RETURNING *")
            .bind(name)
            .bind(colour)
            .fetch_one(&db.0)
            .await?,
    )
}

pub async fn rename(db: &Db, id: i64, name: &str) -> Result<()> {
    sqlx::query("UPDATE tag SET name = ? WHERE id = ?")
        .bind(name)
        .bind(id)
        .execute(&db.0)
        .await?;
    Ok(())
}

pub async fn recolour(db: &Db, id: i64, colour: &str) -> Result<()> {
    sqlx::query("UPDATE tag SET colour = ? WHERE id = ?")
        .bind(colour)
        .bind(id)
        .execute(&db.0)
        .await?;
    Ok(())
}

pub async fn delete(db: &Db, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM tag WHERE id = ?")
        .bind(id)
        .execute(&db.0)
        .await?;
    Ok(())
}

pub async fn list_all(db: &Db) -> Result<Vec<Tag>> {
    Ok(sqlx::query_as("SELECT * FROM tag ORDER BY name")
        .fetch_all(&db.0)
        .await?)
}

pub async fn attach(db: &Db, media_id: &str, tag_id: i64) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO media_tag (media_id, tag_id) VALUES (?, ?)")
        .bind(media_id)
        .bind(tag_id)
        .execute(&db.0)
        .await?;
    Ok(())
}

pub async fn detach(db: &Db, media_id: &str, tag_id: i64) -> Result<()> {
    sqlx::query("DELETE FROM media_tag WHERE media_id = ? AND tag_id = ?")
        .bind(media_id)
        .bind(tag_id)
        .execute(&db.0)
        .await?;
    Ok(())
}

pub async fn for_media(db: &Db, media_id: &str) -> Result<Vec<Tag>> {
    Ok(sqlx::query_as(
        "SELECT t.* FROM tag t JOIN media_tag mt ON mt.tag_id = t.id \
         WHERE mt.media_id = ? ORDER BY t.name",
    )
    .bind(media_id)
    .fetch_all(&db.0)
    .await?)
}

/// Attach the tag called `name` to every media id, creating the tag first if missing.
pub async fn attach_by_name(db: &Db, media_ids: &[String], name: &str) -> Result<Tag> {
    let mut tx = db.0.begin().await?;
    sqlx::query("INSERT OR IGNORE INTO tag (name, colour) VALUES (?, ?)")
        .bind(name)
        .bind(DEFAULT_COLOUR)
        .execute(&mut *tx)
        .await?;
    let tag: Tag = sqlx::query_as("SELECT * FROM tag WHERE name = ?")
        .bind(name)
        .fetch_one(&mut *tx)
        .await?;
    for media_id in media_ids {
        sqlx::query("INSERT OR IGNORE INTO media_tag (media_id, tag_id) VALUES (?, ?)")
            .bind(media_id)
            .bind(tag.id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(tag)
}
