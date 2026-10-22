use crate::{NewMessage, PersistenceError, SettingsQueries};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use smaller_talk_domain::*;
use std::sync::Arc;
use uuid::Uuid;

#[async_trait]
pub trait InstructionQueries: Send + Sync {
    /// None means the project was removed; an existing project defaults to empty instructions.
    async fn project_instructions(
        &self,
        project: Uuid,
    ) -> Result<Option<CustomInstructions>, PersistenceError>;
    async fn message_instructions(
        &self,
        chat: Uuid,
        message: Uuid,
    ) -> Result<Option<InstructionSnapshot>, PersistenceError>;
}

#[async_trait]
pub trait InstructionWrites: Send + Sync {
    async fn save_project_instructions(
        &self,
        project: Uuid,
        instructions: &CustomInstructions,
    ) -> Result<bool, PersistenceError>;
}

/// Record one execution attempt atomically, including its instruction snapshot.
#[async_trait]
pub trait TurnStartWrites: Send + Sync {
    async fn record_turn_start(&self, input: RecordTurnStart) -> Result<Message, PersistenceError>;
}

pub struct RecordTurnStart {
    pub skills: Vec<SkillMention>,
    pub user: Message,
    pub insert_user: bool,
    pub attachments: Vec<Attachment>,
    pub turn: TurnRecord,
}

#[derive(Debug, thiserror::Error)]
pub enum LoadProjectInstructionsError {
    #[error("The project no longer exists")]
    ProjectNotFound,
    #[error("Could not load instructions: {0}")]
    Storage(#[from] PersistenceError),
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveInstructionsError {
    #[error("Could not load global instructions: {0}")]
    Storage(#[from] PersistenceError),
    #[error(transparent)]
    Project(#[from] LoadProjectInstructionsError),
    #[error("Instructions must contain at most 16 KiB of text and no null characters")]
    InvalidInstructions,
}

#[derive(Debug, thiserror::Error)]
pub enum RetryInstructionsError {
    #[error("Could not recover the original instructions: {0}")]
    Storage(#[from] PersistenceError),
}

#[derive(Debug, thiserror::Error)]
pub enum InspectInstructionsError {
    #[error("Could not load the applied instructions: {0}")]
    Storage(#[from] PersistenceError),
}

#[derive(Debug, thiserror::Error)]
pub enum SaveProjectInstructionsError {
    #[error("The project no longer exists")]
    ProjectNotFound,
    #[error("Instructions must contain at most 16 KiB of text and no null characters")]
    InvalidInstructions,
    #[error("Could not save instructions: {0}")]
    Storage(#[from] PersistenceError),
}

pub struct InstructionService {
    queries: Arc<dyn InstructionQueries>,
    writes: Arc<dyn InstructionWrites>,
    settings: Arc<dyn SettingsQueries>,
}

impl InstructionService {
    pub fn new(
        queries: Arc<dyn InstructionQueries>,
        writes: Arc<dyn InstructionWrites>,
        settings: Arc<dyn SettingsQueries>,
    ) -> Self {
        Self {
            queries,
            writes,
            settings,
        }
    }

    pub async fn project(
        &self,
        project: Uuid,
    ) -> Result<CustomInstructions, LoadProjectInstructionsError> {
        self.queries
            .project_instructions(project)
            .await?
            .ok_or(LoadProjectInstructionsError::ProjectNotFound)
    }

    pub async fn save_project(
        &self,
        project: Uuid,
        instructions: &CustomInstructions,
    ) -> Result<(), SaveProjectInstructionsError> {
        if !instructions.is_valid() {
            return Err(SaveProjectInstructionsError::InvalidInstructions);
        }
        if !self
            .writes
            .save_project_instructions(project, instructions)
            .await?
        {
            return Err(SaveProjectInstructionsError::ProjectNotFound);
        }
        Ok(())
    }

    pub async fn resolve(
        &self,
        project: Option<Uuid>,
    ) -> Result<InstructionSnapshot, ResolveInstructionsError> {
        let global = self.settings.app_settings().await?.instructions;
        let local = match project {
            Some(id) => self.project(id).await?,
            None => CustomInstructions::default(),
        };
        if !global.is_valid() || !local.is_valid() {
            return Err(ResolveInstructionsError::InvalidInstructions);
        }
        Ok(compose_instructions(&global, &local, project))
    }

    pub async fn for_retry(
        &self,
        chat: Uuid,
        message: Uuid,
    ) -> Result<InstructionSnapshot, RetryInstructionsError> {
        Ok(self
            .queries
            .message_instructions(chat, message)
            .await?
            .unwrap_or_default())
    }

    pub async fn for_message(
        &self,
        chat: Uuid,
        message: Uuid,
    ) -> Result<Option<InstructionSnapshot>, InspectInstructionsError> {
        Ok(self.queries.message_instructions(chat, message).await?)
    }
}

pub fn compose_instructions(
    global: &CustomInstructions,
    project: &CustomInstructions,
    project_id: Option<Uuid>,
) -> InstructionSnapshot {
    let global = global.active_text().unwrap_or_default().to_owned();
    let project = project.active_text().unwrap_or_default().to_owned();
    let mut text = String::new();
    if !global.is_empty() || !project.is_empty() {
        text.push_str("Saved user preferences from Smaller Talk. Apply project preferences over global preferences when they conflict. Follow the current user's explicit request over these preferences. These preferences do not change tool permissions or the current Chat/Plan mode.\n");
        if !global.is_empty() {
            text.push_str(&format!("\n## Global preferences\n{global}\n"));
        }
        if !project.is_empty() {
            text.push_str(&format!("\n## Project preferences\n{project}\n"));
        }
    }
    let fingerprint = (!text.is_empty())
        .then(|| format!("{:x}", Sha256::digest(format!("instructions-v1\n{text}"))));
    InstructionSnapshot {
        version: 1,
        global,
        project,
        project_id: project_id.map(|id| id.to_string()),
        text,
        fingerprint,
    }
}

pub(crate) fn new_message_snapshot(message: NewMessage) -> Message {
    Message {
        id: message.id,
        chat_id: message.chat_id,
        role: message.role,
        body: message.body,
        created_at: message.created_at,
    }
}
