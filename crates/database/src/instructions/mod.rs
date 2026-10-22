use crate::{
    SqliteChatStore,
    workspace::{insert_transactional_message, save_turn_record},
};
use async_trait::async_trait;
use smaller_talk_application::*;
use smaller_talk_domain::*;
use uuid::Uuid;

fn snapshot(value: Option<String>) -> Result<InstructionSnapshot, PersistenceError> {
    value
        .map(|s| serde_json::from_str(&s).map_err(PersistenceError::new))
        .transpose()
        .map(|s| s.unwrap_or_default())
}

#[async_trait]
impl InstructionQueries for SqliteChatStore {
    async fn project_instructions(
        &self,
        project: Uuid,
    ) -> Result<Option<CustomInstructions>, PersistenceError> {
        let id = project.to_string();
        Ok(sqlx::query_file!("src/instructions/sql/project.sql", id)
            .fetch_optional(self.pool())
            .await
            .map_err(PersistenceError::new)?
            .map(|r| CustomInstructions {
                enabled: r.enabled != 0,
                text: r.text,
            }))
    }

    async fn message_instructions(
        &self,
        chat: Uuid,
        message: Uuid,
    ) -> Result<Option<InstructionSnapshot>, PersistenceError> {
        let chat = chat.to_string();
        let message = message.to_string();
        sqlx::query_file!("src/instructions/sql/message.sql", chat, message, message)
            .fetch_optional(self.pool())
            .await
            .map_err(PersistenceError::new)?
            .map(|r| snapshot(r.instructions))
            .transpose()
    }
}

#[async_trait]
impl InstructionWrites for SqliteChatStore {
    async fn save_project_instructions(
        &self,
        project: Uuid,
        instructions: &CustomInstructions,
    ) -> Result<bool, PersistenceError> {
        let id = project.to_string();
        let enabled = instructions.enabled;
        Ok(sqlx::query_file!(
            "src/instructions/sql/save_project.sql",
            enabled,
            instructions.text,
            id
        )
        .execute(self.pool())
        .await
        .map_err(PersistenceError::new)?
        .rows_affected()
            > 0)
    }
}

#[async_trait]
impl TurnStartWrites for SqliteChatStore {
    async fn record_turn_start(&self, input: RecordTurnStart) -> Result<Message, PersistenceError> {
        let mut tx = self.pool().begin().await.map_err(PersistenceError::new)?;
        let user = if input.insert_user {
            insert_transactional_message(
                &mut tx,
                NewMessage {
                    id: input.user.id,
                    chat_id: input.user.chat_id,
                    role: input.user.role,
                    body: input.user.body,
                    created_at: input.user.created_at,
                },
            )
            .await?
        } else {
            input.user
        };
        let message = user.id.to_string();
        for attachment in input.attachments {
            let payload = serde_json::to_string(&attachment).map_err(PersistenceError::new)?;
            let path = attachment.path.to_string_lossy().into_owned();
            sqlx::query_file!(
                "src/workspace/sql/save_attachment.sql",
                attachment.id,
                message,
                path,
                payload
            )
            .execute(&mut *tx)
            .await
            .map_err(PersistenceError::new)?;
        }
        if input.insert_user {
            crate::skills::save(&mut tx, user.id, &input.skills).await?;
        }
        save_turn_record(&mut *tx, &input.turn).await?;
        tx.commit().await.map_err(PersistenceError::new)?;
        Ok(user)
    }
}
