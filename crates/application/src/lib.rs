use std::sync::Arc;

use async_trait::async_trait;
use smaller_talk_domain::Chat;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
#[error("{message}")]
pub struct PersistenceError {
    message: String,
}

impl PersistenceError {
    pub fn new(error: impl std::fmt::Display) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeleteChatOutcome {
    Deleted,
    NotFound,
}

#[async_trait]
pub trait ChatQueries: Send + Sync {
    async fn list_chats(&self) -> Result<Vec<Chat>, PersistenceError>;
}

#[async_trait]
pub trait ChatWrites: Send + Sync {
    async fn create_chat(&self, chat: Chat) -> Result<Chat, PersistenceError>;
    async fn delete_chat(&self, id: Uuid) -> Result<DeleteChatOutcome, PersistenceError>;
}

#[derive(Debug, Error)]
pub enum ChatServiceError {
    #[error("chat storage failed: {0}")]
    Storage(#[from] PersistenceError),
    #[error("the selected chat no longer exists")]
    NotFound,
}

pub struct ChatService {
    queries: Arc<dyn ChatQueries>,
    writes: Arc<dyn ChatWrites>,
}

impl ChatService {
    pub fn new(queries: Arc<dyn ChatQueries>, writes: Arc<dyn ChatWrites>) -> Self {
        Self { queries, writes }
    }

    pub async fn list_chats(&self) -> Result<Vec<Chat>, ChatServiceError> {
        self.queries.list_chats().await.map_err(Into::into)
    }

    pub async fn create_empty_chat(&self) -> Result<Chat, ChatServiceError> {
        self.writes
            .create_chat(Chat::new())
            .await
            .map_err(Into::into)
    }

    pub async fn delete_chat(&self, id: Uuid) -> Result<(), ChatServiceError> {
        match self.writes.delete_chat(id).await? {
            DeleteChatOutcome::Deleted => Ok(()),
            DeleteChatOutcome::NotFound => Err(ChatServiceError::NotFound),
        }
    }
}
