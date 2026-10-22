use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(super) enum InstructionScope {
    Global,
    Project(Uuid),
}

pub(super) enum LoadedInstructions {
    Project(CustomInstructions),
    Applied(Option<InstructionSnapshot>),
}

pub(super) struct InstructionUi {
    pub editor: Entity<Composer>,
    preview_view: Entity<RichText>,
    scope: InstructionScope,
    pub draft: CustomInstructions,
    drafts: HashMap<InstructionScope, CustomInstructions>,
    saved: HashMap<InstructionScope, CustomInstructions>,
    pub loading: bool,
    load_failed: bool,
    pub read_only: bool,
    generation: u64,
    pending: Option<(InstructionScope, CustomInstructions)>,
    preview: bool,
    applied: Option<InstructionSnapshot>,
    pub notice: Option<String>,
    error: Option<String>,
}

impl InstructionUi {
    pub fn new(cx: &mut Context<ChatsView>) -> Self {
        let editor = cx.new(Composer::new_editor);
        editor.update(cx, |e, _| {
            e.set_placeholder(
                "Use plain language and concise explanations.\nInclude examples when they help.",
            )
        });
        Self {
            editor,
            preview_view: cx.new(RichText::new),
            scope: InstructionScope::Global,
            draft: CustomInstructions::default(),
            drafts: HashMap::new(),
            saved: HashMap::new(),
            loading: false,
            load_failed: false,
            read_only: false,
            generation: 0,
            pending: None,
            preview: false,
            applied: None,
            notice: None,
            error: None,
        }
    }

    pub fn remember(&mut self) {
        self.drafts.insert(self.scope, self.draft.clone());
    }

    fn dirty(&self) -> bool {
        self.saved.get(&self.scope) != Some(&self.draft)
    }
}

fn description(text: impl Into<String>) -> gpui::Div {
    div()
        .text_size(px(13.))
        .line_height(px(20.))
        .text_color(rgb(MUTED))
        .child(text.into())
}

