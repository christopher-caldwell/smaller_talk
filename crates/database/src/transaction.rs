use crate::SqliteChatStore;
use async_trait::async_trait;
use smaller_talk_application::{PersistenceError, UnitOfWork, UnitOfWorkFactory};

/// Dropping SQLx's transaction schedules rollback before its connection is reused.
pub struct SqliteUnitOfWork {
    pub(crate) transaction: sqlx::Transaction<'static, sqlx::Sqlite>,
}

#[async_trait]
impl UnitOfWork for SqliteUnitOfWork {
    async fn commit(self) -> Result<(), PersistenceError> {
        self.transaction
            .commit()
            .await
            .map_err(PersistenceError::new)
    }
}

#[async_trait]
impl UnitOfWorkFactory<SqliteUnitOfWork> for SqliteChatStore {
    async fn begin(&self) -> Result<SqliteUnitOfWork, PersistenceError> {
        // Acquire SQLite's writer reservation before reading capacity. Concurrent
        // enqueue services must observe the previous writer's committed count.
        let transaction = self
            .pool()
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(PersistenceError::new)?;
        Ok(SqliteUnitOfWork { transaction })
    }
}
