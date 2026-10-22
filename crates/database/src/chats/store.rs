use smaller_talk_application::{
    ChatQueries, ChatSetupWrites, ChatWrites, DeleteChatOutcome, MessageQueries, MessageWrites,
    NewChat, NewMessage, PersistenceError, RenameChatOutcome,
};
use smaller_talk_domain::{Chat, ExecutionBinding, Message, MessageRole};
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
    instructions_fingerprint: Option<String>,
}

struct MessageRow {
    id: String,
    chat_id: String,
    role: String,
    body: String,
    created_at: i64,
}

#[derive(Debug, Error)]
enum ChatRowMappingError {
    #[error("chat has an invalid creation timestamp: {0}")]
    InvalidCreationTimestamp(i64),
    #[error("chat has an incomplete provider binding")]
    IncompleteProviderBinding,
}

#[derive(Debug, Error)]
enum MessageRowMappingError {
    #[error("message has an invalid creation timestamp: {0}")]
    InvalidCreationTimestamp(i64),
    #[error("message has an invalid stored role: {0}")]
    InvalidRole(String),
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
                instructions_fingerprint: row.instructions_fingerprint,
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

impl TryFrom<MessageRow> for Message {
    type Error = PersistenceError;

    fn try_from(row: MessageRow) -> Result<Self, Self::Error> {
        let role = match row.role.as_str() {
            "user" => MessageRole::User,
            "assistant" => MessageRole::Assistant,
            _ => {
                return Err(PersistenceError::new(MessageRowMappingError::InvalidRole(
                    row.role,
                )));
            }
        };
        let created_at = chrono::DateTime::from_timestamp_millis(row.created_at)
            .ok_or(MessageRowMappingError::InvalidCreationTimestamp(
                row.created_at,
            ))
            .map_err(PersistenceError::new)?;
        Ok(Message {
            id: Uuid::parse_str(&row.id).map_err(PersistenceError::new)?,
            chat_id: Uuid::parse_str(&row.chat_id).map_err(PersistenceError::new)?,
            role,
            body: row.body,
            created_at,
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

    async fn find_chat(&self, id: Uuid) -> Result<Option<Chat>, PersistenceError> {
        let id = id.to_string();
        let row = sqlx::query_file_as!(ChatRow, "src/chats/sql/find_chat.sql", id)
            .fetch_optional(&self.pool)
            .await
            .map_err(PersistenceError::new)?;
        row.map(Chat::try_from).transpose()
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
        let instructions_fingerprint = chat
            .binding
            .as_ref()
            .and_then(|b| b.instructions_fingerprint.clone());
        let row = sqlx::query_file_as!(
            ChatRow,
            "src/chats/sql/insert_chat.sql",
            id,
            title,
            created_at,
            adapter_key,
            native_session_id,
            instructions_fingerprint,
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

    async fn rename_chat(
        &self,
        id: Uuid,
        title: String,
    ) -> Result<RenameChatOutcome, PersistenceError> {
        let id = id.to_string();
        let row = sqlx::query_file_as!(ChatRow, "src/chats/sql/rename_chat.sql", title, id)
            .fetch_optional(&self.pool)
            .await
            .map_err(PersistenceError::new)?;
        match row {
            Some(row) => Ok(RenameChatOutcome::Renamed(Chat::try_from(row)?)),
            None => Ok(RenameChatOutcome::NotFound),
        }
    }

    async fn save_execution_binding(
        &self,
        id: Uuid,
        binding: ExecutionBinding,
    ) -> Result<bool, PersistenceError> {
        let id = id.to_string();
        let adapter_key = binding.adapter_key;
        let native_session_id = binding.native_session_id;
        let instructions_fingerprint = binding.instructions_fingerprint;
        let updated_id = sqlx::query_file_scalar!(
            "src/chats/sql/save_execution_binding.sql",
            adapter_key,
            native_session_id,
            instructions_fingerprint,
            id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(PersistenceError::new)?;
        Ok(updated_id.is_some())
    }
}

#[async_trait::async_trait]
impl ChatSetupWrites for SqliteChatStore {
    async fn start_chat(
        &self,
        chat: NewChat,
        preferences: smaller_talk_domain::ChatPreferences,
        first_message: smaller_talk_domain::QueuedMessage,
    ) -> Result<(), PersistenceError> {
        let mut tx = self.pool.begin().await.map_err(PersistenceError::new)?;
        let id = chat.id.to_string();
        let created_at = chat.created_at.timestamp_millis();
        sqlx::query_file!("src/chats/sql/start_chat.sql", id, chat.title, created_at)
            .execute(&mut *tx)
            .await
            .map_err(PersistenceError::new)?;
        let preferences_payload =
            serde_json::to_string(&preferences).map_err(PersistenceError::new)?;
        sqlx::query_file!(
            "src/workspace/sql/save_preferences.sql",
            preferences.project_id,
            preferences_payload,
            id
        )
        .execute(&mut *tx)
        .await
        .map_err(PersistenceError::new)?;
        let entry_id = first_message.id.to_string();
        let payload = serde_json::to_string(&first_message.draft).map_err(PersistenceError::new)?;
        sqlx::query_file!("src/queue/sql/enqueue.sql", entry_id, id, payload)
            .execute(&mut *tx)
            .await
            .map_err(PersistenceError::new)?;
        tx.commit().await.map_err(PersistenceError::new)
    }

    async fn create_chat_with_preferences(
        &self,
        chat: NewChat,
        preferences: Option<smaller_talk_domain::ChatPreferences>,
    ) -> Result<Chat, PersistenceError> {
        let mut tx = self.pool.begin().await.map_err(PersistenceError::new)?;
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
        let instructions_fingerprint = chat
            .binding
            .as_ref()
            .and_then(|b| b.instructions_fingerprint.clone());
        let row = sqlx::query_file_as!(
            ChatRow,
            "src/chats/sql/insert_chat.sql",
            id,
            title,
            created_at,
            adapter_key,
            native_session_id,
            instructions_fingerprint,
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(PersistenceError::new)?;
        let created = Chat::try_from(row)?;
        if let Some(preferences) = preferences {
            let payload = serde_json::to_string(&preferences).map_err(PersistenceError::new)?;
            let id = created.id.to_string();
            sqlx::query_file!(
                "src/workspace/sql/save_preferences.sql",
                preferences.project_id,
                payload,
                id
            )
            .execute(&mut *tx)
            .await
            .map_err(PersistenceError::new)?;
        }
        tx.commit().await.map_err(PersistenceError::new)?;
        Ok(created)
    }
}

#[async_trait::async_trait]
impl MessageQueries for SqliteChatStore {
    async fn list_messages(&self, chat_id: Uuid) -> Result<Vec<Message>, PersistenceError> {
        let chat_id = chat_id.to_string();
        let rows = sqlx::query_file_as!(MessageRow, "src/chats/sql/list_messages.sql", chat_id)
            .fetch_all(&self.pool)
            .await
            .map_err(PersistenceError::new)?;
        rows.into_iter().map(Message::try_from).collect()
    }
}

#[async_trait::async_trait]
impl MessageWrites for SqliteChatStore {
    async fn append_message(&self, message: NewMessage) -> Result<Message, PersistenceError> {
        let id = message.id.to_string();
        let chat_id = message.chat_id.to_string();
        let role = match message.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
        };
        let body = message.body;
        let created_at = message.created_at.timestamp_millis();
        let row = sqlx::query_file_as!(
            MessageRow,
            "src/chats/sql/insert_message.sql",
            id,
            chat_id,
            role,
            body,
            created_at,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(PersistenceError::new)?;
        Message::try_from(row)
    }
}
