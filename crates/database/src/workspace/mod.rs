use std::path::PathBuf;

use async_trait::async_trait;
use futures_util::TryStreamExt;
use smaller_talk_application::{
    PersistenceError, SteeredInputStore, WorkspaceQueries, WorkspaceWrites,
};
use smaller_talk_domain::*;
use uuid::Uuid;

use crate::SqliteChatStore;

fn encode(value: &impl serde::Serialize) -> Result<String, PersistenceError> {
    serde_json::to_string(value).map_err(PersistenceError::new)
}
fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, PersistenceError> {
    serde_json::from_str(value).map_err(PersistenceError::new)
}

struct WorkspaceMessageRow {
    id: String,
    chat_id: String,
    role: String,
    body: String,
    created_at: i64,
}

fn stored_message(row: WorkspaceMessageRow) -> Result<Message, PersistenceError> {
    Ok(Message {
        id: Uuid::parse_str(&row.id).map_err(PersistenceError::new)?,
        chat_id: Uuid::parse_str(&row.chat_id).map_err(PersistenceError::new)?,
        role: match row.role.as_str() {
            "user" => MessageRole::User,
            "assistant" => MessageRole::Assistant,
            other => {
                return Err(PersistenceError::new(std::io::Error::other(format!(
                    "invalid stored message role: {other}"
                ))));
            }
        },
        body: row.body,
        created_at: chrono::DateTime::from_timestamp_millis(row.created_at).ok_or_else(|| {
            PersistenceError::new(std::io::Error::other("invalid message timestamp"))
        })?,
    })
}

async fn insert_transactional_message(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    message: smaller_talk_application::NewMessage,
) -> Result<Message, PersistenceError> {
    let id = message.id.to_string();
    let chat_id = message.chat_id.to_string();
    let role = match message.role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
    };
    let created_at = message.created_at.timestamp_millis();
    let row = sqlx::query_file_as!(
        WorkspaceMessageRow,
        "src/chats/sql/insert_message.sql",
        id,
        chat_id,
        role,
        message.body,
        created_at
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(PersistenceError::new)?;
    stored_message(row)
}

#[async_trait]
impl WorkspaceQueries for SqliteChatStore {
    async fn chat_previews(&self) -> Result<Vec<(Uuid, String)>, PersistenceError> {
        let rows = sqlx::query_file!("src/workspace/sql/chat_previews.sql")
            .fetch_all(self.pool())
            .await
            .map_err(PersistenceError::new)?;
        rows.into_iter()
            .filter_map(|row| row.preview.map(|text| (row.id, text)))
            .map(|(id, text)| Ok((Uuid::parse_str(&id).map_err(PersistenceError::new)?, text)))
            .collect()
    }

