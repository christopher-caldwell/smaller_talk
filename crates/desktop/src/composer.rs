// SPDX-License-Identifier: Apache-2.0
// Adapted from GPUI 0.2.2 examples/input.rs; see ../LICENSE-GPUI-APACHE.
// A local prompt field using GPUI native input/IME hooks; no editor subsystem.
use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, KeyBinding,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
    SharedString, Style, TextRun, UTF16Selection, Window, WrappedLine, actions, div, fill, point,
    prelude::*, px, relative, rgb, rgba, size,
};
use unicode_segmentation::*;

actions!(
    composer,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        ShowCharacterPalette,
        Paste,
        Cut,
        Copy,
        Undo,
        Redo,
        Up,
        Down,
        SelectUp,
        SelectDown,
        SelectHome,
        SelectEnd,
        Submit,
        Cancel,
        Newline,
    ]
);

pub struct Changed;
impl EventEmitter<Changed> for Composer {}

pub struct Submitted;
impl EventEmitter<Submitted> for Composer {}
pub struct Canceled;
impl EventEmitter<Canceled> for Composer {}
pub struct Navigate(pub i8);
impl EventEmitter<Navigate> for Composer {}

#[derive(Clone)]
struct EditSnapshot {
    content: SharedString,
    range: Range<usize>,
    reversed: bool,
}

pub struct Composer {
    focus_handle: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Vec<(usize, WrappedLine, Point<Pixels>)>,
    undo: Vec<EditSnapshot>,
    redo: Vec<EditSnapshot>,
    scroll: Point<Pixels>,
    last_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
    picker_navigation: bool,
    single_line: bool,
    max_rows: usize,
    font_size: f32,
    content_height: Pixels,
    reveal_cursor: bool,
}

