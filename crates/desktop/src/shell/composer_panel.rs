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
        let settings_locked = working || state.settings_pending;
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
        let can_send = state.loaded
            && provider_enabled
            && provider_ready
            && model_available
            && !self.global_settings_save.is_saving()
            && !state.enqueue_pending
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
            .p_3()
            .rounded_2xl()
            .border_1()
            .border_color(rgb(if state.composer.focus_handle(cx).is_focused(window) {
                0x66666b
            } else {
                BORDER
            }))
            .bg(rgb(COMPOSER))
            .flex()
            .flex_col()
            .gap_2()
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
                        .max_w(px(260.))
                        .when_some(file.preview_path.clone(), |row, path| {
                            row.child(thumbnail(
                                element_id("draft-image-cache", &file.id),
                                path,
                                56.,
                                48.,
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
                            button(element_id("remove-attachment", &file.id), "×")
                                .tooltip(hint(format!("Remove {}", file.name)))
                                .w(px(28.))
                                .h(px(28.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .px_0()
                                .py_0()
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
        if state.importing {
            composer = composer.child(muted("Saving attachments…"));
        }
        composer = composer.child(state.composer.clone());
        let attach = button("attach-files", "+")
            .tooltip(hint("Attach images or files"))
            .flex_shrink_0()
            .px_2()
            .on_click(cx.listener(move |this, _, _, cx| this.pick_files(id, cx)));
        let model = self
            .picker_control(
                "model-picker",
                button("model-picker", model.clone())
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
                    .truncate()
                    .px_2()
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
                button("effort-picker", format!("{effort} ▾"))
                    .when(self.popup.is_none(), |b| {
                        b.tooltip(hint(if settings_locked {
                            "Effort changes are available after the current operation finishes."
                        } else {
                            "Reasoning effort for the selected model"
                        }))
                    })
                    .opacity(if settings_locked { 0.5 } else { 1. })
                    .tab_stop(!settings_locked)
                    .truncate()
                    .px_2()
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
        let mode = button(
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
        .px_2()
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
        let send = button("send-message", if queued_action { "" } else { "↑" })
            .tooltip(hint(if !provider_enabled {
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
            .w(px(34.))
            .h(px(34.))
            .px_0()
            .py_1()
            .flex()
            .justify_center()
            .rounded_full()
            .bg(rgb(if can_send { SEND } else { PANEL_HOVER }))
            .text_color(rgb(if can_send { PANEL } else { MUTED }))
            .text_lg()
            .when(queued_action, |b| {
                b.child(icon("queue").text_color(rgb(if can_send { PANEL } else { MUTED })))
            })
            .when(can_send, |b| {
                b.on_click(cx.listener(move |this, _, _, cx| this.send(id, cx)))
            });
        let selection = row()
            .gap_0()
            .min_w(px(0.))
            .max_w(px(310.))
            .rounded_full()
            .child(model)
            .child(effort);
        let controls = row()
            .w_full()
            .gap_1()
            .child(attach)
            .child(mode)
            .child(div().flex_1().min_w(px(0.)))
            .child(selection)
            .child(send);
        composer.child(controls).into_any_element()
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
            .rounded_t_lg()
            .bg(rgb(COMPOSER))
            .border_1()
            .border_color(rgb(BORDER))
            .text_sm();
        if let Some(reason) = &queue.paused {
            panel =
                panel.child(
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
                        .child(button("resume-queue", "Resume").px_2().py_1().on_click(
                            cx.listener(move |this, _, window, cx| {
                                this.services.queue.resume(chat);
                                if let Some(state) = this.states.get(&chat) {
                                    state.composer.focus_handle(cx).focus(window);
                                }
                                cx.notify();
                            }),
                        )),
                );
        }
        let mut rows = div()
            .id("queued-messages")
            .max_h(px(if waiting && height < px(600.) {
                36.
            } else {
                108.
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
                .h(px(36.))
                .px_2()
                .gap_1()
                .child(icon("queue").text_color(rgb(MUTED)))
                .child(div().flex_1().min_w(px(0.)).truncate().child(preview));
            if sending {
                line = line.child(div().text_xs().text_color(rgb(MUTED)).child("Sending…"));
            } else if uncertain {
                line = line.child(button(element_id("retry-queued", id), "Retry").px_2().py_1()
                    .tooltip(hint("Delivery is uncertain. Check the conversation first; Retry makes this entry eligible after Resume."))
                    .on_click(cx.listener(move |this, _, window, cx| { this.services.queue.retry(RetryQueueInput { chat_id: chat, entry_id: id }); if let Some(state) = this.states.get(&chat) { state.composer.focus_handle(cx).focus(window); } cx.notify(); })));
            } else {
                line = line.child(
                    button(element_id("steer-queued", id), "↪ Steer")
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
                                    state.composer.focus_handle(cx).focus(window);
                                }
                                cx.notify();
                            }))
                        }),
                );
            }
            line = line.child(
                button(element_id("remove-queued", id), "")
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
                                state.composer.focus_handle(cx).focus(window);
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