    async fn last_turn(&self, chat_id: Uuid) -> Result<Option<TurnSummary>, PersistenceError> {
        let id = chat_id.to_string();
        sqlx::query_file!("src/workspace/sql/last_turn.sql", id)
            .fetch_optional(self.pool())
            .await
            .map_err(PersistenceError::new)?
            .map(|row| {
                Ok(TurnSummary {
                    user_message_id: Uuid::parse_str(&row.user_message_id)
                        .map_err(PersistenceError::new)?,
                    status: row.status,
                    error: row.error,
                })
            })
            .transpose()
    }
    async fn accepted_turn_inputs(
        &self,
        chat_id: Uuid,
        original_message_id: Uuid,
    ) -> Result<Vec<Uuid>, PersistenceError> {
        let chat_id = chat_id.to_string();
        let original_message_id = original_message_id.to_string();
        sqlx::query_file_scalar!(
            "src/workspace/sql/accepted_turn_inputs.sql",
            chat_id,
            original_message_id
        )
        .fetch_all(self.pool())
        .await
        .map_err(PersistenceError::new)?
        .into_iter()
        .map(|id| Uuid::parse_str(&id).map_err(PersistenceError::new))
        .collect()
    }
    async fn list_projects(&self) -> Result<Vec<Project>, PersistenceError> {
        sqlx::query_file!("src/workspace/sql/list_projects.sql")
            .fetch_all(self.pool())
            .await
            .map_err(PersistenceError::new)?
            .into_iter()
            .map(|r| {
                Ok(Project {
                    id: Uuid::parse_str(&r.id).map_err(PersistenceError::new)?,
                    name: r.name,
                    folder: r.folder.map(PathBuf::from),
                })
            })
            .collect()
    }
    async fn preferences(&self, chat_id: Uuid) -> Result<ChatPreferences, PersistenceError> {
        let id = chat_id.to_string();
        let row = sqlx::query_file!("src/workspace/sql/preferences.sql", id)
            .fetch_optional(self.pool())
            .await
            .map_err(PersistenceError::new)?;
        match row {
            Some(row) => {
                let mut p: ChatPreferences = decode(&row.payload)?;
                p.project_id = row.project_id;
                Ok(p)
            }
            None => Ok(ChatPreferences::default()),
        }
    }
    async fn draft(&self, chat_id: Uuid) -> Result<Draft, PersistenceError> {
        let id = chat_id.to_string();
        let row = sqlx::query_file_scalar!("src/workspace/sql/draft.sql", id)
            .fetch_optional(self.pool())
            .await
            .map_err(PersistenceError::new)?;
        row.map(|r| decode(&r))
            .transpose()
            .map(|d| d.unwrap_or_default())
    }
    async fn session(&self) -> Result<SessionState, PersistenceError> {
        let row = sqlx::query_file_scalar!("src/workspace/sql/session.sql")
            .fetch_optional(self.pool())
            .await
            .map_err(PersistenceError::new)?;
        row.map(|r| decode(&r))
            .transpose()
            .map(|d| d.unwrap_or_default())
    }
    async fn search(&self, query: &str) -> Result<Vec<SearchHit>, PersistenceError> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        // SQLite's built-in lower()/NOCASE only fold ASCII. Stream rows in chat order
        // and match here so Unicode names work without loading history into the UI
        // or requiring a non-portable SQLite extension at build time.
        let mut rows = sqlx::query_file!("src/workspace/sql/search.sql").fetch(self.pool());
        let mut hits: Vec<SearchHit> = Vec::new();
        while let Some(row) = rows.try_next().await.map_err(PersistenceError::new)? {
            let id = Uuid::parse_str(&row.chat_id).map_err(PersistenceError::new)?;
            let existing = hits.iter().position(|h| h.chat_id == id);
            if existing.is_none() && hits.len() == 50 {
                break;
            }
            if existing.is_some_and(|index| hits[index].message_id.is_some()) {
                continue;
            }
            let message_match = row
                .body
                .as_ref()
                .is_some_and(|body| body.to_lowercase().contains(&needle));
            if !message_match
                && !row.title.to_lowercase().contains(&needle)
                && !row
                    .project
                    .as_ref()
                    .is_some_and(|p| p.to_lowercase().contains(&needle))
            {
                continue;
            }
            let excerpt = if message_match {
                search_excerpt(row.body.as_deref().unwrap_or_default(), &needle)
            } else {
                row.title.clone()
            };
            let hit = SearchHit {
                chat_id: id,
                title: row.title,
                project: row.project,
                message_id: if message_match {
                    row.message_id
                        .map(|id| Uuid::parse_str(&id).map_err(PersistenceError::new))
                        .transpose()?
                } else {
                    None
                },
                excerpt,
            };
            if let Some(index) = existing {
                hits[index] = hit;
            } else {
                hits.push(hit);
            }
        }
        Ok(hits)
    }
    async fn message_attachments(
        &self,
        chat_id: Uuid,
    ) -> Result<Vec<(Uuid, Attachment)>, PersistenceError> {
        let id = chat_id.to_string();
        sqlx::query_file!("src/workspace/sql/message_attachments.sql", id)
            .fetch_all(self.pool())
            .await
            .map_err(PersistenceError::new)?
            .into_iter()
            .map(|r| {
                Ok((
                    Uuid::parse_str(&r.message_id).map_err(PersistenceError::new)?,
                    decode(&r.payload)?,
                ))
            })
            .collect()
    }
    async fn attachment_references(&self) -> Result<Vec<PathBuf>, PersistenceError> {
        let mut paths: Vec<_> =
            sqlx::query_file_scalar!("src/workspace/sql/attachment_references.sql")
                .fetch_all(self.pool())
                .await
                .map_err(PersistenceError::new)?
                .into_iter()
                .map(PathBuf::from)
                .collect();
        if let Some(new_chat) = self.session().await?.new_chat {
            for file in new_chat.draft.attachments {
                paths.push(file.path);
                if let Some(preview) = file.preview_path {
                    paths.push(preview);
                }
            }
        }
        Ok(paths)
    }
}

