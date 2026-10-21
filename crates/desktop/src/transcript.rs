use gpui::{
    AnyElement, Context, Entity, HighlightStyle, IntoElement, ScrollHandle, Styled, Window, div,
    list, prelude::*, px, rgb,
};
use smaller_talk_domain::MessageRole;
use std::ops::Range;
use uuid::Uuid;

use crate::{
    markdown::{MarkdownBlock, MarkdownBlockKind},
    rich_text::RichText,
    shell::{ChatsView, element_id},
    theme::*,
};

impl ChatsView {
    pub(crate) fn render_transcript(
        &mut self,
        id: Uuid,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = &self.states[&id];
        if !state.loaded {
            return div()
                .flex_1()
                .min_h(px(0.))
                .overflow_hidden()
                .flex()
                .items_center()
                .justify_center()
                .child(muted("Loading conversation…"))
                .into_any_element();
        }
        if state.messages.is_empty() {
            return div()
                .flex_1()
                .min_h(px(0.))
                .overflow_hidden()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .px_3()
                .text_center()
                .child(div().text_lg().child("Start a conversation"))
                .when(_window.viewport_size().height >= px(600.), |view| {
                    view.child(muted("Choose a model below, then ask a question."))
                })
                .into_any_element();
        }
        let list_state = state.list.clone();
        let weak = cx.entity().downgrade();
        let conversation = list(list_state, move |index, window, cx| {
            weak.update(cx, |this, cx| this.render_message(id, index, window, cx))
                .unwrap_or_else(|_| div().into_any_element())
        })
        .flex_1()
        .min_h(px(0.));
        div()
            .id("transcript-viewport")
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .w_full()
            .overflow_hidden()
            .relative()
            .flex()
            .flex_col()
            .child(conversation)
            .when(!state.follow, |view| {
                view.child(
                    div()
                        .flex_shrink_0()
                        .h(px(44.))
                        .w_full()
                        .flex()
                        .justify_center()
                        .items_center()
                        .child(
                            compact_button("return-to-latest", "↓  Latest")
                                .bg(rgb(PANEL))
                                .border_color(quiet_border())
                                .tooltip(hint("Return to the latest message and follow new output"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(state) = this.states.get_mut(&id) {
                                        state.follow = true;
                                        state.list.scroll_to(gpui::ListOffset {
                                            item_ix: state.messages.len(),
                                            offset_in_item: px(0.),
                                        });
                                    }
                                    this.save_session();
                                    cx.notify();
                                })),
                        ),
                )
            })
            .into_any_element()
    }

    pub(super) fn text_view(
        &mut self,
        chat: Uuid,
        key: (Uuid, String),
        text: String,
        highlights: Vec<(Range<usize>, HighlightStyle)>,
        links: Vec<(Range<usize>, String)>,
        cx: &mut Context<Self>,
    ) -> Entity<RichText> {
        let state = self.states.get_mut(&chat).expect("chat has a view");
        // Bound view state to recently rendered blocks, including large tables.
        if state.rich.len() > 192 {
            state.rich.clear();
        }
        let owner_key = key.1.clone();
        let message_id = key.0;
        let entity = state
            .rich
            .entry(key)
            .or_insert_with(|| cx.new(RichText::new))
            .clone();
        let weak = cx.entity().downgrade();
        entity.update(cx, |view, _| {
            view.set(text, highlights, links);
            view.coordinate_selection(weak, chat, message_id, owner_key);
        });
        entity
    }

    fn render_message(
        &mut self,
        chat: Uuid,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(mut message) = self.states[&chat].messages.get(index).cloned() else {
            return div().into_any_element();
        };
        message.body = self.displayed_response_body(&message);
        let files = self.states[&chat]
            .files
            .get(&message.id)
            .cloned()
            .unwrap_or_default();
        let mut body = div()
            .w(self.last_width)
            .max_w(self.last_width)
            .flex_shrink_0()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .gap_2()
            .py_3();
        if message.role == MessageRole::User {
            let text = self.text_view(
                chat,
                (message.id, "user".into()),
                message.body.clone(),
                vec![],
                vec![],
                cx,
            );
            body = body.items_end().child(
                div()
                    .max_w(px(650.))
                    .min_w(px(0.))
                    .px_3()
                    .py_2()
                    .rounded_lg()
                    .bg(rgb(PANEL_HOVER))
                    .text_size(px(15.))
                    .line_height(px(24.))
                    .child(text),
            );
        } else {
            body = body.child(
                row()
                    .text_size(px(12.))
                    .line_height(px(16.))
                    .text_color(rgb(MUTED))
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child("Assistant"),
                    )
                    .when(self.showing_shortened(message.id), |header| {
                        header.child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(ACCENT))
                                .child("Shortened"),
                        )
                    }),
            );
            let blocks = self
                .states
                .get_mut(&chat)
                .expect("chat exists")
                .markdown
                .get_or_parse(message.id, &message.body);
            let mut content = div().w_full().min_w(px(0.)).flex().flex_col();
            for (index, block) in blocks.iter().enumerate() {
                let previous = index.checked_sub(1).map(|i| &blocks[i].kind);
                let gap = match (previous, &block.kind) {
                    (None, _) => 0.,
                    (_, MarkdownBlockKind::Heading(_)) => 14.,
                    (Some(MarkdownBlockKind::Heading(_)), _) => 6.,
                    (Some(MarkdownBlockKind::ListItem), MarkdownBlockKind::ListItem) => 3.,
                    (Some(MarkdownBlockKind::Quote), MarkdownBlockKind::Quote) => 4.,
                    _ => 10.,
                };
                content = content.child(
                    div()
                        .w_full()
                        .min_w(px(0.))
                        .mt(px(gap))
                        .child(self.render_block(chat, message.id, index, block, window, cx)),
                );
            }
            body = body.child(content);
        }
        for file in files {
            body = body.child(
                div()
                    .max_w(px(360.))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(quiet_border())
                    .bg(rgb(PANEL))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_size(px(12.))
                    .text_color(rgb(MUTED))
                    .when_some(file.preview_path.clone(), |chip, path| {
                        chip.child(thumbnail(
                            element_id("sent-image-cache", format!("{}:{}", message.id, file.id)),
                            path,
                            160.,
                            120.,
                        ))
                    })
                    .child(format!(
                        "{} · {} KiB",
                        file.name,
                        file.byte_len.div_ceil(1024)
                    )),
            );
        }
        if message.role == MessageRole::Assistant {
            let source = message.body.clone();
            let message_id = message.id;
            let copied = self.states[&chat].copied == Some(message_id);
            body = body.child(
                row()
                    .gap_1()
                    .flex_wrap()
                    .child(
                        button(element_id("copy-response", message.id), "")
                            .w(px(28.))
                            .h(px(28.))
                            .px_0()
                            .py_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .flex_shrink_0()
                            .tooltip(hint("Copy response"))
                            .when(copied, |b| {
                                b.child(div().text_color(rgb(ACCENT)).child("✓"))
                            })
                            .when(!copied, |b| b.child(icon("copy").text_color(rgb(MUTED))))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                    source.clone(),
                                ));
                                if let Some(state) = this.states.get_mut(&chat) {
                                    state.copied = Some(message_id);
                                }
                                cx.notify();
                                cx.spawn(async move |this, cx| {
                                    cx.background_executor()
                                        .timer(std::time::Duration::from_secs(2))
                                        .await;
                                    let _ = this.update(cx, |this, cx| {
                                        if let Some(state) = this.states.get_mut(&chat)
                                            && state.copied == Some(message_id)
                                        {
                                            state.copied = None;
                                        }
                                        cx.notify();
                                    });
                                })
                                .detach();
                            })),
                    )
                    .child(self.render_rewrite_controls(chat, message.id, cx)),
            );
        }
        div()
            .id(element_id("message", message.id))
            .w_full()
            .min_w(px(0.))
            .px(self.content_inset)
            .flex()
            .flex_col()
            .items_center()
            .child(body)
            .into_any_element()
    }

    fn render_block(
        &mut self,
        chat: Uuid,
        message: Uuid,
        index: usize,
        block: &MarkdownBlock,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match &block.kind {
            MarkdownBlockKind::Rule => div()
                .w_full()
                .h(px(1.))
                .my_1()
                .bg(quiet_border())
                .into_any_element(),
            MarkdownBlockKind::Table(alignments) => {
                let columns = alignments.len();
                let widths: Vec<_> = (0..columns)
                    .map(|col| {
                        let longest = block
                            .table_rows
                            .iter()
                            .filter_map(|r| r.get(col))
                            .map(|c| c.text.chars().count())
                            .max()
                            .unwrap_or(10);
                        px((longest as f32 * 7. + 22.).clamp(110., 320.))
                    })
                    .collect();
                let table_width: gpui::Pixels = widths.iter().copied().fold(px(0.), |a, b| a + b);
                let mut table = div()
                    .flex()
                    .flex_col()
                    .w(table_width)
                    .border_1()
                    .border_color(quiet_border())
                    .rounded_md();
                for (row_index, cells) in block.table_rows.iter().enumerate() {
                    let mut row = div()
                        .flex()
                        .w_full()
                        .flex_shrink_0()
                        .when(row_index == 0, |r| {
                            r.bg(rgb(PANEL)).font_weight(gpui::FontWeight::SEMIBOLD)
                        });
                    for (column, cell) in cells.iter().enumerate() {
                        let text = self.text_view(
                            chat,
                            (message, format!("table-{index}-{row_index}-{column}")),
                            cell.text.clone(),
                            cell.highlights
                                .iter()
                                .map(|(r, s)| (r.clone(), (*s).into()))
                                .collect(),
                            cell.links.clone(),
                            cx,
                        );
                        row = row.child(
                            div()
                                .w(widths.get(column).copied().unwrap_or(px(120.)))
                                .flex_shrink_0()
                                .px(px(10.))
                                .py(px(6.))
                                .border_b_1()
                                .border_r_1()
                                .border_color(quiet_border())
                                .text_size(px(13.))
                                .line_height(px(21.))
                                .when(
                                    alignments.get(column)
                                        == Some(&pulldown_cmark::Alignment::Right),
                                    |c| c.text_right(),
                                )
                                .when(
                                    alignments.get(column)
                                        == Some(&pulldown_cmark::Alignment::Center),
                                    |c| c.text_center(),
                                )
                                .child(text),
                        );
                    }
                    table = table.child(row);
                }
                let scroll = self
                    .states
                    .get_mut(&chat)
                    .expect("chat exists")
                    .horizontal
                    .entry((message, index))
                    .or_default()
                    .clone();
                horizontal_scroller(
                    element_id("table-scroll", format!("{message}:{index}")),
                    &scroll,
                )
                .child(table)
                .into_any_element()
            }
            MarkdownBlockKind::Code(language) => {
                let source = block.text.clone();
                let copied = self.states[&chat].copied_code == Some((message, index));
                let highlights = block
                    .code_highlights
                    .iter()
                    .map(|(r, color)| {
                        (
                            r.clone(),
                            HighlightStyle {
                                color: Some(rgb(*color).into()),
                                ..Default::default()
                            },
                        )
                    })
                    .collect();
                let text = self.text_view(
                    chat,
                    (message, format!("code-{index}")),
                    block.text.clone(),
                    highlights,
                    vec![],
                    cx,
                );
                let mut style = window.text_style();
                style.font_family = "Menlo".into();
                let code_width = block
                    .text
                    .lines()
                    .map(|line| {
                        window
                            .text_system()
                            .shape_text(
                                line.to_owned().into(),
                                px(13.),
                                &[style.to_run(line.len())],
                                None,
                                None,
                            )
                            .ok()
                            .and_then(|lines| lines.iter().map(|l| l.size(px(22.)).width).max())
                            .unwrap_or(px(0.))
                    })
                    .max()
                    .unwrap_or(px(0.))
                    .max(self.last_width - px(26.));
                let scroll: ScrollHandle = self
                    .states
                    .get_mut(&chat)
                    .expect("chat exists")
                    .horizontal
                    .entry((message, index))
                    .or_default()
                    .clone();
                div()
                    .w_full()
                    .min_w(px(0.))
                    .rounded_lg()
                    .border_1()
                    .border_color(quiet_border())
                    .bg(rgb(PANEL))
                    .child(
                        crate::theme::row()
                            .px_3()
                            .h(px(30.))
                            .justify_between()
                            .border_b_1()
                            .border_color(quiet_border())
                            .child(div().text_size(px(11.)).text_color(rgb(MUTED)).child(
                                if language.is_empty() {
                                    "text".into()
                                } else {
                                    language.clone()
                                },
                            ))
                            .child(
                                compact_button(
                                    element_id("copy-code", format!("{message}:{index}")),
                                    if copied { "✓ Copied" } else { "Copy" },
                                )
                                .text_size(px(12.))
                                .text_color(rgb(MUTED))
                                .py_0()
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                            source.clone(),
                                        ));
                                        if let Some(state) = this.states.get_mut(&chat) {
                                            state.copied_code = Some((message, index));
                                        }
                                        cx.notify();
                                        cx.spawn(async move |this, cx| {
                                            cx.background_executor()
                                                .timer(std::time::Duration::from_secs(2))
                                                .await;
                                            let _ = this.update(cx, |this, cx| {
                                                if let Some(state) = this.states.get_mut(&chat)
                                                    && state.copied_code == Some((message, index))
                                                {
                                                    state.copied_code = None;
                                                }
                                                cx.notify();
                                            });
                                        })
                                        .detach();
                                    },
                                )),
                            ),
                    )
                    .child(
                        horizontal_scroller(
                            element_id("code-scroll", format!("{message}:{index}")),
                            &scroll,
                        )
                        .px_3()
                        .py_2()
                        .child(
                            div()
                                .w(code_width + px(12.))
                                .font_family("Menlo")
                                .text_size(px(13.))
                                .line_height(px(22.))
                                .whitespace_nowrap()
                                .child(text),
                        ),
                    )
                    .into_any_element()
            }
            kind => {
                let text = self.text_view(
                    chat,
                    (message, format!("block-{index}")),
                    block.text.clone(),
                    block
                        .highlights
                        .iter()
                        .map(|(r, s)| (r.clone(), (*s).into()))
                        .collect(),
                    block.links.clone(),
                    cx,
                );
                let mut view = div()
                    .w_full()
                    .min_w(px(0.))
                    .text_size(px(15.))
                    .line_height(px(24.))
                    .child(text);
                match kind {
                    MarkdownBlockKind::Heading(level) => {
                        view = view
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .line_height(px(match level {
                                1 => 28.,
                                2 => 25.,
                                _ => 24.,
                            }))
                            .text_size(px(match level {
                                1 => 22.,
                                2 => 19.,
                                _ => 17.,
                            }));
                    }
                    MarkdownBlockKind::Quote => {
                        view = view
                            .pl_3()
                            .border_l_2()
                            .border_color(quiet_border())
                            .text_color(rgb(MUTED));
                    }
                    _ => {}
                }
                view.into_any_element()
            }
        }
    }
}
