use std::{error::Error, fmt, sync::Arc};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use smaller_talk_domain::{Chat, ExecutionBinding};
use thiserror::Error;
use uuid::Uuid;

/// An adapter failure with its original diagnostic retained behind an application-owned type.
#[derive(Debug)]
pub struct PersistenceError(Box<dyn Error + Send + Sync + 'static>);

impl PersistenceError {
    pub fn new(error: impl Error + Send + Sync + 'static) -> Self {
        Self(Box::new(error))
    }
}

impl fmt::Display for PersistenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl Error for PersistenceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}

/// Values proposed by the application for a new row; the adapter returns the stored snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewChat {
    pub id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub binding: Option<ExecutionBinding>,
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
    async fn create_chat(&self, chat: NewChat) -> Result<Chat, PersistenceError>;
    async fn delete_chat(&self, id: Uuid) -> Result<DeleteChatOutcome, PersistenceError>;
}

#[derive(Debug, Error)]
pub enum ListChatsError {
    #[error("chat storage failed: {0}")]
    Storage(#[source] PersistenceError),
}

#[derive(Debug, Error)]
pub enum CreateChatError {
    #[error("chat storage failed: {0}")]
    Storage(#[source] PersistenceError),
}

#[derive(Debug, Error)]
pub enum DeleteChatError {
    #[error("chat storage failed: {0}")]
    Storage(#[source] PersistenceError),
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

    pub async fn list_chats(&self) -> Result<Vec<Chat>, ListChatsError> {
        self.queries
            .list_chats()
            .await
            .map_err(ListChatsError::Storage)
    }

    pub async fn create_empty_chat(&self) -> Result<Chat, CreateChatError> {
        self.writes
            .create_chat(NewChat {
                id: Uuid::new_v4(),
                title: "New chat".to_owned(),
                created_at: Utc::now(),
                binding: None,
            })
            .await
            .map_err(CreateChatError::Storage)
    }

    pub async fn delete_chat(&self, id: Uuid) -> Result<(), DeleteChatError> {
        match self
            .writes
            .delete_chat(id)
            .await
            .map_err(DeleteChatError::Storage)?
        {
            DeleteChatOutcome::Deleted => Ok(()),
            DeleteChatOutcome::NotFound => Err(DeleteChatError::NotFound),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct AdapterDiagnostic;

    impl fmt::Display for AdapterDiagnostic {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("adapter detail")
        }
    }

    impl Error for AdapterDiagnostic {}

    #[test]
    fn persistence_error_preserves_the_opaque_adapter_source() {
        let error = PersistenceError::new(AdapterDiagnostic);
        let source = Error::source(&error).expect("diagnostic source is retained");
        assert!(source.downcast_ref::<AdapterDiagnostic>().is_some());
        assert_eq!(error.to_string(), "adapter detail");
    }
}
