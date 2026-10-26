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
        self.management = None;
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
        if let Some(id) = self.active() {
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
            archived_at: None,
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
            archived_at: None,
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

/// Archived chats cannot reappear through a stale session snapshot after a crash.
pub(super) fn restore_tabs(session: &mut SessionState, chats: &[Chat]) {
    session.open_tabs.retain(|id| {
        chats
            .iter()
            .any(|c| c.id.to_string() == *id && c.archived_at.is_none())
            || session
                .new_chat
                .as_ref()
                .is_some_and(|draft| draft.id == *id)
    });
    session.active_tab = session
        .active_tab
        .take()
        .filter(|id| session.open_tabs.contains(id))
        .or_else(|| session.open_tabs.last().cloned());
}

#[cfg(test)]
mod archive_restart_tests {
    use super::*;
    #[test]
    fn stale_archived_tabs_and_panes_are_cleared_while_other_tabs_survive() {
        let archived = Uuid::from_u128(1);
        let available = Uuid::from_u128(2);
        let now = chrono::Utc::now();
        let chats = [
            Chat {
                id: archived,
                title: "Archived".into(),
                created_at: now,
                binding: None,
                archived_at: Some(now),
            },
            Chat {
                id: available,
                title: "Active".into(),
                created_at: now,
                binding: None,
                archived_at: None,
            },
        ];
        let mut session = SessionState {
            open_tabs: vec![archived.to_string(), available.to_string()],
            active_tab: Some(archived.to_string()),
            pane_layout: Some(PaneLayout {
                focused: 0,
                root: PaneNode::Pane {
                    id: 0,
                    chat: Some(archived),
                },
            }),
            ..Default::default()
        };
        restore_tabs(&mut session, &chats);
        assert_eq!(session.open_tabs, vec![available.to_string()]);
        assert_eq!(session.active_tab, Some(available.to_string()));
        let panes = super::super::panes::Panes::restore(
            session.pane_layout.as_ref().unwrap(),
            &[available],
        )
        .unwrap();
        assert_eq!(panes.snapshot().root, PaneNode::Pane { id: 0, chat: None });
    }
}
