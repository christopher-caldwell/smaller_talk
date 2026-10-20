use crate::ExecutionError;
use async_trait::async_trait;
use smaller_talk_domain::*;
use std::path::{Path, PathBuf};

#[async_trait]
pub trait RepositoryReader: Send + Sync {
    async fn status(
        &self,
        folder: &Path,
        scope: DiffScope,
    ) -> Result<Option<RepositoryStatus>, ExecutionError>;
    async fn diff(
        &self,
        root: &Path,
        file: &ChangedFile,
        scope: DiffScope,
    ) -> Result<FileDiff, ExecutionError>;
    async fn pull_request(&self, root: PathBuf) -> Option<String>;
}
