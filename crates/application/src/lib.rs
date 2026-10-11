use std::{error::Error, fmt, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use smaller_talk_domain::{Chat, ExecutionBinding, Message, MessageRole};
use thiserror::Error;
use tokio::sync::mpsc;
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewMessage {
    pub id: Uuid,
    pub chat_id: Uuid,
    pub role: MessageRole,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

#[async_trait]
pub trait ChatQueries: Send + Sync {
    async fn list_chats(&self) -> Result<Vec<Chat>, PersistenceError>;
    async fn find_chat(&self, id: Uuid) -> Result<Option<Chat>, PersistenceError>;
}

#[async_trait]
pub trait ChatWrites: Send + Sync {
    async fn create_chat(&self, chat: NewChat) -> Result<Chat, PersistenceError>;
    async fn delete_chat(&self, id: Uuid) -> Result<DeleteChatOutcome, PersistenceError>;
    async fn save_execution_binding(
        &self,
        id: Uuid,
        binding: ExecutionBinding,
    ) -> Result<bool, PersistenceError>;
}

#[async_trait]
pub trait MessageQueries: Send + Sync {
    async fn list_messages(&self, chat_id: Uuid) -> Result<Vec<Message>, PersistenceError>;
}

#[async_trait]
pub trait MessageWrites: Send + Sync {
    async fn append_message(&self, message: NewMessage) -> Result<Message, PersistenceError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionRequest {
    pub chat_id: Uuid,
    pub binding: Option<ExecutionBinding>,
    pub workspace: PathBuf,
    pub prompt: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionEvent {
    BindingEstablished(ExecutionBinding),
    TextDelta(String),
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct ExecutionError {
    message: String,
    #[source]
    source: Option<Box<dyn Error + Send + Sync + 'static>>,
}

impl ExecutionError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    pub fn with_source(
        message: impl Into<String>,
        source: impl Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SendMessageInput {
    pub chat_id: Uuid,
    pub prompt: String,
}

#[async_trait]
pub trait ExecutionAdapter: Send + Sync {
    async fn execute(
        &self,
        request: ExecutionRequest,
        events: mpsc::UnboundedSender<ExecutionEvent>,
    ) -> Result<(), ExecutionError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConversationEvent {
    UserMessage(Message),
    AssistantDelta {
        chat_id: Uuid,
        message_id: Uuid,
        delta: String,
    },
    AssistantMessage(Message),
}

#[derive(Debug, Error)]
pub enum SendMessageError {
    #[error("the selected chat no longer exists")]
    ChatNotFound,
    #[error("chat storage failed: {0}")]
    Storage(#[source] PersistenceError),
    #[error("Codex execution failed: {0}")]
    Execution(#[source] ExecutionError),
    #[error("the Codex execution task stopped unexpectedly: {0}")]
    Task(String),
    #[error("Codex completed without an assistant response")]
    EmptyResponse,
}

#[derive(Debug, Error)]
pub enum ListMessagesError {
    #[error("message storage failed: {0}")]
    Storage(#[source] PersistenceError),
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

pub struct ConversationService {
    chats: Arc<dyn ChatQueries>,
    chat_writes: Arc<dyn ChatWrites>,
    messages: Arc<dyn MessageQueries>,
    message_writes: Arc<dyn MessageWrites>,
    executor: Arc<dyn ExecutionAdapter>,
    workspace: PathBuf,
}

impl ConversationService {
    pub fn new(
        chats: Arc<dyn ChatQueries>,
        chat_writes: Arc<dyn ChatWrites>,
        messages: Arc<dyn MessageQueries>,
        message_writes: Arc<dyn MessageWrites>,
        executor: Arc<dyn ExecutionAdapter>,
        workspace: PathBuf,
    ) -> Self {
        Self {
            chats,
            chat_writes,
            messages,
            message_writes,
            executor,
            workspace,
        }
    }

    pub async fn list_messages(&self, chat_id: Uuid) -> Result<Vec<Message>, ListMessagesError> {
        self.messages
            .list_messages(chat_id)
            .await
            .map_err(ListMessagesError::Storage)
    }

    pub async fn send_message(
        &self,
        input: SendMessageInput,
        output: mpsc::UnboundedSender<ConversationEvent>,
    ) -> Result<(), SendMessageError> {
        let SendMessageInput { chat_id, prompt } = input;
        let chat = self
            .chats
            .find_chat(chat_id)
            .await
            .map_err(SendMessageError::Storage)?
            .ok_or(SendMessageError::ChatNotFound)?;
        let user_message = self
            .message_writes
            .append_message(NewMessage {
                id: Uuid::new_v4(),
                chat_id,
                role: MessageRole::User,
                body: prompt.clone(),
                created_at: Utc::now(),
            })
            .await
            .map_err(SendMessageError::Storage)?;
        let user_created_at = user_message.created_at;
        let _ = output.send(ConversationEvent::UserMessage(user_message));

        let (provider_events, mut provider_stream) = mpsc::unbounded_channel();
        let executor = self.executor.clone();
        let request = ExecutionRequest {
            chat_id,
            binding: chat.binding.clone(),
            workspace: self.workspace.clone(),
            prompt,
        };
        let task = tokio::spawn(async move { executor.execute(request, provider_events).await });
        let assistant_id = Uuid::new_v4();
        let assistant_created_at =
            Utc::now().max(user_created_at + chrono::Duration::milliseconds(1));
        let mut response = String::new();

        while let Some(event) = provider_stream.recv().await {
            match event {
                ExecutionEvent::BindingEstablished(binding) => {
                    let saved = match self
                        .chat_writes
                        .save_execution_binding(chat_id, binding)
                        .await
                    {
                        Ok(saved) => saved,
                        Err(error) => {
                            task.abort();
                            let _ = task.await;
                            return Err(SendMessageError::Storage(error));
                        }
                    };
                    if !saved {
                        task.abort();
                        let _ = task.await;
                        return Err(SendMessageError::ChatNotFound);
                    }
                }
                ExecutionEvent::TextDelta(delta) if !delta.is_empty() => {
                    response.push_str(&delta);
                    let _ = output.send(ConversationEvent::AssistantDelta {
                        chat_id,
                        message_id: assistant_id,
                        delta,
                    });
                }
                ExecutionEvent::TextDelta(_) => {}
            }
        }

        task.await
            .map_err(|error| SendMessageError::Task(error.to_string()))?
            .map_err(SendMessageError::Execution)?;
        if response.is_empty() {
            return Err(SendMessageError::EmptyResponse);
        }

        let assistant_message = self
            .message_writes
            .append_message(NewMessage {
                id: assistant_id,
                chat_id,
                role: MessageRole::Assistant,
                body: response,
                created_at: assistant_created_at,
            })
            .await
            .map_err(SendMessageError::Storage)?;
        let _ = output.send(ConversationEvent::AssistantMessage(assistant_message));
        Ok(())
    }
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

    #[test]
    fn execution_error_preserves_an_opaque_adapter_source() {
        let error = ExecutionError::with_source(
            "the Codex request could not be written",
            std::io::Error::other("pipe closed"),
        );
        let source = Error::source(&error).expect("I/O diagnostic is retained");
        assert!(source.downcast_ref::<std::io::Error>().is_some());
        assert_eq!(error.to_string(), "the Codex request could not be written");
    }
}
