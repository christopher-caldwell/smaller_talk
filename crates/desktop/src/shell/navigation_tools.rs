use super::*;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum ManagementView {
    Projects,
    Archive(Option<Uuid>),
}

impl ChatsView {
    pub(super) fn refresh_project_activity(&mut self) {
        self.activity_generation += 1;
        let generation = self.activity_generation;
        let service = self.services.workspace.clone();
        let sender = self.sender.clone();
        self.services.runtime.spawn(async move {
            let result = service.project_activity().await.map_err(|e| e.to_string());
            let _ = sender.send(UiEvent::ProjectActivity(generation, result));
        });
    }
    pub(super) fn navigation_changed(&mut self, cx: &mut Context<Self>) {
        self.save_session();
        self.search_changed(cx);
        cx.notify();
    }
    pub(super) fn open_management(
        &mut self,
        view: ManagementView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.popup = None;
        self.settings_open = false;
        self.management = Some(view);
        self.management_query.clear();
        self.management_search
            .update(cx, |e, cx| e.set_content(String::new(), window, cx));
        self.management_search.focus_handle(cx).focus(window, cx);
        if window.viewport_size().width < px(720.) {
            self.session.sidebar_collapsed = true;
            self.save_session();
        }
        cx.notify();
    }
    pub(super) fn management_search_changed(&mut self, cx: &mut Context<Self>) {
        self.management_query = self.management_search.read(cx).content();
        self.management_generation += 1;
        self.archive_hits = None;
        if matches!(self.management, Some(ManagementView::Archive(None)))
            && !self.management_query.trim().is_empty()
        {
            let generation = self.management_generation;
            let input = SearchChatsInput {
                text: self.management_query.clone(),
                archived: true,
                ..Default::default()
            };
            let service = self.services.workspace.clone();
            let sender = self.sender.clone();
            self.services.runtime.spawn(async move {
                let result = service
                    .search_scoped(&input)
                    .await
                    .map_err(|e| e.to_string());
                let _ = sender.send(UiEvent::ArchiveSearch(generation, result));
            });
        }
        cx.notify();
    }
    pub(super) fn archive_eligible(&self, id: Uuid) -> bool {
        !self.navigation_pending.contains(&id)
            && !self.command_pending.contains(&id)
            && self.states.get(&id).is_none_or(|s| {
                s.activity.is_none()
                    && !s.enqueue_pending
                    && !s.queue_edit_pending
                    && !s.settings_pending
                    && !s.importing
            })
            && self.queues.get(&id).is_some_and(|q| q.entries.is_empty())
    }
    pub(super) fn change_archive(&mut self, id: Uuid, archived: bool, cx: &mut Context<Self>) {
        if self.navigation_pending.contains(&id) || (archived && !self.archive_eligible(id)) {
            return;
        }
        self.navigation_pending.insert(id);
        self.popup = None;
        self.draft_changed(id, cx);
        let queue = self.services.queue.clone();
        let sender = self.sender.clone();
        // Persist pending draft edits before archive acknowledgement closes the tab.
        let (tx, rx) = oneshot::channel();
        let _ = self.persist.send(Persist::Barrier(tx));
        self.services.runtime.spawn(async move {
            let _ = rx.await;
            let result = queue
                .set_archived(id, archived)
                .await
                .map_err(|e| e.to_string());
            let _ = sender.send(UiEvent::ArchiveChanged(id, result));
        });
        cx.notify();
    }
    pub(super) fn render_management(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let view = self.management.expect("management view");
        let title = if view == ManagementView::Projects {
            "Project visibility"
        } else {
            "Archived chats"
        };
        let mut content = div()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .flex()
            .flex_col()
            .p_4()
            .gap_3();
        content = content.child(
            row()
                .child(
                    button("management-back", "‹  Back to chat").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.management = None;
                            this.focus_composer = true;
                            cx.notify();
                        },
                    )),
                )
                .child(div().flex_1())
                .child(div().text_lg().child(title)),
        );
        if let ManagementView::Archive(Some(id)) = view {
            content =
                content.child(
                    row()
                        .child(button("archive-list-back", "‹  Archived chats").on_click(
                            cx.listener(|this, _, _, cx| {
                                this.management = Some(ManagementView::Archive(None));
                                cx.notify();
                            }),
                        ))
                        .child(div().flex_1())
                        .child(muted("Archived · Read only"))
                        .child(button("restore-chat", "Restore").when(
                            !self.navigation_pending.contains(&id),
                            |b| {
                                b.on_click(cx.listener(move |this, _, _, cx| {
                                    this.change_archive(id, false, cx)
                                }))
                            },
                        )),
                );
            if let Some(chat) = self.chats.iter().find(|c| c.id == id) {
                content = content.child(div().text_lg().child(self.display_title(chat)));
            }
            if let Some(state) = self.states.get_mut(&id) {
                state.width = window.viewport_size().width
                    - px(if self.session.sidebar_collapsed {
                        SIDEBAR_RAIL_WIDTH
                    } else {
                        SIDEBAR_WIDTH
                    })
                    - px(32.);
                content = content.child(self.render_transcript(id, window, cx));
            }
            return content;
        }
        content = content.child(
            div()
                .p_2()
                .rounded_md()
                .bg(rgb(COMPOSER))
                .child(self.management_search.clone()),
        );
        let mut list = div()
            .id("management-list")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2();
        if view == ManagementView::Projects {
            content = content
                .child(
                    row()
                        .child(
                            button("projects-show-all", "Show all").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.session.navigation.hidden_projects.clear();
                                    this.navigation_changed(cx);
                                },
                            )),
                        )
                        .child(
                            button("projects-hide-all", "Hide all").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.session.navigation.hidden_projects =
                                        this.projects.iter().map(|p| p.id).collect();
                                    this.navigation_changed(cx);
                                },
                            )),
                        ),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(MUTED))
                        .child("Controls sidebar and search visibility"),
                );
            let query = self.management_query.to_lowercase();
            let mut count = 0;
            for project in self
                .session
                .navigation
                .sorted_projects(&self.projects, &BTreeMap::new())
                .into_iter()
                .filter(|p| p.name.to_lowercase().contains(&query))
            {
                count += 1;
                let id = project.id;
                let visible = !self.session.navigation.hidden_projects.contains(&id);
                list = list.child(
                    row()
                        .p_3()
                        .rounded_lg()
                        .bg(rgb(PANEL))
                        .child(div().flex_1().min_w(px(0.)).child(project.name))
                        .child(settings::toggle(
                            element_id("project-visible", id),
                            visible,
                            "Project visibility".into(),
                            cx,
                            move |this, _, cx| {
                                if !this.session.navigation.hidden_projects.remove(&id) {
                                    this.session.navigation.hidden_projects.insert(id);
                                }
                                this.navigation_changed(cx);
                            },
                        )),
                );
            }
            if count == 0 {
                list = list.child(muted("No matching projects"));
            }
        } else {
            let query = !self.management_query.trim().is_empty();
            let mut chats: Vec<_> = self
                .chats
                .iter()
                .filter(|c| {
                    c.archived_at.is_some()
                        && (!query
                            || self
                                .archive_hits
                                .as_ref()
                                .is_some_and(|hits| hits.iter().any(|h| h.chat_id == c.id)))
                })
                .cloned()
                .collect();
            chats.sort_by_key(|c| (std::cmp::Reverse(c.archived_at), c.id));
            if chats.is_empty() {
                list = list.child(muted(if query && self.archive_hits.is_none() {
                    "Searching…"
                } else {
                    "No archived chats found"
                }));
            }
            for chat in chats {
                let id = chat.id;
                let project = self
                    .metadata
                    .get(&id)
                    .and_then(|p| p.project_id.as_ref())
                    .and_then(|id| self.projects.iter().find(|p| p.id.to_string() == *id))
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "Chats".into());
                list = list.child(
                    button(element_id("archived-chat", id), "")
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap_1()
                        .bg(rgb(PANEL))
                        .child(div().child(self.display_title(&chat)))
                        .child(muted(project))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.management = Some(ManagementView::Archive(Some(id)));
                            this.load_chat(id, cx);
                            cx.notify();
                        })),
                );
            }
        }
        content.child(list)
    }
}
