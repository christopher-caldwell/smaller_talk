// The app owns skill identity and durable drafts; GPUI Kit owns text editing.
use gpui::{
    App, AppContext, Context, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    IntoElement, ParentElement, Render, Window, div, prelude::*, px, rgb,
};
use gpui_base::input::{
    InlineToken, Input, InputContent, InputEditorStyle, InputState, Textarea, TextareaState,
};
use smaller_talk_domain::{
    Draft, SkillDescriptor, SkillMention, SkillReference, edit_skill_mentions, skill_tokens,
};
use std::ops::Range;

fn skill_query_at_cursor(
    content: &str,
    cursor: usize,
    skills: &[SkillMention],
) -> Option<(Range<usize>, String)> {
    let start = content[..cursor]
        .char_indices()
        .rfind(|(_, character)| character.is_whitespace())
        .map_or(0, |(i, character)| i + character.len_utf8());
    let token = &content[start..cursor];
    let query = token.strip_prefix('$')?;
    if !query.is_empty() && !query.bytes().any(|c| c.is_ascii_lowercase()) {
        return None;
    }
    if !query
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || b"_:-".contains(&c))
        || skills.iter().any(|m| m.range == (start..cursor))
    {
        return None;
    }
    let probe = format!("{}x ", &content[..cursor]);
    if !skill_tokens(&probe).iter().any(|(r, _)| r.start == start) {
        return None;
    }
    Some((start..cursor, query.into()))
}

pub struct Changed;
impl EventEmitter<Changed> for Composer {}
pub struct Submitted;
impl EventEmitter<Submitted> for Composer {}
pub struct Canceled;
impl EventEmitter<Canceled> for Composer {}
pub struct Navigate(pub i8);
impl EventEmitter<Navigate> for Composer {}

#[derive(Clone)]
enum Editor {
    Input(Entity<InputState>),
    Textarea(Entity<TextareaState>),
}

// The two purpose-specific controls share the same editing API.
macro_rules! update_editor {
    ($editor:expr, $cx:expr, |$state:ident, $inner:ident| $body:expr) => {
        match $editor {
            Editor::Input(input) => input.update($cx, |$state, $inner| $body),
            Editor::Textarea(input) => input.update($cx, |$state, $inner| $body),
        }
    };
}

enum Edit {
    Load(Draft),
    Token(Range<usize>, SkillReference),
    Skill {
        range: Range<usize>,
        reference: SkillReference,
        prefix: bool,
        suffix: bool,
    },
    Remove(Range<usize>),
}

struct Snapshot {
    text: String,
    skills: Vec<SkillMention>,
    selection: Range<usize>,
}

macro_rules! snapshot {
    ($state:expr) => {{
        let state = $state;
        Snapshot {
            text: state.value().to_string(),
            skills: state
                .tokens()
                .iter()
                .filter_map(|span| {
                    serde_json::from_str(span.token().id())
                        .ok()
                        .map(|reference| SkillMention {
                            range: span.range(),
                            reference,
                        })
                })
                .collect(),
            selection: state.selected_range(),
        }
    }};
}

fn token(reference: &SkillReference) -> InlineToken {
    InlineToken::new(
        serde_json::to_string(reference).expect("skill references serialize"),
        format!("${}", reference.name),
    )
}

fn render_skill_token(
    token: &gpui_base::input::InlineTokenContext,
    _: &mut Window,
    _: &mut App,
) -> gpui::Div {
    div()
        .h(token.line_height())
        .max_w(token.available_width())
        .text_color(rgb(crate::theme::ACCENT))
        .bg(gpui::rgba(if token.is_selected() {
            0x84d7c155
        } else {
            0x6798cf15
        }))
        .child(token.token().label().clone())
}

fn draft_content(draft: &Draft) -> InputContent {
    let mut content = InputContent::new(draft.text.clone());
    for mention in draft
        .skills
        .iter()
        .filter(|mention| mention.matches(&draft.text))
    {
        if let Ok(next) = content
            .clone()
            .with_token(mention.range.clone(), token(&mention.reference))
        {
            content = next;
        }
    }
    content
}

pub struct Composer {
    editor: Option<Editor>,
    focus_handle: FocusHandle,
    content: String,
    skills: Vec<SkillMention>,
    selected_range: Range<usize>,
    pending: Vec<Edit>,
    placeholder: String,
    editable: bool,
    single_line: bool,
    enter_inserts_newline: bool,
    picker_navigation: bool,
    composing: bool,
    max_rows: usize,
    font_size: f32,
    configuration_dirty: bool,
}

