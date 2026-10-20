use std::{
    collections::HashMap,
    ops::Range,
    sync::{Arc, OnceLock},
};

use gpui::{FontStyle, FontWeight, HighlightStyle};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use syntect::{easy::HighlightLines, highlighting::ThemeSet, parsing::SyntaxSet};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq)]
pub enum MarkdownBlockKind {
    Paragraph,
    Heading(u8),
    Code(String),
    ListItem,
    Quote,
    Rule,
    Table(Vec<Alignment>),
}

type HighlightedCodeLine = (String, Vec<(Range<usize>, u32)>);

#[derive(Clone, Debug, PartialEq)]
pub struct MarkdownBlock {
    pub kind: MarkdownBlockKind,
    pub text: String,
    pub highlights: Vec<(Range<usize>, InlineStyle)>,
    pub code_highlights: Vec<(Range<usize>, u32)>,
    pub code_lines: Vec<HighlightedCodeLine>,
    pub links: Vec<(Range<usize>, String)>,
    pub table_rows: Vec<Vec<MarkdownBlock>>,
}

#[derive(Default)]
pub struct MarkdownCache {
    entries: HashMap<Uuid, CachedMarkdown>,
}

struct CachedMarkdown {
    source: String,
    blocks: Arc<[MarkdownBlock]>,
}

impl MarkdownCache {
    pub fn get_or_parse(&mut self, message_id: Uuid, source: &str) -> Arc<[MarkdownBlock]> {
        if let Some(cached) = self.entries.get(&message_id)
            && cached.source == source
        {
            return cached.blocks.clone();
        }

        if self.entries.len() >= 64 && !self.entries.contains_key(&message_id) {
            self.entries.clear();
        }
        let blocks: Arc<[MarkdownBlock]> = Arc::from(parse_markdown(source));
        self.entries.insert(
            message_id,
            CachedMarkdown {
                source: source.to_owned(),
                blocks: blocks.clone(),
            },
        );
        blocks
    }

