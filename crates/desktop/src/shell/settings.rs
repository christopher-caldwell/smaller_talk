use super::*;

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
        self.focus_composer = false;
        self.model_query.clear();
        self.model_search.update(cx, |e, cx| {
            e.set_picker_navigation(false);
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
        self.focus.focus(window);
        cx.notify();
    }
    pub(super) fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = false;
        if let Some(focus) = self.settings_focus.take() {
            focus.focus(window);
        } else {
            self.focus_composer = true;
        }
        cx.notify();
    }
    fn save_global(&mut self, settings: AppSettings, cx: &mut Context<Self>) {
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
    fn settings_for_edit(&self) -> AppSettings {
        self.global_settings_save
            .edit_base()
            .cloned()
            .unwrap_or_else(|| self.app_settings.clone())
    }
    pub(super) fn render_settings(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let mut content = div()
            .w_full()
            .max_w(px(800.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                row()
                    .child(
                        div()
                            .flex_1()
                            .text_lg()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Settings"),
                    )
                    .child(button("close-settings", "Back to chat").on_click(
                        cx.listener(|this, _, window, cx| this.close_settings(window, cx)),
                    )),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Providers & Models"),
            )
            .child(muted(
                "Use your installed agents and their existing sign-ins.",
            ))
            .child(
                div()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(BORDER))
                    .child(
                        row()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .child(self.model_search.clone()),
                            )
                            .when(!self.model_query.is_empty(), |row| {
                                row.child(
                                    button("clear-settings-model-search", "×").py_0().on_click(
                                        cx.listener(|this, _, window, cx| {
                                            this.model_query.clear();
                                            this.model_search.update(cx, |editor, cx| {
                                                editor.set_content(String::new(), window, cx)
                                            });
                                            this.model_search.focus_handle(cx).focus(window);
                                            cx.notify();
                                        }),
                                    ),
                                )
                            }),
                    ),
            );
        for provider in PROVIDERS {
            let prefs = self.app_settings.provider(provider);
            let label = provider.label();
            let status = self
                .provider_status
                .get(&provider)
                .map(|s| s.label())
                .unwrap_or("Checking sign-in…");
            let expanded = self.expanded_providers.contains(&provider);
            let mut section = div()
                .w_full()
                .p_3()
                .rounded_lg()
                .border_1()
                .border_color(rgb(BORDER))
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    row()
                        .flex_wrap()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(label),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(rgb(MUTED))
                                .child(status.to_owned()),
                        )
                        .child(
                            button(
                                element_id("provider-enabled", label),
                                if prefs.enabled { "On" } else { "Off" },
                            )
                            .text_color(rgb(if prefs.enabled { ACCENT } else { MUTED }))
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    let mut next = this.settings_for_edit();
                                    let enabled = next.provider(provider).enabled;
                                    next.providers.entry(provider).or_default().enabled = !enabled;
                                    this.save_global(next, cx);
                                },
                            )),
                        )
                        .child(
                            button(element_id("provider-refresh", label), "Refresh").on_click(
                                cx.listener(move |this, _, _, cx| {
                                    if !this.global_settings_save.is_saving() {
                                        this.discover(provider);
                                        cx.notify();
                                    }
                                }),
                            ),
                        )
                        .child(
                            button(
                                element_id("provider-advanced", label),
                                if expanded { "Less" } else { "Advanced" },
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if !this.expanded_providers.remove(&provider) {
                                        this.expanded_providers.insert(provider);
                                    }
                                    cx.notify();
                                },
                            )),
                        ),
                );
            if let Some(ProviderReadiness::Unavailable(detail)) =
                self.provider_status.get(&provider)
            {
                section = section.child(muted(detail.clone()));
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
                        .child(muted("Install the official CLI, sign in, then Refresh."))
                        .child(
                            button(element_id("copy-signin", label), "Copy sign-in command")
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                        command.into(),
                                    ))
                                }),
                        ),
                );
            }
            if expanded {
                let env_key = match provider {
                    Provider::Codex => "SMALLER_TALK_CODEX_BIN",
                    Provider::Claude => "SMALLER_TALK_CLAUDE_BIN",
                };
                section = section
                    .child(muted(format!(
                        "Executable override · {env_key} takes precedence"
                    )))
                    .child(
                        row()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .p_2()
                                    .border_1()
                                    .border_color(rgb(BORDER))
                                    .child(self.path_editors[&provider].clone()),
                            )
                            .child(
                                button(element_id("save-executable", label), "Save path").on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        let value = this.path_editors[&provider].read(cx).content();
                                        let mut next = this.settings_for_edit();
                                        next.providers.entry(provider).or_default().executable =
                                            if value.trim().is_empty() {
                                                None
                                            } else {
                                                Some(PathBuf::from(value.trim()))
                                            };
                                        this.save_global(next, cx);
                                    }),
                                ),
                            ),
                    );
            }
            if let Some(Ok(catalog)) = self.catalogs.get(&provider) {
                let mut count = 0;
                for model in &catalog.models {
                    if !model_matches(&self.model_query, provider, &model.label, &model.id) {
                        continue;
                    }
                    count += 1;
                    let model_id = model.id.clone();
                    let key = format!("{}:{}", provider.key(), model.id);
                    let visible = !self
                        .app_settings
                        .provider(provider)
                        .hidden_models
                        .contains(&model.id);
                    let is_default =
                        self.app_settings
                            .default_execution
                            .as_ref()
                            .is_some_and(|s| {
                                s.provider == provider && s.model.as_ref() == Some(&model.id)
                            });
                    let is_rewriter = self.app_settings.rewrite_model.as_ref().is_some_and(|s| {
                        s.provider == provider && s.model.as_ref() == Some(&model.id)
                    });
                    let mut item = row()
                        .flex_wrap()
                        .py_1()
                        .border_t_1()
                        .border_color(rgb(BORDER))
                        .child(
                            div()
                                .id(element_id("model-name", &key))
                                .flex_1()
                                .min_w(px(120.))
                                .text_sm()
                                .child(model.label.clone())
                                .tooltip(hint(model.id.clone())),
                        );
                    let id = model_id.clone();
                    item = item.child(
                        button(
                            element_id("model-visibility", &key),
                            if visible { "Visible" } else { "Hidden" },
                        )
                        .text_xs()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let mut next = this.settings_for_edit();
                            let hidden =
                                &mut next.providers.entry(provider).or_default().hidden_models;
                            if !hidden.remove(&id) {
                                hidden.insert(id.clone());
                            }
                            this.save_global(next, cx);
                        })),
                    );
                    let id = model_id.clone();
                    item = item.child(
                        button(
                            element_id("model-default", &key),
                            if is_default {
                                "✓ Default"
                            } else {
                                "Set default"
                            },
                        )
                        .text_xs()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.app_settings.visible(provider, &id) {
                                return;
                            }
                            let mut next = this.settings_for_edit();
                            next.default_execution = Some(ExecutionSettings {
                                provider,
                                model: Some(id.clone()),
                                ..Default::default()
                            });
                            this.save_global(next, cx);
                        })),
                    );
                    let rewrite_available = self
                        .rewriting
                        .support
                        .get(&provider)
                        .is_some_and(|r| r.is_ok());
                    item = item.child(
                        button(
                            element_id("model-rewriter", &key),
                            if is_rewriter {
                                "✓ Rewrite"
                            } else if rewrite_available {
                                "Use to rewrite"
                            } else {
                                "Rewrite unavailable"
                            },
                        )
                        .text_xs()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !rewrite_available || !this.app_settings.visible(provider, &model_id)
                            {
                                return;
                            }
                            let mut next = this.settings_for_edit();
                            next.rewrite_model = Some(ExecutionSettings {
                                provider,
                                model: Some(model_id.clone()),
                                ..Default::default()
                            });
                            this.save_global(next, cx);
                        })),
                    );
                    section = section.child(item);
                    if is_default && !model.efforts.is_empty() {
                        let mut efforts = row().flex_wrap().child(muted("Default effort"));
                        for effort in
                            std::iter::once(None).chain(model.efforts.iter().cloned().map(Some))
                        {
                            let selected = self
                                .app_settings
                                .default_execution
                                .as_ref()
                                .is_some_and(|s| s.effort == effort);
                            let label = effort
                                .as_deref()
                                .map(effort_label)
                                .unwrap_or_else(|| "Provider default".into());
                            efforts = efforts.child(
                                button(
                                    element_id("default-effort", &label),
                                    format!("{}{label}", if selected { "✓ " } else { "" }),
                                )
                                .text_xs()
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        let mut next = this.settings_for_edit();
                                        if let Some(s) = next.default_execution.as_mut() {
                                            s.effort = effort.clone();
                                        }
                                        this.save_global(next, cx);
                                    },
                                )),
                            );
                        }
                        section = section.child(efforts);
                    }
                }
                if count == 0 {
                    section = section.child(muted("No matching models"));
                }
            } else if let Some(Err(error)) = self.catalogs.get(&provider) {
                section = section.child(muted(error.clone()));
            } else {
                section = section.child(muted(if self.app_settings.provider(provider).enabled {
                    "Models appear after a successful sign-in and refresh."
                } else {
                    "Enable this provider to discover models."
                }));
            }
            if let Some(Err(reason)) = self.rewriting.support.get(&provider) {
                section =
                    section.child(div().text_xs().text_color(rgb(MUTED)).child(reason.clone()));
            }
            content = content.child(section);
        }
        let rewrite = self
            .app_settings
            .rewrite_model
            .as_ref()
            .map(|s| {
                format!(
                    "{} · {}",
                    s.provider.label(),
                    s.model.as_deref().unwrap_or("Provider default")
                )
            })
            .unwrap_or_else(|| "Choose a model above using ‘Use to rewrite’.".into());
        content = content.child(div().mt_3().text_sm().font_weight(gpui::FontWeight::SEMIBOLD).child("Response Display"))
            .child(muted(format!("Rewrite model: {rewrite}")))
            .child(muted("Shortening uses additional provider quota and adds latency. Originals remain available. Only successful Chat answers can be shortened."))
            .child(button("automatic-shortening", format!("{}  Automatically shorten long answers",if self.app_settings.automatic_shortening { "✓" } else { "○" })).on_click(cx.listener(|this,_,_,cx| {
                let mut next = this.settings_for_edit();
                if next.rewrite_model.is_none() { this.banner = Some("Choose a rewrite model first.".into()); cx.notify(); return; }
                next.automatic_shortening = !next.automatic_shortening;
                this.save_global(next,cx);
            })))
            .child(muted("Automatic shortening starts above 300 prose words. Rewrite availability depends on verified CLI isolation support."));
        if self.global_settings_save.is_saving() {
            content = content.child(muted("Saving settings…"));
        }
        if let Some(error) = &self.banner {
            content = content.child(div().text_sm().text_color(rgb(DANGER)).child(error.clone()));
        }
        div()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("settings-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .p_4()
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(content),
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
