use crate::SqliteChatStore;
use async_trait::async_trait;
use smaller_talk_application::{
    DisplayQueries, DisplayWrites, PersistenceError, SettingsQueries, SettingsWrites,
};
use smaller_talk_domain::*;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct FinalAnswerRow {
    message_id: String,
    text: String,
}

#[derive(sqlx::FromRow)]
struct VariantRow {
    message_id: String,
    source_hash: String,
    provider: String,
    model: String,
    prompt_version: i64,
    text: String,
}

#[async_trait]
impl SettingsQueries for SqliteChatStore {
    async fn app_settings(&self) -> Result<AppSettings, PersistenceError> {
        let payload = sqlx::query_file_scalar!("src/settings/sql/load.sql")
            .fetch_optional(self.pool())
            .await
            .map_err(PersistenceError::new)?;
        payload
            .map(|text| serde_json::from_str(&text).map_err(PersistenceError::new))
            .transpose()
            .map(|s| s.unwrap_or_default())
    }
}
#[async_trait]
impl SettingsWrites for SqliteChatStore {
    async fn save_app_settings(&self, settings: &AppSettings) -> Result<(), PersistenceError> {
        let payload = serde_json::to_string(settings).map_err(PersistenceError::new)?;
        sqlx::query_file!("src/settings/sql/save.sql", payload)
            .execute(self.pool())
            .await
            .map_err(PersistenceError::new)?;
        Ok(())
    }
}
#[async_trait]
impl DisplayQueries for SqliteChatStore {
    async fn final_answers(&self, chat: Uuid) -> Result<Vec<FinalAnswer>, PersistenceError> {
        let chat = chat.to_string();
        sqlx::query_file_as!(FinalAnswerRow, "src/settings/sql/final_answers.sql", chat)
            .fetch_all(self.pool())
            .await
            .map_err(PersistenceError::new)?
            .into_iter()
            .map(|r| {
                Ok(FinalAnswer {
                    message_id: Uuid::parse_str(&r.message_id).map_err(PersistenceError::new)?,
                    text: r.text,
                })
            })
            .collect()
    }
    async fn variants(&self, chat: Uuid) -> Result<Vec<ResponseVariant>, PersistenceError> {
        let chat = chat.to_string();
        sqlx::query_file_as!(VariantRow, "src/settings/sql/variants.sql", chat)
            .fetch_all(self.pool())
            .await
            .map_err(PersistenceError::new)?
            .into_iter()
            .map(|r| {
                Ok(ResponseVariant {
                    message_id: Uuid::parse_str(&r.message_id).map_err(PersistenceError::new)?,
                    source_hash: r.source_hash,
                    provider: match r.provider.as_str() {
                        "codex" => Provider::Codex,
                        "claude" => Provider::Claude,
                        _ => {
                            return Err(PersistenceError::new(std::io::Error::other(
                                "Unknown rewrite provider",
                            )));
                        }
                    },
                    model: r.model,
                    prompt_version: r.prompt_version,
                    text: r.text,
                })
            })
            .collect()
    }
}
#[async_trait]
impl DisplayWrites for SqliteChatStore {
    async fn save_final_answer(&self, answer: &FinalAnswer) -> Result<(), PersistenceError> {
        let id = answer.message_id.to_string();
        sqlx::query_file!("src/settings/sql/save_final.sql", id, answer.text)
            .execute(self.pool())
            .await
            .map_err(PersistenceError::new)?;
        Ok(())
    }
    async fn save_variant(&self, variant: &ResponseVariant) -> Result<(), PersistenceError> {
        let id = variant.message_id.to_string();
        let provider = variant.provider.key();
        sqlx::query_file!(
            "src/settings/sql/save_variant.sql",
            id,
            variant.source_hash,
            provider,
            variant.model,
            variant.prompt_version,
            variant.text
        )
        .execute(self.pool())
        .await
        .map_err(PersistenceError::new)?;
        Ok(())
    }
}
