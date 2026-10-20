use super::*;

impl ChatsView {
    pub(super) fn render_popup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let popup = self.popup.as_ref()?;
        let mut content = div()
            .id("popup-panel")
            .w(px(420.))
            .max_h(px(520.))
            .overflow_y_scroll()
            .p_3()
            .rounded_xl()
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(MENU))
            .shadow_lg()
            .flex()
            .flex_col()
            .gap_1()
            .on_click(|_, _, cx| cx.stop_propagation());
        match *popup {
            Popup::Models | Popup::Effort => {
                if matches!(popup, Popup::Models) {
                    content = content.child(
                        row()
                            .px_2()
                            .py_1()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .child(self.model_search.clone()),
                            )
                            .child(button("clear-model-search", "×").on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.model_search.update(cx, |e, cx| {
                                        e.set_content(String::new(), window, cx)
                                    });
                                    this.model_query.clear();
                                    this.popup_index = 0;
                                    this.model_search.focus_handle(cx).focus(window);
                                    cx.notify();
                                },
                            ))),
                    );
                }
                let choices = self.picker_choices();
                let no_choices = choices.is_empty();
                let mut items = div()
                    .id("picker-items")
                    .overflow_y_scroll()
                    .track_scroll(&self.popup_scroll)
                    .min_h(px(0.))
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_1();
                for (index, choice) in choices.into_iter().enumerate() {
                    let action = choice.action;
                    items = items.child(
                        button(
                            element_id("picker-choice", index),
                            format!(
                                "{}{}",
                                if choice.selected { "✓  " } else { "    " },
                                choice.label
                            ),
                        )
                        .w_full()
                        .py_1()
                        .bg(rgb(if index == self.popup_index {
                            PANEL_HOVER
                        } else {
                            MENU
                        }))
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.choose_picker(action.clone(), window, cx)
                            },
                        )),
                    );
                }
                if matches!(popup, Popup::Models) {
                    if no_choices {
                        items = items.child(div().p_2().child(muted("No matching models")));
                    }
                    for provider in [Provider::Codex, Provider::Claude] {
                        if !self.app_settings.provider(provider).enabled {
                            continue;
                        }
                        match self.catalogs.get(&provider) {
                            None => {
                                let status = self
                                    .provider_status
                                    .get(&provider)
                                    .map(|s| s.label())
                                    .unwrap_or("Checking sign-in…");
                                items =
                                    items.child(muted(format!("{}: {status}", provider.label())));
                            }
                            Some(Err(error)) => {
                                items = items.child(
                                    div()
                                        .p_2()
                                        .text_xs()
                                        .text_color(rgb(DANGER))
                                        .child(error.clone()),
                                );
                            }
                            _ => {}
                        }
                    }
                }
                content = content
                    .child(div().px_3().py_1().text_xs().text_color(rgb(MUTED)).child(
                        if matches!(popup, Popup::Models) {
                            "Select model"
                        } else {
                            "Reasoning effort"
                        },
                    ))
                    .child(items);
                if matches!(popup, Popup::Models) {
                    content = content.child(
                        button("manage-models", "Manage models…")
                            .text_xs()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_settings(window, cx)),
                            ),
                    );
                }
            }
            Popup::Chat(id) => {
                let busy = self.states.get(&id).is_some_and(|s| s.activity.is_some());
                content = content.child(menu_item("rename-chat", "pencil", "Rename").on_click(
                    cx.listener(move |this, _, window, cx| {
                        let title = this
                            .chats
                            .iter()
                            .find(|c| c.id == id)
                            .map(|c| c.title.clone())
                            .unwrap_or_default();
                        this.name_editor
                            .update(cx, |s, cx| s.set_content(title, window, cx));
                        this.popup_return_focus = window.focused(cx);
                        this.name_editor.focus_handle(cx).focus(window);
                        this.popup = Some(Popup::Rename(id));
                        cx.notify();
                    }),
                ));
                if !busy {
                    content = content
                        .child(
                            menu_item("assign-project", "folder", "Project")
                                .child(div().flex_1())
                                .child(icon("chevron"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.popup = Some(Popup::Assign(id));
                                    cx.notify();
                                })),
                        )
                        .child(div().h(px(1.)).mx_2().my_1().bg(rgb(BORDER)))
                        .child(
                            menu_item("delete-chat", "trash", "Delete chat…")
                                .text_color(rgb(DANGER))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.popup = Some(Popup::Delete(id));
                                    cx.notify();
                                })),
                        );
                } else {
                    content = content.child(div().p_3().child(muted(
                        "Project assignment and deletion are unavailable during a response.",
                    )));
                }
            }
            Popup::Assign(id) => {
                content = content.child(
                    button("back-to-chat-actions", "‹  Project")
                        .text_color(rgb(MUTED))
                        .py_1()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.popup = Some(Popup::Chat(id));
                            cx.notify();
                        })),
                );
                let current = self
                    .states
                    .get(&id)
                    .map(|s| s.preferences.project_id.clone())
                    .or_else(|| self.metadata.get(&id).map(|p| p.project_id.clone()))
                    .flatten();
                let mut projects: Vec<(Option<String>, String)> = vec![(None, "No project".into())];
                projects.extend(
                    self.projects
                        .iter()
                        .map(|p| (Some(p.id.to_string()), p.name.clone())),
                );
                for (project, name) in projects {
                    content = content.child(
                        button(
                            element_id("assign-to", project.clone().unwrap_or_default()),
                            format!(
                                "{}{}",
                                if current == project { "✓  " } else { "    " },
                                name
                            ),
                        )
                        .w_full()
                        .py_1()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let mut prefs = this
                                .states
                                .get(&id)
                                .map(|s| s.preferences.clone())
                                .or_else(|| this.metadata.get(&id).cloned())
                                .unwrap_or_default();
                            prefs.project_id = project.clone();
                            this.save_preferences(id, prefs, cx);
                        })),
                    );
                }
            }
            Popup::Rename(id) => {
                content = content
                    .child(div().p_2().child("Rename chat"))
                    .child(
                        div()
                            .p_2()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(BORDER))
                            .child(self.name_editor.clone()),
                    )
                    .child(button("save-chat-name", "Save").on_click(cx.listener(
                        move |this, _, _, cx| {
                            let _ = this
                                .persist
                                .send(Persist::Rename(id, this.name_editor.read(cx).content()));
                            this.popup = None;
                            this.focus_composer = true;
                            cx.notify();
                        },
                    )));
            }
            Popup::Delete(id) => {
                content = content
                    .child(
                        div()
                            .p_2()
                            .child("Delete this chat and its saved messages?"),
                    )
                    .child(
                        button("confirm-delete", "Delete chat")
                            .text_color(rgb(DANGER))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let _ = this.persist.send(Persist::DeleteChat(id));
                                this.popup = None;
                                this.focus_composer = true;
                                cx.notify();
                            })),
                    );
            }
            Popup::Project(id) => {
                content = content
                    .child(div().p_2().child(if id.is_some() {
                        "Edit project"
                    } else {
                        "New project"
                    }))
                    .child(muted("Name"))
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(BORDER))
                            .child(self.name_editor.clone()),
                    )
                    .child(div().mt_2().child(muted("Working folder (optional)")))
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(BORDER))
                            .child(self.folder_editor.clone()),
                    )
                    .child(
                        button("choose-project-folder", "Choose folder…").on_click(cx.listener(
                            |_, _, _, cx| {
                                let chosen = cx.prompt_for_paths(gpui::PathPromptOptions {
                                    files: false,
                                    directories: true,
                                    multiple: false,
                                    prompt: Some("Project folder".into()),
                                });
                                cx.spawn(async move |this, cx| {
                                    if let Ok(Ok(Some(paths))) = chosen.await
                                        && let Some(path) = paths.first()
                                    {
                                        let path = path.display().to_string();
                                        let _ = this.update(cx, |this, cx| {
                                            this.folder_editor.update(cx, |s, cx| {
                                                let draft = Draft {
                                                    text: path,
                                                    ..Default::default()
                                                };
                                                s.load_draft(&draft, cx);
                                            });
                                            cx.notify();
                                        });
                                    }
                                })
                                .detach();
                            },
                        )),
                    )
                    .child(
                        button("save-project", "Save project")
                            .bg(rgb(PANEL_HOVER))
                            .on_click(cx.listener(move |this, _, _, cx| this.save_project(id, cx))),
                    );
            }
            Popup::ProjectMenu(id) => {
                if let Some(project) = self.projects.iter().find(|p| p.id == id) {
                    content = content.child(
                        div()
                            .p_2()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(project.name.clone()),
                    );
                    if let Some(folder) = &project.folder {
                        content = content.child(
                            div()
                                .p_2()
                                .text_sm()
                                .text_color(rgb(MUTED))
                                .child(folder.display().to_string()),
                        );
                    }
                }
                content = content
                    .child(
                        button("edit-project", "Edit project…").on_click(cx.listener(
                            move |this, _, window, cx| this.begin_project(Some(id), window, cx),
                        )),
                    )
                    .child(
                        button("remove-project", "Remove project (keep chats)")
                            .text_color(rgb(DANGER))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if this.states.values().any(|s| {
                                    s.activity.is_some()
                                        && s.preferences.project_id == Some(id.to_string())
                                }) {
                                    this.banner = Some(
                                        "Wait for this project’s active chats before removing it."
                                            .into(),
                                    );
                                } else {
                                    let _ = this.persist.send(Persist::RemoveProject(id));
                                }
                                this.popup = None;
                                this.focus_composer = true;
                                cx.notify();
                            })),
                    );
            }
        }
        let anchored = matches!(popup, Popup::Models | Popup::Effort);
        let context_menu = matches!(
            popup,
            Popup::Chat(_) | Popup::Assign(_) | Popup::ProjectMenu(_)
        );
        if !anchored && !context_menu {
            content = content.child(
                button("close-popup", "Cancel")
                    .text_color(rgb(MUTED))
                    .on_click(cx.listener(|this, _, window, cx| this.dismiss_popup(window, cx))),
            );
        }
        let mut backdrop = div()
            .id("popup-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .on_click(cx.listener(|this, _, window, cx| this.dismiss_popup(window, cx)));
        if anchored {
            let key = if matches!(popup, Popup::Models) {
                "model-picker"
            } else {
                "effort-picker"
            };
            let viewport = window.viewport_size();
            let bounds = self.picker_bounds.get(key).copied().unwrap_or_default();
            let width = px(350.).min(viewport.width - px(24.));
            let left = bounds
                .left()
                .max(px(12.))
                .min(viewport.width - width - px(12.));
            let top = bounds.top().max(px(120.));
            content = content
                .w(width)
                .max_h((top - px(18.)).min(px(400.)))
                .p_2()
                .rounded_lg();
            backdrop = backdrop.child(
                div()
                    .absolute()
                    .left(left)
                    .bottom((viewport.height - top + px(6.)).max(px(12.)))
                    .track_focus(&self.popup_focus)
                    .child(content),
            );
        } else if context_menu {
            let viewport = window.viewport_size();
            let width = px(250.).min(viewport.width - px(24.));
            let left = self
                .context_anchor
                .x
                .min(viewport.width - width - px(12.))
                .max(px(12.));
            let height = match popup {
                Popup::Assign(_) => (self.projects.len() as f32 * 34. + 90.).min(350.),
                Popup::ProjectMenu(_) => 210.,
                _ => 150.,
            };
            let top = (self.context_anchor.y - px(12.))
                .min(viewport.height - px(height) - px(12.))
                .max(px(12.));
            content = content
                .w(width)
                .max_h((viewport.height - top - px(12.)).min(px(350.)))
                .p_1()
                .rounded_lg()
                .track_focus(&self.popup_focus);
            backdrop = backdrop.child(div().absolute().left(left).top(top).child(content));
        } else {
            content = content.track_focus(&self.popup_focus);
            backdrop = backdrop
                .bg(gpui::rgba(0x00000044))
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                .child(content);
        }
        Some(backdrop.into_any_element())
    }
}
