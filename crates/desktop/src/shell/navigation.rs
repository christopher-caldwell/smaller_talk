use super::*;

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
                div().px_2().pb_2().child(
                    compact_button_with_hover("new-chat", "", NAV_HOVER)
                        .w_full()
                        .flex()
                        .items_center()
                        .gap_2()
                        .h(px(32.))
                        .child(icon("compose").size(px(15.)).text_color(rgb(MUTED)))
                        .child("New chat")
                        .on_click(cx.listener(|this, _, _, cx| this.new_chat(cx))),
                ),
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
                                    this.search.focus_handle(cx).focus(window);
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
                let selected = self.active == Some(id);
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
            for project in self.projects.clone() {
                let id = project.id;
                list = list.child(
                    row()
                        .mt_2()
                        .h(px(28.))
                        .px_2()
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(11.))
                                .text_color(rgb(MUTED))
                                .truncate()
                                .child(project.name),
                        )
                        .child(
                            compact_button_with_hover(
                                element_id("project-menu", id),
                                "···",
                                NAV_HOVER,
                            )
                            .px_1()
                            .py_0()
                            .text_color(rgb(MUTED))
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
                let chats: Vec<_> = self
                    .chats
                    .iter()
                    .filter(|c| {
                        self.metadata.get(&c.id).and_then(|p| p.project_id.as_ref())
                            == Some(&id.to_string())
                    })
                    .cloned()
                    .collect();
                if chats.is_empty() {
                    list = list.child(
                        div()
                            .px_2()
                            .py_1()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .child("No chats"),
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
                    self.metadata
                        .get(&c.id)
                        .and_then(|p| p.project_id.as_ref())
                        .is_none()
                })
                .cloned()
                .collect();
            for chat in unassigned {
                list = list.child(self.render_chat_row(chat, cx));
            }
            if self.chats.is_empty() {
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

    fn render_chat_row(&self, chat: Chat, cx: &mut Context<Self>) -> gpui::AnyElement {
        let id = chat.id;
        let title = self.display_title(&chat);
        let selected = self.active == Some(id);
        let state = self.states.get(&id);
        let status = state
            .and_then(|s| {
                s.activity
                    .filter(|_| s.pending.is_none())
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
                                        .child(status),
                                )
                            }),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_chat(id, None, cx);
                        if let Some(state) = this.states.get(&id) {
                            state.composer.focus_handle(cx).focus(window);
                        }
                    })),
            )
            .child(
                compact_button_with_hover(
                    element_id("chat-actions", id),
                    "···",
                    if selected { NAV_SELECTED } else { CHROME },
                )
                .when(self.popup.is_none(), |b| b.tooltip(hint("Chat actions")))
                .w(px(26.))
                .h(px(26.))
                .px_0()
                .py_0()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(if selected { SELECTED_INK } else { MUTED }))
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
            let active = self.active == Some(id);
            let working = self.states.get(&id).is_some_and(|s| s.activity.is_some());
            strip = strip.child(
                row()
                    .id(element_id("tab", id))
                    .h(px(28.))
                    .min_w(px(96.))
                    .max_w(px(216.))
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
                            .tooltip(hint(title.clone()))
                            .when(working, |view| {
                                view.child(
                                    div()
                                        .size(px(5.))
                                        .rounded_full()
                                        .flex_shrink_0()
                                        .bg(rgb(ACCENT)),
                                )
                            })
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .max_w(px(160.))
                                    .text_size(px(12.))
                                    .text_color(rgb(if active { TEXT } else { MUTED }))
                                    .truncate()
                                    .child(title),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_chat(id, None, cx);
                                if let Some(s) = this.states.get(&id) {
                                    s.composer.focus_handle(cx).focus(window);
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
