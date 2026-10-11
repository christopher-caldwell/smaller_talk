use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Chat {
    pub id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub binding: Option<ExecutionBinding>,
}

impl Chat {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            title: "New chat".to_owned(),
            created_at: Utc::now(),
            binding: None,
        }
    }
}

impl Default for Chat {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExecutionBinding {
    pub adapter_key: String,
    pub native_session_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: Uuid,
    pub chat_id: Uuid,
    pub role: MessageRole,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
}