    pub fn retain_messages(&mut self, message_ids: impl IntoIterator<Item = Uuid>) {
        let message_ids = message_ids
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        self.entries
            .retain(|message_id, _| message_ids.contains(message_id));
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InlineStyle {
    bold: bool,
    italic: bool,
    code: bool,
    link: bool,
    strike: bool,
}

impl InlineStyle {
    fn include(&mut self, mark: InlineMark) {
        match mark {
            InlineMark::Bold => self.bold = true,
            InlineMark::Italic => self.italic = true,
            InlineMark::Code => self.code = true,
            InlineMark::Link => self.link = true,
            InlineMark::Strike => self.strike = true,
        }
    }

    fn is_empty(self) -> bool {
        !self.bold && !self.italic && !self.code && !self.link && !self.strike
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InlineMark {
    Bold,
    Italic,
    Code,
    Link,
    Strike,
}

impl From<InlineStyle> for HighlightStyle {
    fn from(style: InlineStyle) -> Self {
        HighlightStyle {
            color: if style.code {
                Some(gpui::rgb(0xf0c890).into())
            } else if style.link {
                Some(gpui::rgb(0x8ab4f8).into())
            } else {
                None
            },
            font_weight: style.bold.then_some(FontWeight::BOLD),
            font_style: style.italic.then_some(FontStyle::Italic),
            background_color: style.code.then(|| gpui::rgb(0x272b32).into()),
            underline: style.link.then_some(gpui::UnderlineStyle {
                color: None,
                thickness: gpui::px(1.0),
                wavy: false,
            }),
            strikethrough: style.strike.then_some(gpui::StrikethroughStyle {
                color: None,
                thickness: gpui::px(1.),
            }),
            ..Default::default()
        }
    }
}

#[derive(Default)]
struct BlockBuilder {
    kind: Option<MarkdownBlockKind>,
    text: String,
    highlights: Vec<(Range<usize>, InlineStyle)>,
    marks: Vec<InlineMark>,
    links: Vec<(Range<usize>, String)>,
    open_links: Vec<(usize, String)>,
}

impl BlockBuilder {
    fn begin(&mut self, kind: MarkdownBlockKind) {
        if self.kind.is_none() {
            self.kind = Some(kind);
        }
    }

    fn append(&mut self, text: &str) {
        self.append_with_mark(text, None);
    }

    fn append_with_mark(&mut self, text: &str, additional_mark: Option<InlineMark>) {
        self.begin(MarkdownBlockKind::Paragraph);
        let start = self.text.len();
        self.text.push_str(text);
        let end = self.text.len();
        let mut style = InlineStyle::default();
        for mark in &self.marks {
            style.include(*mark);
        }
        if let Some(mark) = additional_mark {
            style.include(mark);
        }
        if style.is_empty() || start == end {
            return;
        }
        if let Some((previous_range, previous_style)) = self.highlights.last_mut()
            && previous_range.end == start
            && *previous_style == style
        {
            previous_range.end = end;
        } else {
            self.highlights.push((start..end, style));
        }
    }

    fn finish(&mut self, blocks: &mut Vec<MarkdownBlock>) {
        if let Some(kind) = self.kind.take() {
            if self.text.is_empty() && kind != MarkdownBlockKind::Rule {
                self.highlights.clear();
                self.marks.clear();
                return;
            }
            blocks.push(MarkdownBlock {
                kind,
                text: std::mem::take(&mut self.text),
                highlights: std::mem::take(&mut self.highlights),
                code_highlights: Vec::new(),
                code_lines: Vec::new(),
                links: std::mem::take(&mut self.links),
                table_rows: Vec::new(),
            });
            self.marks.clear();
        }
    }
}

pub fn parse_markdown(source: &str) -> Vec<MarkdownBlock> {
    let mut blocks = Vec::new();
    let mut current = BlockBuilder::default();

    let mut list_numbers: Vec<Option<u64>> = Vec::new();
    let mut table_alignments = Vec::new();
    let mut table_rows = Vec::new();
    let mut row = Vec::new();
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH;
    for event in Parser::new_ext(source, options) {
        match event {
            Event::Start(Tag::Paragraph) => current.begin(MarkdownBlockKind::Paragraph),
            Event::End(TagEnd::Paragraph) => current.finish(&mut blocks),
            Event::Start(Tag::Heading { level, .. }) => {
                current.begin(MarkdownBlockKind::Heading(heading_level(level)));
            }
            Event::End(TagEnd::Heading(_)) => current.finish(&mut blocks),
            Event::Start(Tag::CodeBlock(kind)) => {
                let language = match kind {
                    CodeBlockKind::Fenced(language) => language.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                current.begin(MarkdownBlockKind::Code(language));
            }
            Event::End(TagEnd::CodeBlock) => current.finish(&mut blocks),
            Event::Start(Tag::List(start)) => {
                current.finish(&mut blocks);
                list_numbers.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                current.finish(&mut blocks);
                list_numbers.pop();
            }
            Event::Start(Tag::Item) => {
                current.finish(&mut blocks);
                current.begin(MarkdownBlockKind::ListItem);
                current.append(&"  ".repeat(list_numbers.len().saturating_sub(1)));
                match list_numbers.last_mut() {
                    Some(Some(number)) => {
                        current.append(&format!("{}.  ", *number));
                        *number += 1;
                    }
                    _ => current.append("•  "),
                }
            }
            Event::Start(Tag::Table(alignments)) => {
                current.finish(&mut blocks);
                table_alignments = alignments;
                table_rows.clear();
                row.clear();
            }
            Event::Start(Tag::TableHead | Tag::TableRow) => row.clear(),
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                table_rows.push(std::mem::take(&mut row))
            }
            Event::Start(Tag::TableCell) => current.begin(MarkdownBlockKind::Paragraph),
            Event::End(TagEnd::TableCell) => {
                // Empty cells must retain their column position.
                if current.text.is_empty() {
                    current.append(" ");
                }
                current.finish(&mut row);
            }
            Event::End(TagEnd::Table) => {
                let text = table_rows
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|c| c.text.as_str())
                            .collect::<Vec<_>>()
                            .join(" | ")
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                blocks.push(MarkdownBlock {
                    kind: MarkdownBlockKind::Table(std::mem::take(&mut table_alignments)),
                    text,
                    highlights: vec![],
                    code_highlights: vec![],
                    code_lines: vec![],
                    links: vec![],
                    table_rows: std::mem::take(&mut table_rows),
                });
            }
            Event::End(TagEnd::Item) => current.finish(&mut blocks),
            Event::Start(Tag::BlockQuote(_)) => current.begin(MarkdownBlockKind::Quote),
            Event::End(TagEnd::BlockQuote(_)) => current.finish(&mut blocks),
            Event::Start(Tag::Strong) => current.marks.push(InlineMark::Bold),
            Event::End(TagEnd::Strong) => remove_last_mark(&mut current.marks, InlineMark::Bold),
            Event::Start(Tag::Emphasis) => current.marks.push(InlineMark::Italic),
            Event::End(TagEnd::Emphasis) => {
                remove_last_mark(&mut current.marks, InlineMark::Italic);
            }
            Event::Start(Tag::Strikethrough) => current.marks.push(InlineMark::Strike),
            Event::End(TagEnd::Strikethrough) => {
                remove_last_mark(&mut current.marks, InlineMark::Strike)
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                current.marks.push(InlineMark::Link);
                current
                    .open_links
                    .push((current.text.len(), dest_url.into_string()));
            }
            Event::End(TagEnd::Link) => {
                remove_last_mark(&mut current.marks, InlineMark::Link);
                if let Some((start, url)) = current.open_links.pop() {
                    current.links.push((start..current.text.len(), url));
                }
            }
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                current.append(&text);
            }
            Event::Code(text) => current.append_with_mark(&text, Some(InlineMark::Code)),
            Event::SoftBreak | Event::HardBreak => current.append("\n"),
            Event::Rule => {
                current.begin(MarkdownBlockKind::Rule);
                current.finish(&mut blocks);
            }
            Event::TaskListMarker(checked) => {
                current.append(if checked { "☑ " } else { "☐ " });
            }
            Event::Start(_)
            | Event::End(_)
            | Event::FootnoteReference(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_) => {}
        }
    }
    current.finish(&mut blocks);
    for block in &mut blocks {
        if let MarkdownBlockKind::Code(language) = &block.kind {
            block.code_highlights = highlight_code(language, &block.text);
            block.code_lines = block
                .text
                .trim_end_matches('\n')
                .split('\n')
                .scan(0usize, |offset, line| {
                    let line_start = *offset;
                    *offset += line.len() + 1;
                    Some((line, line_start))
                })
                .map(|(line, line_start)| {
                    let (line, highlights) =
                        display_code_line(line, line_start, &block.code_highlights);
                    if line.is_empty() {
                        (" ".to_owned(), Vec::new())
                    } else {
                        (line, highlights)
                    }
                })
                .collect();
        }
    }
    blocks
}

fn display_code_line(
    line: &str,
    source_offset: usize,
    highlights: &[(Range<usize>, u32)],
) -> (String, Vec<(Range<usize>, u32)>) {
    let mut displayed = String::with_capacity(line.len());
    let mut mapped: Vec<(Range<usize>, u32)> = Vec::new();
    let mut previous: Option<(usize, u32)> = None;
    for (offset, character) in line.char_indices() {
        let token = highlights
            .iter()
            .find(|(range, _)| range.contains(&(source_offset + offset)))
            .map(|(_, token)| *token);
        let start = displayed.len();
        match character {
            ' ' => displayed.push(' '),
            '\t' => displayed.push_str("    "),
            _ => displayed.push(character),
        }
        let end = displayed.len();
        if let Some(token) = token {
            if let Some((last, last_token)) = previous.as_mut()
                && *last == start
                && *last_token == token
            {
                *last = end;
                if let Some((range, _)) = mapped.last_mut() {
                    range.end = end;
                }
            } else {
                mapped.push((start..end, token));
                previous = Some((end, token));
            }
        } else {
            previous = None;
        }
    }
    (displayed, mapped)
}

struct SyntaxResources {
    syntaxes: SyntaxSet,
    themes: ThemeSet,
}

static SYNTAX_RESOURCES: OnceLock<SyntaxResources> = OnceLock::new();

fn syntax_resources() -> &'static SyntaxResources {
    SYNTAX_RESOURCES.get_or_init(|| SyntaxResources {
        syntaxes: SyntaxSet::load_defaults_newlines(),
        themes: ThemeSet::load_defaults(),
    })
}

fn highlight_code(language: &str, source: &str) -> Vec<(Range<usize>, u32)> {
    let resources = syntax_resources();
    let language = language.trim();
    let language = if language.eq_ignore_ascii_case("typescript") {
        "js"
    } else {
        language
    };
    let Some(syntax) = resources.syntaxes.find_syntax_by_token(language) else {
        return Vec::new();
    };
    if syntax.name == "Plain Text" {
        return Vec::new();
    }
    let Some(theme) = resources.themes.themes.get("base16-ocean.dark") else {
        return Vec::new();
    };

    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut result: Vec<(Range<usize>, u32)> = Vec::new();
    let mut line_start = 0;
    for line in source.split_inclusive('\n') {
        let Ok(ranges) = highlighter.highlight_line(line, &resources.syntaxes) else {
            return Vec::new();
        };
        let mut offset = line_start;
        for (style, text) in ranges {
            let end = offset + text.len();
            let color = ((style.foreground.r as u32) << 16)
                | ((style.foreground.g as u32) << 8)
                | style.foreground.b as u32;
            if let Some((previous_range, previous_color)) = result.last_mut()
                && previous_range.end == offset
                && *previous_color == color
            {
                previous_range.end = end;
            } else if offset < end {
                result.push((offset..end, color));
            }
            offset = end;
        }
        line_start += line.len();
    }
    result
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn remove_last_mark(marks: &mut Vec<InlineMark>, style: InlineMark) {
    if let Some(index) = marks.iter().rposition(|mark| *mark == style) {
        marks.remove(index);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{InlineStyle, MarkdownBlock, MarkdownBlockKind, MarkdownCache, parse_markdown};

    fn style_at(block: &MarkdownBlock, byte: usize) -> InlineStyle {
        block
            .highlights
            .iter()
            .find(|(range, _)| range.contains(&byte))
            .map(|(_, style)| *style)
            .unwrap_or_default()
    }

    fn assert_highlights_are_valid(block: &MarkdownBlock) {
        for (range, _) in &block.highlights {
            assert!(block.text.is_char_boundary(range.start));
            assert!(block.text.is_char_boundary(range.end));
            assert!(block.text.get(range.clone()).is_some());
        }
        for pair in block.highlights.windows(2) {
            assert!(pair[0].0.end <= pair[1].0.start);
        }
    }

    #[test]
    fn renders_headings_emphasis_lists_and_fenced_code_as_blocks() {
        let blocks = parse_markdown(
            "# Hello\n\nA **bold** and `inline` answer.\n\n- One\n- Two\n\n```rust\nfn main() {}\n```",
        );
        assert!(matches!(blocks[0].kind, MarkdownBlockKind::Heading(1)));
        assert_eq!(blocks[0].text, "Hello");
        assert_eq!(blocks[1].text, "A bold and inline answer.");
        assert_eq!(blocks[1].highlights.len(), 2);
        assert!(matches!(blocks[2].kind, MarkdownBlockKind::ListItem));
        assert_eq!(blocks[2].text, "•  One");
        assert!(matches!(
            &blocks[4].kind,
            MarkdownBlockKind::Code(language) if language == "rust"
        ));
        assert_eq!(blocks[4].text, "fn main() {}\n");
    }

    #[test]
    fn composes_nested_inline_styles_into_nonoverlapping_utf8_safe_spans() {
        let chunks = [
            "é Before **",
            "bold ",
            "_italic_",
            " `code`",
            "** after 后.",
        ];
        let mut streamed = String::new();
        for chunk in chunks {
            streamed.push_str(chunk);
            for block in parse_markdown(&streamed) {
                assert_highlights_are_valid(&block);
            }
        }

        let blocks = parse_markdown(&streamed);
        let block = &blocks[0];
        assert_eq!(block.text, "é Before bold italic code after 后.");
        assert_highlights_are_valid(block);

        let bold = block.text.find("bold").expect("bold text present");
        let italic = block.text.find("italic").expect("italic text present");
        let code = block.text.find("code").expect("inline code present");
        let after = block.text.find("after").expect("following text present");
        assert!(style_at(block, bold).bold);
        assert!(style_at(block, italic).bold && style_at(block, italic).italic);
        assert!(style_at(block, code).bold && style_at(block, code).code);
        assert!(!style_at(block, after).bold);
        assert!(!style_at(block, after).italic);

        let reloaded = parse_markdown(&streamed);
        assert_eq!(reloaded, blocks);
    }

    #[test]
    fn highlights_supported_languages_and_leaves_unknown_or_unlabeled_code_plain() {
        let markdown = "```rs\nfn main() { // note\n let s = \"hello\"; }\n```\n```typescript\nconst count = 42;\n```\n```python\n# note\nprint('ok')\n```\n```sql\nSELECT 1 FROM items;\n```\n```json\n{\"ok\": true}\n```\n```bash\necho \"ok\" # note\n```\n```nonsense\nfn ordinary() {}\n```\n```\nplain code\n```";
        let blocks = parse_markdown(markdown);
        assert_eq!(blocks.len(), 8);
        assert!(!blocks[0].code_highlights.is_empty());
        assert!(!blocks[1].code_highlights.is_empty());
        for block in &blocks[2..6] {
            assert!(!block.code_highlights.is_empty());
        }
        assert!(blocks[6].code_highlights.is_empty());
        assert!(blocks[7].code_highlights.is_empty());
        for block in &blocks[..6] {
            for (range, _) in &block.code_highlights {
                assert!(block.text.is_char_boundary(range.start));
                assert!(block.text.is_char_boundary(range.end));
                assert!(block.text.get(range.clone()).is_some());
            }
        }
    }

    #[test]
    fn incomplete_streaming_code_is_safe_and_highlights_available_tokens() {
        let blocks = parse_markdown("```rust\nfn mai");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(blocks[0].kind, MarkdownBlockKind::Code(_)));
        assert!(!blocks[0].code_highlights.is_empty());
    }

    #[test]
    fn markdown_cache_reuses_unchanged_results_and_replaces_streaming_versions() {
        let mut cache = MarkdownCache::default();
        let first_id = uuid::Uuid::new_v4();
        let second_id = uuid::Uuid::new_v4();

        let first = cache.get_or_parse(first_id, "```rust\nfn main() {\n```");
        let same = cache.get_or_parse(first_id, "```rust\nfn main() {\n```");
        assert!(Arc::ptr_eq(&first, &same));
        assert!(!first[0].code_highlights.is_empty());

        let updated = cache.get_or_parse(first_id, "```rust\nfn main() {}\n```");
        assert!(!Arc::ptr_eq(&first, &updated));
        assert_eq!(updated[0].text, "fn main() {}\n");
        assert!(!updated[0].code_highlights.is_empty());
        assert_eq!(first[0].text, "fn main() {\n");

        let other = cache.get_or_parse(second_id, "**unchanged**");
        let other_again = cache.get_or_parse(second_id, "**unchanged**");
        assert!(Arc::ptr_eq(&other, &other_again));
        let other_after_first_changed = cache.get_or_parse(second_id, "**unchanged**");
        assert!(Arc::ptr_eq(&other, &other_after_first_changed));

        let updated_weak = Arc::downgrade(&updated);
        drop(updated);
        cache.retain_messages([second_id]);
        assert_eq!(cache.entries.len(), 1);
        assert!(updated_weak.upgrade().is_none());
        let reloaded = cache.get_or_parse(first_id, "```rust\nfn main() {}\n```");
        assert_eq!(reloaded[0].text, "fn main() {}\n");
    }
}

#[cfg(test)]
mod workspace_markdown_tests {
    use super::*;
    #[test]
    fn tables_preserve_empty_cells_alignment_and_inline_styles() {
        let blocks = parse_markdown(
            "| Precon | Why | Empty |\n|:--|--:|---|\n| **Explorers** | [Details](https://example.com) | |\n",
        );
        let table = &blocks[0];
        assert!(
            matches!(&table.kind, MarkdownBlockKind::Table(a) if a[0] == Alignment::Left && a[1] == Alignment::Right)
        );
        assert_eq!(table.table_rows.len(), 2);
        assert_eq!(table.table_rows[1].len(), 3);
        assert!(table.table_rows[1][0].highlights[0].1.bold);
        assert_eq!(table.table_rows[1][1].links[0].1, "https://example.com");
    }
    #[test]
    fn streamed_tables_and_nested_unicode_formatting_never_drop_text_or_invalidate_ranges() {
        let source = "| A | B |\n|---|---|\n| **é🙂** | ~~old~~ |\n\n1. First\n   - [x] Nested\n\n<a>inert</a>";
        for end in (0..=source.len()).filter(|i| source.is_char_boundary(*i)) {
            for block in parse_markdown(&source[..end]) {
                for cell in std::iter::once(&block).chain(block.table_rows.iter().flatten()) {
                    for (range, _) in &cell.highlights {
                        assert!(cell.text.get(range.clone()).is_some());
                    }
                    for (range, _) in &cell.links {
                        assert!(cell.text.get(range.clone()).is_some());
                    }
                }
            }
        }
        let blocks = parse_markdown(source);
        assert!(blocks.iter().any(|b| b.text.contains("1.  First")));
        assert!(blocks.iter().any(|b| b.text.contains("☑ Nested")));
        assert!(blocks.iter().any(|b| b.text.contains("<a>")));
    }
}
