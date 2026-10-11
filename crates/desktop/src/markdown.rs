use std::{ops::Range, sync::OnceLock};

use gpui::{FontStyle, FontWeight, HighlightStyle};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Parser, Tag, TagEnd};
use syntect::{easy::HighlightLines, highlighting::ThemeSet, parsing::SyntaxSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MarkdownBlockKind {
    Paragraph,
    Heading(u8),
    Code(String),
    ListItem,
    Quote,
    Rule,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownBlock {
    pub kind: MarkdownBlockKind,
    pub text: String,
    pub highlights: Vec<(Range<usize>, InlineStyle)>,
    pub code_highlights: Vec<(Range<usize>, u32)>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InlineStyle {
    bold: bool,
    italic: bool,
    code: bool,
    link: bool,
}

impl InlineStyle {
    fn include(&mut self, mark: InlineMark) {
        match mark {
            InlineMark::Bold => self.bold = true,
            InlineMark::Italic => self.italic = true,
            InlineMark::Code => self.code = true,
            InlineMark::Link => self.link = true,
        }
    }

    fn is_empty(self) -> bool {
        !self.bold && !self.italic && !self.code && !self.link
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InlineMark {
    Bold,
    Italic,
    Code,
    Link,
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
            });
            self.marks.clear();
        }
    }
}

pub fn parse_markdown(source: &str) -> Vec<MarkdownBlock> {
    let mut blocks = Vec::new();
    let mut current = BlockBuilder::default();

    for event in Parser::new(source) {
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
            Event::Start(Tag::Item) => {
                current.begin(MarkdownBlockKind::ListItem);
                if current.text.is_empty() {
                    current.append("•  ");
                }
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
            Event::Start(Tag::Link { .. }) => current.marks.push(InlineMark::Link),
            Event::End(TagEnd::Link) => remove_last_mark(&mut current.marks, InlineMark::Link),
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
        }
    }
    blocks
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
    let Some(syntax) = resources.syntaxes.find_syntax_by_token(language.trim()) else {
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
    use super::{InlineStyle, MarkdownBlock, MarkdownBlockKind, parse_markdown};

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
}
