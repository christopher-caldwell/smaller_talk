use super::{
    panes::{Axis, Layout, PaneId},
    *,
};

// Narrower panes cannot fit the composer controls.
const MIN_PANE_WIDTH: f32 = 280.;
const MIN_PANE_HEIGHT: f32 = 200.;

impl ChatsView {
    pub(super) fn render_panes(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let live = self.panes.panes();
        self.pane_sizes.retain(|pane, _| live.contains(pane));
        self.pane_focus.retain(|pane, _| live.contains(pane));
        let layout = self.panes.root().clone();
        let weak = cx.entity().downgrade();
        div()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .flex()
            .child(self.render_layout(&layout, window, cx))
            .on_children_prepainted(move |bounds, _, cx| {
                if let Some(bounds) = bounds.first() {
                    let size = bounds.size;
                    let weak = weak.clone();
                    cx.defer(move |cx| {
                        let _ = weak.update(cx, |this, cx| {
                            if this.panes_area != size {
                                this.panes_area = size;
                                cx.notify();
                            }
                        });
                    });
                }
            })
            .into_any_element()
    }

    fn render_layout(
        &mut self,
        layout: &Layout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let split = match layout {
            Layout::Pane(pane) => return self.render_pane(*pane, window, cx),
            Layout::Split(split) => split,
        };
        let (id, axis, ratio) = (split.id, split.axis, split.ratio);
        let share = |element: gpui::AnyElement, grow: f32| {
            div()
                .flex_grow(grow)
                .flex_basis(px(0.))
                .min_w(px(0.))
                .min_h(px(0.))
                .flex()
                .child(element)
        };
        let first = share(self.render_layout(&split.first, window, cx), ratio);
        let second = share(self.render_layout(&split.second, window, cx), 1. - ratio);
        let line = div().bg(rgb(BORDER));
        let divider = div()
            .id(element_id("pane-divider", id))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .hover(|style| style.bg(rgb(PANEL_HOVER)))
            .map(|divider| match axis {
                Axis::Row => divider
                    .w(px(7.))
                    .h_full()
                    .cursor_col_resize()
                    .child(line.w(px(1.)).h_full()),
                Axis::Column => divider
                    .h(px(7.))
                    .w_full()
                    .cursor_row_resize()
                    .child(line.h(px(1.)).w_full()),
            })
            .tooltip(hint("Drag to resize; double-click to even out"))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.pane_resize = Some(id);
                    cx.stop_propagation();
                }),
            )
            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                if event.click_count() == 2 && this.panes.resize(id, 0.5) {
                    this.save_session();
                    cx.notify();
                }
            }));
        let weak = cx.entity().downgrade();
        div()
            .on_children_prepainted(move |bounds, _, cx| {
                let (Some(first), Some(last)) = (bounds.first(), bounds.last()) else {
                    return;
                };
                let size = gpui::size(last.right() - first.left(), last.bottom() - first.top());
                let bounds = gpui::Bounds::new(first.origin, size);
                let weak = weak.clone();
                cx.defer(move |cx| {
                    let _ = weak.update(cx, |this, cx| {
                        if this.split_bounds.insert(id, bounds) != Some(bounds) {
                            cx.notify();
                        }
                    });
                });
            })
            .id(element_id("pane-split", id))
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .flex()
            .when(axis == Axis::Column, |split| split.flex_col())
            .child(first)
            .child(divider)
            .child(second)
            .on_mouse_move(
                cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                    if this.pane_resize != Some(id) {
                        return;
                    }
                    if event.pressed_button != Some(gpui::MouseButton::Left) {
                        this.finish_pane_resize(cx);
                        return;
                    }
                    let Some(bounds) = this.split_bounds.get(&id).copied() else {
                        return;
                    };
                    let (offset, length, minimum) = match axis {
                        Axis::Row => (
                            event.position.x - bounds.left(),
                            bounds.size.width,
                            MIN_PANE_WIDTH,
                        ),
                        Axis::Column => (
                            event.position.y - bounds.top(),
                            bounds.size.height,
                            MIN_PANE_HEIGHT,
                        ),
                    };
                    let length = f32::from(length);
                    if length <= 0. {
                        return;
                    }
                    let limit = (minimum / length).min(0.5);
                    let ratio = (f32::from(offset) / length).clamp(limit, 1. - limit);
                    if this.panes.resize(id, ratio) {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_pane_resize(cx)),
            )
            .on_mouse_up_out(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_pane_resize(cx)),
            )
            .into_any_element()
    }

    fn render_pane(
        &mut self,
        pane: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let split = self.panes.is_split();
        let focused = self.panes.focused() == pane;
        let chat = self
            .panes
            .chat(pane)
            .filter(|id| self.states.contains_key(id));
        let size = self
            .pane_sizes
            .get(&pane)
            .copied()
            .unwrap_or(self.panes_area);
        let focus = self
            .pane_focus
            .entry(pane)
            .or_insert_with(|| cx.focus_handle())
            .clone();
        let header = split.then(|| self.render_pane_header(pane, chat, focused, cx));
        let body = match chat {
            Some(id) => self.render_chat_pane(id, focused, size, window, cx),
            None => self.render_empty_pane(focused, window, cx),
        };
        let weak = cx.entity().downgrade();
        div()
            .on_children_prepainted(move |bounds, _, cx| {
                if let Some(bounds) = bounds.last() {
                    let size = bounds.size;
                    let weak = weak.clone();
                    cx.defer(move |cx| {
                        let _ = weak.update(cx, |this, cx| {
                            if this.pane_sizes.insert(pane, size) != Some(size) {
                                cx.notify();
                            }
                        });
                    });
                }
            })
            .id(element_id("pane", pane.index()))
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .flex()
            .flex_col()
            // Split panes take focus from plain clicks so the click can pick the pane.
            .when(split, |view| view.track_focus(&focus))
            .capture_any_mouse_down(
                cx.listener(move |this, _, window, cx| this.focus_pane(pane, window, cx)),
            )
            .children(header)
            .child(body)
            .into_any_element()
    }

    fn render_pane_header(
        &self,
        pane: PaneId,
        chat: Option<Uuid>,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let title = match chat {
            Some(id) => match self.chats.iter().find(|c| c.id == id) {
                Some(chat) => self.display_title(chat),
                None if self.is_new_chat(id) => "New chat".into(),
                None => "Chat".into(),
            },
            None => "Empty pane".into(),
        };
        let working = chat
            .and_then(|id| self.states.get(&id))
            .is_some_and(|s| s.activity.is_some());
        let waiting = chat
            .and_then(|id| self.states.get(&id))
            .is_some_and(|state| {
                state.pending.is_some() || state.activity == Some(Activity::Waiting)
            });
        row()
            .h(px(28.))
            .flex_shrink_0()
            .pl_3()
            .pr_1()
            .border_t_2()
            .border_color(if focused { rgb(ACCENT) } else { quiet_border() })
            .bg(rgb(if focused { PANEL } else { BACKGROUND }))
            .when(working, |header| {
                header.child(
                    div()
                        .size(px(5.))
                        .rounded_full()
                        .flex_shrink_0()
                        .bg(rgb(if waiting { ATTENTION } else { ACCENT })),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .text_size(px(12.))
                    .text_color(rgb(if waiting {
                        ATTENTION
                    } else if focused {
                        TEXT
                    } else {
                        MUTED
                    }))
                    .child(title),
            )
            .child(
                icon_button(element_id("close-pane", pane.index()), "close")
                    .size(px(22.))
                    .tooltip(hint("Close pane; its chat stays open as a tab"))
                    .on_click(cx.listener(move |this, _, _, cx| this.close_pane(pane, cx))),
            )
            .into_any_element()
    }

    fn render_chat_pane(
        &mut self,
        id: Uuid,
        focused: bool,
        size: gpui::Size<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let width = (size.width - self.content_inset * 2.)
            .max(px(1.))
            .min(px(800.));
        if let Some(state) = self.states.get_mut(&id)
            && state.width != width
        {
            state.width = width;
            let offset = state.list.logical_scroll_top();
            state.list.reset(state.messages.len());
            if state.follow {
                state.list.scroll_to(ListOffset {
                    item_ix: state.messages.len(),
                    offset_in_item: px(0.),
                });
            } else {
                state.list.scroll_to(offset);
            }
        }
        // Size to the window less whatever this pane gives up to its neighbours,
        // so an unsplit pane behaves exactly like the window it fills.
        let height =
            window.viewport_size().height - (self.panes_area.height - size.height).max(px(0.));
        let mut detail = div().flex_1().min_w(px(0.)).min_h(px(0.)).flex().flex_col();
        let pending = self.states[&id].pending.is_some();
        if focused && self.focus_composer && self.states[&id].loaded && !self.settings_open {
            if pending && height < px(600.) {
                self.focus.focus(window, cx);
            } else {
                self.states[&id].composer.focus_handle(cx).focus(window, cx);
            }
            self.focus_composer = false;
        }
        self.states[&id].composer.update(cx, |composer, cx| {
            composer.set_max_rows(composer_rows(f32::from(height), pending), cx)
        });
        detail = detail.child(self.render_transcript(id, window, cx));
        let compact_request = pending && height < px(600.);
        let interaction_height = if compact_request {
            (height * 0.35).clamp(px(140.), px(180.))
        } else {
            (height * 0.3).clamp(px(180.), px(280.))
        };
        if let Some(interaction) = self.render_interaction(id, interaction_height, cx) {
            detail = detail.child(
                div()
                    .flex_shrink_0()
                    .id("interaction-viewport")
                    .max_h(interaction_height)
                    .w_full()
                    .flex()
                    .justify_center()
                    .px(self.content_inset)
                    .child(interaction),
            );
        }
        let state = &self.states[&id];
        let status = state.activity.filter(|_| state.pending.is_none());
        let error = state.error.clone();
        if let Some(status) = status.filter(|_| !compact_request) {
            let status = self.render_activity(id, status, ACCENT, false, cx);
            detail = detail.child(
                div()
                    .flex_shrink_0()
                    .px(self.content_inset)
                    .py_1()
                    .text_size(px(13.))
                    .text_color(rgb(ACCENT))
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(div().w_full().max_w(px(800.)).child(status)),
            );
        }
        if let Some(notice) = self.command_notices.get(&id) {
            detail = detail.child(
                div()
                    .px(self.content_inset)
                    .py_1()
                    .flex()
                    .justify_center()
                    .child(div().w_full().max_w(px(800.)).child(muted(notice.clone()))),
            );
        }
        if let Some(error) = error {
            let expanded = self.states[&id].error_details;
            let summary = error_summary(&error, self.states[&id].settings_error, pending);
            detail = detail.child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .px(self.content_inset)
                    .py_2()
                    .flex_shrink_0()
                    .child(
                        div()
                            .id("chat-error")
                            .w_full()
                            .max_w(px(800.))
                            .max_h(px(if compact_request { 36. } else { 120. }))
                            .overflow_y_scroll()
                            .text_sm()
                            .child(
                                row()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.))
                                            .text_color(rgb(DANGER))
                                            .child(summary),
                                    )
                                    .child(
                                        button(
                                            "chat-error-details",
                                            if expanded { "Hide details" } else { "Details" },
                                        )
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                if let Some(s) = this.states.get_mut(&id) {
                                                    s.error_details = !s.error_details;
                                                }
                                                cx.notify();
                                            }),
                                        ),
                                    ),
                            )
                            .when(expanded, |d| {
                                d.child(div().text_color(rgb(MUTED)).child(error))
                            }),
                    ),
            );
        }
        if let Some(queue) = self.render_queue(id, height, cx) {
            detail = detail.child(
                div()
                    .flex_shrink_0()
                    .w_full()
                    .flex()
                    .justify_center()
                    .px(self.content_inset)
                    .child(queue),
            );
        }
        // Keep a compact composer available for queuing while an approval is pending.
        detail = detail.child(
            div()
                .flex_shrink_0()
                .w_full()
                .flex()
                .justify_center()
                .px(self.content_inset)
                .pt_2()
                .pb_2()
                .child(self.render_composer(id, window, cx)),
        );
        if pending && !compact_request {
            detail = detail.child(
                div()
                    .flex_shrink_0()
                    .px_3()
                    .pb_3()
                    .text_center()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child("Your draft is saved."),
            );
        }
        detail.into_any_element()
    }

    fn render_empty_pane(
        &mut self,
        focused: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if focused && self.focus_composer && !self.settings_open {
            self.focus.focus(window, cx);
            self.focus_composer = false;
        }
        // Offer open tabs no other pane is showing.
        let hidden: Vec<(Uuid, String)> = if self.panes.is_split() {
            self.session
                .open_tabs
                .iter()
                .filter_map(|id| Uuid::parse_str(id).ok())
                .filter(|id| self.panes.pane_of(*id).is_none())
                .filter_map(|id| match self.chats.iter().find(|c| c.id == id) {
                    Some(chat) => Some((id, self.display_title(chat))),
                    None if self.is_new_chat(id) => Some((id, "New chat".into())),
                    None => None,
                })
                .take(8)
                .collect()
        } else {
            Vec::new()
        };
        div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .p_4()
            .child(
                div()
                    .text_2xl()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child("What’s on your mind?"),
            )
            .child(muted(if hidden.is_empty() {
                "Open a chat from the sidebar, or start a new one."
            } else {
                "Open a chat here, or start a new one."
            }))
            .when(!hidden.is_empty(), |view| {
                view.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_1()
                        .max_w(px(320.))
                        .w_full()
                        .children(hidden.into_iter().map(|(id, title)| {
                            compact_button(element_id("pane-open-tab", id), "")
                                .w_full()
                                .child(div().truncate().text_center().child(title))
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.open_chat(id, None, cx)),
                                )
                        })),
                )
            })
            .child(
                button("empty-new-chat", "+  New chat")
                    .on_click(cx.listener(|this, _, _, cx| this.new_chat(cx))),
            )
            .into_any_element()
    }

    fn finish_pane_resize(&mut self, cx: &mut Context<Self>) {
        if self.pane_resize.take().is_some() {
            self.save_session();
            cx.notify();
        }
    }

    fn focus_pane(&mut self, pane: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        if !self.panes.focus(pane) {
            return;
        }
        self.session.active_tab = self.active().map(|id| id.to_string());
        self.save_session();
        // A click on plain pane space focuses the pane itself; pass that to its composer.
        cx.defer_in(window, move |this, window, cx| {
            if this
                .pane_focus
                .get(&pane)
                .is_some_and(|focus| focus.is_focused(window))
            {
                this.focus_composer = true;
            }
            cx.notify();
        });
        cx.notify();
    }

    pub(super) fn switch_pane(
        &mut self,
        direction: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_open {
            return;
        }
        let panes = self.panes.panes();
        let index = panes
            .iter()
            .position(|pane| *pane == self.panes.focused())
            .unwrap_or(0);
        let next = (index as isize + direction).rem_euclid(panes.len() as isize) as usize;
        self.focus_pane(panes[next], window, cx);
        self.focus_composer = true;
        cx.notify();
    }

    pub(super) fn split_pane(&mut self, axis: Axis, cx: &mut Context<Self>) {
        self.popup = None;
        self.pane_resize = None;
        self.panes.split(axis);
        self.session.active_tab = None;
        self.focus_composer = true;
        self.save_session();
        cx.notify();
    }

    pub(super) fn close_pane(&mut self, pane: PaneId, cx: &mut Context<Self>) {
        self.popup = None;
        if self.panes.close(pane) {
            self.pane_resize = None;
            self.split_bounds.clear();
            self.session.active_tab = self.active().map(|id| id.to_string());
            self.focus_composer = true;
            self.save_session();
        }
        cx.notify();
    }

    pub(super) fn render_pane_menu(&self, cx: &mut Context<Self>) -> gpui::Div {
        let split = self.panes.is_split();
        let focused = self.panes.focused();
        div()
            .flex()
            .flex_col()
            .child(
                menu_item("split-right", "split-right", "Split right  ⌘D")
                    .on_click(cx.listener(|this, _, _, cx| this.split_pane(Axis::Row, cx))),
            )
            .child(
                menu_item("split-down", "split-down", "Split down  ⇧⌘D")
                    .on_click(cx.listener(|this, _, _, cx| this.split_pane(Axis::Column, cx))),
            )
            .when(split, |menu| {
                menu.child(div().h(px(1.)).mx_2().my_1().bg(rgb(BORDER)))
                    .child(
                        menu_item("next-pane", "panes", "Next pane  ⌥⌘]").on_click(cx.listener(
                            |this, _, window, cx| {
                                this.popup = None;
                                this.switch_pane(1, window, cx);
                            },
                        )),
                    )
                    .child(
                        menu_item("previous-pane", "panes", "Previous pane  ⌥⌘[").on_click(
                            cx.listener(|this, _, window, cx| {
                                this.popup = None;
                                this.switch_pane(-1, window, cx);
                            }),
                        ),
                    )
                    .child(
                        menu_item("even-out-panes", "panes", "Even out panes").on_click(
                            cx.listener(|this, _, _, cx| {
                                this.panes.even_out();
                                this.save_session();
                                this.popup = None;
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        menu_item("close-focused-pane", "close", "Close pane").on_click(
                            cx.listener(move |this, _, _, cx| this.close_pane(focused, cx)),
                        ),
                    )
            })
    }
}