impl Composer {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self::with_mode(false, cx)
    }
    pub fn new_single_line(cx: &mut Context<Self>) -> Self {
        Self::with_mode(true, cx)
    }
    pub fn new_editor(cx: &mut Context<Self>) -> Self {
        let mut input = Self::new(cx);
        input.enter_inserts_newline = true;
        input.max_rows = 10;
        input.font_size = 14.;
        input
    }
    fn with_mode(single_line: bool, cx: &mut Context<Self>) -> Self {
        Self {
            editor: None,
            focus_handle: cx.focus_handle(),
            content: String::new(),
            skills: Vec::new(),
            selected_range: 0..0,
            pending: Vec::new(),
            placeholder: "Type a message…".into(),
            editable: true,
            single_line,
            enter_inserts_newline: false,
            picker_navigation: false,
            composing: false,
            max_rows: 7,
            font_size: 16.,
            configuration_dirty: true,
        }
    }
    fn accept_snapshot(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        if !self.pending.is_empty() {
            return;
        }
        let changed = self.content != snapshot.text || self.skills != snapshot.skills;
        self.content = snapshot.text;
        self.skills = snapshot.skills;
        self.selected_range = snapshot.selection;
        if changed {
            cx.emit(Changed);
        }
        cx.notify();
    }
    fn initialize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.is_some() {
            return;
        }
        let had_focus = self.focus_handle.is_focused(window);
        let editor = if self.single_line {
            let input = cx.new(|cx| InputState::new(window, cx));
            cx.observe_in(&input, window, |this, input, window, cx| {
                this.composing = input.update(cx, |state, cx| {
                    state.marked_text_range(window, cx).is_some()
                });
                this.accept_snapshot(snapshot!(input.read(cx)), cx);
            })
            .detach();
            self.focus_handle = input.focus_handle(cx);
            Editor::Input(input)
        } else {
            let input = cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(1, self.max_rows)
                    .submit_on_enter(!self.enter_inserts_newline)
            });
            cx.observe_in(&input, window, |this, input, window, cx| {
                this.composing = input.update(cx, |state, cx| {
                    state.marked_text_range(window, cx).is_some()
                });
                this.accept_snapshot(snapshot!(input.read(cx)), cx);
            })
            .detach();
            self.focus_handle = input.focus_handle(cx);
            Editor::Textarea(input)
        };
        if self.pending.is_empty() {
            self.pending.push(Edit::Load(self.draft(Vec::new())));
        }
        self.editor = Some(editor);
        if had_focus {
            self.focus_handle.focus(window, cx);
        }
        // Refresh host frames that captured the placeholder focus handle.
        cx.emit(Changed);
        cx.notify();
    }
    pub fn set_content(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let end = text.len();
        self.load_draft(
            &Draft {
                text,
                selection_start: end,
                selection_end: end,
                ..Default::default()
            },
            cx,
        );
        self.focus_handle.focus(window, cx);
        cx.emit(Changed);
    }
    pub fn draft(&self, attachments: Vec<smaller_talk_domain::Attachment>) -> Draft {
        Draft {
            text: self.content.clone(),
            skills: self.skills.clone(),
            attachments,
            selection_start: self.selected_range.start,
            selection_end: self.selected_range.end,
        }
    }
    pub fn load_draft(&mut self, draft: &Draft, cx: &mut Context<Self>) {
        self.content = draft.text.clone();
        self.skills = draft
            .skills
            .iter()
            .filter(|m| m.matches(&draft.text))
            .cloned()
            .collect();
        let boundary = |n: usize| {
            let mut n = n.min(draft.text.len());
            while !draft.text.is_char_boundary(n) {
                n -= 1;
            }
            n
        };
        self.selected_range = boundary(draft.selection_start)
            ..boundary(draft.selection_end).max(boundary(draft.selection_start));
        self.pending.clear();
        self.pending
            .push(Edit::Load(self.draft(draft.attachments.clone())));
        self.composing = false;
        cx.notify();
    }
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.load_draft(&Draft::default(), cx);
        cx.emit(Changed);
    }
    pub fn set_max_rows(&mut self, rows: usize, cx: &mut Context<Self>) {
        if self.max_rows != rows {
            self.max_rows = rows;
            self.configuration_dirty = true;
            cx.notify();
        }
    }
    pub fn set_placeholder(&mut self, text: &str) {
        self.placeholder = text.into();
        self.configuration_dirty = true;
    }
    pub fn set_font_size(&mut self, size: f32) {
        self.font_size = size;
    }
    pub fn set_editable(&mut self, editable: bool, cx: &mut Context<Self>) {
        self.editable = editable;
        self.configuration_dirty = true;
        cx.notify();
    }
    pub fn set_picker_navigation(&mut self, enabled: bool) {
        self.picker_navigation = enabled;
    }
    pub fn content(&self) -> String {
        self.content.clone()
    }
    pub fn skills(&self) -> &[SkillMention] {
        &self.skills
    }
    pub fn composing(&self) -> bool {
        self.composing
    }
    pub fn selection(&self) -> Range<usize> {
        self.selected_range.clone()
    }
    pub fn skill_query(&self) -> Option<(Range<usize>, String)> {
        if self.composing || !self.selected_range.is_empty() {
            return None;
        }
        skill_query_at_cursor(&self.content, self.selected_range.end, &self.skills)
    }
    pub fn select_skill(
        &mut self,
        range: Range<usize>,
        reference: SkillReference,
        cx: &mut Context<Self>,
    ) {
        if !self.editable || self.composing || self.content.get(range.clone()).is_none() {
            return;
        }
        let prefix = self.content[..range.start]
            .chars()
            .next_back()
            .is_some_and(|c| !c.is_whitespace());
        let suffix = self.content[range.end..]
            .chars()
            .next()
            .is_some_and(|c| !c.is_whitespace());
        let text = format!(
            "{}${}{}",
            if prefix { " " } else { "" },
            reference.name,
            if suffix { " " } else { "" }
        );
        self.pending.push(Edit::Skill {
            range: range.clone(),
            reference: reference.clone(),
            prefix,
            suffix,
        });
        edit_skill_mentions(&mut self.skills, range.clone(), &text);
        self.content.replace_range(range.clone(), &text);
        let start = range.start + usize::from(prefix);
        self.skills.push(SkillMention {
            range: start..start + reference.name.len() + 1,
            reference,
        });
        self.skills.sort_by_key(|m| m.range.start);
        self.selected_range = range.start + text.len()..range.start + text.len();
        cx.emit(Changed);
        cx.notify();
    }
    pub fn resolve_known_skills(
        &mut self,
        entries: &[SkillDescriptor],
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.editable || self.composing() {
            return false;
        }
        let mut changed = false;
        let links = smaller_talk_application::resolve_skill_links(&self.content, entries);
        for (range, reference) in links.into_iter().rev() {
            let token = format!("${}", reference.name);
            self.pending
                .push(Edit::Token(range.clone(), reference.clone()));
            edit_skill_mentions(&mut self.skills, range.clone(), &token);
            self.content =
                self.content[..range.start].to_owned() + &token + &self.content[range.end..];
            self.skills.push(SkillMention {
                range: range.start..range.start + token.len(),
                reference,
            });
            let adjust = |cursor: usize| {
                if cursor >= range.end {
                    cursor - range.len() + token.len()
                } else if cursor > range.start {
                    range.start + token.len()
                } else {
                    cursor
                }
            };
            self.selected_range =
                adjust(self.selected_range.start)..adjust(self.selected_range.end);
            changed = true;
        }
        for (range, name) in skill_tokens(&self.content) {
            if self.skills.iter().any(|m| m.range == range) {
                continue;
            }
            if let smaller_talk_application::SkillNameResolution::Selected(reference) =
                smaller_talk_application::resolve_skill_name(entries, &name)
            {
                self.pending
                    .push(Edit::Token(range.clone(), reference.clone()));
                self.skills.push(SkillMention { range, reference });
                changed = true;
            }
        }
        if changed {
            self.skills.sort_by_key(|m| m.range.start);
            cx.notify();
        }
        changed
    }
    pub fn remove_skill(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        if !self.editable || self.composing || self.content.get(range.clone()).is_none() {
            return;
        }
        self.pending.push(Edit::Remove(range.clone()));
        edit_skill_mentions(&mut self.skills, range.clone(), "");
        self.content.replace_range(range.clone(), "");
        self.selected_range = range.start..range.start;
        cx.emit(Changed);
        cx.notify();
    }
}

