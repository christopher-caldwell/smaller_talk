use super::*;

fn sidebar_menu_button(
    id: impl Into<gpui::ElementId>,
    hover: u32,
    color: u32,
) -> gpui::Stateful<gpui::Div> {
    compact_button_with_hover(id, "", hover)
        .size(px(24.))
        .flex_shrink_0()
        .px_0()
        .py_0()
        .flex()
        .items_center()
        .justify_center()
        .child(icon("ellipsis").size(px(16.)).text_color(rgb(color)))
}

impl ChatsView {
    pub(super) fn render_sidebar(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.session.sidebar_collapsed {
            return div()
                .w(px(SIDEBAR_RAIL_WIDTH))
                .h_full()
                .flex_shrink_0()
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .py_2()
                .bg(rgb(NAVIGATION))
                .border_r_1()
                .border_color(quiet_border())
                .child(
                    icon_button_with_hover("expand-sidebar", "sidebar", NAV_HOVER)
                        .tooltip(hint("Expand sidebar (⌘B)"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.session.sidebar_collapsed = false;
                            this.save_session();
                            cx.notify();
                        })),
                )
                .child(
                    icon_button_with_hover("rail-new-chat", "compose", NAV_HOVER)
                        .tooltip(hint("New chat (⌘N)"))
                        .on_click(cx.listener(|this, _, _, cx| this.new_chat(cx))),
                )
                .child(
                    icon_button_with_hover("rail-search", "search", NAV_HOVER)
                        .tooltip(hint("Search chats (⌘F)"))
                        .on_click(cx.listener(|this, _, window, cx| this.focus_search(window, cx))),
                )
                .child(
                    icon_button_with_hover("rail-archive", "archive", NAV_HOVER)
                        .tooltip(hint("Archived chats"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_management(ManagementView::Archive(None), window, cx)
                        })),
                )
                .child(div().flex_1())
                .child(
                    icon_button_with_hover("rail-settings", "settings", NAV_HOVER)
                        .bg(rgb(if self.settings_open {
                            CHROME
                        } else {
                            NAVIGATION
                        }))
                        .tooltip(hint("Settings (⌘,)"))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_settings(window, cx)),
                        ),
                )
                .into_any_element();
        }
        let mut sidebar = div()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(rgb(NAVIGATION))
            .border_r_1()
            .border_color(quiet_border())
            .child(
                row()
                    .h(px(42.))
                    .px_3()
                    .child(
                        div()
                            .flex_1()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_size(px(13.))
                            .child("Smaller Talk"),
                    )
                    .child(
                        icon_button_with_hover("collapse-sidebar", "sidebar", NAV_HOVER)
                            .tooltip(hint("Collapse sidebar (⌘B)"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.session.sidebar_collapsed = true;
                                this.save_session();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                row()
                    .px_2()
                    .pb_2()
                    .gap_1()
                    .child(
                        compact_button_with_hover("new-chat", "", NAV_HOVER)
                            .flex_1()
                            .flex()
                            .items_center()
                            .gap_2()
                            .h(px(32.))
                            .child(icon("compose").size(px(15.)).text_color(rgb(MUTED)))
                            .child("New chat")
                            .on_click(cx.listener(|this, _, _, cx| this.new_chat(cx))),
                    )
                    .child(
                        icon_button_with_hover("sort-projects", "sort", NAV_HOVER)
                            .tooltip(hint("Sort projects"))
                            .on_click(cx.listener(|this, event: &gpui::ClickEvent, window, cx| {
                                this.open_context(Popup::ProjectSort, event.position(), window, cx)
                            })),
                    )
                    .child(
                        icon_button_with_hover("filter-projects", "filter", NAV_HOVER)
                            .bg(rgb(if self.session.navigation.hidden_projects.is_empty() {
                                NAVIGATION
                            } else {
                                CHROME
                            }))
                            .tooltip(hint("Filter projects"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_management(ManagementView::Projects, window, cx)
                            })),
                    )
                    .when(!self.session.navigation.hidden_projects.is_empty(), |r| {
                        r.child(
                            icon_button_with_hover("reset-project-filter", "close", NAV_HOVER)
                                .tooltip(hint("Reset project filter · Show all"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.session.navigation.hidden_projects.clear();
                                    this.navigation_changed(cx);
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .mx_2()
                    .mb_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(quiet_border())
                    .bg(gpui::rgba((NAV_HOVER << 8) | 0x66))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(icon("search").size(px(14.)).text_color(rgb(MUTED)))
                    .child(div().flex_1().min_w(px(0.)).child(self.search.clone()))
                    .when(!self.search_query.is_empty(), |row| {
                        row.child(
                            compact_button("clear-search", "×")
                                .px_1()
                                .py_0()
                                .tooltip(hint("Clear search (Escape)"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.clear_search(cx);
                                    this.search.focus_handle(cx).focus(window, cx);
                                })),
                        )
                    }),
            );
        let mut list = div()
            .id("sidebar-list")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_0()
            .px_2();
        if !self.search_query.trim().is_empty() {
            if self.search_loading {
                list = list.child(muted("Searching…"));
            } else if self.search_results.is_empty() {
                list = list.child(div().p_3().child(muted("No matching chats")));
            }
            for hit in &self.search_results {
                let id = hit.chat_id;
                let message = hit.message_id;
                let selected = self.active() == Some(id);
                list = list.child(
                    compact_button_with_hover(
                        element_id("search-hit", id),
                        "",
                        if selected { NAV_SELECTED } else { NAV_HOVER },
                    )
                    .when(selected, |button| {
                        button.bg(rgb(NAV_SELECTED)).text_color(rgb(SELECTED_INK))
                    })
                    .focus(move |style| {
                        style.border_color(rgb(FOCUS)).bg(rgb(if selected {
                            NAV_SELECTED
                        } else {
                            PANEL_HOVER
                        }))
                    })
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_1()
                    .child(
                        div().w_full().text_size(px(13.)).truncate().child(
                            self.chats
                                .iter()
                                .find(|c| c.id == hit.chat_id)
                                .map(|c| self.display_title(c))
                                .unwrap_or_else(|| hit.title.clone()),
                        ),
                    )
                    .when_some(hit.project.clone(), |row, project| {
                        row.child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(if selected { SELECTED_INK } else { ACCENT }))
                                .truncate()
                                .child(project),
                        )
                    })
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(if selected { SELECTED_INK } else { MUTED }))
                            .line_clamp(2)
                            .child(hit.excerpt.clone()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.open_chat(id, message, cx))),
                );
            }
        } else {
            for project in self
                .session
                .navigation
                .sorted_projects(&self.projects, &self.project_activity)
            {
                if self
                    .session
                    .navigation
                    .hidden_projects
                    .contains(&project.id)
                {
                    continue;
                }
                let id = project.id;
                list = list.child(
                    row()
                        .mt_2()
                        .h(px(28.))
                        .pl_2()
                        .child(
                            compact_button_with_hover(
                                element_id("toggle-project", id),
                                "",
                                NAV_HOVER,
                            )
                            .flex_1()
                            .min_w(px(0.))
                            .px_0()
                            .py_0()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child(
                                icon(
                                    if self.session.navigation.collapsed_projects.contains(&id) {
                                        "chevron"
                                    } else {
                                        "chevron-down"
                                    },
                                )
                                .size(px(12.)),
                            )
                            .child(div().truncate().child(project.name))
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if !this.session.navigation.collapsed_projects.remove(&id) {
                                        this.session.navigation.collapsed_projects.insert(id);
                                    }
                                    this.save_session();
                                    cx.notify();
                                },
                            )),
                        )
                        .child(
                            icon_button_with_hover(
                                element_id("new-project-chat", id),
                                "plus",
                                NAV_HOVER,
                            )
                            .size(px(24.))
                            .tooltip(hint("New chat in this project"))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.new_chat_in_project(id, cx)),
                            ),
                        )
                        .child(
                            sidebar_menu_button(element_id("project-menu", id), NAV_HOVER, MUTED)
                                .tooltip(hint("Project actions"))
                                .on_click(cx.listener(
                                    move |this, event: &gpui::ClickEvent, window, cx| {
                                        this.open_context(
                                            Popup::ProjectMenu(id),
                                            event.position(),
                                            window,
                                            cx,
                                        );
                                    },
                                )),
                        ),
                );
                if self.session.navigation.collapsed_projects.contains(&id) {
                    continue;
                }
                let chats: Vec<_> = self
                    .chats
                    .iter()
                    .filter(|c| {
                        c.archived_at.is_none()
                            && self.metadata.get(&c.id).and_then(|p| p.project_id.as_ref())
                                == Some(&id.to_string())
                    })
                    .cloned()
                    .collect();
                let draft_id = self
                    .session
                    .new_chat
                    .as_ref()
                    .filter(|draft| {
                        draft.preferences.project_id.as_deref() == Some(&id.to_string())
                    })
                    .and_then(|draft| Uuid::parse_str(&draft.id).ok());
                if let Some(draft_id) = draft_id {
                    let selected = self.active() == Some(draft_id);
                    list = list.child(
                        compact_button_with_hover(
                            element_id("project-chat-draft", draft_id),
                            "New chat",
                            if selected { NAV_SELECTED } else { NAV_HOVER },
                        )
                        .w_full()
                        .when(selected, |button| {
                            button.bg(rgb(NAV_SELECTED)).text_color(rgb(SELECTED_INK))
                        })
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.open_chat(draft_id, None, cx)),
                        ),
                    );
                } else if chats.is_empty() {
                    list = list.child(
                        compact_button_with_hover(
                            element_id("start-project-chat", id),
                            "",
                            NAV_HOVER,
                        )
                        .w_full()
                        .px_2()
                        .py_1()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.))
                        .text_color(rgb(MUTED))
                        .child(icon("compose").size(px(13.)).text_color(rgb(MUTED)))
                        .child("Start a chat")
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.new_chat_in_project(id, cx)),
                        ),
                    );
                }
                for chat in chats {
                    list = list.child(self.render_chat_row(chat, cx));
                }
            }
            if !self.projects.is_empty() {
                list = list.child(
                    div()
                        .mt_2()
                        .px_2()
                        .py_1()
                        .text_size(px(11.))
                        .text_color(rgb(MUTED))
                        .child("Chats"),
                );
            }
            let unassigned: Vec<_> = self
                .chats
                .iter()
                .filter(|c| {
                    c.archived_at.is_none()
                        && self
                            .metadata
                            .get(&c.id)
                            .and_then(|p| p.project_id.as_ref())
                            .is_none()
                })
                .cloned()
                .collect();
            for chat in unassigned {
                list = list.child(self.render_chat_row(chat, cx));
            }
            if self.chats.iter().all(|c| c.archived_at.is_some()) {
                list = list.child(div().p_3().child(muted("Your chats will appear here.")));
            }
        }
        sidebar = sidebar.child(list).child(
            row()
                .m_2()
                .pt_1()
                .border_t_1()
                .border_color(quiet_border())
                .child(
                    compact_button_with_hover("new-project", "", NAV_HOVER)
                        .flex_1()
                        .flex()
                        .items_center()
                        .gap_2()
                        .h(px(30.))
                        .text_color(rgb(MUTED))
                        .child(icon("plus").size(px(14.)).text_color(rgb(MUTED)))
                        .child("New project")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.begin_project(None, window, cx)),
                        ),
                )
                .child(
                    icon_button_with_hover("archive-browser", "archive", NAV_HOVER)
                        .tooltip(hint("Archived chats"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_management(ManagementView::Archive(None), window, cx)
                        })),
                )
                .child(
                    icon_button_with_hover("settings", "settings", NAV_HOVER)
                        .bg(rgb(if self.settings_open {
                            CHROME
                        } else {
                            NAVIGATION
                        }))
                        .tooltip(hint("Settings (⌘,)"))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_settings(window, cx)),
                        ),
                ),
        );
        sidebar.into_any_element()
    }

    fn render_chat_row(&mut self, chat: Chat, cx: &mut Context<Self>) -> gpui::AnyElement {
        let id = chat.id;
        let title = self.display_title(&chat);
        let selected = self.active() == Some(id);
        let state = self.states.get(&id);
        let status = state
            .and_then(|s| {
                s.activity
                    .filter(|a| s.pending.is_none() || *a == Activity::Waiting)
                    .map(|a| a.label().to_string())
            })
            .or_else(|| state.and_then(|s| s.error.as_ref().map(|_| "Needs attention".into())));
        let status_color = if selected {
            SELECTED_INK
        } else if state.is_some_and(|s| s.error.is_some() && s.activity.is_none()) {
            DANGER
        } else {
            ACCENT
        };
        let indicator = state
            .and_then(|s| {
                s.activity
                    .filter(|a| s.pending.is_none() || *a == Activity::Waiting)
            })
            .map(|a| self.render_activity(id, a, status_color, true, cx));
        row()
            .id(element_id("chat-row", id))
            .w_full()
            .gap_0()
            .rounded_md()
            .bg(rgb(if selected { NAV_SELECTED } else { NAVIGATION }))
            .text_color(rgb(if selected { SELECTED_INK } else { TEXT }))
            .hover(move |s| s.bg(rgb(if selected { NAV_SELECTED } else { NAV_HOVER })))
            .child(
                div()
                    .id(element_id("open-chat", id))
                    .flex_1()
                    .min_w(px(0.))
                    .min_h(px(32.))
                    .pl_2()
                    .pr_1()
                    .py_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .tab_index(0)
                    .rounded_md()
                    .border_1()
                    .border_color(gpui::rgba(0))
                    .focus(|s| s.border_color(rgb(FOCUS)))
                    .tooltip(hint(title.clone()))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .child(div().text_size(px(13.)).truncate().child(title))
                            .when_some(status, |view, status| {
                                view.child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(rgb(status_color))
                                        .truncate()
                                        .child(
                                            indicator
                                                .map(IntoElement::into_any_element)
                                                .unwrap_or_else(|| {
                                                    div().child(status).into_any_element()
                                                }),
                                        ),
                                )
                            }),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_chat(id, None, cx);
                        if let Some(state) = this.states.get(&id) {
                            state.composer.focus_handle(cx).focus(window, cx);
                        }
                    })),
            )
            .child(
                sidebar_menu_button(
                    element_id("chat-actions", id),
                    if selected { NAV_SELECTED } else { CHROME },
                    if selected { SELECTED_INK } else { MUTED },
                )
                .when(self.popup.is_none(), |b| b.tooltip(hint("Chat actions")))
                .on_click(cx.listener(
                    move |this, event: &gpui::ClickEvent, window, cx| {
                        this.open_context(Popup::Chat(id), event.position(), window, cx);
                    },
                )),
            )
            .into_any_element()
    }

    pub(super) fn render_tabs(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let mut strip = row()
            .id("tab-overflow")
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .overflow_x_scroll()
            .track_scroll(&self.tabs_scroll)
            .gap_1()
            .px_2();
        for id in self
            .session
            .open_tabs
            .iter()
            .filter_map(|id| Uuid::parse_str(id).ok())
        {
            let title = match self.chats.iter().find(|c| c.id == id) {
                Some(chat) => self.display_title(chat),
                None if self.is_new_chat(id) => "New chat".into(),
                None => continue,
            };
            let project = self
                .metadata
                .get(&id)
                .and_then(|preferences| preferences.project_id.as_deref())
                .and_then(|project_id| {
                    self.projects
                        .iter()
                        .find(|project| project.id.to_string() == project_id)
                });
            let mut tooltip = match project {
                Some(project) => format!("{} / {title}", project.name),
                None => title.clone(),
            };
            let active = self.active() == Some(id);
            let state = self.states.get(&id);
            let waiting =
                state.is_some_and(|s| s.pending.is_some() || s.activity == Some(Activity::Waiting));
            let working = waiting || state.is_some_and(|s| s.activity.is_some());
            if waiting {
                tooltip.push_str("\nWaiting for your input");
            }
            strip = strip.child(
                row()
                    .id(element_id("tab", id))
                    .h(px(28.))
                    .min_w(px(96.))
                    .max_w(px(if project.is_some() { 448. } else { 296. }))
                    .flex_shrink_0()
                    .gap_0()
                    .rounded_md()
                    .bg(rgb(if active { PANEL } else { BACKGROUND }))
                    .hover(|s| s.bg(rgb(PANEL)))
                    .child(
                        div()
                            .id(element_id("focus-tab", id))
                            .min_w(px(0.))
                            .h_full()
                            .px_2()
                            .flex()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .tab_index(0)
                            .rounded_md()
                            .border_1()
                            .border_color(gpui::rgba(0))
                            .focus(|s| s.border_color(rgb(FOCUS)))
                            .tooltip(hint(tooltip))
                            .when(working, |view| {
                                view.child(
                                    div()
                                        .size(px(5.))
                                        .rounded_full()
                                        .flex_shrink_0()
                                        .bg(rgb(if waiting { ATTENTION } else { ACCENT })),
                                )
                            })
                            .child(
                                row()
                                    .gap_1()
                                    .text_size(px(12.))
                                    .when_some(project, |label, project| {
                                        label
                                            .child(
                                                div()
                                                    .max_w(px(144.))
                                                    .flex_shrink_0()
                                                    .text_color(rgb(MUTED))
                                                    .opacity(0.75)
                                                    .truncate()
                                                    .child(project.name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .flex_shrink_0()
                                                    .text_color(rgb(MUTED))
                                                    .opacity(0.5)
                                                    .child("/"),
                                            )
                                    })
                                    .child(
                                        div()
                                            .min_w(px(0.))
                                            .max_w(px(240.))
                                            .text_color(rgb(if waiting {
                                                ATTENTION
                                            } else if active {
                                                TEXT
                                            } else {
                                                MUTED
                                            }))
                                            .truncate()
                                            .child(title),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_chat(id, None, cx);
                                if let Some(s) = this.states.get(&id) {
                                    s.composer.focus_handle(cx).focus(window, cx);
                                }
                            })),
                    )
                    .child(
                        icon_button(element_id("close-tab", id), "close")
                            .size(px(22.))
                            .mr_1()
                            .tooltip(hint(if self.is_new_chat(id) {
                                "Close tab (⌘W); keep this draft"
                            } else {
                                "Close tab (⌘W); keep this chat"
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| this.close_tab(id, cx))),
                    ),
            );
        }
        row()
            .id("chat-tabs")
            .h(px(38.))
            .flex_shrink_0()
            .w_full()
            .gap_1()
            .border_b_1()
            .border_color(quiet_border())
            .bg(rgb(BACKGROUND))
            .child(strip)
            .child(
                icon_button("tab-new", "plus")
                    .mr_2()
                    .tooltip(hint("New chat (⌘N)"))
                    .on_click(cx.listener(|this, _, _, cx| this.new_chat(cx))),
            )
            .into_any_element()
    }
}
