use crate::{SqliteChatStore, SqliteUnitOfWork};
use async_trait::async_trait;
use smaller_talk_application::{PersistenceError, QueueQueries, QueueStore, QueueWrites};
use smaller_talk_domain::*;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct QueueRow {
    id: String,
    chat_id: String,
    payload: String,
    state: String,
    error: Option<String>,
}

#[async_trait]
impl QueueQueries for SqliteChatStore {
    async fn queue(&self, chat: Uuid) -> Result<MessageQueue, PersistenceError> {
        let chat = chat.to_string();
        let rows = sqlx::query_file_as!(QueueRow, "src/queue/sql/list.sql", chat)
            .fetch_all(self.pool())
            .await
            .map_err(PersistenceError::new)?;
        let paused = sqlx::query_file_scalar!("src/queue/sql/reason.sql", chat)
            .fetch_optional(self.pool())
            .await
            .map_err(PersistenceError::new)?
            .flatten();
        let entries = rows
            .into_iter()
            .map(|r| {
                Ok(QueuedMessage {
                    id: Uuid::parse_str(&r.id).map_err(PersistenceError::new)?,
                    chat_id: Uuid::parse_str(&r.chat_id).map_err(PersistenceError::new)?,
                    draft: serde_json::from_str(&r.payload).map_err(PersistenceError::new)?,
                    state: r.state,
                    error: r.error,
                })
            })
            .collect::<Result<_, PersistenceError>>()?;
        Ok(MessageQueue {
            entries,
            paused,
            steer_available: false,
        })
    }
}

#[async_trait]
impl QueueWrites for SqliteChatStore {
    async fn edit_queued(
        &self,
        chat: Uuid,
        entry: Uuid,
    ) -> Result<Option<Draft>, PersistenceError> {
        let chat = chat.to_string();
        let entry = entry.to_string();
        let mut tx = self.pool().begin().await.map_err(PersistenceError::new)?;
        let payload = sqlx::query_file_scalar!("src/queue/sql/edit.sql", chat, entry)
            .fetch_optional(&mut *tx)
            .await
            .map_err(PersistenceError::new)?;
        let draft = if let Some(payload) = payload {
            let draft: Draft = serde_json::from_str(&payload).map_err(PersistenceError::new)?;
            sqlx::query_file!("src/workspace/sql/save_draft.sql", payload, chat)
                .execute(&mut *tx)
                .await
                .map_err(PersistenceError::new)?;
            Some(draft)
        } else {
            None
        };
        tx.commit().await.map_err(PersistenceError::new)?;
        Ok(draft)
    }

    async fn transition(
        &self,
        id: Uuid,
        from: &str,
        to: &str,
        error: Option<&str>,
    ) -> Result<bool, PersistenceError> {
        let id = id.to_string();
        Ok(
            sqlx::query_file!("src/queue/sql/transition.sql", to, error, id, from)
                .execute(self.pool())
                .await
                .map_err(PersistenceError::new)?
                .rows_affected()
                == 1,
        )
    }
    async fn remove_queued(
        &self,
        chat: Uuid,
        id: Uuid,
        delivered: bool,
    ) -> Result<(), PersistenceError> {
        let chat = chat.to_string();
        let id = id.to_string();
        if delivered {
            sqlx::query_file!("src/queue/sql/delivered.sql", id, chat)
                .execute(self.pool())
                .await
                .map_err(PersistenceError::new)?;
        } else {
            sqlx::query_file!("src/queue/sql/remove.sql", id, chat)
                .execute(self.pool())
                .await
                .map_err(PersistenceError::new)?;
        }
        Ok(())
    }
    async fn pause_queue(&self, chat: Uuid, reason: Option<&str>) -> Result<(), PersistenceError> {
        let chat = chat.to_string();
        sqlx::query_file!("src/queue/sql/pause.sql", reason, chat)
            .execute(self.pool())
            .await
            .map_err(PersistenceError::new)?;
        Ok(())
    }
    async fn recover_queues(&self) -> Result<(), PersistenceError> {
        let mut tx = self.pool().begin().await.map_err(PersistenceError::new)?;
        sqlx::query_file!("src/queue/sql/recover.sql")
            .execute(&mut *tx)
            .await
            .map_err(PersistenceError::new)?;
        sqlx::query_file!("src/queue/sql/recover_delivery.sql")
            .execute(&mut *tx)
            .await
            .map_err(PersistenceError::new)?;
        tx.commit().await.map_err(PersistenceError::new)
    }
}

#[async_trait]
impl QueueStore<SqliteUnitOfWork> for SqliteChatStore {
    async fn capacity(
        &self,
        uow: &mut SqliteUnitOfWork,
        chat: Uuid,
    ) -> Result<QueueCapacity, PersistenceError> {
        let chat = chat.to_string();
        let count: i64 = sqlx::query_file_scalar!("src/queue/sql/count.sql", chat)
            .fetch_one(&mut *uow.transaction)
            .await
            .map_err(PersistenceError::new)?;
        Ok(QueueCapacity {
            queued: u64::try_from(count).map_err(PersistenceError::new)?,
        })
    }
    async fn enqueue(
        &self,
        uow: &mut SqliteUnitOfWork,
        entry: &QueuedMessage,
    ) -> Result<(), PersistenceError> {
        let chat = entry.chat_id.to_string();
        let id = entry.id.to_string();
        let payload = serde_json::to_string(&entry.draft).map_err(PersistenceError::new)?;
        sqlx::query_file!("src/queue/sql/enqueue.sql", id, chat, payload)
            .execute(&mut *uow.transaction)
            .await
            .map_err(PersistenceError::new)?;
        sqlx::query_file!("src/queue/sql/clear_draft.sql", chat, payload, payload)
            .execute(&mut *uow.transaction)
            .await
            .map_err(PersistenceError::new)?;
        Ok(())
    }
}
