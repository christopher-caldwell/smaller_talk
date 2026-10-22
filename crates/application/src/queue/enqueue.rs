use crate::{PersistenceError, UnitOfWork, UnitOfWorkFactory};
use async_trait::async_trait;
use smaller_talk_domain::{MAX_QUEUED_MESSAGES, QueueCapacity, QueuedMessage};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum EnqueueError {
    #[error("This chat already has {limit} queued messages. Wait for delivery or remove an entry.")]
    Full { limit: u64 },
    #[error("Add a message or attachment first")]
    EmptyMessage,
    #[error("Could not enqueue the message: {0}")]
    Storage(#[from] PersistenceError),
    #[error("Message queue is unavailable")]
    Unavailable,
}

/// Authoritative reads and draft transfer share the supplied transaction.
#[async_trait]
pub trait QueueStore<U: UnitOfWork>: Send + Sync {
    async fn capacity(&self, uow: &mut U, chat: Uuid) -> Result<QueueCapacity, PersistenceError>;
    async fn enqueue(&self, uow: &mut U, entry: &QueuedMessage) -> Result<(), PersistenceError>;
}

pub struct EnqueueService<U: UnitOfWork> {
    transactions: Arc<dyn UnitOfWorkFactory<U>>,
    store: Arc<dyn QueueStore<U>>,
}
impl<U: UnitOfWork> EnqueueService<U> {
    pub fn new(transactions: Arc<dyn UnitOfWorkFactory<U>>, store: Arc<dyn QueueStore<U>>) -> Self {
        Self {
            transactions,
            store,
        }
    }

    pub async fn enqueue(&self, entry: &QueuedMessage) -> Result<(), EnqueueError> {
        if entry.draft.text.trim().is_empty() && entry.draft.attachments.is_empty() {
            return Err(EnqueueError::EmptyMessage);
        }
        let mut uow = self.transactions.begin().await?;
        let capacity = self.store.capacity(&mut uow, entry.chat_id).await?;
        if !capacity.can_enqueue() {
            return Err(EnqueueError::Full {
                limit: MAX_QUEUED_MESSAGES,
            });
        }
        self.store.enqueue(&mut uow, entry).await?;
        uow.commit().await?;
        Ok(())
    }
}
