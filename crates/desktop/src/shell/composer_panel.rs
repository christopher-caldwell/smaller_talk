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
        let can_send = state.loaded
            && !working
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
                                38.,
                                32.,
                            ))
                        })
                        .child(div().text_xs().truncate().child(file.name.clone()))
                        .child(
                            button(element_id("remove-attachment", &file.id), "×")
                                .tooltip(hint(format!("Remove {}", file.name)))
                                .px_1()
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
                    .when(self.popup.is_none(), |b| b.tooltip(hint(model_hint)))
                    .tab_stop(!working)
                    .truncate()
                    .px_2()
                    .when(!working, |b| {
                        b.on_click(cx.listener(|this, _, window, cx| {
                            this.open_picker(Popup::Models, window, cx)
                        }))
                    }),
                cx,
            )
            .min_w(px(0.))
            .max_w(px(210.));
        let effort = self
            .picker_control(
                "effort-picker",
                button("effort-picker", format!("{effort} ▾"))
                    .when(self.popup.is_none(), |b| {
                        b.tooltip(hint("Reasoning effort for the selected model"))
                    })
                    .tab_stop(!working)
                    .truncate()
                    .px_2()
                    .text_color(rgb(MUTED))
                    .when(!working, |b| {
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
        .tooltip(hint(if settings.plan_mode {
            "Plan mode: discuss and propose a plan. Implementation requires your choice."
        } else {
            "Chat mode: switch to Plan to request a plan before implementation."
        }))
        .tab_stop(!working)
        .flex_shrink_0()
        .px_2()
        .bg(rgb(if settings.plan_mode {
            PANEL_HOVER
        } else {
            COMPOSER
        }))
        .text_color(rgb(if settings.plan_mode { ACCENT } else { MUTED }))
        .when(!working, |b| {
            b.on_click(cx.listener(move |this, _, _, cx| {
                if let Some(state) = this.states.get(&id) {
                    let mut prefs = state.preferences.clone();
                    prefs.execution.plan_mode = !prefs.execution.plan_mode;
                    this.save_preferences(id, prefs, cx);
                }
            }))
        });
        let send = button("send-message", "↑")
            .tooltip(hint(if working {
                "A response is in progress"
            } else {
                "Send message (Enter)"
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