impl Composer {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self::with_content("", false, cx)
    }

    pub fn new_single_line(cx: &mut Context<Self>) -> Self {
        Self::with_content("", true, cx)
    }

    fn with_content(content: &str, single_line: bool, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: content.to_owned().into(),
            placeholder: "Type a message…".into(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: Vec::new(),
            last_bounds: None,
            is_selecting: false,
            picker_navigation: false,
            single_line,
            max_rows: 7,
            font_size: 16.,
            content_height: px(24.),
            reveal_cursor: true,
            undo: Vec::new(),
            redo: Vec::new(),
            scroll: point(px(0.), px(0.)),
        }
    }

    pub fn set_content(&mut self, content: String, window: &mut Window, cx: &mut Context<Self>) {
        self.reveal_cursor = true;
        self.content = content.into();
        self.selected_range = self.content.len()..self.content.len();
        self.selection_reversed = false;
        self.marked_range = None;
        self.last_layout.clear();
        self.focus_handle.focus(window);
        cx.emit(Changed);
        cx.notify();
    }

    pub fn draft(
        &self,
        attachments: Vec<smaller_talk_domain::Attachment>,
    ) -> smaller_talk_domain::Draft {
        smaller_talk_domain::Draft {
            text: self.content.to_string(),
            attachments,
            selection_start: self.selected_range.start,
            selection_end: self.selected_range.end,
        }
    }
    pub fn load_draft(&mut self, draft: &smaller_talk_domain::Draft, cx: &mut Context<Self>) {
        self.reveal_cursor = true;
        self.scroll = point(px(0.), px(0.));
        self.content = draft.text.clone().into();
        let boundary = |mut n: usize| {
            n = n.min(self.content.len());
            while !self.content.is_char_boundary(n) {
                n -= 1;
            }
            n
        };
        self.selected_range = boundary(draft.selection_start)
            ..boundary(draft.selection_end).max(boundary(draft.selection_start));
        self.marked_range = None;
        self.undo.clear();
        self.redo.clear();
        self.last_layout.clear();
        cx.notify();
    }
    pub fn set_max_rows(&mut self, rows: usize, cx: &mut Context<Self>) {
        if self.max_rows != rows {
            self.max_rows = rows;
            cx.notify();
        }
    }
    pub fn set_placeholder(&mut self, text: &str) {
        self.placeholder = text.to_owned().into();
    }

    pub fn set_font_size(&mut self, font_size: f32) {
        self.font_size = font_size;
    }

    pub fn content(&self) -> String {
        self.content.to_string()
    }

    fn snapshot(&self) -> EditSnapshot {
        EditSnapshot {
            content: self.content.clone(),
            range: self.selected_range.clone(),
            reversed: self.selection_reversed,
        }
    }

    fn remember_edit(&mut self) {
        self.reveal_cursor = true;
        // Small, local history for the prompt field, including selection state.
        if self.undo.len() == 100 {
            self.undo.remove(0);
        }
        self.undo.push(self.snapshot());
        self.redo.clear();
    }

    fn restore(&mut self, edit: EditSnapshot, cx: &mut Context<Self>) {
        self.reveal_cursor = true;
        self.content = edit.content;
        self.selected_range = edit.range;
        self.selection_reversed = edit.reversed;
        self.marked_range = None;
        cx.emit(Changed);
        cx.notify();
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(edit) = self.undo.pop() {
            self.redo.push(self.snapshot());
            self.restore(edit, cx);
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(edit) = self.redo.pop() {
            self.undo.push(self.snapshot());
            self.restore(edit, cx);
        }
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        if self.marked_range.is_none() {
            cx.emit(Submitted);
        }
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(Canceled);
    }

    fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if !self.single_line {
            self.replace_text_in_range(None, "\n", window, cx);
        }
    }

    fn vertical(&mut self, down: bool, select: bool, cx: &mut Context<Self>) {
        let cursor = self.cursor_offset();
        if let Some((start, line, origin)) = self
            .last_layout
            .iter()
            .rev()
            .find(|(start, _, _)| *start <= cursor)
            && let Some(position) = line.position_for_index(cursor.saturating_sub(*start), px(24.))
        {
            let target = *origin
                + point(
                    position.x,
                    position.y + if down { px(25.) } else { px(-1.) },
                );
            let offset = self.index_for_mouse_position(target);
            if select {
                self.select_to(offset, cx);
            } else {
                self.move_to(offset, cx);
            }
            return;
        }
        let start = self.content[..cursor].rfind('\n').map_or(0, |i| i + 1);
        let column = self.content[start..cursor].graphemes(true).count();
        let target = if down {
            self.content[cursor..].find('\n').map(|i| cursor + i + 1)
        } else if start > 0 {
            Some(self.content[..start - 1].rfind('\n').map_or(0, |i| i + 1))
        } else {
            None
        };
        if let Some(start) = target {
            let end = self.content[start..]
                .find('\n')
                .map_or(self.content.len(), |i| start + i);
            let offset = self.content[start..end]
                .grapheme_indices(true)
                .nth(column)
                .map_or(end, |(i, _)| start + i);
            if select {
                self.select_to(offset, cx);
            } else {
                self.move_to(offset, cx);
            }
        }
    }
    pub fn set_picker_navigation(&mut self, enabled: bool) {
        self.picker_navigation = enabled;
    }
    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        if self.picker_navigation {
            cx.emit(Navigate(-1));
            return;
        }
        self.vertical(false, false, cx);
    }
    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        if self.picker_navigation {
            cx.emit(Navigate(1));
            return;
        }
        self.vertical(true, false, cx);
    }
    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(false, true, cx);
    }
    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(true, true, cx);
    }
    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx);
    }
    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.content.len(), cx);
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx)
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx)
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx)
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(_window);
        self.is_selecting = true;

        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text.replace("\r\n", "\n"), window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }
    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx)
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.reveal_cursor = true;
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        cx.notify()
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }

        let Some((start, line, origin)) = self
            .last_layout
            .iter()
            .rev()
            .find(|(_, _, origin)| position.y >= origin.y)
            .or(self.last_layout.first())
        else {
            return 0;
        };
        let local = line
            .closest_index_for_position(
                point(position.x - origin.x, position.y - origin.y),
                px(24.),
            )
            .unwrap_or_else(|index| index);
        start + local
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.reveal_cursor = true;
        if self.selection_reversed {
            self.selected_range.start = offset
        } else {
            self.selected_range.end = offset
        };
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify()
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;

        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }

        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;

        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }

        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.reveal_cursor = true;
        self.content = "".into();
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.marked_range = None;
        self.last_layout.clear();
        self.undo.clear();
        self.redo.clear();
        self.scroll = point(px(0.), px(0.));
        cx.emit(Changed);
        cx.notify();
        self.last_bounds = None;
        self.is_selecting = false;
    }
}

