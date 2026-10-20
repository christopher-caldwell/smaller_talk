use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use smaller_talk_domain::*;
use thiserror::Error;
use uuid::Uuid;

use crate::PersistenceError;

#[async_trait]
pub trait WorkspaceQueries: Send + Sync {
    async fn chat_previews(&self) -> Result<Vec<(Uuid, String)>, PersistenceError>;
    async fn last_turn(&self, chat_id: Uuid) -> Result<Option<TurnSummary>, PersistenceError>;
    async fn accepted_turn_inputs(
        &self,
        chat_id: Uuid,
        original_message_id: Uuid,
    ) -> Result<Vec<Uuid>, PersistenceError>;
    async fn list_projects(&self) -> Result<Vec<Project>, PersistenceError>;
    async fn preferences(&self, chat_id: Uuid) -> Result<ChatPreferences, PersistenceError>;
    async fn draft(&self, chat_id: Uuid) -> Result<Draft, PersistenceError>;
    async fn session(&self) -> Result<SessionState, PersistenceError>;
    async fn search(&self, query: &str) -> Result<Vec<SearchHit>, PersistenceError>;
    async fn message_attachments(
        &self,
        chat_id: Uuid,
    ) -> Result<Vec<(Uuid, Attachment)>, PersistenceError>;
    async fn attachment_references(&self) -> Result<Vec<PathBuf>, PersistenceError>;
}

#[async_trait]
pub trait WorkspaceWrites: Send + Sync {
    async fn recover_interrupted_turns(&self) -> Result<(), PersistenceError>;
    async fn save_project(&self, project: &Project) -> Result<(), PersistenceError>;
    async fn remove_project(&self, id: Uuid) -> Result<bool, PersistenceError>;
    async fn save_preferences(
        &self,
        chat_id: Uuid,
        preferences: &ChatPreferences,
    ) -> Result<bool, PersistenceError>;
    async fn save_draft(&self, chat_id: Uuid, draft: &Draft) -> Result<(), PersistenceError>;
    async fn save_session(&self, session: &SessionState) -> Result<(), PersistenceError>;
    async fn save_message_attachments(
        &self,
        message_id: Uuid,
        attachments: &[Attachment],
    ) -> Result<(), PersistenceError>;
    async fn save_turn_input(&self, turn: Uuid, message: Uuid) -> Result<(), PersistenceError>;
    async fn save_turn(&self, turn: &TurnRecord) -> Result<(), PersistenceError>;
}

#[async_trait]
pub trait SteeredInputStore: Send + Sync {
    /// Persist the assistant segment and accepted steering input as one atomic
    /// local mutation, including its attachments and turn association.
    async fn persist_steered_input(
        &self,
        assistant: Option<crate::NewMessage>,
        user: crate::NewMessage,
        attachments: &[Attachment],
        turn: Uuid,
    ) -> Result<(Option<Message>, Message), PersistenceError>;
}

#[async_trait]
pub trait AttachmentStorage: Send + Sync {
    async fn import(&self, paths: Vec<PathBuf>) -> Result<Vec<Attachment>, crate::ExecutionError>;
    async fn collect_unreferenced(
        &self,
        referenced: Vec<PathBuf>,
    ) -> Result<(), crate::ExecutionError>;
}

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("Workspace storage failed: {0}")]
    Storage(#[from] PersistenceError),
    #[error("A project name cannot be empty")]
    EmptyName,
    #[error("The working folder does not exist: {0}")]
    MissingFolder(String),
    #[error("The selected chat or project no longer exists")]
    NotFound,
}

pub struct WorkspaceService {
    queries: Arc<dyn WorkspaceQueries>,
    writes: Arc<dyn WorkspaceWrites>,
}

impl WorkspaceService {
    pub fn new(queries: Arc<dyn WorkspaceQueries>, writes: Arc<dyn WorkspaceWrites>) -> Self {
        Self { queries, writes }
    }
    pub async fn list_projects(&self) -> Result<Vec<Project>, WorkspaceError> {
        Ok(self.queries.list_projects().await?)
    }
    pub async fn preferences(&self, id: Uuid) -> Result<ChatPreferences, WorkspaceError> {
        Ok(self.queries.preferences(id).await?)
    }
    pub async fn draft(&self, id: Uuid) -> Result<Draft, WorkspaceError> {
        Ok(self.queries.draft(id).await?)
    }
    pub async fn session(&self) -> Result<SessionState, WorkspaceError> {
        Ok(self.queries.session().await?)
    }
    pub async fn chat_previews(&self) -> Result<Vec<(Uuid, String)>, WorkspaceError> {
        Ok(self.queries.chat_previews().await?)
    }
    pub async fn search(&self, text: &str) -> Result<Vec<SearchHit>, WorkspaceError> {
        Ok(self.queries.search(text).await?)
    }
    pub async fn message_attachments(
        &self,
        id: Uuid,
    ) -> Result<Vec<(Uuid, Attachment)>, WorkspaceError> {
        Ok(self.queries.message_attachments(id).await?)
    }
    pub async fn attachment_references(&self) -> Result<Vec<PathBuf>, WorkspaceError> {
        Ok(self.queries.attachment_references().await?)
    }
    pub async fn last_turn(&self, id: Uuid) -> Result<Option<TurnSummary>, WorkspaceError> {
        Ok(self.queries.last_turn(id).await?)
    }
    pub async fn save_draft(&self, id: Uuid, draft: &Draft) -> Result<(), WorkspaceError> {
        Ok(self.writes.save_draft(id, draft).await?)
    }
    pub async fn save_session(&self, session: &SessionState) -> Result<(), WorkspaceError> {
        Ok(self.writes.save_session(session).await?)
    }
    pub async fn recover_interrupted_turns(&self) -> Result<(), WorkspaceError> {
        Ok(self.writes.recover_interrupted_turns().await?)
    }
    pub async fn save_project(&self, mut project: Project) -> Result<Project, WorkspaceError> {
        project.name = project.name.trim().to_owned();
        if project.name.is_empty() {
            return Err(WorkspaceError::EmptyName);
        }
        if let Some(folder) = &project.folder {
            if !folder.is_dir() {
                return Err(WorkspaceError::MissingFolder(folder.display().to_string()));
            }
            project.folder = Some(folder.canonicalize().map_err(PersistenceError::new)?);
        }
        self.writes.save_project(&project).await?;
        Ok(project)
    }
    pub async fn remove_project(&self, id: Uuid) -> Result<(), WorkspaceError> {
        if self.writes.remove_project(id).await? {
            Ok(())
        } else {
            Err(WorkspaceError::NotFound)
        }
    }
    pub async fn save_preferences(
        &self,
        chat_id: Uuid,
        mut preferences: ChatPreferences,
    ) -> Result<(), WorkspaceError> {
        if let Some(id) = &preferences.project_id
            && !self
                .queries
                .list_projects()
                .await?
                .iter()
                .any(|p| p.id.to_string() == *id)
        {
            return Err(WorkspaceError::NotFound);
        }
        let old = self.queries.preferences(chat_id).await?;
        if old.execution.provider != preferences.execution.provider
            || old.project_id != preferences.project_id
        {
            preferences.new_session = true;
        }
        if self.writes.save_preferences(chat_id, &preferences).await? {
            Ok(())
        } else {
            Err(WorkspaceError::NotFound)
        }
    }
}
