use super::*;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum SettingsSection {
    #[default]
    Providers,
    ResponseDisplay,
    Instructions,
}

#[derive(Clone, Copy)]
pub(crate) enum SettingsPicker {
    DefaultModel,
    DefaultEffort,
    RewriteModel,
}

impl SettingsPicker {
    pub(super) fn anchor(self) -> &'static str {
        match self {
            Self::DefaultModel => "settings-default-model",
            Self::DefaultEffort => "settings-default-effort",
            Self::RewriteModel => "settings-rewrite-model",
        }
    }
    pub(super) fn title(self) -> &'static str {
        match self {
            Self::DefaultModel => "Default model",
            Self::DefaultEffort => "Default effort",
            Self::RewriteModel => "Rewrite model",
        }
    }
}

// GPUI's focusable click controls already activate on Enter/Space release.
// Keep Settings density separate from the chat controls.
fn control(
    id: impl Into<gpui::ElementId>,
    cx: &Context<ChatsView>,
    action: impl Fn(&mut ChatsView, &mut Window, &mut Context<ChatsView>) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .tab_index(0)
        .border_1()
        .border_color(gpui::rgba(0))
        .rounded_md()
        .text_size(px(13.))
        .cursor_pointer()
        .focus(|s| s.border_color(rgb(FOCUS)))
        .hover(|s| s.bg(rgb(PANEL_HOVER)))
        .on_click(cx.listener(move |this, _, window, cx| action(this, window, cx)))
}

