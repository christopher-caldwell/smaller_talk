use super::*;

/// Reconcile a first send that committed before the UI could acknowledge it.
/// A queued message owns its submitted content; preserve only subsequent edits.
pub(super) fn recover_started_chat(
    session: &mut SessionState,
    chats: &[Chat],
) -> Option<(Uuid, Draft)> {
    let draft = session.new_chat.as_ref()?;
    let id = match Uuid::parse_str(&draft.id) {
        Ok(id) => id,
        Err(_) => {
            session.new_chat = None;
            return None;
        }
    };
    if !chats.iter().any(|chat| chat.id == id) {
        return None;
    }
    let remaining = draft.remaining_draft();
    session.new_chat = None;
    (!remaining.text.is_empty() || !remaining.attachments.is_empty()).then_some((id, remaining))
}

impl ChatsView {
    pub(super) fn is_new_chat(&self, id: Uuid) -> bool {
        self.session
            .new_chat
            .as_ref()
            .is_some_and(|draft| draft.id == id.to_string())
    }

    pub(super) fn new_chat(&mut self, cx: &mut Context<Self>) {
        if self.session.new_chat.is_none() {
            self.session.new_chat = Some(NewChatDraft {
                id: Uuid::new_v4().to_string(),
                preferences: ChatPreferences {
                    execution: self.new_chat_settings().unwrap_or_default(),
                    ..Default::default()
                },
                draft: Draft::default(),
                submitted: None,
            });
        }
        if let Some(draft) = &self.session.new_chat
            && let Ok(id) = Uuid::parse_str(&draft.id)
        {
            self.metadata.insert(id, draft.preferences.clone());
            self.open_chat(id, None, cx);
        }
    }

    pub(super) fn new_chat_in_project(&mut self, project_id: Uuid, cx: &mut Context<Self>) {
        if !self.projects.iter().any(|project| project.id == project_id) {
            return;
        }
        if self
            .session
            .new_chat
            .as_ref()
            .is_some_and(|draft| draft.submitted.is_some())
        {
            self.banner =
                Some("Wait for the current chat to start before changing projects.".into());
            cx.notify();
            return;
        }
        self.new_chat(cx);
        if let Some(id) = self.active {
            let mut preferences = self.metadata.get(&id).cloned().unwrap_or_default();
            preferences.project_id = Some(project_id.to_string());
            self.save_preferences(id, preferences, cx);
        }
    }

    pub(super) fn save_new_chat_draft(&mut self, cx: &Context<Self>) {
        if let Some(draft) = &mut self.session.new_chat
            && let Ok(id) = Uuid::parse_str(&draft.id)
            && let Some(state) = self.states.get(&id).filter(|state| state.loaded)
        {
            draft.draft = state.composer.read(cx).draft(state.attachments.clone());
            draft.preferences = state.preferences.clone();
        }
    }

    pub(super) fn started_chat(
        &mut self,
        id: Uuid,
        draft: Draft,
        result: Result<Chat, String>,
        cx: &mut Context<Self>,
    ) {
        let result = match result {
            Ok(chat) => {
                if self.is_new_chat(id) {
                    self.session.new_chat = None;
                }
                self.chats.insert(0, chat);
                Ok(())
            }
            Err(error) => {
                if let Some(new_chat) = self
                    .session
                    .new_chat
                    .as_mut()
                    .filter(|chat| chat.id == id.to_string())
                {
                    new_chat.submitted = None;
                }
                Err(error)
            }
        };
        self.apply_event(UiEvent::Enqueued(id, draft, result), cx);
        self.save_session();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(id: Uuid) -> NewChatDraft {
        NewChatDraft {
            id: id.to_string(),
            preferences: ChatPreferences::default(),
            draft: Draft {
                skills: Vec::new(),
                text: "first message".into(),
                ..Default::default()
            },
            submitted: None,
        }
    }

    #[test]
    fn unsent_draft_stays_outside_history_and_survives_reconciliation() {
        let draft = pending(Uuid::new_v4());
        let mut session = SessionState {
            new_chat: Some(draft.clone()),
            ..Default::default()
        };
        assert!(recover_started_chat(&mut session, &[]).is_none());
        assert_eq!(session.new_chat, Some(draft));
    }

    #[test]
    fn acknowledged_first_send_is_not_restored_as_another_draft() {
        let id = Uuid::new_v4();
        let mut draft = pending(id);
        draft.submitted = Some(draft.draft.clone());
        let mut session = SessionState {
            new_chat: Some(draft),
            ..Default::default()
        };
        let chats = [Chat {
            id,
            title: "first message".into(),
            created_at: chrono::Utc::now(),
            binding: None,
        }];
        assert!(recover_started_chat(&mut session, &chats).is_none());
        assert!(session.new_chat.is_none());
    }

    #[test]
    fn edits_after_first_send_survive_restart_without_resending_original() {
        let id = Uuid::new_v4();
        let mut draft = pending(id);
        draft.submitted = Some(draft.draft.clone());
        draft.draft.text = "next message 🦀".into();
        let expected = draft.draft.clone();
        let mut session = SessionState {
            new_chat: Some(draft),
            ..Default::default()
        };
        let chats = [Chat {
            id,
            title: "first message".into(),
            created_at: chrono::Utc::now(),
            binding: None,
        }];
        assert_eq!(
            recover_started_chat(&mut session, &chats),
            Some((id, expected))
        );
        assert!(session.new_chat.is_none());
    }
}