impl Render for Composer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.initialize(window, cx);
        let editor = self.editor.clone().expect("initialized input");
        let style = InputEditorStyle {
            foreground: rgb(crate::theme::TEXT).into(),
            muted_foreground: rgb(crate::theme::MUTED).into(),
            background: rgb(crate::theme::COMPOSER).into(),
            caret: rgb(crate::theme::ACCENT).into(),
            selection: gpui::rgba(0x84d7c155).into(),
            ..Default::default()
        };
        update_editor!(&editor, cx, |state, cx| {
            state.set_editor_style(style);
            if self.configuration_dirty {
                state.set_placeholder(self.placeholder.clone(), window, cx);
                state.set_readonly(!self.editable, cx);
            }
            let pending = std::mem::take(&mut self.pending);
            let edited = !pending.is_empty();
            for edit in pending {
                match edit {
                    Edit::Load(draft) => {
                        state.set_value(draft_content(&draft), window, cx);
                        state.set_selected_range(draft.selection_start..draft.selection_end, cx);
                    }
                    Edit::Token(range, reference) => {
                        let _ =
                            state.replace_range_with_token(range, token(&reference), window, cx);
                    }
                    Edit::Skill {
                        range,
                        reference,
                        prefix,
                        suffix,
                    } => {
                        state.transact(window, cx, |state, window, cx| {
                            if suffix {
                                state.set_selected_range(range.end..range.end, cx);
                                state.insert(" ", window, cx);
                            }
                            if prefix {
                                state.set_selected_range(range.start..range.start, cx);
                                state.insert(" ", window, cx);
                            }
                            let start = range.start + usize::from(prefix);
                            let end = range.end + usize::from(prefix);
                            state
                                .replace_range_with_token(start..end, token(&reference), window, cx)
                                .expect("validated skill token outside IME composition");
                            let cursor = start + reference.name.len() + 1 + usize::from(suffix);
                            state.set_selected_range(cursor..cursor, cx);
                        });
                    }
                    Edit::Remove(range) => {
                        state.set_selected_range(range, cx);
                        state.replace("", window, cx);
                    }
                }
            }
            if edited {
                state.set_selected_range(self.selected_range.clone(), cx);
            }
            self.composing = state.marked_text_range(window, cx).is_some();
        });
        if self.configuration_dirty
            && let Editor::Textarea(input) = &editor
        {
            input.update(cx, |state, cx| {
                state.set_auto_grow(1, self.max_rows, cx);
            });
        }
        self.configuration_dirty = false;
        let child = match editor {
            Editor::Input(input) => Input::new(&input)
                .token(render_skill_token)
                .into_any_element(),
            Editor::Textarea(input) => Textarea::new(&input)
                .token(render_skill_token)
                .into_any_element(),
        };
        div()
            .w_full()
            .min_w(px(0.))
            .text_size(px(self.font_size))
            .line_height(px(24.))
            .on_action(
                cx.listener(|this, action: &gpui_base::input::Enter, _, cx| {
                    // Kit propagates Enter so the host can submit. Consume it here
                    // after the input has handled Shift+Enter and IME.
                    if this.editable
                        && !this.enter_inserts_newline
                        && !action.shift
                        && !this.composing
                    {
                        cx.emit(Submitted);
                    }
                    cx.stop_propagation();
                }),
            )
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                if key == "escape" && !this.composing {
                    cx.emit(Canceled);
                } else if this.picker_navigation && matches!(key, "up" | "down") {
                    cx.emit(Navigate(if key == "up" { -1 } else { 1 }));
                } else {
                    return;
                }
                window.prevent_default();
                cx.stop_propagation();
            }))
            .child(child)
    }
}
impl Focusable for Composer {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(test)]
mod skill_query_tests {
    use super::skill_query_at_cursor;

