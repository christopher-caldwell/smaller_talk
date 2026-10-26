use std::{collections::BTreeSet, ops::Range};

use gpui::{
    App, ClipboardItem, Context, FocusHandle, Focusable, HighlightStyle, IntoElement, KeyBinding,
    MouseButton, Render, StyledText, WeakEntity, Window, actions, div, prelude::*, px, rgba,
};
use unicode_segmentation::UnicodeSegmentation;
use uuid::Uuid;

use crate::shell::ChatsView;

actions!(rich_text, [Copy, SelectAll]);

#[derive(Default)]
struct TextSelection {
    anchor: usize,
    caret: usize,
    dragging: bool,
}

impl TextSelection {
    fn clear(&mut self) {
        self.anchor = 0;
        self.caret = 0;
        self.dragging = false;
    }

    fn select_all(&mut self, text_len: usize) {
        self.anchor = 0;
        self.caret = text_len;
        self.dragging = false;
    }

    fn has_selection(&self) -> bool {
        self.anchor != self.caret || self.dragging
    }

    fn range(&self) -> Range<usize> {
        self.anchor.min(self.caret)..self.anchor.max(self.caret)
    }

    fn boundary(text: &str, byte: usize) -> usize {
        text.grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(text.len()))
            .take_while(|i| *i <= byte)
            .last()
            .unwrap_or(0)
    }
}

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
    selection: TextSelection,
    selection_owner: Option<(WeakEntity<ChatsView>, Uuid, Uuid, String)>,
}

impl RichText {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            text: String::new(),
            highlights: vec![],
            links: vec![],
            focus: cx.focus_handle(),
            selection: TextSelection::default(),
            selection_owner: None,
        }
    }
    pub fn coordinate_selection(
        &mut self,
        owner: WeakEntity<ChatsView>,
        chat: Uuid,
        message: Uuid,
        key: String,
    ) {
        self.selection_owner = Some((owner, chat, message, key));
    }
    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }
    fn coordinate_with_siblings(&self, cx: &mut Context<Self>) {
        let Some((owner, chat, active_message, active_key)) = self.selection_owner.clone() else {
            return;
        };
        let _ = owner.update(cx, |view, cx| {
            if let Some(state) = view.states.get(&chat) {
                for ((message, key), entity) in &state.rich {
                    if *message != active_message || key != &active_key {
                        entity.update(cx, |text, cx| {
                            if text.has_selection() {
                                text.clear_selection();
                                cx.notify();
                            }
                        });
                    }
                }
            }
        });
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
        self.selection.anchor = TextSelection::boundary(&self.text, self.selection.anchor);
        self.selection.caret = TextSelection::boundary(&self.text, self.selection.caret);
    }
    pub fn has_selection(&self) -> bool {
        self.selection.has_selection()
    }
}

impl Focusable for RichText {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for RichText {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.selection.range();
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
                let range = this.selection.range();
                if !range.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(this.text[range].to_owned()));
                }
            }))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| {
                this.coordinate_with_siblings(cx);
                this.selection.select_all(this.text.len());
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    this.coordinate_with_siblings(cx);
                    this.focus.focus(window, cx);
                    let byte = down_layout
                        .index_for_position(event.position)
                        .unwrap_or_else(|i| i);
                    this.selection.anchor = TextSelection::boundary(&this.text, byte);
                    this.selection.caret = this.selection.anchor;
                    this.selection.dragging = true;
                    if event.click_count >= 2
                        && let Some((offset, word)) = this
                            .text
                            .unicode_word_indices()
                            .find(|(i, word)| *i <= byte && byte <= *i + word.len())
                    {
                        this.selection.anchor = TextSelection::boundary(&this.text, offset);
                        this.selection.caret =
                            TextSelection::boundary(&this.text, offset + word.len());
                        this.selection.dragging = false;
                    }
                    cx.notify();
                }),
            )
            .on_mouse_move(
                cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                    if this.selection.dragging {
                        this.selection.caret = TextSelection::boundary(
                            &this.text,
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
                    if this.selection.dragging && this.selection.range().is_empty() {
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
                    this.selection.dragging = false;
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.selection.dragging = false;
                    cx.notify();
                }),
            )
            .w_full()
            .min_w(px(0.))
            .child(styled)
    }
}

#[cfg(test)]
mod tests {
    use super::TextSelection;

    #[test]
    fn clearing_selection_removes_copy_and_rewrite_selection_state() {
        let mut selection = TextSelection {
            anchor: 2,
            caret: 7,
            dragging: true,
        };
        assert!(selection.has_selection());
        assert_eq!(selection.range(), 2..7);

        selection.clear();

        assert!(!selection.has_selection());
        assert!(selection.range().is_empty());
        assert!(!selection.dragging);
    }

    #[test]
    fn selection_boundaries_do_not_split_grapheme_clusters() {
        let text = "a👩‍💻b";
        let grapheme_start = 1;
        let grapheme_end = text.len() - 1;

        assert_eq!(
            TextSelection::boundary(text, grapheme_start + 1),
            grapheme_start
        );
        assert_eq!(
            TextSelection::boundary(text, grapheme_end - 1),
            grapheme_start
        );
        assert_eq!(TextSelection::boundary(text, grapheme_end), grapheme_end);
    }

    #[test]
    fn select_all_replaces_prior_range_and_stops_dragging() {
        let mut selection = TextSelection {
            anchor: 5,
            caret: 8,
            dragging: true,
        };

        selection.select_all(12);

        assert_eq!(selection.range(), 0..12);
        assert!(selection.has_selection());
        assert!(!selection.dragging);
    }
}
