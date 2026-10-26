use super::*;

impl ChatsView {
    pub(super) fn render_composer(
        &mut self,
        id: Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let state = &self.states[&id];
        let settings = state.preferences.execution.clone();
        let working = state.activity.is_some();
        let settings_locked =
            working || state.settings_pending || state.enqueue_pending || state.queue_edit_pending;
        // Use the measured conversation width; leave room for the fixed controls.
        let model_width = (self.last_width - px(240.)).clamp(px(48.), px(210.));
        let model = self
            .selected_model(id)
            .map(|m| m.label.clone())
            .unwrap_or_else(|| {
                settings.model.clone().unwrap_or_else(|| {
                    match self.catalogs.get(&settings.provider) {
                        None => "Loading models…".into(),
                        Some(Err(_)) => "Models unavailable".into(),
                        Some(Ok(_)) => "Choose model".into(),
                    }
                })
            });
        let model_hint = format!("{} · {}", settings.provider.label(), model);
        let effort = settings
            .effort
            .as_deref()
            .map(effort_label)
            .unwrap_or_else(|| {
                self.selected_model(id)
                    .and_then(|m| m.default_effort.as_deref())
                    .map(effort_label)
                    .unwrap_or_else(|| "Default".into())
            });
        let queued_action = working
            || self
                .queues
                .get(&id)
                .is_some_and(|q| q.paused.is_some() || !q.entries.is_empty());
        let provider_enabled = self.app_settings.provider(settings.provider).enabled;
        let provider_ready = matches!(
            self.provider_status.get(&settings.provider),
            Some(ProviderReadiness::Ready)
        );
        let model_available = self.selected_model(id).is_some();
        let current_workspace = effective_workspace(
            &self.services.launch_folder,
            &state.preferences,
            &self.projects,
        )
        .ok();
        let entries = self.skill_entries();
        let selected_skills_valid = state.composer.read(cx).skills().iter().all(|m| {
            m.reference.provider == settings.provider
                && current_workspace.as_ref() == Some(&m.reference.workspace)
                && (!self.skill_catalog_checked()
                    || entries
                        .iter()
                        .any(|e| e.reference == m.reference && e.unavailable.is_none()))
        });
        let can_send = selected_skills_valid
            && state.loaded
            && provider_enabled
            && provider_ready
            && model_available
            && !self.global_settings_save.is_saving()
            && !state.enqueue_pending
            && !state.queue_edit_pending
            && !state.importing
            && !state.settings_pending
            && !state.settings_error
            && (!state.composer.read(cx).content().trim().is_empty()
                || !state.attachments.is_empty());
        let mut composer = div()
            .id("chat-composer")
            .w_full()
            .max_w(px(800.))
            .min_w(px(0.))
            .px_3()
            .py(px(10.))
            .rounded_xl()
            .border_1()
            .border_color(if state.composer.focus_handle(cx).is_focused(window) {
                gpui::rgba((FOCUS << 8) | 0x88)
            } else {
                quiet_border()
            })
            .bg(rgb(COMPOSER))
            .flex()
            .flex_col()
            .gap(px(6.))
            .on_drop(cx.listener(move |this, files: &ExternalPaths, _, cx| {
                this.import_files(id, files.paths().to_vec(), cx)
            }))
            .drag_over::<ExternalPaths>(|style, _, _, _| {
                style.border_color(rgb(FOCUS)).bg(rgb(PANEL_HOVER))
            });
        if !state.attachments.is_empty() {
            let mut files = row()
                .id("draft-attachments")
                .overflow_x_scroll()
                .flex_shrink_0()
                .gap_2()
                .py_1();
            for file in &state.attachments {
                let file_id = file.id.clone();
                files = files.child(
                    row()
                        .flex_shrink_0()
                        .rounded_md()
                        .bg(rgb(PANEL_HOVER))
                        .px_2()
                        .py_1()
                        .max_w(px(240.))
                        .when_some(file.preview_path.clone(), |row, path| {
                            row.child(thumbnail(
                                element_id("draft-image-cache", &file.id),
                                path,
                                48.,
                                40.,
                            ))
                        })
                        .child(
                            div()
                                .id(element_id("attachment-name", &file.id))
                                .tooltip(hint(format!(
                                    "{} · {} KiB",
                                    file.name,
                                    file.byte_len.div_ceil(1024)
                                )))
                                .text_xs()
                                .truncate()
                                .child(file.name.clone()),
                        )
                        .child(
                            icon_button(element_id("remove-attachment", &file.id), "close")
                                .tooltip(hint(format!("Remove {}", file.name)))
                                .size(px(22.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(state) = this.states.get_mut(&id) {
                                        state.attachments.retain(|a| a.id != file_id);
                                    }
                                    this.draft_changed(id, cx);
                                })),
                        ),
                );
            }
            composer = composer.child(files);
        }
        for mention in state.composer.read(cx).skills() {
            let unavailable = mention.reference.provider != settings.provider
                || current_workspace.as_ref() != Some(&mention.reference.workspace)
                || (self.skill_catalog_checked()
                    && !entries
                        .iter()
                        .any(|d| d.reference == mention.reference && d.unavailable.is_none()));
            if unavailable {
                let remove = mention.range.clone();
                let replace = mention.range.clone();
                composer = composer.child(
                    row()
                        .gap_1()
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(rgb(DANGER))
                                .child(format!(
                                    "${} needs a source for this provider and folder",
                                    mention.reference.name
                                )),
                        )
                        .child(
                            compact_button(
                                element_id("replace-skill", mention.range.start),
                                "Replace",
                            )
                            .text_xs()
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.open_skills(window, cx);
                                    this.skill_ui.range = Some(replace.clone());
                                },
                            )),
                        )
                        .child(
                            compact_button(
                                element_id("remove-skill", mention.range.start),
                                "Remove",
                            )
                            .text_xs()
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if let Some(state) = this.states.get(&id) {
                                        state
                                            .composer
                                            .update(cx, |e, cx| e.remove_skill(remove.clone(), cx));
                                    }
                                },
                            )),
                        ),
                );
            }
        }
        if state.queue_edit_pending {
            composer = composer.child(muted("Restoring queued message…"));
        }
        if state.importing {
            composer = composer.child(muted("Saving attachments…"));
        }
        composer = composer.child(state.composer.clone());
        let attach = icon_button("attach-files", "plus")
            .when(self.popup.is_none(), |b| {
                b.tooltip(hint("Attach files or choose skills"))
            })
            .flex_shrink_0()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.open_picker(Popup::Attachments, window, cx)
            }));
        let model = self
            .picker_control(
                "model-picker",
                compact_button("model-picker", "")
                    .h(px(28.))
                    .py_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(div().min_w(px(0.)).truncate().child(model.clone()))
                    .child(icon("chevron-down").size(px(11.)).text_color(rgb(MUTED)))
                    .when(self.popup.is_none(), |b| {
                        b.tooltip(hint(if settings_locked {
                            "Model changes are available after the current operation finishes."
                                .into()
                        } else {
                            model_hint
                        }))
                    })
                    .opacity(if settings_locked { 0.5 } else { 1. })
                    .tab_stop(!settings_locked)
                    .min_w(px(0.))
                    .when(!settings_locked, |b| {
                        b.on_click(cx.listener(|this, _, window, cx| {
                            this.open_picker(Popup::Models, window, cx)
                        }))
                    }),
                cx,
            )
            .min_w(px(0.))
            .max_w(model_width);
        let effort = self
            .picker_control(
                "effort-picker",
                compact_button("effort-picker", "")
                    .h(px(28.))
                    .py_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(div().min_w(px(0.)).truncate().child(effort))
                    .child(icon("chevron-down").size(px(11.)).text_color(rgb(MUTED)))
                    .when(self.popup.is_none(), |b| {
                        b.tooltip(hint(if settings_locked {
                            "Effort changes are available after the current operation finishes."
                        } else {
                            "Reasoning effort for the selected model"
                        }))
                    })
                    .opacity(if settings_locked { 0.5 } else { 1. })
                    .tab_stop(!settings_locked)
                    .text_color(rgb(MUTED))
                    .when(!settings_locked, |b| {
                        b.on_click(cx.listener(|this, _, window, cx| {
                            this.open_picker(Popup::Effort, window, cx)
                        }))
                    }),
                cx,
            )
            .min_w(px(0.))
            .flex_shrink_0()
            .max_w(px(105.));
        let mode = compact_button(
            "plan-mode",
            if settings.plan_mode { "Plan" } else { "Chat" },
        )
        .opacity(if settings_locked { 0.5 } else { 1. })
        .tooltip(hint(if settings_locked {
            "Mode changes are available after the current operation finishes."
        } else if settings.plan_mode {
            "Plan mode: discuss and propose a plan. Implementation requires your choice."
        } else {
            "Chat mode: switch to Plan to request a plan before implementation."
        }))
        .tab_stop(!settings_locked)
        .flex_shrink_0()
        .h(px(28.))
        .py_0()
        .flex()
        .items_center()
        .bg(rgb(if settings.plan_mode {
            PANEL_HOVER
        } else {
            COMPOSER
        }))
        .text_color(rgb(if settings.plan_mode { ACCENT } else { MUTED }))
        .when(!settings_locked, |b| {
            b.on_click(cx.listener(move |this, _, _, cx| {
                if let Some(state) = this.states.get(&id) {
                    let mut prefs = state.preferences.clone();
                    prefs.execution.plan_mode = !prefs.execution.plan_mode;
                    this.save_preferences(id, prefs, cx);
                }
            }))
        });
        let send = compact_button("send-message", "")
            .tooltip(hint(if !selected_skills_valid {
                "Replace or remove unresolved skills before sending"
            } else if !provider_enabled {
                "Enable this provider in Settings or choose another model"
            } else if !provider_ready {
                "Check provider installation and sign-in in Settings, then Refresh"
            } else if !model_available {
                "Choose an available model before sending"
            } else if self.global_settings_save.is_saving() {
                "Saving global settings…"
            } else if state.enqueue_pending {
                "Saving message…"
            } else if state.importing {
                "Wait for attachments to finish saving"
            } else if state.settings_pending {
                "Saving chat settings…"
            } else if state.settings_error {
                "Resolve the settings error before sending"
            } else if queued_action {
                "Queue message"
            } else {
                "Send message (Enter or Cmd+Enter)"
            }))
            .tab_stop(can_send)
            .flex_shrink_0()
            .size(px(30.))
            .px_0()
            .py_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(rgb(if can_send { SEND } else { PANEL_HOVER }))
            .text_color(rgb(if can_send { PANEL } else { MUTED }))
            .child(
                icon(if queued_action { "queue" } else { "arrow-up" })
                    .size(px(16.))
                    .text_color(rgb(if can_send { PANEL } else { MUTED })),
            )
            .when(can_send, |b| {
                b.on_click(cx.listener(move |this, _, _, cx| this.send(id, cx)))
            });
        let selection = row()
            .gap_1()
            .min_w(px(0.))
            .max_w(px(310.))
            .child(model)
            .child(effort);
        let controls = row()
            .w_full()
            .gap_1()
            .child(attach)
            .child(mode)
            .child(selection)
            .child(div().flex_1().min_w(px(0.)))
            .child(send);
        self.picker_control("skills-picker", composer.child(controls), cx)
            .w_full()
            .max_w(px(800.))
            .into_any_element()
    }
}

