use std::collections::HashMap;

use serde_json::Value;

use super::{Db, Result};

pub async fn get_json(db: &Db, key: &str) -> Result<Option<Value>> {
    let text: Option<String> = sqlx::query_scalar("SELECT value FROM setting WHERE key = ?")
        .bind(key)
        .fetch_optional(&db.0)
        .await?;
    Ok(text.map(|t| serde_json::from_str(&t)).transpose()?)
}

pub async fn set_json(db: &Db, key: &str, value: &Value) -> Result<()> {
    sqlx::query(
        "INSERT INTO setting (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(serde_json::to_string(value)?)
    .execute(&db.0)
    .await?;
    Ok(())
}

pub async fn all(db: &Db) -> Result<HashMap<String, Value>> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT key, value FROM setting")
        .fetch_all(&db.0)
        .await?;
    rows.into_iter()
        .map(|(k, v)| Ok((k, serde_json::from_str(&v)?)))
        .collect()
}
