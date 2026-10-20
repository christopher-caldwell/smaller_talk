use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[default]
    Codex,
    Claude,
}

impl Provider {
    pub fn key(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExecutionSettings {
    pub provider: Provider,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub plan_mode: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub folder: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChatPreferences {
    pub project_id: Option<String>,
    pub execution: ExecutionSettings,
    pub binding_workspace: Option<PathBuf>,
    /// A settings change invalidates the old session even when switched back before sending.
    pub new_session: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub mime_type: String,
    pub byte_len: u64,
    #[serde(default)]
    pub preview_path: Option<PathBuf>,
}

impl Attachment {
    pub fn is_image(&self) -> bool {
        self.mime_type.starts_with("image/")
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    pub text: String,
    pub attachments: Vec<Attachment>,
    pub selection_start: usize,
    pub selection_end: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    pub open_tabs: Vec<String>,
    pub active_tab: Option<String>,
    pub sidebar_collapsed: bool,
    /// Message index plus offset within the message, independent of window size.
    pub scroll_positions: BTreeMap<String, (usize, f32)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchHit {
    pub chat_id: Uuid,
    pub title: String,
    pub project: Option<String>,
    pub message_id: Option<Uuid>,
    pub excerpt: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelCapability {
    pub id: String,
    pub label: String,
    pub efforts: Vec<String>,
    pub default_effort: Option<String>,
    pub images: bool,
    pub is_default: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderCapabilities {
    pub provider: Provider,
    pub models: Vec<ModelCapability>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Activity {
    Starting,
    Working,
    Thinking,
    Responding,
    Waiting,
}

impl Activity {
    pub fn label(self) -> &'static str {
        match self {
            Self::Starting => "Starting…",
            Self::Working => "Working…",
            Self::Thinking => "Thinking…",
            Self::Responding => "Responding…",
            Self::Waiting => "Waiting for you",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Question {
    pub id: String,
    pub prompt: String,
    pub options: Vec<String>,
    pub multiple: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InteractionKind {
    Approval { detail: String },
    Questions(Vec<Question>),
    LeavePlan { detail: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Interaction {
    pub id: String,
    pub title: String,
    pub kind: InteractionKind,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InteractionResponse {
    pub id: String,
    pub approved: bool,
    pub answers: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug)]
pub struct TurnRecord {
    pub id: Uuid,
    pub chat_id: Uuid,
    pub user_message_id: Uuid,
    pub settings: ExecutionSettings,
    pub workspace: PathBuf,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct TurnSummary {
    pub user_message_id: Uuid,
    pub status: String,
    pub error: Option<String>,
}
