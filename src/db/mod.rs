pub mod jobs;
pub mod media;
pub mod sessions;
pub mod settings;
pub mod tags;
pub mod tokens;
pub mod views;

use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::SqlitePool;

pub use crate::media::kind::{MediaKind, Source};
pub use media::Media;
pub use tags::Tag;

pub type Result<T> = anyhow::Result<T>;

/// Handle to the SQLite library database.
#[derive(Clone, Debug)]
pub struct Db(pub SqlitePool);

impl Db {
    /// Open (creating if needed) the database at `path` and run migrations.
    pub async fn open(path: &Path) -> Result<Db> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_millis(5000));
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(options)
            .await
            .with_context(|| format!("opening {}", path.display()))?;
        Self::migrate(pool).await
    }

    /// In-memory database for tests. Single connection, since each in-memory
    /// connection would otherwise be its own database.
    pub async fn open_memory() -> Result<Db> {
        let options = SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        Self::migrate(pool).await
    }

    async fn migrate(pool: SqlitePool) -> Result<Db> {
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .context("running migrations")?;
        Ok(Db(pool))
    }
}
