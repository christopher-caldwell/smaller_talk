use crate::{ExecutionError, PersistenceError};
use async_trait::async_trait;
use smaller_talk_domain::*;
use std::{path::PathBuf, sync::Arc};

#[async_trait]
pub trait SettingsQueries: Send + Sync {
    async fn app_settings(&self) -> Result<AppSettings, PersistenceError>;
}

#[async_trait]
pub trait SettingsWrites: Send + Sync {
    async fn save_app_settings(&self, settings: &AppSettings) -> Result<(), PersistenceError>;
}

#[async_trait]
pub trait ProviderControl: Send + Sync {
    /// Entry resolves the executable. Existing executions retain their original adapter.
    fn configure(&self, provider: Provider, executable: PathBuf, enabled: bool);
    async fn readiness(&self, provider: Provider) -> ProviderReadiness;
    async fn refresh(&self, provider: Provider) -> Result<ProviderCapabilities, ExecutionError>;
}

#[derive(Debug, thiserror::Error)]
pub enum LoadSettingsError {
    #[error("Settings storage failed: {0}")]
    Storage(#[from] PersistenceError),
}

#[derive(Debug, thiserror::Error)]
pub enum SaveSettingsError {
    #[error("Settings storage failed: {0}")]
    Storage(#[from] PersistenceError),
    #[error("Choose a rewrite model before enabling automatic shortening")]
    RewriteModelRequired,
}

pub struct SettingsService {
    queries: Arc<dyn SettingsQueries>,
    writes: Arc<dyn SettingsWrites>,
}
impl SettingsService {
    pub fn new(queries: Arc<dyn SettingsQueries>, writes: Arc<dyn SettingsWrites>) -> Self {
        Self { queries, writes }
    }
    pub async fn load(&self) -> Result<AppSettings, LoadSettingsError> {
        Ok(self.queries.app_settings().await?)
    }
    pub async fn save(&self, settings: &AppSettings) -> Result<(), SaveSettingsError> {
        settings.validate().map_err(|error| match error {
            AppSettingsValidationError::RewriteModelRequired => {
                SaveSettingsError::RewriteModelRequired
            }
        })?;
        Ok(self.writes.save_app_settings(settings).await?)
    }
}

#[async_trait]
pub trait DisplayQueries: Send + Sync {
    async fn final_answers(&self, chat: uuid::Uuid) -> Result<Vec<FinalAnswer>, PersistenceError>;
    async fn variants(&self, chat: uuid::Uuid) -> Result<Vec<ResponseVariant>, PersistenceError>;
}

#[async_trait]
pub trait DisplayWrites: Send + Sync {
    async fn save_final_answer(&self, answer: &FinalAnswer) -> Result<(), PersistenceError>;
    async fn save_variant(&self, variant: &ResponseVariant) -> Result<(), PersistenceError>;
}
