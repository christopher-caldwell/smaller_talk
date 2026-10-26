use super::*;

impl ChatsView {
    pub(super) fn compaction_unavailable(&self, id: Uuid) -> Option<&'static str> {
        let Some(state) = self.states.get(&id) else {
            return Some("Load a conversation first");
        };
        if self.is_new_chat(id)
            || state.preferences.new_session
            || self.chats.iter().find(|c| c.id == id).is_none_or(|c| {
                c.binding
                    .as_ref()
                    .is_none_or(|b| b.adapter_key != state.preferences.execution.provider.key())
            })
        {
            return Some("Send a normal message first to establish the current provider session");
        }
        if !self
            .app_settings
            .provider(state.preferences.execution.provider)
            .enabled
            || !matches!(
                self.provider_status
                    .get(&state.preferences.execution.provider),
                Some(ProviderReadiness::Ready)
            )
        {
            return Some("Enable this provider and check sign-in in Settings");
        }
        if !self.archive_eligible(id) {
            return Some("Wait until the chat is idle and its queue is empty");
        }
        None
    }

    pub(super) fn update_command_picker(&mut self, id: Uuid, cx: &mut Context<Self>) -> bool {
        let Some(state) = self.states.get(&id) else {
            return false;
        };
        if state.composer.read(cx).composing() {
            return false;
        }
        let text = state.composer.read(cx).content();
        let trimmed = text.trim_start();
        let query = trimmed.strip_prefix('/').filter(|s| {
            s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        });
        if let Some(query) = query {
            if self.command_dismissed.as_ref() != Some(&(id, text.clone())) {
                self.popup = Some(Popup::Commands);
                self.command_query = query.into();
                self.popup_index = 0;
                self.states[&id]
                    .composer
                    .update(cx, |e, _| e.set_picker_navigation(true));
            }
            self.draft_changed(id, cx);
            cx.notify();
            return true;
        }
        if matches!(self.popup, Some(Popup::Commands)) {
            self.popup = None;
            self.states[&id]
                .composer
                .update(cx, |e, _| e.set_picker_navigation(false));
        }
        false
    }
    pub(super) fn accept_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.active() else {
            return;
        };
        self.command_dismissed = Some((id, "/compact".into()));
        self.states[&id].composer.update(cx, |e, cx| {
            e.set_content("/compact".into(), window, cx);
            e.set_picker_navigation(false);
        });
        self.dismiss_popup(window, cx);
        self.draft_changed(id, cx);
    }
    pub(super) fn try_command(&mut self, id: Uuid, cx: &mut Context<Self>) -> bool {
        let Some(state) = self.states.get(&id) else {
            return true;
        };
        let draft = state.composer.read(cx).draft(state.attachments.clone());
        let error = match composer_command(&draft.text) {
            ComposerCommand::Message => return false,
            ComposerCommand::Invalid(reason) => Some(reason),
            ComposerCommand::Compact
                if !draft.attachments.is_empty() || !draft.skills.is_empty() =>
            {
                Some("Use /compact alone, without attachments or skills.".into())
            }
            ComposerCommand::Compact
                if self.is_new_chat(id)
                    || state.preferences.new_session
                    || self.chats.iter().find(|c| c.id == id).is_none_or(|c| {
                        c.binding.as_ref().is_none_or(|b| {
                            b.adapter_key != state.preferences.execution.provider.key()
                        })
                    }) =>
            {
                Some(
                    "Send a normal message first to establish the current provider session.".into(),
                )
            }
            ComposerCommand::Compact if !self.archive_eligible(id) => Some(
                "Wait until this chat is idle and its queue is empty before compacting.".into(),
            ),
            ComposerCommand::Compact => None,
        };
        if let Some(error) = error {
            self.states.get_mut(&id).unwrap().error = Some(error);
            cx.notify();
            return true;
        }
        self.popup = None;
        self.command_pending.insert(id);
        self.states.get_mut(&id).unwrap().error = None;
        let queue = self.services.queue.clone();
        let sender = self.sender.clone();
        self.services.runtime.spawn(async move {
            let result = queue.compact(id).await;
            let _ = sender.send(UiEvent::CompactionAccepted(id, draft, result));
        });
        cx.notify();
        true
    }
}
