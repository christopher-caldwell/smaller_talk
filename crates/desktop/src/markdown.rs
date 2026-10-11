use std::ops::Range;

use gpui::{FontStyle, FontWeight, HighlightStyle};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Parser, Tag, TagEnd};

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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InlineStyle {
    Bold,
    Italic,
    Code,
    Link,
}

impl From<InlineStyle> for HighlightStyle {
    fn from(style: InlineStyle) -> Self {
        match style {
            InlineStyle::Bold => FontWeight::BOLD.into(),
            InlineStyle::Italic => FontStyle::Italic.into(),
            InlineStyle::Code => HighlightStyle {
                background_color: Some(gpui::rgb(0x272b32).into()),
                color: Some(gpui::rgb(0xf0c890).into()),
                ..Default::default()
            },
            InlineStyle::Link => HighlightStyle {
                color: Some(gpui::rgb(0x8ab4f8).into()),
                underline: Some(gpui::UnderlineStyle {
                    color: None,
                    thickness: gpui::px(1.0),
                    wavy: false,
                }),
                ..Default::default()
            },
        }
    }
}

#[derive(Default)]
struct BlockBuilder {
    kind: Option<MarkdownBlockKind>,
    text: String,
    highlights: Vec<(Range<usize>, InlineStyle)>,
    marks: Vec<InlineStyle>,
}

impl BlockBuilder {
    fn begin(&mut self, kind: MarkdownBlockKind) {
        if self.kind.is_none() {
            self.kind = Some(kind);
        }
    }

    fn append(&mut self, text: &str) {
        self.begin(MarkdownBlockKind::Paragraph);
        let start = self.text.len();
        self.text.push_str(text);
        let end = self.text.len();
        for style in &self.marks {
            self.highlights.push((start..end, *style));
        }
    }

    fn append_styled(&mut self, text: &str, style: InlineStyle) {
        self.begin(MarkdownBlockKind::Paragraph);
        let start = self.text.len();
        self.text.push_str(text);
        let end = self.text.len();
        self.highlights.push((start..end, style));
        for outer_style in &self.marks {
            if *outer_style != style {
                self.highlights.push((start..end, *outer_style));
            }
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
            Event::Start(Tag::Strong) => current.marks.push(InlineStyle::Bold),
            Event::End(TagEnd::Strong) => remove_last_mark(&mut current.marks, InlineStyle::Bold),
            Event::Start(Tag::Emphasis) => current.marks.push(InlineStyle::Italic),
            Event::End(TagEnd::Emphasis) => {
                remove_last_mark(&mut current.marks, InlineStyle::Italic);
            }
            Event::Start(Tag::Link { .. }) => current.marks.push(InlineStyle::Link),
            Event::End(TagEnd::Link) => remove_last_mark(&mut current.marks, InlineStyle::Link),
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                current.append(&text);
            }
            Event::Code(text) => current.append_styled(&text, InlineStyle::Code),
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
    blocks
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

fn remove_last_mark(marks: &mut Vec<InlineStyle>, style: InlineStyle) {
    if let Some(index) = marks.iter().rposition(|mark| *mark == style) {
        marks.remove(index);
    }
}

#[cfg(test)]
mod tests {
    use super::{MarkdownBlockKind, parse_markdown};

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
}
