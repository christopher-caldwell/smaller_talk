use smaller_talk_application::{
    ChatQueries, ChatWrites, DeleteChatOutcome, NewChat, PersistenceError,
};
use smaller_talk_domain::{Chat, ExecutionBinding};
use sqlx::SqlitePool;
use thiserror::Error;
use uuid::Uuid;

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

#[derive(Debug, Error)]
enum ChatRowMappingError {
    #[error("chat has an invalid creation timestamp: {0}")]
    InvalidCreationTimestamp(i64),
    #[error("chat has an incomplete provider binding")]
    IncompleteProviderBinding,
}

impl TryFrom<ChatRow> for Chat {
    type Error = PersistenceError;

    fn try_from(row: ChatRow) -> Result<Self, Self::Error> {
        let id = Uuid::parse_str(&row.id).map_err(PersistenceError::new)?;
        let created_at = chrono::DateTime::from_timestamp_millis(row.created_at)
            .ok_or(ChatRowMappingError::InvalidCreationTimestamp(
                row.created_at,
            ))
            .map_err(PersistenceError::new)?;
        let binding = match (row.adapter_key, row.native_session_id) {
            (Some(adapter_key), Some(native_session_id)) => Some(ExecutionBinding {
                adapter_key,
                native_session_id,
            }),
            (None, None) => None,
            _ => {
                return Err(PersistenceError::new(
                    ChatRowMappingError::IncompleteProviderBinding,
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
    async fn create_chat(&self, chat: NewChat) -> Result<Chat, PersistenceError> {
        let id = chat.id.to_string();
        let title = chat.title;
        let created_at = chat.created_at.timestamp_millis();
        let adapter_key = chat
            .binding
            .as_ref()
            .map(|binding| binding.adapter_key.clone());
        let native_session_id = chat
            .binding
            .as_ref()
            .map(|binding| binding.native_session_id.clone());
        let row = sqlx::query_file_as!(
            ChatRow,
            "src/chats/sql/insert_chat.sql",
            id,
            title,
            created_at,
            adapter_key,
            native_session_id,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(PersistenceError::new)?;

        Chat::try_from(row)
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