impl EntityInputHandler for Composer {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        let single_line_text;
        let new_text = if self.single_line {
            single_line_text = new_text.replace(['\n', '\r'], "");
            single_line_text.as_str()
        } else {
            new_text
        };

        if self.marked_range.is_none() {
            self.remember_edit();
        }
        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.marked_range.take();
        cx.emit(Changed);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        if self.marked_range.is_none() {
            self.remember_edit();
        }
        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        if !new_text.is_empty() {
            self.marked_range = Some(range.start..range.start + new_text.len());
        } else {
            self.marked_range = None;
        }
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| {
                let marked = &self.content[range.start..range.start + new_text.len()];
                let offset = |n| {
                    marked
                        .chars()
                        .scan(0, |count, ch| {
                            let before = *count;
                            *count += ch.len_utf16();
                            Some((before, ch.len_utf8()))
                        })
                        .take_while(|(before, _)| *before < n)
                        .map(|(_, len)| len)
                        .sum::<usize>()
                };
                range.start + offset(range_utf16.start)..range.start + offset(range_utf16.end)
            })
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());

        cx.emit(Changed);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);
        let (start, line, origin) = self
            .last_layout
            .iter()
            .rev()
            .find(|(start, _, _)| *start <= range.start)?;
        let local_start = range.start.saturating_sub(*start).min(line.text.len());
        let local_end = range.end.saturating_sub(*start).min(line.text.len());
        let left = line.position_for_index(local_start, px(24.))?;
        let right = line.position_for_index(local_end, px(24.))?;
        Some(Bounds::new(
            point(origin.x + left.x, origin.y + left.y),
            size(
                (right.x - left.x).max(px(1.)),
                bounds.size.height.min(px(24.)),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.offset_to_utf16(self.index_for_mouse_position(point)))
    }
}

struct TextElement {
    input: Entity<Composer>,
}