fn heading(text: impl Into<String>) -> gpui::Div {
    div()
        .text_size(px(15.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child(text.into())
}

impl ChatsView {
    fn load_instruction_editor(&mut self, cx: &mut Context<Self>) {
        let text = self.instructions.draft.text.clone();
        self.instructions.editor.update(cx, |e, cx| {
            e.load_draft(
                &Draft {
                    text,
                    ..Default::default()
                },
                cx,
            )
        });
    }

    pub(super) fn open_instruction_scope(
        &mut self,
        scope: InstructionScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.settings_open {
            self.open_settings(window, cx);
        }
        self.popup = None;
        self.settings_section = settings::SettingsSection::Instructions;
        self.instructions.scope = scope;
        self.instructions.read_only = false;
        self.instructions.preview = false;
        self.instructions.applied = None;
        self.instructions.error = None;
        self.instructions.load_failed = false;
        self.instructions.notice = None;
        self.instructions.generation += 1;
        match scope {
            InstructionScope::Global => {
                self.instructions.loading = false;
                let saved = self.app_settings.instructions.clone();
                self.instructions.saved.insert(scope, saved.clone());
                self.instructions.draft = self
                    .instructions
                    .drafts
                    .get(&scope)
                    .cloned()
                    .unwrap_or(saved);
                self.load_instruction_editor(cx);
            }
            InstructionScope::Project(id) => {
                self.instructions.loading = true;
                let generation = self.instructions.generation;
                let service = self.services.instructions.clone();
                let sender = self.sender.clone();
                self.services.runtime.spawn(async move {
                    let result = service
                        .project(id)
                        .await
                        .map(LoadedInstructions::Project)
                        .map_err(|e| e.to_string());
                    let _ = sender.send(UiEvent::InstructionsLoaded(generation, result));
                });
            }
        }
        self.focus_composer = false;
        cx.notify();
    }

    pub(crate) fn inspect_message_instructions(
        &mut self,
        chat: Uuid,
        message: Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_instruction_scope(InstructionScope::Global, window, cx);
        self.instructions.read_only = true;
        self.instructions.loading = true;
        let generation = self.instructions.generation;
        let service = self.services.instructions.clone();
        let sender = self.sender.clone();
        self.services.runtime.spawn(async move {
            let result = service
                .for_message(chat, message)
                .await
                .map(LoadedInstructions::Applied)
                .map_err(|e| e.to_string());
            let _ = sender.send(UiEvent::InstructionsLoaded(generation, result));
        });
    }

    pub(super) fn instructions_loaded(
        &mut self,
        generation: u64,
        result: Result<LoadedInstructions, String>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.instructions.generation {
            return;
        }
        self.instructions.loading = false;
        match result {
            Ok(LoadedInstructions::Project(saved)) => {
                self.instructions
                    .saved
                    .insert(self.instructions.scope, saved.clone());
                self.instructions.draft = self
                    .instructions
                    .drafts
                    .get(&self.instructions.scope)
                    .cloned()
                    .unwrap_or(saved);
                self.load_instruction_editor(cx);
            }
            Ok(LoadedInstructions::Applied(snapshot)) => self.instructions.applied = snapshot,
            Err(error) => {
                self.instructions.load_failed = true;
                self.instructions.error = Some(error);
            }
        }
        cx.notify();
    }

    pub(super) fn save_instructions(&mut self, cx: &mut Context<Self>) {
        if self.instructions.loading
            || self.instructions.load_failed
            || self.instructions.read_only
            || self.instructions.pending.is_some()
            || !self.instructions.dirty()
        {
            return;
        }
        let value = self.instructions.draft.clone();
        if !value.is_valid() {
            self.instructions.error =
                Some("Keep instructions within 16 KiB and remove any null characters.".into());
            cx.notify();
            return;
        }
        let scope = self.instructions.scope;
        self.instructions.pending = Some((scope, value.clone()));
        self.instructions.error = None;
        self.instructions.notice = None;
        match scope {
            InstructionScope::Global => {
                let mut next = self.settings_for_edit();
                next.instructions = value.clone();
                self.save_global(next, cx);
                if !self.global_settings_save.is_saving() {
                    self.instruction_save_result(
                        scope,
                        value,
                        Err("Instructions could not be saved. Your draft is retained.".into()),
                        cx,
                    );
                }
            }
            InstructionScope::Project(id) => {
                if self
                    .persist
                    .send(Persist::ProjectInstructions(id, value.clone()))
                    .is_err()
                {
                    self.instruction_save_result(
                        scope,
                        value,
                        Err("Instructions could not be saved. Your draft is retained.".into()),
                        cx,
                    );
                }
            }
        }
        cx.notify();
    }

    pub(super) fn instruction_save_result(
        &mut self,
        scope: InstructionScope,
        value: CustomInstructions,
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        if self.instructions.pending.as_ref() != Some(&(scope, value.clone())) {
            return;
        }
        self.instructions.pending = None;
        match result {
            Ok(()) => {
                self.instructions.saved.insert(scope, value);
                if self.instructions.scope == scope {
                    self.instructions.notice = Some(
                        if self.instructions.dirty() {
                            "Earlier changes saved. Your newer edits are still unsaved."
                        } else {
                            "Saved. Applies to new turns."
                        }
                        .into(),
                    );
                }
            }
            Err(error) => {
                self.instructions.error = Some(format!("{error}. Your draft is retained."));
            }
        }
        cx.notify();
    }

    fn reset_instruction_draft(&mut self, cx: &mut Context<Self>) {
        if self.instructions.pending.is_some() {
            return;
        }
        self.instructions.draft = self
            .instructions
            .saved
            .get(&self.instructions.scope)
            .cloned()
            .unwrap_or_default();
        self.instructions.remember();
        self.instructions.error = None;
        self.instructions.notice = None;
        self.load_instruction_editor(cx);
        cx.notify();
    }

    pub(super) fn render_instructions(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let mut page = div().w_full().flex().flex_col().gap_5();
        if self.instructions.loading {
            return page.child(description("Loading instructions…"));
        }
        if self.instructions.load_failed {
            let scope = self.instructions.scope;
            return page
                .child(heading("Instructions could not be loaded"))
                .child(
                    description(self.instructions.error.clone().unwrap_or_default())
                        .text_color(rgb(DANGER)),
                )
                .child(
                    button("retry-instructions", "Back to instructions").on_click(cx.listener(
                        move |this, _, window, cx| this.open_instruction_scope(scope, window, cx),
                    )),
                );
        }
        if self.instructions.read_only
            && let Some(error) = &self.instructions.error
        {
            return page
                .child(heading("Applied instructions"))
                .child(description(error.clone()).text_color(rgb(DANGER)));
        }
        if self.instructions.read_only {
            let snapshot = self.instructions.applied.clone();
            let text = snapshot
                .as_ref()
                .map(|s| s.text.clone())
                .unwrap_or_default();
            let summary = match &snapshot {
                None => {
                    "This message has no recorded execution snapshot. It may have been sent before custom instructions were available."
                }
                Some(s) if s.version == 0 => {
                    "This turn was recorded before custom instructions were available."
                }
                Some(s) if s.text.is_empty() => {
                    "No Smaller Talk instructions were enabled for this turn."
                }
                Some(_) => {
                    "The saved Smaller Talk instructions used for this turn. Provider and repository instructions may also apply."
                }
            };
            page = page
                .child(heading("Applied instructions"))
                .child(description(summary));
            if !text.is_empty() {
                page = page.child(self.instruction_preview(text, cx));
            }
            return page.child(
                button("edit-global-instructions", "Global instructions").on_click(cx.listener(
                    |this, _, window, cx| {
                        this.open_instruction_scope(InstructionScope::Global, window, cx)
                    },
                )),
            );
        }
        let (title, context) = match self.instructions.scope {
            InstructionScope::Global => (
                "Global instructions".to_owned(),
                "Set your preferences once for conversations across all projects and providers."
                    .to_owned(),
            ),
            InstructionScope::Project(id) => (
                "Project instructions".to_owned(),
                format!(
                    "Applies to conversations in {}. Global instructions are included, with project preferences taking precedence where they conflict.",
                    self.projects
                        .iter()
                        .find(|p| p.id == id)
                        .map(|p| p.name.as_str())
                        .unwrap_or("this project")
                ),
            ),
        };
        page = page.child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_size(px(20.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(description(context)),
        );
        let enabled = self.instructions.draft.enabled;
        page = page.child(
            row()
                .id("instruction-enabled")
                .tab_index(0)
                .border_1()
                .border_color(gpui::rgba(0))
                .rounded_md()
                .focus(|s| s.border_color(rgb(FOCUS)))
                .gap_3()
                .min_h(px(32.))
                .cursor_pointer()
                .child(
                    div()
                        .size(px(18.))
                        .rounded_sm()
                        .border_1()
                        .border_color(rgb(if enabled { ACCENT } else { MUTED }))
                        .bg(rgb(if enabled { ACCENT } else { BACKGROUND }))
                        .text_color(rgb(SELECTED_INK))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(if enabled { "✓" } else { "" }),
                )
                .child(heading("Use these instructions"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.instructions.draft.enabled = !this.instructions.draft.enabled;
                    this.instructions.remember();
                    this.instructions.notice = None;
                    cx.notify();
                })),
        );
        page = page.child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(heading("What should the assistant keep in mind?"))
                .child(description(
                    "Describe your preferred response style, terminology, or working conventions.",
                ))
                .child(
                    div()
                        .w_full()
                        .min_h(px(200.))
                        .p_3()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(BORDER))
                        .bg(rgb(PANEL))
                        .child(self.instructions.editor.clone()),
                )
                .child(description(format!(
                    "{} / {} bytes",
                    self.instructions.draft.text.len(),
                    MAX_INSTRUCTION_BYTES
                ))),
        );
        let saving = self.instructions.pending.is_some();
        let dirty = self.instructions.dirty();
        page = page.child(
            row()
                .gap_2()
                .child(
                    button(
                        "save-instructions",
                        if saving {
                            "Saving…"
                        } else {
                            "Save instructions"
                        },
                    )
                    .bg(rgb(ACCENT))
                    .text_color(rgb(SELECTED_INK))
                    .when(!dirty || saving, |b| b.opacity(0.5))
                    .on_click(cx.listener(|this, _, _, cx| this.save_instructions(cx))),
                )
                .child(
                    button("cancel-instructions", "Cancel changes")
                        .when(!dirty || saving, |b| b.opacity(0.5))
                        .on_click(cx.listener(|this, _, _, cx| this.reset_instruction_draft(cx))),
                ),
        );
        if let Some(error) = &self.instructions.error {
            page = page.child(description(error.clone()).text_color(rgb(DANGER)));
        }
        if let Some(notice) = &self.instructions.notice {
            page = page.child(description(notice.clone()).text_color(rgb(ACCENT)));
        }
        page = page.child(div().h(px(1.)).bg(rgb(BORDER))).child(div().flex().flex_col().gap_2().child(heading("When instructions apply"))
            .child(description("Saved changes apply to the next turn. An active response and any steering keep the instructions they started with. Retries use the original instructions."))
            .child(description("Changing instructions starts a new agent session with your saved conversation. Agent-only reasoning and tool state are not transferred.")));
        page = page.child(
            button(
                "preview-instructions",
                if self.instructions.preview {
                    "Hide combined preview"
                } else {
                    "Preview combined instructions"
                },
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.instructions.preview = !this.instructions.preview;
                cx.notify();
            })),
        );
        if self.instructions.preview {
            let (global, local, project_id) = match self.instructions.scope {
                InstructionScope::Global => (
                    self.instructions.draft.clone(),
                    CustomInstructions::default(),
                    None,
                ),
                InstructionScope::Project(id) => (
                    self.app_settings.instructions.clone(),
                    self.instructions.draft.clone(),
                    Some(id),
                ),
            };
            let snapshot = compose_instructions(&global, &local, project_id);
            page = page.child(description(
                "Preview includes your unsaved changes. Only saved instructions are sent.",
            ));
            page = page.child(self.instruction_preview(
                if snapshot.text.is_empty() {
                    "No instructions are enabled.".into()
                } else {
                    snapshot.text
                },
                cx,
            ));
        }
        page
    }

    fn instruction_preview(&mut self, text: String, cx: &mut Context<Self>) -> gpui::Div {
        self.instructions.preview_view.update(cx, |view, cx| {
            if view.text != text {
                view.text = text.clone();
                view.clear_selection();
                cx.notify();
            }
        });
        div()
            .w_full()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(PANEL))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                row()
                    .justify_end()
                    .child(button("copy-instructions", "Copy").on_click(cx.listener(
                        move |_, _, _, cx| {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.clone()))
                        },
                    ))),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .line_height(px(20.))
                    .child(self.instructions.preview_view.clone()),
            )
    }
}
