use crate::ApprovalPresentation;
use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
)]
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
    /// Each conversation remembers its choice separately for each provider.
    #[serde(default)]
    pub permissions: BTreeMap<Provider, PermissionMode>,
}

impl ExecutionSettings {
    pub fn permission_mode(&self) -> PermissionMode {
        self.permissions
            .get(&self.provider)
            .copied()
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    #[default]
    ProviderDefault,
    Supervised,
    AcceptEdits,
    Auto,
    FullAccess,
}

impl PermissionMode {
    pub const ALL: [Self; 5] = [
        Self::ProviderDefault,
        Self::Supervised,
        Self::AcceptEdits,
        Self::Auto,
        Self::FullAccess,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::ProviderDefault => "Provider default",
            Self::Supervised => "Supervised",
            Self::AcceptEdits => "Auto-accept edits",
            Self::Auto => "Auto",
            Self::FullAccess => "Full access",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::ProviderDefault => "Use the provider's own permission settings.",
            Self::Supervised => "Ask before commands and file changes.",
            Self::AcceptEdits => "Allow file edits; other actions can require approval.",
            Self::Auto => "Let the provider review routine actions automatically.",
            Self::FullAccess => "Allow commands and edits without routine approvals.",
        }
    }
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
    #[serde(default)]
    pub skills: Vec<crate::SkillMention>,
    pub text: String,
    pub attachments: Vec<Attachment>,
    pub selection_start: usize,
    pub selection_end: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    #[serde(default)]
    pub navigation: crate::NavigationPreferences,
    pub open_tabs: Vec<String>,
    pub active_tab: Option<String>,
    pub sidebar_collapsed: bool,
    /// Message index plus offset within the message, independent of window size.
    pub scroll_positions: BTreeMap<String, (usize, f32)>,
    /// One reusable, unsent conversation; it is not part of saved chat history.
    #[serde(default)]
    pub new_chat: Option<NewChatDraft>,
    #[serde(default)]
    pub pane_layout: Option<PaneLayout>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaneLayout {
    pub root: PaneNode,
    pub focused: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneAxis {
    Row,
    Column,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PaneNode {
    Pane {
        id: u64,
        chat: Option<Uuid>,
    },
    Split {
        id: u64,
        axis: PaneAxis,
        ratio: f32,
        first: Box<PaneNode>,
        second: Box<PaneNode>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NewChatDraft {
    pub id: String,
    pub preferences: ChatPreferences,
    pub draft: Draft,
    /// Allows restart recovery if the first send committed before the UI acknowledged it.
    pub submitted: Option<Draft>,
}

impl NewChatDraft {
    pub fn remaining_draft(&self) -> Draft {
        let mut remaining = self.draft.clone();
        if let Some(submitted) = &self.submitted {
            if remaining.text == submitted.text && remaining.skills == submitted.skills {
                remaining.text.clear();
                remaining.skills.clear();
                remaining.selection_start = 0;
                remaining.selection_end = 0;
            }
            remaining
                .attachments
                .retain(|file| !submitted.attachments.iter().any(|sent| sent.id == file.id));
        }
        remaining
    }
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
    Compacting,
    Starting,
    Working,
    Thinking,
    Responding,
    Waiting,
}

impl Activity {
    pub fn label(self) -> &'static str {
        match self {
            Self::Compacting => "Compacting…",
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
    Approval(ApprovalPresentation),
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
    pub instructions: crate::InstructionSnapshot,
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

/// Unsent content owned by the durable queue, not by conversation history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueuedMessage {
    pub id: uuid::Uuid,
    pub chat_id: uuid::Uuid,
    pub draft: Draft,
    pub state: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MessageQueue {
    pub steer_available: bool,
    pub entries: Vec<QueuedMessage>,
    pub paused: Option<String>,
}

pub const MAX_QUEUED_MESSAGES: u64 = 100;

/// Authoritative queue occupancy used by the enqueue policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueCapacity {
    pub queued: u64,
}
impl QueueCapacity {
    pub fn can_enqueue(self) -> bool {
        self.queued < MAX_QUEUED_MESSAGES
    }
}
