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
            Popup::Attachments => {
                if let Some(id) = self.active() {
                    content = content
                        .child(
                            compact_button("attach-menu-files", "Attach files…").on_click(
                                cx.listener(move |this, _, window, cx| {
                                    this.dismiss_popup(window, cx);
                                    this.pick_files(id, cx);
                                }),
                            ),
                        )
                        .child(compact_button("attach-menu-skills", "Skills…").on_click(
                            cx.listener(|this, _, window, cx| this.open_skills(window, cx)),
                        ));
                }
            }
            Popup::ProjectSort => {
                for (sort, label) in [
                    (ProjectSort::Alphabetical, "Alphabetical"),
                    (ProjectSort::LastChatUpdated, "Last chat updated"),
                ] {
                    content = content.child(
                        button(
                            element_id("project-sort", label),
                            format!(
                                "{}  {label}",
                                if self.session.navigation.project_sort == sort {
                                    "✓"
                                } else {
                                    " "
                                }
                            ),
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.session.navigation.project_sort = sort;
                                this.navigation_changed(cx);
                                this.dismiss_popup(window, cx);
                            },
                        )),
                    );
                }
            }
            Popup::Commands => {
                content = content.child(muted("Commands · Skills use $name"));
                if "compact".starts_with(&self.command_query) {
                    let reason = self.active().and_then(|id| self.compaction_unavailable(id));
                    content = content.child(
                        button(
                            "compact-command",
                            "/compact  ·  Reduce conversation context",
                        )
                        .tab_stop(reason.is_none())
                        .text_color(rgb(if reason.is_some() { MUTED } else { TEXT }))
                        .when(reason.is_none(), |b| {
                            b.on_click(
                                cx.listener(|this, _, window, cx| this.accept_command(window, cx)),
                            )
                        }),
                    );
                    if let Some(reason) = reason {
                        content = content.child(muted(reason));
                    }
                } else {
                    content = content.child(muted("Unsupported command. Available: /compact"));
                }
            }
            Popup::Skills => {
                content = content.child(self.render_skills(cx));
            }
            Popup::SkillDetails => {
                content = content.child(
                    div()
                        .text_sm()
                        .child(self.skill_ui.preview.clone().unwrap_or_default()),
                );
            }

            Popup::Models | Popup::Effort | Popup::Permissions | Popup::Settings(_) => {
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
                            .child(
                                icon_button("clear-model-search", "close")
                                    .size(px(22.))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.model_search.update(cx, |e, cx| {
                                            e.set_content(String::new(), window, cx)
                                        });
                                        this.model_query.clear();
                                        this.popup_index = 0;
                                        this.model_search.focus_handle(cx).focus(window, cx);
                                        cx.notify();
                                    })),
                            ),
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
                    let description = match &action {
                        PickerAction::Permission(mode)
                        | PickerAction::SettingsPermission(_, mode) => Some(mode.description()),
                        _ => None,
                    };
                    items = items.child(
                        compact_button(element_id("picker-choice", index), "")
                            .w_full()
                            .py_1()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().w(px(12.)).flex_shrink_0().child(if choice.selected {
                                "✓"
                            } else {
                                ""
                            }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .child(div().truncate().child(choice.label))
                                    .when_some(description, |item, description| {
                                        item.child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(rgb(MUTED))
                                                .child(description),
                                        )
                                    }),
                            )
                            .bg(rgb(if index == self.popup_index {
                                PANEL_HOVER
                            } else {
                                MENU
                            }))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.choose_picker(action.clone(), window, cx)
                            })),
                    );
                }
                if no_choices && matches!(popup, Popup::Settings(_)) {
                    items = items.child(div().p_2().child(muted(
                        "No available models. Check provider status and model visibility.",
                    )));
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
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child(match popup {
                                Popup::Models => "Select model".to_owned(),
                                Popup::Settings(picker) => picker.title().to_owned(),
                                Popup::Permissions => format!(
                                    "{} permissions · This conversation",
                                    self.active()
                                        .and_then(|id| self.states.get(&id))
                                        .map(|state| state.preferences.execution.provider.label())
                                        .unwrap_or("Provider")
                                ),
                                _ => "Reasoning effort".to_owned(),
                            }),
                    )
                    .child(items);
                if matches!(popup, Popup::Models) {
                    content = content.child(
                        compact_button("manage-models", "Manage models…")
                            .text_xs()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_settings(window, cx)),
                            ),
                    );
                }
            }
            Popup::Chat(id) => {
                let busy = self.states.get(&id).is_some_and(|s| s.activity.is_some());
                let eligible = self.archive_eligible(id);
                content =
                    content.child(
                        menu_item("archive-chat", "archive", "Archive chat")
                            .tab_stop(eligible)
                            .text_color(rgb(if eligible { TEXT } else { MUTED }))
                            .tooltip(hint(if eligible {
                                "Archive locally; preserve the provider session"
                            } else {
                                "Wait until the chat is idle and its queue is empty"
                            }))
                            .when(eligible, |b| {
                                b.on_click(cx.listener(move |this, _, _, cx| {
                                    this.change_archive(id, true, cx)
                                }))
                            }),
                    );
                content = content.child(menu_item("rename-chat", "pencil", "Rename").on_click(
                    cx.listener(move |this, _, window, cx| {
                        let title = this
                            .chats
                            .iter()
                            .find(|c| c.id == id)
                            .map(|c| c.title.clone())
                            .unwrap_or_default();
                        this.name_editor.update(cx, |s, cx| {
                            s.set_placeholder("Chat name");
                            s.set_content(title, window, cx);
                        });
                        this.popup_return_focus = window.focused(cx);
                        this.name_editor.focus_handle(cx).focus(window, cx);
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
                let can_save = !self.name_editor.read(cx).content().trim().is_empty();
                content = content
                    .w(px(520.).min(window.viewport_size().width - px(32.)))
                    .max_h(window.viewport_size().height - px(32.))
                    .p_6()
                    .gap_0()
                    .child(
                        row()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(24.))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child(if id.is_some() {
                                        "Edit project"
                                    } else {
                                        "Create a project"
                                    }),
                            )
                            .child(
                                icon_button("close-project", "close")
                                    .tooltip(hint("Close (Escape)"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.dismiss_popup(window, cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .mt_2()
                            .child(muted("Keep related chats together in one place.")),
                    )
                    .child(
                        div()
                            .mt_6()
                            .mb_2()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Name"),
                    )
                    .child(
                        div()
                            .track_focus(&self.name_editor.focus_handle(cx))
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .border_2()
                            .border_color(rgb(BORDER))
                            .bg(rgb(BACKGROUND))
                            .focus(|field| field.border_color(rgb(NAV_SELECTED)))
                            .child(self.name_editor.clone()),
                    )
                    .child(
                        row()
                            .mt_5()
                            .mb_2()
                            .text_sm()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("Working folder"),
                            )
                            .child(muted("Optional")),
                    )
                    .child(
                        row()
                            .gap_2()
                            .child(
                                div()
                                    .track_focus(&self.folder_editor.focus_handle(cx))
                                    .flex_1()
                                    .min_w(px(0.))
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .border_2()
                                    .border_color(rgb(BORDER))
                                    .bg(rgb(BACKGROUND))
                                    .focus(|field| field.border_color(rgb(NAV_SELECTED)))
                                    .child(self.folder_editor.clone()),
                            )
                            .child(
                                button("choose-project-folder", "Browse…")
                                    .flex_shrink_0()
                                    .h(px(44.))
                                    .border_color(rgb(BORDER))
                                    .on_click(cx.listener(|_, _, _, cx| {
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
                                                            skills: Vec::new(),
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
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .mt_2()
                            .child(muted("Leave empty to use the app’s working folder.")),
                    )
                    .child(
                        row()
                            .mt_7()
                            .justify_end()
                            .child(button("cancel-project", "Cancel").on_click(
                                cx.listener(|this, _, window, cx| this.dismiss_popup(window, cx)),
                            ))
                            .child(
                                button_with_hover(
                                    "save-project",
                                    if id.is_some() {
                                        "Save changes"
                                    } else {
                                        "Create project"
                                    },
                                    if can_save { ACCENT } else { PANEL_HOVER },
                                )
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .bg(rgb(if can_save { NAV_SELECTED } else { PANEL_HOVER }))
                                .text_color(rgb(if can_save { SELECTED_INK } else { MUTED }))
                                .when(can_save, |button| {
                                    button
                                        .focus(|style| {
                                            style.bg(rgb(NAV_SELECTED)).border_color(rgb(TEXT))
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.save_project(id, cx)
                                        }))
                                })
                                .when(!can_save, |button| button.tab_stop(false).cursor_default()),
                            ),
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
                content =
                    content
                        .child(
                            menu_item("new-chat-in-project", "compose", "New chat").on_click(
                                cx.listener(move |this, _, _, cx| this.new_chat_in_project(id, cx)),
                            ),
                        )
                        .child(
                            button("edit-project", "Edit project…").on_click(cx.listener(
                                move |this, _, window, cx| this.begin_project(Some(id), window, cx),
                            )),
                        )
                        .child(button("project-instructions", "Instructions…").on_click(
                            cx.listener(move |this, _, window, cx| {
                                this.open_instruction_scope(
                                    super::instructions::InstructionScope::Project(id),
                                    window,
                                    cx,
                                );
                            }),
                        ))
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
            Popup::Panes => {
                content = content.child(self.render_pane_menu(cx));
            }
        }
        let anchored = matches!(
            popup,
            Popup::Models
                | Popup::Effort
                | Popup::Permissions
                | Popup::Commands
                | Popup::Skills
                | Popup::Attachments
                | Popup::Settings(_)
        );
        let context_menu = matches!(
            popup,
            Popup::Chat(_)
                | Popup::Assign(_)
                | Popup::ProjectMenu(_)
                | Popup::Panes
                | Popup::ProjectSort
        );
        if !anchored && !context_menu && !matches!(popup, Popup::Project(_)) {
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
        if let Popup::Settings(picker) = popup {
            let viewport = window.viewport_size();
            let bounds = self
                .picker_bounds
                .get(&(None, picker.anchor()))
                .copied()
                .unwrap_or_default();
            let width = px(300.).min(viewport.width - px(24.));
            let left = (bounds.right() - width)
                .max(px(12.))
                .min(viewport.width - width - px(12.));
            let below = viewport.height - bounds.bottom() - px(12.);
            let above = bounds.top() - px(12.);
            let upward = below < px(220.) && above > below;
            content = content
                .w(width)
                .max_h(if upward { above } else { below }.min(px(320.)))
                .p_1()
                .rounded_lg();
            let mut anchor = div().absolute().left(left).track_focus(&self.popup_focus);
            anchor = if upward {
                anchor.bottom(viewport.height - bounds.top() + px(4.))
            } else {
                anchor.top(bounds.bottom() + px(4.))
            };
            backdrop = backdrop.child(anchor.child(content));
        } else if anchored {
            let key = if matches!(popup, Popup::Skills | Popup::Commands | Popup::Attachments) {
                "skills-picker"
            } else if matches!(popup, Popup::Models) {
                "model-picker"
            } else if matches!(popup, Popup::Permissions) {
                "permissions-picker"
            } else {
                "effort-picker"
            };
            let viewport = window.viewport_size();
            let bounds = self
                .picker_bounds
                .get(&(self.active(), key))
                .copied()
                .unwrap_or_default();
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
                Popup::ProjectMenu(_) => 245.,
                Popup::Panes if self.panes.is_split() => 170.,
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
