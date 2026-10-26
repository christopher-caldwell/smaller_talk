use super::*;

pub(super) struct RepositoryUi {
    pub open: bool,
    pub focused: bool,
    observing: bool,
    folder: Option<PathBuf>,
    context_chat: Option<Uuid>,
    pub context_generation: u64,
    generation: u64,
    diff_generation: u64,
    loading: bool,
    scope: DiffScope,
    status: Option<RepositoryStatus>,
    error: Option<String>,
    selected: Option<PathBuf>,
    diff: Option<FileDiff>,
    diff_view: Entity<RichText>,
    pub pull_request: Option<String>,
}
impl RepositoryUi {
    pub fn new(cx: &mut Context<ChatsView>) -> Self {
        Self {
            open: false,
            focused: false,
            observing: false,
            folder: None,
            context_chat: None,
            context_generation: 0,
            generation: 0,
            diff_generation: 0,
            loading: false,
            scope: DiffScope::Unstaged,
            status: None,
            error: None,
            selected: None,
            diff: None,
            diff_view: cx.new(RichText::new),
            pull_request: None,
        }
    }
}
impl ChatsView {
    fn effective_folder(&self) -> PathBuf {
        self.active()
            .and_then(|id| self.states.get(&id))
            .and_then(|s| s.preferences.project_id.as_ref())
            .and_then(|id| self.projects.iter().find(|p| p.id.to_string() == *id))
            .and_then(|p| p.folder.clone())
            .unwrap_or_else(|| self.services.launch_folder.clone())
    }
    pub(super) fn sync_repository(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.repository.observing {
            self.repository.observing = true;
            cx.observe_window_activation(window, |this, window, cx| {
                this.repository.focused = window.is_window_active();
                if this.repository.focused {
                    this.refresh_repository(true);
                }
                cx.notify();
            })
            .detach();
        }
        self.repository.focused = window.is_window_active();
        let folder = self.effective_folder();
        if self.repository.folder.as_ref() != Some(&folder)
            || self.repository.context_chat != self.active()
        {
            self.repository.folder = Some(folder);
            self.repository.context_chat = self.active();
            self.repository.context_generation += 1;
            self.repository.loading = false;
            self.repository.status = None;
            self.repository.error = None;
            self.repository.pull_request = None;
            self.repository.selected = None;
            self.repository.diff = None;
            self.repository.diff_generation += 1;
            self.refresh_repository(true);
        }
    }
    pub(super) fn refresh_repository(&mut self, include_pr: bool) {
        if self.repository.loading {
            return;
        }
        let Some(folder) = self.repository.folder.clone() else {
            return;
        };
        self.repository.loading = true;
        self.repository.generation += 1;
        let generation = self.repository.generation;
        let context_generation = self.repository.context_generation;
        let scope = self.repository.scope;
        let reader = self.services.repositories.clone();
        let sender = self.sender.clone();
        self.services.runtime.spawn(async move {
            let result = reader
                .status(&folder, scope)
                .await
                .map_err(|e| e.to_string());
            let root = result
                .as_ref()
                .ok()
                .and_then(|s| s.as_ref())
                .map(|s| s.root.clone());
            let _ = sender.send(UiEvent::Repository(generation, result));
            if include_pr && let Some(root) = root {
                let url = reader.pull_request(root).await;
                let _ = sender.send(UiEvent::PullRequest(context_generation, url));
            }
        });
    }
    pub(super) fn repository_result(
        &mut self,
        generation: u64,
        result: Result<Option<RepositoryStatus>, String>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.repository.generation {
            return;
        }
        self.repository.loading = false;
        match result {
            Ok(status) => {
                self.repository.error = None;
                let old_identity = self
                    .repository
                    .status
                    .as_ref()
                    .map(|status| (status.root.clone(), status.branch.clone()));
                let new_identity = status
                    .as_ref()
                    .map(|status| (status.root.clone(), status.branch.clone()));
                let identity_changed = old_identity != new_identity;
                if identity_changed {
                    self.repository.context_generation += 1;
                    self.repository.pull_request = None;
                }
                self.repository.status = status;
                if self.repository.open {
                    let selected = self
                        .repository
                        .selected
                        .as_ref()
                        .and_then(|path| {
                            self.repository
                                .status
                                .as_ref()
                                .and_then(|s| s.files.iter().find(|f| &f.path == path))
                        })
                        .cloned()
                        .or_else(|| {
                            self.repository
                                .status
                                .as_ref()
                                .and_then(|s| s.files.first().cloned())
                        });
                    if let Some(file) = selected {
                        self.select_repository_file(file, cx);
                    } else {
                        self.repository.selected = None;
                        self.repository.diff = None;
                    }
                }
                if identity_changed {
                    self.refresh_repository(true);
                }
            }
            Err(error) => {
                self.repository.error = Some(error);
                self.repository.pull_request = None;
                self.repository.status = None;
                self.repository.diff = None;
            }
        }
    }
    fn select_repository_file(&mut self, file: ChangedFile, cx: &mut Context<Self>) {
        let Some(status) = &self.repository.status else {
            return;
        };
        let root = status.root.clone();
        let scope = status.scope;
        let changed = self.repository.selected.as_ref() != Some(&file.path);
        self.repository.selected = Some(file.path.clone());
        if changed {
            self.repository.diff = None;
        }
        self.repository.diff_generation += 1;
        let generation = self.repository.diff_generation;
        let sender = self.sender.clone();
        let reader = self.services.repositories.clone();
        self.services.runtime.spawn(async move {
            let result = reader
                .diff(&root, &file, scope)
                .await
                .map_err(|e| e.to_string());
            let _ = sender.send(UiEvent::RepositoryDiff(generation, file.path, result));
        });
        cx.notify();
    }
    pub(super) fn repository_diff_result(
        &mut self,
        generation: u64,
        path: PathBuf,
        result: Result<FileDiff, String>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.repository.diff_generation
            || self.repository.selected.as_ref() != Some(&path)
        {
            return;
        }
        match result {
            Ok(diff) => {
                let mut start = 0;
                let mut highlights = Vec::new();
                for line in diff.text.split_inclusive('\n') {
                    let color = if line.starts_with('+') {
                        Some(ACCENT)
                    } else if line.starts_with('-') {
                        Some(DANGER)
                    } else {
                        None
                    };
                    if let Some(color) = color {
                        highlights.push((
                            start..start + line.len(),
                            gpui::HighlightStyle {
                                color: Some(rgb(color).into()),
                                ..Default::default()
                            },
                        ));
                    }
                    start += line.len();
                }
                self.repository
                    .diff_view
                    .update(cx, |v, _| v.set(diff.text.clone(), highlights, vec![]));
                self.repository.diff = Some(diff);
            }
            Err(error) => {
                self.repository.error = Some(error);
                self.repository.diff = None;
            }
        }
    }
    pub(super) fn render_repository_header(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let status = self.repository.status.as_ref().map(|status| {
            let additions: u64 = status.files.iter().filter_map(|f| f.additions).sum();
            let deletions: u64 = status.files.iter().filter_map(|f| f.deletions).sum();
            let label = format!(
                "⑂ {} · {} files · +{additions} −{deletions}",
                status.branch,
                status.files.len()
            );
            compact_button("repo-status", label)
                .text_xs()
                .tooltip(hint(format!(
                    "{} · {:?} changes",
                    status.root.display(),
                    status.scope
                )))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.repository.open = !this.repository.open;
                    if this.repository.open {
                        this.refresh_repository(false);
                    }
                    cx.notify();
                }))
        });
        row()
            .h(px(30.))
            .flex_shrink_0()
            .px_3()
            .border_b_1()
            .border_color(quiet_border())
            .children(status)
            .when(
                self.repository.status.is_none() && self.repository.error.is_some(),
                |d| {
                    d.child(
                        button("repo-error", "Git review unavailable · Retry")
                            .text_xs()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.repository.open = true;
                                this.refresh_repository(true);
                                cx.notify();
                            })),
                    )
                },
            )
            .child(div().flex_1())
            .when_some(self.repository.pull_request.clone(), |d, url| {
                d.child(
                    compact_button("open-pull-request", "Pull request ↗")
                        .text_xs()
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                )
            })
            .child(
                icon_button("pane-menu", "panes")
                    .size(px(24.))
                    .when(self.popup.is_none(), |b| {
                        b.tooltip(hint("Split into panes"))
                    })
                    .on_click(cx.listener(|this, event: &gpui::ClickEvent, window, cx| {
                        this.open_context(Popup::Panes, event.position(), window, cx)
                    })),
            )
            .into_any_element()
    }
    pub(super) fn render_repository_panel(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut panel = div()
            .id("repository-review")
            .h_full()
            .w(px(420.))
            .max_w(window.viewport_size().width - px(52.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(rgb(PANEL))
            .border_l_1()
            .border_color(rgb(BORDER))
            .child(
                row()
                    .p_2()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Changes"),
                    )
                    .child(
                        button(
                            "refresh-repository",
                            if self.repository.loading {
                                "Refreshing…"
                            } else {
                                "Refresh"
                            },
                        )
                        .text_xs()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.refresh_repository(true);
                            cx.notify();
                        })),
                    )
                    .child(button("close-repository", "×").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.repository.open = false;
                            cx.notify();
                        },
                    ))),
            );
        let mut tabs = row().px_2();
        for (scope, label) in [
            (DiffScope::Unstaged, "Unstaged"),
            (DiffScope::Staged, "Staged"),
        ] {
            tabs = tabs.child(
                button(
                    element_id("diff-scope", label),
                    format!(
                        "{}{label}",
                        if self.repository.scope == scope {
                            "✓ "
                        } else {
                            ""
                        }
                    ),
                )
                .text_xs()
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.repository.scope != scope {
                        this.repository.scope = scope;
                        this.repository.loading = false;
                        this.repository.selected = None;
                        this.repository.diff = None;
                        this.repository.diff_generation += 1;
                        this.refresh_repository(false);
                    }
                    cx.notify();
                })),
            );
        }
        panel = panel.child(tabs);
        if let Some(error) = &self.repository.error {
            panel = panel.child(
                div()
                    .p_2()
                    .text_xs()
                    .text_color(rgb(DANGER))
                    .child(error.clone()),
            );
        }
        let mut files = div()
            .id("changed-files")
            .max_h(px(210.))
            .overflow_y_scroll()
            .p_2()
            .flex()
            .flex_col()
            .gap_1();
        if let Some(status) = &self.repository.status {
            if status.files.is_empty() {
                files = files.child(muted("No changes in this view"));
            }
            for file in &status.files {
                let selected = self.repository.selected.as_ref() == Some(&file.path);
                let counts = match (file.additions, file.deletions) {
                    (Some(a), Some(d)) => format!("+{a} −{d}"),
                    _ => "Binary / uncounted".into(),
                };
                let choice = file.clone();
                files = files.child(
                    button(
                        element_id("changed-file", format!("{:?}", file.path.as_os_str())),
                        "",
                    )
                    .w_full()
                    .flex()
                    .items_start()
                    .flex_col()
                    .bg(rgb(if selected { PANEL_HOVER } else { PANEL }))
                    .text_xs()
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .child(file.path.to_string_lossy().into_owned()),
                    )
                    .child(
                        div()
                            .text_color(rgb(MUTED))
                            .child(format!("{} · {counts}", file.status)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_repository_file(choice.clone(), cx)
                    })),
                );
            }
        }
        panel = panel.child(files);
        if let Some(diff) = &self.repository.diff {
            if diff.truncated {
                panel = panel.child(
                    div()
                        .p_2()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child("Preview truncated at 1 MiB"),
                );
            }
            panel = panel.child(
                div()
                    .id("diff-preview")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .p_3()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .font_family("Menlo")
                    .text_size(px(12.))
                    .line_height(px(19.))
                    .child(self.repository.diff_view.clone()),
            );
        } else {
            panel = panel.child(
                div()
                    .p_3()
                    .child(muted(if self.repository.selected.is_some() {
                        "Loading diff…"
                    } else {
                        "Choose a file to inspect its diff."
                    })),
            );
        }
        if window.viewport_size().width < px(1100.) {
            panel = panel
                .absolute()
                .right_0()
                .top_0()
                .bottom_0()
                .occlude()
                .shadow_lg();
        }
        panel.into_any_element()
    }
}
