use std::path::Path;

use sqlx::{SqlitePool, sqlite::SqliteConnectOptions, sqlite::SqlitePoolOptions};
use thiserror::Error;

mod chats;
mod queue;
mod settings;
mod workspace;
pub use chats::SqliteChatStore;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(Debug, Error)]
pub enum DatabaseSetupError {
    #[error("filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("SQLite operation failed: {0}")]
    Sqlite(#[from] sqlx::Error),
    #[error("schema migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
}

pub async fn open_existing(path: &Path) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .foreign_keys(true);

    SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
}

pub async fn create_empty_database(path: &Path) -> Result<(), DatabaseSetupError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;
    pool.close().await;
    Ok(())
}

pub async fn migrate_existing(path: &Path) -> Result<(), DatabaseSetupError> {
    let pool = open_existing(path).await?;
    MIGRATOR.run(&pool).await?;
    pool.close().await;
    Ok(())
}