#[async_trait]
impl WorkspaceWrites for SqliteChatStore {
    async fn recover_interrupted_turns(&self) -> Result<(), PersistenceError> {
        sqlx::query_file!("src/workspace/sql/recover_turns.sql")
            .execute(self.pool())
            .await
            .map_err(PersistenceError::new)?;
        Ok(())
    }
    async fn save_project(&self, project: &Project) -> Result<(), PersistenceError> {
        let id = project.id.to_string();
        let folder = project
            .folder
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());
        sqlx::query_file!(
            "src/workspace/sql/save_project.sql",
            id,
            project.name,
            folder
        )
        .execute(self.pool())
        .await
        .map_err(PersistenceError::new)?;
        Ok(())
    }
    async fn remove_project(&self, id: Uuid) -> Result<bool, PersistenceError> {
        let id = id.to_string();
        Ok(
            sqlx::query_file!("src/workspace/sql/remove_project.sql", id)
                .execute(self.pool())
                .await
                .map_err(PersistenceError::new)?
                .rows_affected()
                > 0,
        )
    }
    async fn save_preferences(
        &self,
        chat_id: Uuid,
        preferences: &ChatPreferences,
    ) -> Result<bool, PersistenceError> {
        let id = chat_id.to_string();
        let payload = encode(preferences)?;
        Ok(sqlx::query_file!(
            "src/workspace/sql/save_preferences.sql",
            preferences.project_id,
            payload,
            id
        )
        .execute(self.pool())
        .await
        .map_err(PersistenceError::new)?
        .rows_affected()
            > 0)
    }
    async fn save_draft(&self, chat_id: Uuid, draft: &Draft) -> Result<(), PersistenceError> {
        let id = chat_id.to_string();
        let payload = encode(draft)?;
        sqlx::query_file!("src/workspace/sql/save_draft.sql", payload, id)
            .execute(self.pool())
            .await
            .map_err(PersistenceError::new)?;
        Ok(())
    }
    async fn save_session(&self, session: &SessionState) -> Result<(), PersistenceError> {
        let payload = encode(session)?;
        sqlx::query_file!("src/workspace/sql/save_session.sql", payload)
            .execute(self.pool())
            .await
            .map_err(PersistenceError::new)?;
        Ok(())
    }
    async fn save_message_attachments(
        &self,
        message_id: Uuid,
        attachments: &[Attachment],
    ) -> Result<(), PersistenceError> {
        let message = message_id.to_string();
        // One logical attachment association; either all files are linked or none are.
        let mut tx = self.pool().begin().await.map_err(PersistenceError::new)?;
        for a in attachments {
            let payload = encode(a)?;
            let path = a.path.to_string_lossy().into_owned();
            sqlx::query_file!(
                "src/workspace/sql/save_attachment.sql",
                a.id,
                message,
                path,
                payload
            )
            .execute(&mut *tx)
            .await
            .map_err(PersistenceError::new)?;
        }
        tx.commit().await.map_err(PersistenceError::new)?;
        Ok(())
    }
    async fn save_turn_input(&self, turn: Uuid, message: Uuid) -> Result<(), PersistenceError> {
        let turn = turn.to_string();
        let message = message.to_string();
        sqlx::query_file!("src/workspace/sql/turn_input.sql", turn, message)
            .execute(self.pool())
            .await
            .map_err(PersistenceError::new)?;
        Ok(())
    }
    async fn save_turn(&self, turn: &TurnRecord) -> Result<(), PersistenceError> {
        let id = turn.id.to_string();
        let chat = turn.chat_id.to_string();
        let message = turn.user_message_id.to_string();
        let settings = encode(&turn.settings)?;
        let workspace = turn.workspace.to_string_lossy().into_owned();
        sqlx::query_file!(
            "src/workspace/sql/save_turn.sql",
            id,
            chat,
            message,
            settings,
            workspace,
            turn.status,
            turn.error
        )
        .execute(self.pool())
        .await
        .map_err(PersistenceError::new)?;
        Ok(())
    }
}

#[async_trait]
impl SteeredInputStore for SqliteChatStore {
    async fn persist_steered_input(
        &self,
        assistant: Option<smaller_talk_application::NewMessage>,
        user: smaller_talk_application::NewMessage,
        attachments: &[Attachment],
        turn: Uuid,
    ) -> Result<(Option<Message>, Message), PersistenceError> {
        let mut tx = self.pool().begin().await.map_err(PersistenceError::new)?;
        let assistant = match assistant {
            Some(message) => Some(insert_transactional_message(&mut tx, message).await?),
            None => None,
        };
        let user_message = insert_transactional_message(&mut tx, user).await?;
        let message_id = user_message.id.to_string();
        for attachment in attachments {
            let payload = encode(attachment)?;
            let path = attachment.path.to_string_lossy().into_owned();
            sqlx::query_file!(
                "src/workspace/sql/save_attachment.sql",
                attachment.id,
                message_id,
                path,
                payload
            )
            .execute(&mut *tx)
            .await
            .map_err(PersistenceError::new)?;
        }
        let turn_id = turn.to_string();
        sqlx::query_file!("src/workspace/sql/turn_input.sql", turn_id, message_id)
            .execute(&mut *tx)
            .await
            .map_err(PersistenceError::new)?;
        tx.commit().await.map_err(PersistenceError::new)?;
        Ok((assistant, user_message))
    }
}

fn search_excerpt(body: &str, needle: &str) -> String {
    let offset = body.to_lowercase().find(needle).unwrap_or(0);
    let mut folded_bytes = 0;
    let mut character = 0usize;
    for ch in body.chars() {
        if folded_bytes >= offset {
            break;
        }
        folded_bytes += ch.to_lowercase().map(char::len_utf8).sum::<usize>();
        character += 1;
    }
    body.chars()
        .skip(character.saturating_sub(70))
        .take(240)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
