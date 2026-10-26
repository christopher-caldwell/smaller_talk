use super::*;

#[derive(Debug, Error)]
pub enum ArchiveChatError {
    #[error("Wait until this chat is idle and its queue is empty before archiving.")]
    Busy,
    #[error("This chat no longer exists.")]
    NotFound,
    #[error("Could not update the chat archive: {0}")]
    Storage(#[from] PersistenceError),
}

#[derive(Clone, Debug)]
pub struct CompactionRequest {
    pub chat_id: Uuid,
    pub binding: ExecutionBinding,
    pub workspace: PathBuf,
    pub settings: ExecutionSettings,
    pub instructions: InstructionSnapshot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompactionOutcome {
    Compacted,
    NothingToCompact(String),
}

#[derive(Debug, Error)]
pub enum CompactConversationError {
    #[error("Could not compact this chat: {0}")]
    Storage(#[from] PersistenceError),
    #[error("{0}")]
    Unavailable(String),
    #[error("{0}")]
    Provider(#[from] ExecutionError),
}

impl ConversationService {
    pub async fn set_archived(&self, id: Uuid, archived: bool) -> Result<Chat, ArchiveChatError> {
        let _guard = if archived {
            if !self
                .active
                .lock()
                .map_err(|_| ArchiveChatError::Busy)?
                .insert(id)
            {
                return Err(ArchiveChatError::Busy);
            }
            Some(ActiveTurnGuard {
                id,
                active: self.active.clone(),
            })
        } else {
            None
        };
        match self.chat_writes.set_archived(id, archived).await? {
            ArchiveChatOutcome::Updated(chat) => Ok(chat),
            ArchiveChatOutcome::Busy => Err(ArchiveChatError::Busy),
            ArchiveChatOutcome::NotFound => Err(ArchiveChatError::NotFound),
        }
    }

    pub async fn compact_conversation(
        &self,
        chat_id: Uuid,
    ) -> Result<CompactionOutcome, CompactConversationError> {
        {
            let mut active = self.active.lock().map_err(|_| {
                CompactConversationError::Unavailable("Conversation state is unavailable.".into())
            })?;
            if !active.insert(chat_id) {
                return Err(CompactConversationError::Unavailable(
                    "Wait for this chat to become idle before compacting.".into(),
                ));
            }
        }
        let _guard = ActiveTurnGuard {
            id: chat_id,
            active: self.active.clone(),
        };
        let chat = self.chats.find_chat(chat_id).await?.ok_or_else(|| {
            CompactConversationError::Unavailable(
                "Send a message first to create a conversation.".into(),
            )
        })?;
        if chat.archived_at.is_some() {
            return Err(CompactConversationError::Unavailable(
                "Restore this chat before compacting.".into(),
            ));
        }
        let (queries, _) = self.workspace_data.as_ref().ok_or_else(|| {
            CompactConversationError::Unavailable("Conversation settings are unavailable.".into())
        })?;
        let preferences = queries.preferences(chat_id).await?;
        let projects = queries.list_projects().await?;
        let workspace = effective_workspace(&self.workspace, &preferences, &projects)?;
        let instructions = if let Some((service, _)) = &self.instructions {
            let project = preferences
                .project_id
                .as_deref()
                .and_then(|s| Uuid::parse_str(s).ok());
            service
                .resolve(project)
                .await
                .map_err(|e| CompactConversationError::Unavailable(e.to_string()))?
        } else {
            InstructionSnapshot::default()
        };
        if let Some(store) = &self.settings_store
            && !store
                .app_settings()
                .await?
                .provider(preferences.execution.provider)
                .enabled
        {
            return Err(CompactConversationError::Unavailable(
                "Enable this provider in Settings before compacting.".into(),
            ));
        }
        let binding = chat
            .binding
            .filter(|binding| {
                !preferences.new_session
                    && binding.adapter_key == preferences.execution.provider.key()
                    && binding.instructions_fingerprint == instructions.fingerprint
                    && preferences
                        .binding_workspace
                        .as_ref()
                        .is_none_or(|old| *old == workspace)
            })
            .ok_or_else(|| {
                CompactConversationError::Unavailable(
                    "Send a normal message first to establish the current provider session.".into(),
                )
            })?;
        Ok(self
            .executor
            .compact(CompactionRequest {
                chat_id,
                binding,
                workspace,
                settings: preferences.execution,
                instructions,
            })
            .await?)
    }
}
