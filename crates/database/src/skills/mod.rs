use crate::SqliteChatStore;
use async_trait::async_trait;
use smaller_talk_application::{PersistenceError, SkillQueries};
use smaller_talk_domain::SkillMention;
use uuid::Uuid;

pub(crate) async fn save(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    message: Uuid,
    skills: &[SkillMention],
) -> Result<(), PersistenceError> {
    let message = message.to_string();
    let payload = serde_json::to_string(skills).map_err(PersistenceError::new)?;
    sqlx::query_file!("src/skills/sql/save.sql", message, payload)
        .execute(&mut **tx)
        .await
        .map_err(PersistenceError::new)?;
    Ok(())
}
#[async_trait]
impl SkillQueries for SqliteChatStore {
    async fn message_skills(
        &self,
        chat: Uuid,
        message: Uuid,
    ) -> Result<Vec<SkillMention>, PersistenceError> {
        let chat = chat.to_string();
        let message = message.to_string();
        let payload = sqlx::query_file_scalar!("src/skills/sql/message.sql", chat, message)
            .fetch_optional(self.pool())
            .await
            .map_err(PersistenceError::new)?;
        payload
            .map(|p| serde_json::from_str(&p).map_err(PersistenceError::new))
            .transpose()
            .map(|s| s.unwrap_or_default())
    }
}
