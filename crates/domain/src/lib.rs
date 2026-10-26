use chrono::{DateTime, Utc};
use uuid::Uuid;

mod workspace;
pub use workspace::*;
mod settings;
pub use settings::*;
mod repository;
pub use repository::*;
mod instructions;
pub use instructions::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Chat {
    pub id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub binding: Option<ExecutionBinding>,
    pub archived_at: Option<DateTime<Utc>>,
}

mod navigation;
pub use navigation::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionBinding {
    pub adapter_key: String,
    pub native_session_id: String,
    pub instructions_fingerprint: Option<String>,
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

mod skills;
pub use skills::*;
