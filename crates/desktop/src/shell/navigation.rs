use super::*;

impl ChatsView {
    pub(super) fn render_sidebar(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.session.sidebar_collapsed {
            return div()
                .w(px(52.))
                .h_full()
                .flex_shrink_0()
                .flex()
                .flex_col()
                .items_center()
                .gap_2()
                .py_3()
                .bg(rgb(CHROME))
                .border_r_1()
                .border_color(rgb(BORDER))
                .child(
                    button("expand-sidebar", "☰")
                        .tooltip(hint("Expand sidebar (⌘B)"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.session.sidebar_collapsed = false;
                            this.save_session();
                            cx.notify();
                        })),
                )
                .child(
                    button("rail-new-chat", "+")
                        .tooltip(hint("New chat (⌘N)"))
                        .on_click(cx.listener(|this, _, _, _| {
                            let _ = this
                                .persist
                                .send(Persist::CreateChat(this.new_chat_settings()));
                        })),
                )
                .child(
                    button("rail-search", "⌕")
                        .tooltip(hint("Search chats (⌘F)"))
                        .on_click(cx.listener(|this, _, window, cx| this.focus_search(window, cx))),
                )
                .child(
                    button("rail-settings", "⚙")
                        .tooltip(hint("Settings (⌘,)"))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_settings(window, cx)),
                        ),
                )
                .into_any_element();
        }
        let mut sidebar = div()
            .w(px(260.))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(rgb(NAVIGATION))
            .border_r_1()
            .border_color(rgb(BORDER))
            .child(
                row()
                    .h(px(52.))
                    .px_3()
                    .child(
                        div()
                            .flex_1()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_sm()
                            .child("Smaller Talk"),
                    )
                    .child(
                        button("settings", "⚙")
                            .tooltip(hint("Settings (⌘,)"))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_settings(window, cx)),
                            ),
                    )
                    .child(
                        button("collapse-sidebar", "☰")
                            .tooltip(hint("Collapse sidebar (⌘B)"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.session.sidebar_collapsed = true;
                                this.save_session();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div().px_3().pb_3().child(
                    button_with_hover("new-chat", "", NAV_HOVER)
                        .w_full()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(icon("compose"))
                        .child("New chat")
                        .on_click(cx.listener(|this, _, _, _| {
                            let _ = this
                                .persist
                                .send(Persist::CreateChat(this.new_chat_settings()));
                        })),
                ),
            )
            .child(
                div()
                    .mx_3()
                    .mb_3()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(BORDER))
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(div().flex_1().min_w(px(0.)).child(self.search.clone()))
                    .when(!self.search_query.is_empty(), |row| {
                        row.child(
                            button("clear-search", "×")
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
            .gap_1()
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
                list = list.child(
                    button(element_id("search-hit", id), "")
                        .w_full()
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap_1()
                        .child(
                            div().text_sm().truncate().child(
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
                                    .text_xs()
                                    .text_color(rgb(ACCENT))
                                    .truncate()
                                    .child(project),
                            )
                        })
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(MUTED))
                                .line_clamp(2)
                                .child(hit.excerpt.clone()),
                        )
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.open_chat(id, message, cx)),
                        ),
                );
            }
        } else {
            for project in self.projects.clone() {
                let id = project.id;
                list = list.child(
                    row()
                        .mt_3()
                        .px_2()
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(rgb(MUTED))
                                .truncate()
                                .child(project.name),
                        )
                        .child(
                            button(element_id("project-menu", id), "···")
                                .py_1()
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
                    list = list.child(div().px_3().py_2().child(muted("No chats")));
                }
                for chat in chats {
                    list = list.child(self.render_chat_row(chat, cx));
                }
            }
            if !self.projects.is_empty() {
                list = list.child(
                    div()
                        .mt_3()
                        .px_3()
                        .py_2()
                        .text_xs()
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
        sidebar =
            sidebar
                .child(list)
                .child(div().p_2().border_t_1().border_color(rgb(BORDER)).child(
                    button("new-project", "+  New project").w_full().on_click(
                        cx.listener(|this, _, window, cx| this.begin_project(None, window, cx)),
                    ),
                ));
        sidebar.into_any_element()
    }

    fn render_chat_row(&self, chat: Chat, cx: &mut Context<Self>) -> gpui::AnyElement {
        let id = chat.id;
        let title = self.display_title(&chat);
        let state = self.states.get(&id);
        let status = state
            .and_then(|s| s.activity.map(|a| a.label().to_string()))
            .or_else(|| state.and_then(|s| s.error.as_ref().map(|_| "Needs attention".into())));
        row()
            .id(element_id("chat-row", id))
            .w_full()
            .rounded_md()
            .bg(rgb(if self.active == Some(id) {
                NAV_SELECTED
            } else {
                NAVIGATION
            }))
            .text_color(rgb(if self.active == Some(id) {
                SELECTED_INK
            } else {
                TEXT
            }))
            .hover(|s| {
                s.bg(rgb(if self.active == Some(id) {
                    NAV_SELECTED
                } else {
                    NAV_HOVER
                }))
            })
            .child(
                div()
                    .id(element_id("open-chat", id))
                    .flex_1()
                    .min_w(px(0.))
                    .px_3()
                    .py_2()
                    .cursor_pointer()
                    .tab_index(0)
                    .focus(|s| s.border_color(rgb(FOCUS)))
                    .border_1()
                    .border_color(gpui::rgba(0))
                    .tooltip(hint(title.clone()))
                    .child(div().text_sm().truncate().child(title))
                    .when_some(status, |row, status| {
                        row.child(
                            div()
                                .mt_1()
                                .text_xs()
                                .text_color(rgb(if self.active == Some(id) {
                                    SELECTED_INK
                                } else {
                                    ACCENT
                                }))
                                .truncate()
                                .child(status),
                        )
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_chat(id, None, cx);
                        if let Some(state) = this.states.get(&id) {
                            state.composer.focus_handle(cx).focus(window);
                        }
                    })),
            )
            .child(
                button(element_id("chat-actions", id), "···")
                    .when(self.popup.is_none(), |b| b.tooltip(hint("Chat actions")))
                    .px_2()
                    .text_color(rgb(if self.active == Some(id) {
                        SELECTED_INK
                    } else {
                        MUTED
                    }))
                    .on_click(
                        cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                            this.open_context(Popup::Chat(id), event.position(), window, cx);
                        }),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_tabs(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let mut tabs = row()
            .id("chat-tabs")
            .h(px(48.))
            .flex_shrink_0()
            .w_full()
            .border_b_1()
            .border_color(rgb(BORDER))
            .bg(rgb(PANEL));
        let mut strip = row()
            .id("tab-overflow")
            .flex_1()
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
            let Some(chat) = self.chats.iter().find(|c| c.id == id) else {
                continue;
            };
            let active = self.active == Some(id);
            let working = self.states.get(&id).is_some_and(|s| s.activity.is_some());
            strip = strip.child(
                row()
                    .id(element_id("tab", id))
                    .h(px(34.))
                    .w(px(174.))
                    .flex_shrink_0()
                    .rounded_md()
                    .px_2()
                    .bg(rgb(if active { PANEL_HOVER } else { PANEL }))
                    .hover(|s| s.bg(rgb(PANEL_HOVER)))
                    .child(
                        div()
                            .id(element_id("focus-tab", id))
                            .min_w(px(0.))
                            .flex_1()
                            .text_sm()
                            .truncate()
                            .cursor_pointer()
                            .tab_index(0)
                            .focus(|s| s.bg(rgb(PANEL_HOVER)))
                            .tooltip(hint(self.display_title(chat)))
                            .child(format!(
                                "{}{}",
                                if working { "◌  " } else { "" },
                                self.display_title(chat)
                            ))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_chat(id, None, cx);
                                if let Some(s) = this.states.get(&id) {
                                    s.composer.focus_handle(cx).focus(window);
                                }
                            })),
                    )
                    .child(
                        button(element_id("close-tab", id), "×")
                            .tooltip(hint("Close tab (⌘W); keep this chat"))
                            .px_1()
                            .py_1()
                            .text_color(rgb(MUTED))
                            .on_click(cx.listener(move |this, _, _, cx| this.close_tab(id, cx))),
                    ),
            );
        }
        tabs = tabs.child(strip).child(
            button("tab-new", "+")
                .tooltip(hint("New chat (⌘N)"))
                .mr_2()
                .on_click(cx.listener(|this, _, _, _| {
                    let _ = this
                        .persist
                        .send(Persist::CreateChat(this.new_chat_settings()));
                })),
        );
        tabs.into_any_element()
    }
}
