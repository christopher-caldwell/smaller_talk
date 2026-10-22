use super::*;
use std::ops::Range;

#[derive(Default)]
pub(super) struct SkillUi {
    context: Option<(Uuid, Provider, PathBuf, u64)>,
    pub generation: u64,
    pending_send: Option<(Uuid, Draft)>,
    loading: bool,
    result: Option<Result<Vec<SkillDescriptor>, String>>,
    pub query: String,
    pub range: Option<Range<usize>>,
    pub automatic: bool,
    pub preview: Option<String>,
}

impl ChatsView {
    pub(super) fn edit_queued_message(&mut self, chat: Uuid, entry: Uuid, cx: &mut Context<Self>) {
        let Some(state) = self.states.get_mut(&chat) else {
            return;
        };
        if state.queue_edit_pending
            || state.enqueue_pending
            || state.importing
            || !state.attachments.is_empty()
            || !state.composer.read(cx).content().is_empty()
        {
            state.error =
                Some("Save or clear the current draft before editing a queued message.".into());
            cx.notify();
            return;
        }
        self.draft_changed(chat, cx);
        let state = self.states.get_mut(&chat).unwrap();
        state.queue_edit_pending = true;
        state.composer.update(cx, |e, cx| e.set_editable(false, cx));
        if self.persist.send(Persist::EditQueue(chat, entry)).is_err() {
            let state = self.states.get_mut(&chat).unwrap();
            state.queue_edit_pending = false;
            state.composer.update(cx, |e, cx| e.set_editable(true, cx));
            state.error = Some("The queue is unavailable. The message remains queued.".into());
        }
        cx.notify();
    }
    pub(super) fn skill_catalog_changed(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.skill_ui.context.as_ref().is_some_and(|c| c.0 == id) {
            self.skill_ui.generation += 1;
            self.skill_ui.loading = false;
            self.skill_ui.result = None;
            if self.active == Some(id) && matches!(self.popup, Some(Popup::Skills)) {
                self.refresh_skills(id, true, cx);
            }
        }
    }
    fn skill_context(&self, id: Uuid) -> Result<(Uuid, Provider, PathBuf, u64), String> {
        let state = self.states.get(&id).ok_or("Conversation is unavailable")?;
        let workspace = effective_workspace(
            &self.services.launch_folder,
            &state.preferences,
            &self.projects,
        )
        .map_err(|e| e.to_string())?;
        let provider = state.preferences.execution.provider;
        Ok((
            id,
            provider,
            workspace,
            self.services.skills.revision(provider),
        ))
    }
    pub(super) fn skill_catalog_checked(&self) -> bool {
        self.active
            .and_then(|id| self.skill_context(id).ok())
            .as_ref()
            == self.skill_ui.context.as_ref()
            && matches!(self.skill_ui.result, Some(Ok(_)))
    }
    pub(super) fn skill_entries(&self) -> &[SkillDescriptor] {
        if self
            .active
            .and_then(|id| self.skill_context(id).ok())
            .as_ref()
            != self.skill_ui.context.as_ref()
        {
            return &[];
        }
        self.skill_ui
            .result
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
    pub(super) fn refresh_skills(&mut self, id: Uuid, refresh: bool, cx: &mut Context<Self>) {
        let context = match self.skill_context(id) {
            Ok(c) => c,
            Err(e) => {
                if let Some(state) = self.states.get_mut(&id) {
                    state.error = Some(e.clone());
                }
                self.skill_ui.pending_send = None;
                self.skill_ui.result = Some(Err(e));
                cx.notify();
                return;
            }
        };
        if self.skill_ui.context.as_ref() == Some(&context) && self.skill_ui.loading {
            return;
        }
        self.skill_ui.generation += 1;
        let generation = self.skill_ui.generation;
        self.skill_ui.context = Some(context.clone());
        self.skill_ui.loading = true;
        self.skill_ui.result = None;
        let service = self.services.skills.clone();
        let sender = self.sender.clone();
        self.services.runtime.spawn(async move {
            let result = service
                .list(context.1, &context.2, refresh)
                .await
                .map_err(|e| e.to_string());
            let _ = sender.send(UiEvent::Skills(generation, context, result));
        });
        cx.notify();
    }
    pub(super) fn skills_loaded(
        &mut self,
        generation: u64,
        context: (Uuid, Provider, PathBuf, u64),
        result: Result<Vec<SkillDescriptor>, String>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.skill_ui.generation
            || self.skill_context(context.0).ok().as_ref() != Some(&context)
        {
            return;
        }
        let pending_send = self.skill_ui.pending_send.take().filter(|(id, draft)| {
            self.states
                .get(id)
                .is_some_and(|s| s.composer.read(cx).draft(s.attachments.clone()) == *draft)
        });
        self.skill_ui.loading = false;
        self.skill_ui.result = Some(result);
        let entries = self.skill_entries().to_vec();
        if let Some(state) = self.states.get(&context.0) {
            let changed = state
                .composer
                .update(cx, |editor, cx| editor.resolve_known_skills(&entries, cx));
            if changed {
                self.draft_changed(context.0, cx);
            }
        }
        if let Some((id, _)) = pending_send {
            if let Some(Err(error)) = &self.skill_ui.result {
                self.states.get_mut(&id).unwrap().error = Some(error.clone());
            } else {
                self.send(id, cx);
            }
        }
        cx.notify();
    }
    pub(super) fn prepare_skill_send(&mut self, id: Uuid, cx: &mut Context<Self>) -> bool {
        let draft = self.states[&id]
            .composer
            .read(cx)
            .draft(self.states[&id].attachments.clone());
        if !draft.skills.is_empty() {
            let invalid = match self.skill_context(id) {
                Ok((_, provider, workspace, _)) => draft.skills.iter().any(|m| {
                    m.reference.provider != provider
                        || m.reference.workspace != workspace
                        || !m.matches(&draft.text)
                        || !m.is_prose(&draft.text)
                }),
                Err(_) => true,
            };
            if invalid {
                self.states.get_mut(&id).unwrap().error =
                    Some("Replace or remove the unresolved skill before sending.".into());
                cx.notify();
                return false;
            }
        }
        if skill_tokens(&draft.text).is_empty()
            && !draft.text.contains("[$")
            && draft.skills.is_empty()
        {
            return true;
        }
        if self.skill_ui.context.as_ref() != self.skill_context(id).ok().as_ref()
            || self.skill_ui.result.is_none()
        {
            self.skill_ui.pending_send = Some((id, draft));
            self.refresh_skills(id, false, cx);
            return false;
        }
        let entries = self.skill_entries().to_vec();
        if self.skill_catalog_checked()
            && draft.skills.iter().any(|m| {
                !entries
                    .iter()
                    .any(|e| e.reference == m.reference && e.unavailable.is_none())
            })
        {
            self.states.get_mut(&id).unwrap().error=Some("A selected skill is unavailable. Refresh Skills, or replace or remove its mention.".into());
            cx.notify();
            return false;
        }
        self.states[&id].composer.update(cx, |e, cx| {
            e.resolve_known_skills(&entries, cx);
        });
        let draft = self.states[&id]
            .composer
            .read(cx)
            .draft(self.states[&id].attachments.clone());
        for (range, name) in skill_tokens(&draft.text) {
            if draft.skills.iter().any(|m| m.range == range) {
                continue;
            }
            match resolve_skill_name(&entries, &name) {
                SkillNameResolution::Ambiguous => {
                    self.states.get_mut(&id).unwrap().error = Some(format!(
                        "${name} has multiple sources. Choose its source in Skills."
                    ));
                    self.skill_ui.range = Some(range);
                    self.skill_ui.query = name;
                    self.skill_ui.automatic = true;
                    self.popup = Some(Popup::Skills);
                    cx.notify();
                    return false;
                }
                SkillNameResolution::Unavailable(reason) => {
                    self.states.get_mut(&id).unwrap().error = Some(format!(
                        "${name}: {reason}. Remove or replace this mention."
                    ));
                    cx.notify();
                    return false;
                }
                _ => {}
            }
        }
        true
    }
    pub(super) fn composer_skills_changed(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(state) = self.states.get(&id) else {
            return;
        };
        let query = state.composer.read(cx).skill_query();
        let entries = self.skill_entries().to_vec();
        state.composer.update(cx, |editor, cx| {
            editor.resolve_known_skills(&entries, cx);
        });
        if let Some((range, query)) = query {
            if self.active == Some(id) && matches!(self.popup, None | Some(Popup::Skills)) {
                self.skill_ui.range = Some(range);
                self.skill_ui.query = query;
                self.skill_ui.automatic = true;
                let opening = !matches!(self.popup, Some(Popup::Skills));
                self.popup = Some(Popup::Skills);
                self.popup_index = 0;
                if opening
                    || self.skill_ui.context.as_ref() != self.skill_context(id).ok().as_ref()
                    || self.skill_ui.result.is_none()
                {
                    self.refresh_skills(id, false, cx);
                }
                self.states[&id]
                    .composer
                    .update(cx, |editor, _| editor.set_picker_navigation(true));
            }
        } else if self.skill_ui.automatic && matches!(self.popup, Some(Popup::Skills)) {
            self.popup = None;
            self.states[&id]
                .composer
                .update(cx, |editor, _| editor.set_picker_navigation(false));
        }
        let text = self.states[&id].composer.read(cx).content();
        if (!skill_tokens(&text).is_empty() || text.contains("[$"))
            && self.skill_ui.context.as_ref() != self.skill_context(id).ok().as_ref()
        {
            self.refresh_skills(id, false, cx);
        }
        self.draft_changed(id, cx);
    }
    pub(super) fn open_skills(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.active else {
            return;
        };
        self.skill_ui.automatic = false;
        self.skill_ui.query.clear();
        self.skill_ui.range = self.states[&id]
            .composer
            .read(cx)
            .skill_query()
            .map(|(r, _)| r);
        self.refresh_skills(id, false, cx);
        self.open_picker(Popup::Skills, window, cx);
    }
    pub(super) fn skill_choices(&self) -> Vec<PickerChoice> {
        let query = self.skill_ui.query.to_lowercase();
        self.skill_entries()
            .iter()
            .filter(|d| {
                d.unavailable.is_none()
                    && (d.reference.name.to_lowercase().contains(&query)
                        || d.description.to_lowercase().contains(&query))
            })
            .take(10)
            .map(|d| PickerChoice {
                label: format!("{} · {}\n{}", d.reference.name, d.source, d.description),
                selected: false,
                action: PickerAction::Skill(d.reference.clone()),
            })
            .collect()
    }
    pub(super) fn select_skill(
        &mut self,
        reference: SkillReference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.active else {
            return;
        };
        if self.skill_context(id).ok().as_ref().map(|c| (c.1, &c.2))
            != Some((reference.provider, &reference.workspace))
        {
            return;
        }
        let composer = self.states[&id].composer.clone();
        let range = self
            .skill_ui
            .range
            .take()
            .unwrap_or_else(|| composer.read(cx).selection());
        if reference.provider == Provider::Claude
            && composer
                .read(cx)
                .skills()
                .iter()
                .any(|m| m.reference != reference && m.range != range)
        {
            self.states.get_mut(&id).unwrap().error = Some(
                "Claude supports one explicit skill per message. Remove the current mention first."
                    .into(),
            );
            return;
        }
        self.dismiss_popup(window, cx);
        composer.update(cx, |editor, cx| editor.select_skill(range, reference, cx));
    }
    pub(super) fn inspect_skill(&mut self, reference: SkillReference, cx: &mut Context<Self>) {
        self.skill_ui.preview = Some(format!(
            "{}\n\nLoading instructions…",
            reference.path.display()
        ));
        self.popup = Some(Popup::SkillDetails);
        self.skill_ui.generation += 1;
        let generation = self.skill_ui.generation;
        let service = self.services.skills.clone();
        let sender = self.sender.clone();
        self.services.runtime.spawn(async move {
            let result = service
                .inspect(&reference)
                .await
                .map(|text| format!("{}\n\n{text}", reference.path.display()))
                .unwrap_or_else(|e| e.to_string());
            let _ = sender.send(UiEvent::SkillPreview(generation, result));
        });
        cx.notify();
    }
    pub(super) fn render_skills(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let mut panel = div().flex().flex_col().gap_1();
        if !self.skill_ui.automatic {
            panel = panel.child(self.model_search.clone());
        }
        panel = panel.child(
            row()
                .child(muted(
                    if self
                        .skill_ui
                        .context
                        .as_ref()
                        .is_some_and(|c| c.1 == Provider::Claude)
                    {
                        "Skills · one per message"
                    } else {
                        "Skills"
                    },
                ))
                .child(div().flex_1())
                .child(
                    compact_button("refresh-skills", "Refresh").on_click(cx.listener(
                        |this, _, _, cx| {
                            if let Some(id) = this.active {
                                this.refresh_skills(id, true, cx);
                            }
                        },
                    )),
                ),
        );
        if self.skill_ui.loading {
            panel = panel.child(muted("Loading skills…"));
        }
        if let Some(Err(error)) = &self.skill_ui.result {
            panel = panel.child(div().text_xs().text_color(rgb(DANGER)).child(error.clone()));
        }
        let choices = self.skill_choices();
        if choices.is_empty()
            && !self.skill_ui.loading
            && matches!(self.skill_ui.result, Some(Ok(_)))
        {
            panel = panel.child(muted("No matching skills"));
        }
        for (index, choice) in choices.into_iter().enumerate() {
            let PickerAction::Skill(reference) = choice.action else {
                continue;
            };
            let inspect = reference.clone();
            let path = reference.path.display().to_string();
            panel = panel.child(
                row()
                    .gap_1()
                    .child(
                        compact_button(element_id("skill-choice", index), choice.label)
                            .flex_1()
                            .min_w(px(0.))
                            .text_size(px(12.))
                            .bg(rgb(if index == self.popup_index {
                                PANEL_HOVER
                            } else {
                                MENU
                            }))
                            .tooltip(hint(path))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_skill(reference.clone(), window, cx)
                            })),
                    )
                    .child(
                        compact_button(element_id("inspect-skill", index), "Inspect")
                            .text_xs()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.inspect_skill(inspect.clone(), cx)
                            })),
                    ),
            );
        }
        panel.into_any_element()
    }
}
