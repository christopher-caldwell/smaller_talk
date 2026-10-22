use crate::PersistenceError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ListProjectsError {
    #[error("Workspace storage failed: {0}")]
    Storage(#[from] PersistenceError),
}

#[derive(Debug, Error)]
pub enum LoadChatPreferencesError {
    #[error("Workspace storage failed: {0}")]
    Storage(#[from] PersistenceError),
}

#[derive(Debug, Error)]
pub enum LoadDraftError {
    #[error("Workspace storage failed: {0}")]
    Storage(#[from] PersistenceError),
}

#[derive(Debug, Error)]
pub enum LoadSessionError {
    #[error("Workspace storage failed: {0}")]
    Storage(#[from] PersistenceError),
}

#[derive(Debug, Error)]
pub enum ListChatPreviewsError {
    #[error("Workspace storage failed: {0}")]
    Storage(#[from] PersistenceError),
}

#[derive(Debug, Error)]
pub enum SearchChatsError {
    #[error("Workspace storage failed: {0}")]
    Storage(#[from] PersistenceError),
}

#[derive(Debug, Error)]
pub enum ListMessageAttachmentsError {
    #[error("Workspace storage failed: {0}")]
    Storage(#[from] PersistenceError),
}

#[derive(Debug, Error)]
pub enum ListAttachmentReferencesError {
    #[error("Workspace storage failed: {0}")]
    Storage(#[from] PersistenceError),
}

#[derive(Debug, Error)]
pub enum LoadLastTurnError {
    #[error("Workspace storage failed: {0}")]
    Storage(#[from] PersistenceError),
}

#[derive(Debug, Error)]
pub enum SaveDraftError {
    #[error("Workspace storage failed: {0}")]
    Storage(#[from] PersistenceError),
}

#[derive(Debug, Error)]
pub enum SaveSessionError {
    #[error("Workspace storage failed: {0}")]
    Storage(#[from] PersistenceError),
}

#[derive(Debug, Error)]
pub enum RecoverInterruptedTurnsError {
    #[error("Workspace storage failed: {0}")]
    Storage(#[from] PersistenceError),
}

#[derive(Debug, Error)]
pub enum SaveProjectError {
    #[error("Workspace storage failed: {0}")]
    Storage(#[from] PersistenceError),
    #[error("A project name cannot be empty")]
    EmptyName,
    #[error("The working folder does not exist: {0}")]
    MissingFolder(String),
}

#[derive(Debug, Error)]
pub enum RemoveProjectError {
    #[error("Workspace storage failed: {0}")]
    Storage(#[from] PersistenceError),
    #[error("The selected chat or project no longer exists")]
    NotFound,
}

#[derive(Debug, Error)]
pub enum SaveChatPreferencesError {
    #[error("Workspace storage failed: {0}")]
    Storage(#[from] PersistenceError),
    #[error("The selected chat or project no longer exists")]
    NotFound,
}