struct PrepaintState {
    lines: Vec<(usize, WrappedLine, Point<Pixels>)>,
    cursor: Option<PaintQuad>,
    selections: Vec<PaintQuad>,
    scrollbar: Option<PaintQuad>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        let input = self.input.read(cx);
        let text = if input.content.is_empty() {
            input.placeholder.clone()
        } else {
            input.content.clone()
        };
        let single_line = input.single_line;
        let max_rows = input.max_rows;
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let layout = window.request_measured_layout(style, move |known, available, window, _| {
            let width = known
                .width
                .or(match available.width {
                    gpui::AvailableSpace::Definite(w) => Some(w),
                    _ => None,
                })
                .unwrap_or(px(400.))
                .max(px(1.));
            let rows: usize = text
                .split('\n')
                .map(|line| {
                    let run = text_style.to_run(line.len());
                    window
                        .text_system()
                        .shape_text(line.to_owned().into(), font_size, &[run], Some(width), None)
                        .map(|lines| {
                            lines
                                .iter()
                                .map(|l| l.wrap_boundaries().len() + 1)
                                .sum::<usize>()
                        })
                        .unwrap_or(1)
                })
                .sum();
            size(
                width,
                px(if single_line {
                    24.
                } else {
                    rows.clamp(1, max_rows) as f32 * 24.
                }),
            )
        });
        (layout, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let style = window.text_style();
        let text = if input.content.is_empty() {
            input.placeholder.clone()
        } else {
            input.content.clone()
        };
        let mut lines: Vec<(usize, WrappedLine, Point<Pixels>)> = Vec::new();
        let mut start = 0;
        let mut visual_rows = 0;
        let font_size = style.font_size.to_pixels(window.rem_size());
        for text in text.split('\n') {
            let run = TextRun {
                len: text.len(),
                font: style.font(),
                color: if input.content.is_empty() {
                    rgb(super::MUTED).into()
                } else {
                    style.color
                },
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let line = window
                .text_system()
                .shape_text(
                    text.to_owned().into(),
                    font_size,
                    &[run],
                    (!input.single_line).then_some(bounds.size.width),
                    None,
                )
                .ok()
                .and_then(|mut lines| lines.pop())
                .unwrap_or_default();
            let row_count = line.wrap_boundaries().len() + 1;
            lines.push((
                start,
                line,
                point(bounds.left(), bounds.top() + px(24.) * visual_rows as f32),
            ));
            visual_rows += row_count;
            start += text.len() + 1;
        }
        let cursor = input.cursor_offset();
        let line_index = lines
            .iter()
            .rposition(|(start, _, _)| *start <= cursor)
            .unwrap_or(0);
        let cursor_position = lines[line_index]
            .1
            .position_for_index(cursor.saturating_sub(lines[line_index].0), px(24.))
            .unwrap_or_default();
        let cursor_y = lines[line_index].2.y - bounds.top() + cursor_position.y;
        let content_height = px(lines
            .iter()
            .map(|(_, line, _)| line.wrap_boundaries().len() + 1)
            .sum::<usize>() as f32
            * 24.);
        let mut scroll = input.scroll;
        let resized = input
            .last_bounds
            .is_none_or(|previous| previous.size != bounds.size);
        if input.single_line {
            scroll.y = px(0.);
            if input.reveal_cursor || resized {
                if cursor_position.x < scroll.x {
                    scroll.x = cursor_position.x;
                }
                if cursor_position.x + px(2.) > scroll.x + bounds.size.width {
                    scroll.x = cursor_position.x + px(2.) - bounds.size.width;
                }
            }
        } else {
            scroll.x = px(0.);
            if input.reveal_cursor || resized {
                if cursor_y < scroll.y {
                    scroll.y = cursor_y;
                }
                if cursor_y + px(24.) > scroll.y + bounds.size.height {
                    scroll.y = cursor_y + px(24.) - bounds.size.height;
                }
            }
            scroll.y = scroll
                .y
                .clamp(px(0.), (content_height - bounds.size.height).max(px(0.)));
        }
        let mut selections = Vec::new();
        for (start, line, origin) in &mut lines {
            *origin -= scroll;
            for (offset, character) in line.text.char_indices() {
                let index = *start + offset;
                if index >= input.selected_range.start && index < input.selected_range.end {
                    let next = offset + character.len_utf8();
                    let Some(left) = line.position_for_index(offset, px(24.)) else {
                        continue;
                    };
                    let Some(right) = line.position_for_index(next, px(24.)) else {
                        continue;
                    };
                    selections.push(fill(
                        Bounds::new(
                            point(origin.x + left.x, origin.y + left.y),
                            size(
                                if right.y > left.y {
                                    (bounds.size.width - left.x).max(px(2.))
                                } else {
                                    (right.x - left.x).max(px(2.))
                                },
                                px(24.),
                            ),
                        ),
                        rgba(0x84d7c155),
                    ));
                }
            }
        }
        let cursor = input.selected_range.is_empty().then(|| {
            fill(
                Bounds::new(
                    point(
                        lines[line_index].2.x + cursor_position.x,
                        lines[line_index].2.y + cursor_position.y,
                    ),
                    size(px(2.), px(24.)),
                ),
                rgb(super::ACCENT),
            )
        });
        let scrollbar = (!input.single_line && content_height > bounds.size.height).then(|| {
            let height = (bounds.size.height
                * (f32::from(bounds.size.height) / f32::from(content_height)))
            .max(px(12.));
            let travel = bounds.size.height - height;
            let offset =
                travel * (f32::from(scroll.y) / f32::from(content_height - bounds.size.height));
            fill(
                Bounds::new(
                    point(bounds.right() - px(3.), bounds.top() + offset),
                    size(px(2.), height),
                ),
                rgba(0x959ca780),
            )
        });
        self.input.update(cx, |input, _| {
            input.scroll = scroll;
            input.content_height = content_height;
            input.reveal_cursor = false;
        });
        PrepaintState {
            lines,
            cursor,
            selections,
            scrollbar,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for selection in prepaint.selections.drain(..) {
                window.paint_quad(selection);
            }
            for (_, line, origin) in &prepaint.lines {
                let height = px(24.) * (line.wrap_boundaries().len() + 1) as f32;
                if origin.y >= bounds.bottom() || origin.y + height <= bounds.top() {
                    continue;
                }
                line.paint(
                    *origin,
                    px(24.),
                    gpui::TextAlign::Left,
                    Some(bounds),
                    window,
                    cx,
                )
                .expect("composer text paint");
            }
            if let Some(scrollbar) = prepaint.scrollbar.take() {
                window.paint_quad(scrollbar);
            }
            if focus_handle.is_focused(window)
                && let Some(cursor) = prepaint.cursor.take()
            {
                window.paint_quad(cursor);
            }
        });
        self.input.update(cx, |input, _| {
            input.last_layout = std::mem::take(&mut prepaint.lines);
            input.last_bounds = Some(bounds);
        });
    }
}