    #[test]
    fn unicode_whitespace_never_splits_a_character() {
        for whitespace in ['\u{00a0}', '\u{2003}', '\u{3000}'] {
            for text in [
                format!("hello{whitespace}world"),
                format!("hello{whitespace}$review"),
            ] {
                assert_eq!(skill_query_at_cursor(&text, text.len(), &[]), None);
            }
            let text = format!("🦀{whitespace}world $review");
            let cursor = text.len();
            assert_eq!(
                skill_query_at_cursor(&text, cursor, &[]),
                Some((cursor - 7..cursor, "review".into()))
            );
        }
    }
}

#[cfg(test)]
mod editing_tests {
    use super::*;

    fn view(
        cx: &mut gpui::TestAppContext,
        single_line: bool,
    ) -> (gpui::WindowHandle<Composer>, Entity<Composer>) {
        cx.update(gpui_base::init);
        let window = cx.add_window(move |window, cx| {
            let mut composer = if single_line {
                Composer::new_single_line(cx)
            } else {
                Composer::new(cx)
            };
            composer.initialize(window, cx);
            composer.focus_handle.focus(window, cx);
            composer
        });
        let entity = window.root(cx).unwrap();
        cx.run_until_parked();
        (window, entity)
    }

    #[gpui::test]
    fn option_arrows_select_and_replace_words_in_both_input_modes(cx: &mut gpui::TestAppContext) {
        for single_line in [false, true] {
            let (window, input) = view(cx, single_line);
            cx.simulate_input(window.into(), "alpha café");
            cx.simulate_keystrokes(window.into(), "alt-left");
            assert_eq!(input.read_with(cx, |input, _| input.selection()), 6..6);
            cx.simulate_keystrokes(window.into(), "alt-shift-right");
            assert_eq!(input.read_with(cx, |input, _| input.selection()), 6..11);
            cx.update(|cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("tea".into())));
            cx.simulate_keystrokes(window.into(), "cmd-v");
            assert_eq!(input.read_with(cx, |input, _| input.content()), "alpha tea");
            cx.simulate_keystrokes(window.into(), "cmd-z");
            assert_eq!(
                input.read_with(cx, |input, _| input.content()),
                "alpha café"
            );
            cx.simulate_keystrokes(window.into(), "cmd-shift-z");
            assert_eq!(input.read_with(cx, |input, _| input.content()), "alpha tea");
        }
    }

    #[gpui::test]
    fn skill_token_identity_survives_draft_restore_deletion_and_undo(
        cx: &mut gpui::TestAppContext,
    ) {
        let (window, input) = view(cx, false);
        let reference = SkillReference {
            provider: smaller_talk_domain::Provider::Codex,
            name: "review".into(),
            path: "/skills/review/SKILL.md".into(),
            workspace: "/project".into(),
        };
        let text = "🦀 $review";
        let draft = Draft {
            text: text.into(),
            skills: vec![SkillMention {
                range: 5..12,
                reference: reference.clone(),
            }],
            selection_start: text.len(),
            selection_end: text.len(),
            ..Default::default()
        };
        window
            .update(cx, |input, _, cx| input.load_draft(&draft, cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            input.read_with(cx, |input, _| input.skills().to_vec()),
            draft.skills
        );
        cx.simulate_keystrokes(window.into(), "backspace");
        assert_eq!(input.read_with(cx, |input, _| input.content()), "🦀 ");
        assert!(input.read_with(cx, |input, _| input.skills().is_empty()));
        cx.simulate_keystrokes(window.into(), "cmd-z");
        assert_eq!(
            input.read_with(cx, |input, _| input.draft(Vec::new())),
            draft
        );
    }

    #[gpui::test]
    fn inserting_a_skill_adds_delimiters_as_one_undoable_edit(cx: &mut gpui::TestAppContext) {
        let (window, input) = view(cx, false);
        let reference = SkillReference {
            provider: smaller_talk_domain::Provider::Codex,
            name: "review".into(),
            path: "/skills/review/SKILL.md".into(),
            workspace: "/project".into(),
        };
        let draft = Draft {
            text: "beforeafter".into(),
            selection_start: 6,
            selection_end: 6,
            ..Default::default()
        };
        window
            .update(cx, |input, _, cx| input.load_draft(&draft, cx))
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |input, _, cx| {
                input.select_skill(6..6, reference.clone(), cx)
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            input.read_with(cx, |input, _| input.content()),
            "before $review after"
        );
        assert_eq!(
            input.read_with(cx, |input, _| input.skills().to_vec()),
            vec![SkillMention {
                range: 7..14,
                reference
            }]
        );
        cx.simulate_keystrokes(window.into(), "cmd-z");
        assert_eq!(
            input.read_with(cx, |input, _| input.draft(Vec::new())),
            draft
        );
        cx.simulate_keystrokes(window.into(), "cmd-shift-z");
        assert_eq!(
            input.read_with(cx, |input, _| input.content()),
            "before $review after"
        );
        assert_eq!(input.read_with(cx, |input, _| input.skills().len()), 1);
    }

    #[gpui::test]
    fn ime_composition_is_not_resolved_as_a_skill_and_commit_updates_the_draft(
        cx: &mut gpui::TestAppContext,
    ) {
        let (window, input) = view(cx, false);
        window
            .update(cx, |input, window, cx| {
                update_editor!(input.editor.as_ref().unwrap(), cx, |state, cx| {
                    state.replace_and_mark_text_in_range(None, "$review", Some(7..7), window, cx);
                });
            })
            .unwrap();
        cx.run_until_parked();
        assert!(input.read_with(cx, |input, _| input.composing()));
        assert!(input.read_with(cx, |input, _| input.skill_query().is_none()));
        window
            .update(cx, |input, window, cx| {
                update_editor!(input.editor.as_ref().unwrap(), cx, |state, cx| {
                    state.replace_text_in_range(None, "café", window, cx);
                });
            })
            .unwrap();
        cx.run_until_parked();
        assert!(!input.read_with(cx, |input, _| input.composing()));
        assert_eq!(input.read_with(cx, |input, _| input.content()), "café");
        cx.simulate_keystrokes(window.into(), "cmd-z");
        assert_eq!(input.read_with(cx, |input, _| input.content()), "");
    }

    #[gpui::test]
    fn readonly_input_keeps_its_draft_but_rejects_user_edits(cx: &mut gpui::TestAppContext) {
        let (window, input) = view(cx, false);
        cx.simulate_input(window.into(), "keep me");
        window
            .update(cx, |input, _, cx| input.set_editable(false, cx))
            .unwrap();
        cx.run_until_parked();
        cx.simulate_input(window.into(), "lost");
        cx.simulate_keystrokes(window.into(), "backspace");
        assert_eq!(input.read_with(cx, |input, _| input.content()), "keep me");
        window
            .update(cx, |input, _, cx| input.set_editable(true, cx))
            .unwrap();
        cx.run_until_parked();
        cx.simulate_input(window.into(), "!");
        assert_eq!(input.read_with(cx, |input, _| input.content()), "keep me!");
    }

    #[gpui::test]
    fn long_paste_reveals_caret_after_growing_to_row_limit(cx: &mut gpui::TestAppContext) {
        let (window, input) = view(cx, false);
        cx.simulate_window_resize(window.into(), gpui::size(px(600.), px(168.)));
        cx.run_until_parked();
        let text = (1..=18)
            .map(|i| format!("Line {i}: café 🦀"))
            .collect::<Vec<_>>()
            .join("\n");
        cx.update(|cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.clone())));
        cx.simulate_keystrokes(window.into(), "cmd-v");
        cx.run_until_parked();
        window
            .update(cx, |input, _, cx| {
                update_editor!(input.editor.as_ref().unwrap(), cx, |state, _cx| {
                    assert!(
                        state.scroll_offset().y < px(0.),
                        "paste must reveal the caret below the first seven rows"
                    );
                });
            })
            .unwrap();
        assert_eq!(input.read_with(cx, |input, _| input.content()), text);
    }

    #[gpui::test]
    fn instructions_editor_keeps_enter_as_a_newline(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_base::init);
        let window = cx.add_window(|window, cx| {
            let mut input = Composer::new_editor(cx);
            input.initialize(window, cx);
            input.focus_handle.focus(window, cx);
            input
        });
        let input = window.root(cx).unwrap();
        cx.run_until_parked();
        cx.simulate_input(window.into(), "first");
        cx.simulate_keystrokes(window.into(), "enter");
        cx.simulate_input(window.into(), "second");
        assert_eq!(
            input.read_with(cx, |input, _| input.content()),
            "first\nsecond"
        );
    }

    #[gpui::test]
    fn enter_submits_and_shift_enter_preserves_multiline_text(cx: &mut gpui::TestAppContext) {
        use std::{cell::Cell, rc::Rc};
        let (window, input) = view(cx, false);
        let submitted = Rc::new(Cell::new(0));
        let count = submitted.clone();
        cx.update(|cx| {
            cx.subscribe(&input, move |_, _: &Submitted, _| {
                count.set(count.get() + 1)
            })
            .detach()
        });
        cx.simulate_input(window.into(), "first");
        cx.simulate_keystrokes(window.into(), "shift-enter");
        cx.simulate_input(window.into(), "second");
        assert_eq!(
            input.read_with(cx, |input, _| input.content()),
            "first\nsecond"
        );
        assert_eq!(submitted.get(), 0);
        cx.simulate_keystrokes(window.into(), "enter");
        assert_eq!(submitted.get(), 1);
        assert_eq!(
            input.read_with(cx, |input, _| input.content()),
            "first\nsecond"
        );
    }
}