fn toggle(
    id: impl Into<gpui::ElementId>,
    checked: bool,
    label: String,
    cx: &Context<ChatsView>,
    action: impl Fn(&mut ChatsView, &mut Window, &mut Context<ChatsView>) + 'static,
) -> gpui::Stateful<gpui::Div> {
    control(id, cx, action)
        .w(px(42.))
        .h(px(28.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .tooltip(hint(label))
        .child(
            div()
                .w(px(32.))
                .h(px(18.))
                .px(px(2.))
                .rounded_full()
                .bg(rgb(if checked { ACCENT } else { PANEL_HOVER }))
                .flex()
                .items_center()
                .when(checked, |s| s.justify_end())
                .child(div().size(px(14.)).rounded_full().bg(rgb(TEXT))),
        )
}

fn caption(text: impl Into<String>) -> gpui::Div {
    div()
        .text_size(px(12.))
        .text_color(rgb(MUTED))
        .child(text.into())
}
fn surface() -> gpui::Div {
    div().w_full().rounded_lg().bg(rgb(PANEL)).flex().flex_col()
}
fn setting_row() -> gpui::Div {
    row().min_h(px(44.)).px_3().py_2()
}
fn separator() -> gpui::Div {
    div()
        .mx_3()
        .h(px(1.))
        .flex_shrink_0()
        .bg(gpui::rgba((BORDER << 8) | 0x66))
}

#[derive(Default)]
pub(super) struct GlobalSettingsSave {
    in_flight: Option<AppSettings>,
    pending: Option<AppSettings>,
}

impl GlobalSettingsSave {
    pub(super) fn request(&mut self, settings: AppSettings) -> Option<AppSettings> {
        if self.in_flight.is_some() {
            self.pending = Some(settings);
            None
        } else {
            self.in_flight = Some(settings.clone());
            Some(settings)
        }
    }

    pub(super) fn acknowledge(&mut self) -> (Option<AppSettings>, Option<AppSettings>) {
        let completed = self.in_flight.take();
        let next = self.pending.take();
        if let Some(settings) = &next {
            self.in_flight = Some(settings.clone());
        }
        (completed, next)
    }

    pub(super) fn send_failed(&mut self) {
        self.in_flight = None;
        self.pending = None;
    }

    pub(super) fn is_saving(&self) -> bool {
        self.in_flight.is_some()
    }

    pub(super) fn edit_base(&self) -> Option<&AppSettings> {
        self.pending.as_ref().or(self.in_flight.as_ref())
    }
}

impl ChatsView {
    pub(super) fn handle_settings_key_up(
        &mut self,
        event: &gpui::KeyUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(self.popup, Some(Popup::Settings(_)))
            && matches!(event.keystroke.key.as_str(), "enter" | "space")
        {
            if let Some(choice) = self.picker_choices().get(self.popup_index) {
                self.choose_picker(choice.action.clone(), window, cx);
            }
            window.prevent_default();
            cx.stop_propagation();
        }
    }

    pub(super) fn new_chat_settings(&self) -> Option<ExecutionSettings> {
        self.app_settings
            .default_execution
            .clone()
            .filter(|s| {
                self.app_settings.provider(s.provider).enabled
                    && s.model
                        .as_deref()
                        .is_none_or(|model| self.app_settings.visible(s.provider, model))
            })
            .or_else(|| {
                PROVIDERS.into_iter().find_map(|provider| {
                    if !self.app_settings.provider(provider).enabled {
                        return None;
                    }
                    self.catalogs
                        .get(&provider)
                        .and_then(|r| r.as_ref().ok())
                        .and_then(|c| {
                            c.models
                                .iter()
                                .find(|m| {
                                    m.is_default && self.app_settings.visible(provider, &m.id)
                                })
                                .or_else(|| {
                                    c.models
                                        .iter()
                                        .find(|m| self.app_settings.visible(provider, &m.id))
                                })
                        })
                        .map(|m| ExecutionSettings {
                            provider,
                            model: Some(m.id.clone()),
                            effort: None,
                            plan_mode: false,
                        })
                })
            })
            .or_else(|| {
                PROVIDERS
                    .into_iter()
                    .find(|p| self.app_settings.provider(*p).enabled)
                    .map(|provider| ExecutionSettings {
                        provider,
                        ..Default::default()
                    })
            })
    }
    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let return_focus = self
            .popup_return_focus
            .take()
            .or_else(|| window.focused(cx));
        self.popup = None;
        if !self.settings_open {
            self.settings_focus = return_focus;
        }
        self.settings_open = true;
        self.settings_section = SettingsSection::Providers;
        self.focus_composer = false;
        self.model_query.clear();
        self.model_search.update(cx, |e, cx| {
            e.set_picker_navigation(false);
            e.set_font_size(13.);
            e.set_content(String::new(), window, cx);
        });
        for provider in PROVIDERS {
            let text = self
                .app_settings
                .provider(provider)
                .executable
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            self.path_editors[&provider].update(cx, |e, cx| e.set_content(text, window, cx));
        }
        self.focus.focus(window, cx);
        cx.notify();
    }
    pub(super) fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = false;
        if let Some(focus) = self.settings_focus.take() {
            focus.focus(window, cx);
        } else {
            self.focus_composer = true;
        }
        cx.notify();
    }
    pub(super) fn save_global(&mut self, settings: AppSettings, cx: &mut Context<Self>) {
        let Some(settings) = self.global_settings_save.request(settings) else {
            cx.notify();
            return;
        };
        if self
            .persist
            .send(Persist::GlobalSettings(settings))
            .is_err()
        {
            self.global_settings_save.send_failed();
            self.banner = Some("Settings could not be saved".into());
        }
        cx.notify();
    }
    pub(super) fn settings_for_edit(&self) -> AppSettings {
        self.global_settings_save
            .edit_base()
            .cloned()
            .unwrap_or_else(|| self.app_settings.clone())
    }
    pub(super) fn settings_picker_choices(&self, picker: SettingsPicker) -> Vec<PickerChoice> {
        let settings = self.settings_for_edit();
        if matches!(picker, SettingsPicker::DefaultEffort) {
            let selected = settings.default_execution.as_ref();
            let model = selected.and_then(|s| {
                self.catalogs
                    .get(&s.provider)?
                    .as_ref()
                    .ok()?
                    .models
                    .iter()
                    .find(|m| Some(&m.id) == s.model.as_ref())
            });
            let mut choices = vec![PickerChoice {
                label: "Provider default".into(),
                selected: selected.is_none_or(|s| s.effort.is_none()),
                action: PickerAction::SettingsEffort(None),
            }];
            if let Some(model) = model {
                choices.extend(model.efforts.iter().map(|effort| PickerChoice {
                    label: effort_label(effort),
                    selected: selected.is_some_and(|s| s.effort.as_ref() == Some(effort)),
                    action: PickerAction::SettingsEffort(Some(effort.clone())),
                }));
            }
            return choices;
        }
        let selected = if matches!(picker, SettingsPicker::RewriteModel) {
            &settings.rewrite_model
        } else {
            &settings.default_execution
        };
        PROVIDERS
            .into_iter()
            .filter(|p| settings.provider(*p).enabled)
            .filter(|p| {
                !matches!(picker, SettingsPicker::RewriteModel)
                    || self.rewriting.support.get(p).is_some_and(|r| r.is_ok())
            })
            .flat_map(|provider| {
                self.catalogs
                    .get(&provider)
                    .and_then(|r| r.as_ref().ok())
                    .into_iter()
                    .flat_map(move |c| c.models.iter().map(move |m| (provider, m)))
            })
            .filter(|(p, m)| settings.visible(*p, &m.id))
            .map(|(provider, model)| PickerChoice {
                label: format!("{} · {}", provider.label(), model.label),
                selected: selected
                    .as_ref()
                    .is_some_and(|s| s.provider == provider && s.model.as_ref() == Some(&model.id)),
                action: PickerAction::SettingsModel(picker, provider, model.id.clone()),
            })
            .collect()
    }

    pub(super) fn choose_settings(&mut self, action: PickerAction, cx: &mut Context<Self>) {
        let mut next = self.settings_for_edit();
        match action {
            PickerAction::SettingsModel(picker, provider, model) => {
                if !next.visible(provider, &model) {
                    return;
                }
                if matches!(picker, SettingsPicker::RewriteModel) {
                    if !self
                        .rewriting
                        .support
                        .get(&provider)
                        .is_some_and(|r| r.is_ok())
                    {
                        return;
                    }
                    next.rewrite_model = Some(ExecutionSettings {
                        provider,
                        model: Some(model),
                        ..Default::default()
                    });
                } else if !next
                    .default_execution
                    .as_ref()
                    .is_some_and(|s| s.provider == provider && s.model.as_ref() == Some(&model))
                {
                    next.default_execution = Some(ExecutionSettings {
                        provider,
                        model: Some(model),
                        ..Default::default()
                    });
                }
            }
            PickerAction::SettingsEffort(effort) => {
                if let Some(settings) = &mut next.default_execution {
                    settings.effort = effort;
                }
            }
            _ => return,
        }
        self.save_global(next, cx);
    }

    fn settings_model_label(&self, settings: Option<&ExecutionSettings>, empty: &str) -> String {
        settings
            .map(|s| {
                let model = self
                    .catalogs
                    .get(&s.provider)
                    .and_then(|r| r.as_ref().ok())
                    .and_then(|c| c.models.iter().find(|m| Some(&m.id) == s.model.as_ref()))
                    .map(|m| m.label.as_str())
                    .or(s.model.as_deref())
                    .unwrap_or("Provider default");
                format!("{} · {model}", s.provider.label())
            })
            .unwrap_or_else(|| empty.into())
    }

    fn settings_selector(
        &self,
        picker: SettingsPicker,
        label: String,
        cx: &Context<Self>,
    ) -> gpui::Div {
        let selector = control(picker.anchor(), cx, move |this, window, cx| {
            this.open_picker(Popup::Settings(picker), window, cx)
        })
        .px_2()
        .py_1()
        .max_w(px(260.))
        .min_w(px(0.))
        .bg(rgb(COMPOSER))
        .flex()
        .items_center()
        .gap_2()
        .child(div().flex_1().min_w(px(0.)).truncate().child(label))
        .child(div().text_size(px(11.)).text_color(rgb(MUTED)).child("⌄"));
        self.picker_control(picker.anchor(), selector, cx)
    }

    fn render_provider_settings(
        &self,
        provider: Provider,
        settings: &AppSettings,
        cx: &Context<Self>,
    ) -> gpui::Div {
        let label = provider.label();
        let prefs = settings.provider(provider);
        let status = self
            .provider_status
            .get(&provider)
            .map(|s| s.label())
            .unwrap_or("Checking sign-in…");
        let expanded = self.expanded_providers.contains(&provider);
        let mut section = div().flex().flex_col().gap_2().child(
            row()
                .flex_wrap()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(80.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(label),
                        )
                        .child(caption(status)),
                )
                .child(
                    control(
                        element_id("provider-refresh", label),
                        cx,
                        move |this, _, cx| {
                            if !this.global_settings_save.is_saving() {
                                this.discover(provider);
                                cx.notify();
                            }
                        },
                    )
                    .px_2()
                    .py_1()
                    .text_color(rgb(MUTED))
                    .child("↻")
                    .tooltip(hint(format!("Refresh {label} status and models"))),
                )
                .child(
                    control(
                        element_id("provider-advanced", label),
                        cx,
                        move |this, _, cx| {
                            if !this.expanded_providers.remove(&provider) {
                                this.expanded_providers.insert(provider);
                            }
                            cx.notify();
                        },
                    )
                    .px_2()
                    .py_1()
                    .text_color(rgb(MUTED))
                    .child(if expanded {
                        "Advanced ⌃"
                    } else {
                        "Advanced ⌄"
                    }),
                )
                .child(toggle(
                    element_id("provider-enabled", label),
                    prefs.enabled,
                    format!(
                        "{} {label}",
                        if prefs.enabled { "Disable" } else { "Enable" }
                    ),
                    cx,
                    move |this, _, cx| {
                        let mut next = this.settings_for_edit();
                        let enabled = next.provider(provider).enabled;
                        next.providers.entry(provider).or_default().enabled = !enabled;
                        this.save_global(next, cx);
                    },
                )),
        );
        if let Some(ProviderReadiness::Unavailable(detail)) = self.provider_status.get(&provider) {
            section = section.child(caption(detail.clone()));
        }
        if matches!(
            self.provider_status.get(&provider),
            Some(ProviderReadiness::Missing | ProviderReadiness::SignedOut)
        ) {
            let command = match provider {
                Provider::Codex => "codex login",
                Provider::Claude => "claude auth login",
            };
            section = section.child(
                row()
                    .flex_wrap()
                    .child(caption("Install the CLI, sign in, then refresh."))
                    .child(
                        control(element_id("copy-signin", label), cx, move |_, _, cx| {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(command.into()))
                        })
                        .px_2()
                        .py_1()
                        .child("Copy sign-in command"),
                    ),
            );
        }
        if expanded {
            let env_key = match provider {
                Provider::Codex => "SMALLER_TALK_CODEX_BIN",
                Provider::Claude => "SMALLER_TALK_CLAUDE_BIN",
            };
            section = section.child(
                surface()
                    .p_3()
                    .gap_2()
                    .child(caption(format!(
                        "Executable path · {env_key} takes precedence"
                    )))
                    .child(
                        row()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .bg(rgb(BACKGROUND))
                                    .text_size(px(13.))
                                    .child(self.path_editors[&provider].clone()),
                            )
                            .child(
                                control(
                                    element_id("save-executable", label),
                                    cx,
                                    move |this, _, cx| {
                                        let value = this.path_editors[&provider].read(cx).content();
                                        let mut next = this.settings_for_edit();
                                        next.providers.entry(provider).or_default().executable =
                                            if value.trim().is_empty() {
                                                None
                                            } else {
                                                Some(PathBuf::from(value.trim()))
                                            };
                                        this.save_global(next, cx);
                                    },
                                )
                                .px_2()
                                .py_1()
                                .child("Save path"),
                            ),
                    ),
            );
        }
        let mut models = surface();
        let mut show_models = true;
        match self.catalogs.get(&provider) {
            Some(Ok(catalog)) => {
                show_models = self.model_query.trim().is_empty()
                    || catalog.models.iter().any(|model| {
                        model_matches(&self.model_query, provider, &model.label, &model.id)
                    });
                let mut count = 0;
                for model in &catalog.models {
                    if !model_matches(&self.model_query, provider, &model.label, &model.id) {
                        continue;
                    }
                    if count > 0 {
                        models = models.child(separator());
                    }
                    count += 1;
                    let key = format!("{}:{}", provider.key(), model.id);
                    let visible = !prefs.hidden_models.contains(&model.id);
                    let id = model.id.clone();
                    models = models.child(
                        row()
                            .px_3()
                            .min_h(px(38.))
                            .child(
                                div()
                                    .id(element_id("model-name", &key))
                                    .flex_1()
                                    .min_w(px(0.))
                                    .text_size(px(13.))
                                    .truncate()
                                    .text_color(rgb(if visible { TEXT } else { MUTED }))
                                    .child(model.label.clone())
                                    .tooltip(hint(model.id.clone())),
                            )
                            .child(toggle(
                                element_id("model-visibility", &key),
                                visible,
                                format!(
                                    "{} {} in the model picker",
                                    if visible { "Hide" } else { "Show" },
                                    model.label
                                ),
                                cx,
                                move |this, _, cx| {
                                    let mut next = this.settings_for_edit();
                                    let hidden = &mut next
                                        .providers
                                        .entry(provider)
                                        .or_default()
                                        .hidden_models;
                                    if !hidden.remove(&id) {
                                        hidden.insert(id.clone());
                                    }
                                    this.save_global(next, cx);
                                },
                            )),
                    );
                }
                if count == 0 && self.model_query.trim().is_empty() {
                    models = models.child(div().p_3().child(caption("No matching models")));
                }
            }
            Some(Err(error)) => models = models.child(div().p_3().child(caption(error.clone()))),
            None => {
                models = models.child(div().p_3().child(caption(if prefs.enabled {
                    "Models appear after sign-in and refresh."
                } else {
                    "Enable this provider to discover models."
                })))
            }
        }
        section.when(show_models, |section| section.child(models))
    }

    fn render_provider_page(&self, settings: &AppSettings, cx: &Context<Self>) -> gpui::Div {
        let mut defaults = surface().child(
            setting_row()
                .flex_wrap()
                .child(div().flex_1().text_size(px(13.)).child("Default model"))
                .child(self.settings_selector(
                    SettingsPicker::DefaultModel,
                    self.settings_model_label(
                        settings.default_execution.as_ref(),
                        "Choose a model",
                    ),
                    cx,
                )),
        );
        let model = settings.default_execution.as_ref().and_then(|s| {
            self.catalogs
                .get(&s.provider)?
                .as_ref()
                .ok()?
                .models
                .iter()
                .find(|m| Some(&m.id) == s.model.as_ref())
        });
        if model.is_some_and(|m| !m.efforts.is_empty()) {
            defaults = defaults.child(separator()).child(
                setting_row()
                    .flex_wrap()
                    .child(div().flex_1().text_size(px(13.)).child("Reasoning effort"))
                    .child(
                        self.settings_selector(
                            SettingsPicker::DefaultEffort,
                            settings
                                .default_execution
                                .as_ref()
                                .and_then(|s| s.effort.as_deref())
                                .map(effort_label)
                                .unwrap_or_else(|| "Provider default".into()),
                            cx,
                        ),
                    ),
            );
        }
        let mut page = div()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(defaults)
                    .child(caption(
                        "Defaults apply to new chats. Existing chats keep their selections.",
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Models"),
                    )
                    .child(
                        row()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(rgb(PANEL))
                            .border_1()
                            .border_color(gpui::rgba((BORDER << 8) | 0x88))
                            .child(icon("search").size(px(14.)).text_color(rgb(MUTED)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .text_size(px(13.))
                                    .child(self.model_search.clone()),
                            )
                            .when(!self.model_query.is_empty(), |r| {
                                r.child(
                                    control(
                                        "clear-settings-model-search",
                                        cx,
                                        |this, window, cx| {
                                            this.model_query.clear();
                                            this.model_search.update(cx, |e, cx| {
                                                e.set_content(String::new(), window, cx)
                                            });
                                            this.model_search.focus_handle(cx).focus(window, cx);
                                            cx.notify();
                                        },
                                    )
                                    .px_1()
                                    .child("×"),
                                )
                            }),
                    )
                    .child(caption("Choose which models appear in the chat picker.")),
            );
        for provider in PROVIDERS {
            page = page.child(self.render_provider_settings(provider, settings, cx));
        }
        page
    }

    fn render_response_page(&self, settings: &AppSettings, cx: &Context<Self>) -> gpui::Div {
        let mut choices = surface()
            .child(
                setting_row()
                    .flex_wrap()
                    .child(div().flex_1().text_size(px(13.)).child("Rewrite model"))
                    .child(self.settings_selector(
                        SettingsPicker::RewriteModel,
                        self.settings_model_label(
                            settings.rewrite_model.as_ref(),
                            "Choose a model",
                        ),
                        cx,
                    )),
            )
            .child(separator())
            .child(
                setting_row()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .child("Automatically shorten long answers"),
                            )
                            .child(caption("For completed Chat answers above 300 prose words.")),
                    )
                    .child(toggle(
                        "automatic-shortening",
                        settings.automatic_shortening,
                        "Automatically shorten long answers".into(),
                        cx,
                        |this, _, cx| {
                            let mut next = this.settings_for_edit();
                            if next.rewrite_model.is_none() {
                                this.banner = Some("Choose a rewrite model first.".into());
                                cx.notify();
                                return;
                            }
                            next.automatic_shortening = !next.automatic_shortening;
                            this.save_global(next, cx);
                        },
                    )),
            );
        for provider in PROVIDERS {
            if let Some(Err(reason)) = self.rewriting.support.get(&provider) {
                choices = choices.child(div().px_3().pb_3().child(caption(reason.clone())));
            }
        }
        div().flex().flex_col().gap_3()
            .child(choices)
            .child(caption("Shortening uses additional provider quota and adds latency. Original answers remain available."))
            .child(caption("Only CLI connections with verified isolation support appear here. Plans, questions, and tool details are never automatically shortened."))
    }

    pub(super) fn render_settings(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let settings = self.settings_for_edit();
        let mut tabs = row()
            .gap_4()
            .border_b_1()
            .border_color(gpui::rgba((BORDER << 8) | 0x66));
        for (section, label) in [
            (SettingsSection::Providers, "Providers & Models"),
            (SettingsSection::ResponseDisplay, "Response Display"),
            (SettingsSection::Instructions, "Instructions"),
        ] {
            let selected = self.settings_section == section;
            tabs = tabs.child(
                control(
                    element_id("settings-section", label),
                    cx,
                    move |this, window, cx| {
                        this.settings_section = section;
                        if section == SettingsSection::Instructions {
                            this.open_instruction_scope(
                                super::instructions::InstructionScope::Global,
                                window,
                                cx,
                            );
                        }
                        this.focus.focus(window, cx);
                        cx.notify();
                    },
                )
                .px_1()
                .py_2()
                .rounded_none()
                .border_0()
                .border_b_2()
                .border_color(if selected { rgb(ACCENT) } else { gpui::rgba(0) })
                .text_color(rgb(if selected { TEXT } else { MUTED }))
                .child(label),
            );
        }
        let page = match self.settings_section {
            SettingsSection::Providers => self.render_provider_page(&settings, cx),
            SettingsSection::ResponseDisplay => self.render_response_page(&settings, cx),
            SettingsSection::Instructions => self.render_instructions(cx),
        };
        let header = div()
            .w_full()
            .max_w(px(720.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                row()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(18.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Settings"),
                    )
                    .when(self.global_settings_save.is_saving(), |r| {
                        r.child(caption("Saving…"))
                    })
                    .child(
                        control("close-settings", cx, |this, window, cx| {
                            this.close_settings(window, cx)
                        })
                        .px_2()
                        .py_1()
                        .text_color(rgb(MUTED))
                        .child("Back to chat"),
                    ),
            )
            .child(tabs);
        let mut content = div()
            .w_full()
            .max_w(px(720.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(page);
        if let Some(error) = &self.banner {
            content = content.child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(DANGER))
                    .child(error.clone()),
            );
        }
        let scroll_id = match self.settings_section {
            SettingsSection::Providers => "settings-providers-scroll",
            SettingsSection::ResponseDisplay => "settings-response-scroll",
            SettingsSection::Instructions => "settings-instructions-scroll",
        };
        div()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_5()
                    .pt_4()
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(header),
            )
            .child(
                div().flex_1().min_h(px(0.)).child(
                    div()
                        .id(scroll_id)
                        .size_full()
                        .overflow_y_scroll()
                        .px_5()
                        .py_4()
                        .flex()
                        .flex_col()
                        .items_center()
                        .child(content),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::GlobalSettingsSave;
    use smaller_talk_domain::{AppSettings, Provider, ProviderPreferences};

    fn settings(enabled: bool) -> AppSettings {
        AppSettings {
            providers: [(
                Provider::Codex,
                ProviderPreferences {
                    enabled,
                    ..Default::default()
                },
            )]
            .into(),
            ..Default::default()
        }
    }

    #[test]
    fn acknowledged_pending_settings_are_sent_once_and_later_edits_start_new_saves() {
        let mut saves = GlobalSettingsSave::default();
        let first = settings(false);
        assert_eq!(saves.request(first.clone()), Some(first.clone()));

        let mut second = saves.edit_base().unwrap().clone();
        second.providers.get_mut(&Provider::Codex).unwrap().enabled = true;
        assert_eq!(saves.request(second.clone()), None);

        let (completed, next) = saves.acknowledge();
        assert_eq!(completed, Some(first));
        assert_eq!(next, Some(second.clone()));
        assert_eq!(saves.edit_base(), Some(&second));
        assert!(saves.is_saving());

        let (completed, next) = saves.acknowledge();
        assert_eq!(completed, Some(second.clone()));
        assert_eq!(next, None);
        assert!(!saves.is_saving());

        let later = settings(false);
        assert_eq!(saves.request(later.clone()), Some(later));
        assert!(saves.is_saving());
    }
}
