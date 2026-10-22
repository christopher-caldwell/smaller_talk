//! Application-owned transaction lifetime; concrete resources remain in adapters.
use crate::PersistenceError;
use async_trait::async_trait;

#[async_trait]
pub trait UnitOfWork: Send {
    async fn commit(self) -> Result<(), PersistenceError>;
}

#[async_trait]
pub trait UnitOfWorkFactory<U: UnitOfWork>: Send + Sync {
    async fn begin(&self) -> Result<U, PersistenceError>;
}
