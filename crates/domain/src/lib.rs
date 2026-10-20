use chrono::{DateTime, Utc};
use uuid::Uuid;

mod workspace;
pub use workspace::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Chat {
    pub id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub binding: Option<ExecutionBinding>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionBinding {
    pub adapter_key: String,
    pub native_session_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Message {
    pub id: Uuid,
    pub chat_id: Uuid,
    pub role: MessageRole,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
}
