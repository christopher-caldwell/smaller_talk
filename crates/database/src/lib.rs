use std::path::Path;

use smaller_talk_application::{ChatQueries, ChatWrites, DeleteChatOutcome, PersistenceError};
use smaller_talk_domain::{Chat, ExecutionBinding};
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions, sqlite::SqlitePoolOptions};
use thiserror::Error;
use uuid::Uuid;

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

#[derive(Clone)]
pub struct SqliteChatStore {
    pool: SqlitePool,
}

impl SqliteChatStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

struct ChatRow {
    id: String,
    title: String,
    created_at: i64,
    adapter_key: Option<String>,
    native_session_id: Option<String>,
}

impl TryFrom<ChatRow> for Chat {
    type Error = PersistenceError;

    fn try_from(row: ChatRow) -> Result<Self, Self::Error> {
        let id = Uuid::parse_str(&row.id).map_err(PersistenceError::new)?;
        let created_at = chrono::DateTime::from_timestamp_millis(row.created_at)
            .ok_or_else(|| PersistenceError::new("chat has an invalid creation timestamp"))?;
        let binding = match (row.adapter_key, row.native_session_id) {
            (Some(adapter_key), Some(native_session_id)) => Some(ExecutionBinding {
                adapter_key,
                native_session_id,
            }),
            (None, None) => None,
            _ => {
                return Err(PersistenceError::new(
                    "chat has an incomplete provider binding",
                ));
            }
        };

        Ok(Chat {
            id,
            title: row.title,
            created_at,
            binding,
        })
    }
}

#[async_trait::async_trait]
impl ChatQueries for SqliteChatStore {
    async fn list_chats(&self) -> Result<Vec<Chat>, PersistenceError> {
        let rows = sqlx::query_file_as!(ChatRow, "src/chats/sql/list_chats.sql")
            .fetch_all(&self.pool)
            .await
            .map_err(PersistenceError::new)?;

        rows.into_iter().map(Chat::try_from).collect()
    }
}

#[async_trait::async_trait]
impl ChatWrites for SqliteChatStore {
    async fn create_chat(&self, chat: Chat) -> Result<Chat, PersistenceError> {
        let id = chat.id.to_string();
        let title = chat.title.clone();
        let created_at = chat.created_at.timestamp_millis();
        let adapter_key = chat
            .binding
            .as_ref()
            .map(|binding| binding.adapter_key.clone());
        let native_session_id = chat
            .binding
            .as_ref()
            .map(|binding| binding.native_session_id.clone());
        sqlx::query_file!(
            "src/chats/sql/insert_chat.sql",
            id,
            title,
            created_at,
            adapter_key,
            native_session_id,
        )
        .execute(&self.pool)
        .await
        .map_err(PersistenceError::new)?;

        Ok(chat)
    }

    async fn delete_chat(&self, id: Uuid) -> Result<DeleteChatOutcome, PersistenceError> {
        let id = id.to_string();
        let deleted_id = sqlx::query_file_scalar!("src/chats/sql/delete_chat.sql", id)
            .fetch_optional(&self.pool)
            .await
            .map_err(PersistenceError::new)?;

        Ok(if deleted_id.is_some() {
            DeleteChatOutcome::Deleted
        } else {
            DeleteChatOutcome::NotFound
        })
    }
}