impl ChatsView {
    pub(super) fn render_queue(
        &self,
        chat: Uuid,
        height: gpui::Pixels,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let queue = self.queues.get(&chat)?;
        if queue.entries.is_empty() {
            return None;
        }
        let waiting = self.states.get(&chat).is_some_and(|s| s.pending.is_some());
        let mut panel = div()
            .w_full()
            .max_w(px(800.))
            .min_w(px(0.))
            .rounded_lg()
            .bg(rgb(COMPOSER))
            .border_1()
            .border_color(quiet_border())
            .text_size(px(13.));
        if let Some(reason) = &queue.paused {
            panel = panel.child(
                row()
                    .px_2()
                    .py_1()
                    .child(
                        div()
                            .id("queue-pause-reason")
                            .flex_1()
                            .truncate()
                            .text_color(rgb(MUTED))
                            .tooltip(hint(reason.clone()))
                            .child(reason.clone()),
                    )
                    .child(
                        compact_button("resume-queue", "Resume")
                            .px_2()
                            .py_1()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.services.queue.resume(chat);
                                if let Some(state) = this.states.get(&chat) {
                                    state.composer.focus_handle(cx).focus(window, cx);
                                }
                                cx.notify();
                            })),
                    ),
            );
        }
        let mut rows = div()
            .id("queued-messages")
            .max_h(px(if waiting && height < px(600.) {
                32.
            } else {
                96.
            }))
            .overflow_y_scroll();
        for entry in &queue.entries {
            let id = entry.id;
            let sending = matches!(entry.state.as_str(), "dispatching" | "steering");
            let uncertain = entry.state == "uncertain";
            let preview = if entry.draft.text.trim().is_empty() {
                entry
                    .draft
                    .attachments
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                entry
                    .draft
                    .text
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            let can_steer = queue.steer_available
                && queue.paused.is_none()
                && entry.state == "pending"
                && !waiting;
            let mut line = row()
                .id(element_id("queue-entry", id))
                .h(px(32.))
                .px_2()
                .gap_1()
                .child(icon("queue").text_color(rgb(MUTED)))
                .child(div().flex_1().min_w(px(0.)).truncate().child(preview));
            if sending {
                line = line.child(div().text_xs().text_color(rgb(MUTED)).child("Sending…"));
            } else if uncertain {
                line = line.child(compact_button(element_id("retry-queued", id), "Retry").px_2().py_1()
                    .tooltip(hint("Delivery is uncertain. Check the conversation first; Retry makes this entry eligible after Resume."))
                    .on_click(cx.listener(move |this, _, window, cx| { this.services.queue.retry(RetryQueueInput { chat_id: chat, entry_id: id }); if let Some(state) = this.states.get(&chat) { state.composer.focus_handle(cx).focus(window, cx); } cx.notify(); })));
            } else {
                line = line.child(
                    compact_button(element_id("steer-queued", id), "↪ Steer")
                        .px_2()
                        .py_1()
                        .text_color(rgb(MUTED))
                        .opacity(if can_steer { 1. } else { 0.5 })
                        .tab_stop(can_steer)
                        .tooltip(hint(if waiting {
                            "Answer the pending request before steering"
                        } else if queue.paused.is_some() {
                            "Resume the queue before steering"
                        } else if !queue.steer_available {
                            "Steer is available when a running provider confirms live input support"
                        } else {
                            "Send this message to the running agent now"
                        }))
                        .when(can_steer, |button| {
                            button.on_click(cx.listener(move |this, _, window, cx| {
                                this.services.queue.steer(SteerQueueInput {
                                    chat_id: chat,
                                    entry_id: id,
                                });
                                if let Some(state) = this.states.get(&chat) {
                                    state.composer.focus_handle(cx).focus(window, cx);
                                }
                                cx.notify();
                            }))
                        }),
                );
            }
            if !sending && !uncertain {
                line = line.child(
                    compact_button(element_id("edit-queued", id), "Edit")
                        .text_xs()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.edit_queued_message(chat, id, cx);
                        })),
                );
            }
            line = line.child(
                compact_button(element_id("remove-queued", id), "")
                    .w(px(28.))
                    .h(px(28.))
                    .px_0()
                    .py_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("trash").text_color(rgb(MUTED)))
                    .opacity(if sending { 0.4 } else { 1. })
                    .tab_stop(!sending)
                    .tooltip(hint("Remove queued message"))
                    .when(!sending, |b| {
                        b.on_click(cx.listener(move |this, _, window, cx| {
                            this.services.queue.remove(RemoveQueueInput {
                                chat_id: chat,
                                entry_id: id,
                            });
                            if let Some(state) = this.states.get(&chat) {
                                state.composer.focus_handle(cx).focus(window, cx);
                            }
                            cx.notify();
                        }))
                    }),
            );
            rows = rows.child(line);
        }
        Some(panel.child(rows).into_any_element())
    }
}
