use std::{collections::BTreeSet, ops::Range};

use gpui::{
    App, ClipboardItem, Context, FocusHandle, Focusable, HighlightStyle, IntoElement, KeyBinding,
    MouseButton, Render, StyledText, Window, actions, div, prelude::*, px, rgba,
};
use unicode_segmentation::UnicodeSegmentation;

actions!(rich_text, [Copy, SelectAll]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-c", Copy, Some("ResponseText")),
        KeyBinding::new("cmd-a", SelectAll, Some("ResponseText")),
    ]);
}

pub struct RichText {
    pub text: String,
    pub highlights: Vec<(Range<usize>, HighlightStyle)>,
    pub links: Vec<(Range<usize>, String)>,
    focus: FocusHandle,
    anchor: usize,
    caret: usize,
    dragging: bool,
}

impl RichText {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            text: String::new(),
            highlights: vec![],
            links: vec![],
            focus: cx.focus_handle(),
            anchor: 0,
            caret: 0,
            dragging: false,
        }
    }
    pub fn set(
        &mut self,
        text: String,
        highlights: Vec<(Range<usize>, HighlightStyle)>,
        links: Vec<(Range<usize>, String)>,
    ) {
        self.text = text;
        self.highlights = highlights;
        self.links = links;
        self.anchor = self.boundary(self.anchor);
        self.caret = self.boundary(self.caret);
    }
    fn boundary(&self, byte: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(self.text.len()))
            .take_while(|i| *i <= byte)
            .last()
            .unwrap_or(0)
    }
    pub fn has_selection(&self) -> bool {
        self.anchor != self.caret || self.dragging
    }
    fn selection(&self) -> Range<usize> {
        self.anchor.min(self.caret)..self.anchor.max(self.caret)
    }
}

impl Focusable for RichText {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for RichText {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.selection();
        let mut boundaries = BTreeSet::from([0, self.text.len(), selected.start, selected.end]);
        for (range, _) in &self.highlights {
            boundaries.insert(range.start);
            boundaries.insert(range.end);
        }
        let points: Vec<_> = boundaries.into_iter().collect();
        let default = window.text_style();
        let runs = points
            .windows(2)
            .filter(|p| p[0] < p[1])
            .map(|p| {
                let mut style = default.clone();
                if let Some((_, mark)) = self.highlights.iter().find(|(r, _)| r.contains(&p[0])) {
                    style = style.highlight(*mark);
                }
                if selected.contains(&p[0]) {
                    style = style.highlight(HighlightStyle {
                        background_color: Some(rgba(0x8198b866).into()),
                        ..Default::default()
                    });
                }
                style.to_run(p[1] - p[0])
            })
            .collect();
        let styled = StyledText::new(self.text.clone()).with_runs(runs);
        let down_layout = styled.layout().clone();
        let move_layout = down_layout.clone();
        let up_layout = down_layout.clone();
        div()
            .id("selectable-response")
            .key_context("ResponseText")
            .track_focus(&self.focus)
            .cursor_text()
            .on_action(cx.listener(|this, _: &Copy, _, cx| {
                let range = this.selection();
                if !range.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(this.text[range].to_owned()));
                }
            }))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| {
                this.anchor = 0;
                this.caret = this.text.len();
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    this.focus.focus(window);
                    let byte = down_layout
                        .index_for_position(event.position)
                        .unwrap_or_else(|i| i);
                    this.anchor = this.boundary(byte);
                    this.caret = this.anchor;
                    this.dragging = true;
                    if event.click_count >= 2
                        && let Some((offset, word)) = this
                            .text
                            .unicode_word_indices()
                            .find(|(i, word)| *i <= byte && byte <= *i + word.len())
                    {
                        this.anchor = offset;
                        this.caret = offset + word.len();
                        this.dragging = false;
                    }
                    cx.notify();
                }),
            )
            .on_mouse_move(
                cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                    if this.dragging {
                        this.caret = this.boundary(
                            move_layout
                                .index_for_position(event.position)
                                .unwrap_or_else(|i| i),
                        );
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, event: &gpui::MouseUpEvent, _, cx| {
                    if this.dragging && this.selection().is_empty() {
                        let byte = up_layout
                            .index_for_position(event.position)
                            .unwrap_or_else(|i| i);
                        if let Some((_, target)) =
                            this.links.iter().find(|(r, _)| r.contains(&byte))
                        {
                            if target.starts_with("https://")
                                || target.starts_with("http://")
                                || target.starts_with("mailto:")
                            {
                                cx.open_url(target);
                            } else if std::path::Path::new(target).is_absolute()
                                && let Ok(url) = url::Url::from_file_path(target)
                            {
                                cx.open_url(url.as_str());
                            }
                        }
                    }
                    this.dragging = false;
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.dragging = false;
                    cx.notify();
                }),
            )
            .w_full()
            .min_w(px(0.))
            .child(styled)
    }
}