impl Render for Composer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .key_context("Composer")
            .tab_index(0)
            .focus(|s| s.bg(rgb(crate::theme::COMPOSER)))
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .capture_key_down(
                cx.listener(|this, _: &gpui::KeyDownEvent, _, _| this.reveal_cursor = true),
            )
            .on_scroll_wheel(cx.listener(|this, event: &gpui::ScrollWheelEvent, _, cx| {
                if !this.single_line
                    && let Some(bounds) = this.last_bounds
                {
                    let max = (this.content_height - bounds.size.height).max(px(0.));
                    if max > px(0.) {
                        this.scroll.y =
                            (this.scroll.y - event.delta.pixel_delta(px(24.)).y).clamp(px(0.), max);
                        this.reveal_cursor = false;
                        cx.stop_propagation();
                        cx.notify();
                    }
                }
            }))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .w_full()
            .min_w(px(0.))
            .text_color(rgb(super::TEXT))
            .line_height(px(24.))
            .text_size(px(self.font_size))
            .child(div().w_full().child(TextElement { input: cx.entity() }))
    }
}

impl Focusable for Composer {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, Some("Composer")),
        KeyBinding::new("delete", Delete, Some("Composer")),
        KeyBinding::new("left", Left, Some("Composer")),
        KeyBinding::new("right", Right, Some("Composer")),
        KeyBinding::new("shift-left", SelectLeft, Some("Composer")),
        KeyBinding::new("shift-right", SelectRight, Some("Composer")),
        KeyBinding::new("cmd-a", SelectAll, Some("Composer")),
        KeyBinding::new("cmd-v", Paste, Some("Composer")),
        KeyBinding::new("cmd-c", Copy, Some("Composer")),
        KeyBinding::new("cmd-x", Cut, Some("Composer")),
        KeyBinding::new("home", Home, Some("Composer")),
        KeyBinding::new("end", End, Some("Composer")),
        KeyBinding::new("cmd-left", Home, Some("Composer")),
        KeyBinding::new("cmd-right", End, Some("Composer")),
        KeyBinding::new("shift-home", SelectHome, Some("Composer")),
        KeyBinding::new("shift-end", SelectEnd, Some("Composer")),
        KeyBinding::new("cmd-shift-left", SelectHome, Some("Composer")),
        KeyBinding::new("cmd-shift-right", SelectEnd, Some("Composer")),
        KeyBinding::new("up", Up, Some("Composer")),
        KeyBinding::new("down", Down, Some("Composer")),
        KeyBinding::new("shift-up", SelectUp, Some("Composer")),
        KeyBinding::new("shift-down", SelectDown, Some("Composer")),
        KeyBinding::new("cmd-z", Undo, Some("Composer")),
        KeyBinding::new("cmd-shift-z", Redo, Some("Composer")),
        KeyBinding::new("enter", Submit, Some("Composer")),
        KeyBinding::new("cmd-enter", Submit, Some("Composer")),
        KeyBinding::new("escape", Cancel, Some("Composer")),
        KeyBinding::new("shift-enter", Newline, Some("Composer")),
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, Some("Composer")),
    ]);
}
