//! Transport-neutral semantic Markdown document and terminal renderer
//! (`A2-114`, ADR-0075).
//!
//! Parsing uses `pulldown-cmark` privately with default features disabled and
//! only the required GFM options enabled (`ENABLE_TABLES`,
//! `ENABLE_STRIKETHROUGH`, `ENABLE_TASKLISTS`). Smart punctuation stays off so
//! pretty-printing never alters author characters. No `pulldown-cmark` types
//! appear in the public API.
//!
//! All task-supplied text, code, link destinations, image metadata, and raw
//! HTML pass through [`crate::sanitize::visible`] on ingest so control
//! sequences (CSI, OSC, C0/C1, bidi overrides) can never execute on a
//! terminal. Layout is pure, I/O-free, grapheme-safe, and produces an owned
//! [`RenderedDocument`] of [`RenderedLine`] / [`RenderedSpan`] values that both
//! the CLI `anstyle` sink ([`render_terminal`]) and the TUI Ratatui sink
//! (`A2-115`) consume directly.

use std::fmt::Write as _;

use anstyle::Style;
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr;

use crate::{sanitize, style};

/// Maximum accepted Markdown input size in bytes before deterministic
/// truncation.
pub const MAX_INPUT_BYTES: usize = 512 * 1024;

/// Maximum nesting depth for block quotes and lists.
pub const MAX_NESTING_DEPTH: usize = 32;

/// Maximum top-level blocks retained in one parsed document.
pub const MAX_BLOCKS: usize = 4_096;

/// Exhaustive semantic member inventory (`A2-114`).
///
/// Every construct supported by the parser-to-semantic layer is listed here so
/// the executable member audit fails if a new member is added without a layout
/// and terminal sink disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SemanticMember {
    Heading,
    Paragraph,
    CodeBlock,
    List,
    BlockQuote,
    Table,
    ThematicBreak,
    RawHtml,
    Text,
    Emphasis,
    Strong,
    Strikethrough,
    InlineCode,
    Link,
    Image,
    SoftBreak,
    HardBreak,
    TaskCheckbox,
    InlineHtml,
}

impl SemanticMember {
    /// Complete closed inventory of semantic Markdown members.
    pub const ALL: &'static [Self] = &[
        Self::Heading,
        Self::Paragraph,
        Self::CodeBlock,
        Self::List,
        Self::BlockQuote,
        Self::Table,
        Self::ThematicBreak,
        Self::RawHtml,
        Self::Text,
        Self::Emphasis,
        Self::Strong,
        Self::Strikethrough,
        Self::InlineCode,
        Self::Link,
        Self::Image,
        Self::SoftBreak,
        Self::HardBreak,
        Self::TaskCheckbox,
        Self::InlineHtml,
    ];
}

/// Exhaustive semantic presentation roles carried on [`RenderedSpan`] values.
///
/// Both CLI (`anstyle`) and TUI (`ratatui`) sinks map every role explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SemanticRole {
    Plain,
    Heading,
    Strong,
    Emphasis,
    Strikethrough,
    Code,
    CodeLabel,
    QuoteMarker,
    ListMarker,
    LinkText,
    LinkDestination,
    ImageAlt,
    TableHeader,
    Rule,
    RawHtml,
    Muted,
}

impl SemanticRole {
    /// Complete closed inventory of semantic presentation roles.
    pub const ALL: &'static [Self] = &[
        Self::Plain,
        Self::Heading,
        Self::Strong,
        Self::Emphasis,
        Self::Strikethrough,
        Self::Code,
        Self::CodeLabel,
        Self::QuoteMarker,
        Self::ListMarker,
        Self::LinkText,
        Self::LinkDestination,
        Self::ImageAlt,
        Self::TableHeader,
        Self::Rule,
        Self::RawHtml,
        Self::Muted,
    ];
}

/// Column alignment in a GFM table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableAlignment {
    None,
    Left,
    Center,
    Right,
}

/// Owned, transport-neutral semantic Markdown document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownDocument {
    pub blocks: Vec<Block>,
    /// `true` if the input exceeded [`MAX_INPUT_BYTES`] or [`MAX_BLOCKS`] and
    /// was deterministically truncated.
    pub truncated: bool,
}

/// One block-level semantic Markdown construct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading {
        level: u8,
        inlines: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    CodeBlock {
        language: Option<String>,
        content: String,
    },
    List {
        ordered_start: Option<u64>,
        items: Vec<ListItem>,
    },
    BlockQuote(Vec<Block>),
    Table {
        alignments: Vec<TableAlignment>,
        headers: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    ThematicBreak,
    RawHtml(String),
}

/// One item inside an ordered, unordered, or task list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListItem {
    pub task_checked: Option<bool>,
    pub blocks: Vec<Block>,
}

/// One inline-level semantic Markdown construct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Emphasis(Vec<Inline>),
    Strong(Vec<Inline>),
    Strikethrough(Vec<Inline>),
    Code(String),
    Link {
        label: Vec<Inline>,
        destination: String,
    },
    Image {
        alt: String,
        destination: String,
    },
    SoftBreak,
    HardBreak,
    RawHtml(String),
}

/// Display-width policy for East Asian Ambiguous Unicode characters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WidthPolicy {
    /// Standard Unicode display width (`unicode_width::UnicodeWidthStr::width`,
    /// ambiguous characters = 1 column).
    #[default]
    Standard,
    /// CJK-ambiguous terminal width (`unicode_width::UnicodeWidthStr::width_cjk`,
    /// ambiguous characters = 2 columns).
    WideAmbiguous,
}

/// How link destinations are shown in terminal output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LinkDisplay {
    /// Show `label (destination)` whenever the destination adds information.
    #[default]
    InlineDestination,
    /// Show only the link label.
    LabelOnly,
}

/// Role-based styling table for terminal rendering (no runtime plugins or
/// arbitrary markup execution).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkdownTheme {
    pub plain: Style,
    pub heading: Style,
    pub strong: Style,
    pub emphasis: Style,
    pub strikethrough: Style,
    pub code: Style,
    pub code_label: Style,
    pub quote_marker: Style,
    pub list_marker: Style,
    pub link_text: Style,
    pub link_destination: Style,
    pub image_alt: Style,
    pub table_header: Style,
    pub rule: Style,
    pub raw_html: Style,
    pub muted: Style,
}

impl Default for MarkdownTheme {
    fn default() -> Self {
        Self {
            plain: style::PLAIN,
            heading: style::HEADING,
            strong: style::HEADING,
            emphasis: style::EMPHASIS,
            strikethrough: style::STRIKETHROUGH,
            code: style::ID,
            code_label: style::DIM,
            quote_marker: style::DIM,
            list_marker: style::HEADING,
            link_text: style::LINK,
            link_destination: style::DIM,
            image_alt: style::DIM,
            table_header: style::HEADING,
            rule: style::DIM,
            raw_html: style::DIM,
            muted: style::DIM,
        }
    }
}

impl MarkdownTheme {
    /// Return the exact configured style for one [`SemanticRole`].
    #[must_use]
    pub const fn style_for(self, role: SemanticRole) -> Style {
        match role {
            SemanticRole::Plain => self.plain,
            SemanticRole::Heading => self.heading,
            SemanticRole::Strong => self.strong,
            SemanticRole::Emphasis => self.emphasis,
            SemanticRole::Strikethrough => self.strikethrough,
            SemanticRole::Code => self.code,
            SemanticRole::CodeLabel => self.code_label,
            SemanticRole::QuoteMarker => self.quote_marker,
            SemanticRole::ListMarker => self.list_marker,
            SemanticRole::LinkText => self.link_text,
            SemanticRole::LinkDestination => self.link_destination,
            SemanticRole::ImageAlt => self.image_alt,
            SemanticRole::TableHeader => self.table_header,
            SemanticRole::Rule => self.rule,
            SemanticRole::RawHtml => self.raw_html,
            SemanticRole::Muted => self.muted,
        }
    }
}

/// Caller-provided capabilities controlling Markdown layout and rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderCapabilities {
    /// Maximum display columns per line (clamped to at least 8).
    pub width: usize,
    /// Use plain ASCII markers instead of Unicode box/bullet glyphs.
    pub ascii: bool,
    /// Emit ANSI style sequences via `anstyle`. When `false`, output is pure
    /// plain text byte-identical to stripped colored output.
    pub color: bool,
    /// Link destination display mode.
    pub links: LinkDisplay,
    /// East Asian Ambiguous width policy.
    pub width_policy: WidthPolicy,
    /// Role-based presentation theme.
    pub theme: MarkdownTheme,
}

impl Default for RenderCapabilities {
    fn default() -> Self {
        Self {
            width: 80,
            ascii: false,
            color: false,
            links: LinkDisplay::InlineDestination,
            width_policy: WidthPolicy::Standard,
            theme: MarkdownTheme::default(),
        }
    }
}

/// One styled span inside a [`RenderedLine`], forming the stable adapter
/// boundary shared by CLI and TUI (`A2-115`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedSpan {
    pub text: String,
    pub role: SemanticRole,
}

/// One laid-out terminal line made of semantically tagged spans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedLine {
    pub spans: Vec<RenderedSpan>,
}

impl RenderedLine {
    /// Plain concatenated text of this line without any styling.
    #[must_use]
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for span in &self.spans {
            out.push_str(&span.text);
        }
        out
    }

    /// Display width of this line under `policy`.
    #[must_use]
    pub fn display_width(&self, policy: WidthPolicy) -> usize {
        self.spans
            .iter()
            .map(|span| text_width(&span.text, policy))
            .sum()
    }
}

/// Complete I/O-free laid-out document ready for CLI serialization or TUI cell
/// painting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedDocument {
    pub lines: Vec<RenderedLine>,
}

/// Compute display width of `text` under `policy`.
#[must_use]
pub fn text_width(text: &str, policy: WidthPolicy) -> usize {
    match policy {
        WidthPolicy::Standard => UnicodeWidthStr::width(text),
        WidthPolicy::WideAmbiguous => UnicodeWidthStr::width_cjk(text),
    }
}

fn sanitize_text(input: &str) -> String {
    sanitize::visible(input).into_owned()
}

fn truncate_utf8(input: &str, max_bytes: usize) -> (&str, bool) {
    if input.len() <= max_bytes {
        return (input, false);
    }
    let mut end = max_bytes;
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    (&input[..end], true)
}

#[derive(Debug)]
enum Container {
    Document(Vec<Block>),
    Heading {
        level: u8,
        inlines: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    CodeBlock {
        language: Option<String>,
        content: String,
    },
    List {
        ordered_start: Option<u64>,
        items: Vec<ListItem>,
    },
    ListItem {
        task_checked: Option<bool>,
        blocks: Vec<Block>,
        inline_buf: Vec<Inline>,
    },
    BlockQuote(Vec<Block>),
    Table {
        alignments: Vec<TableAlignment>,
        headers: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
        in_head: bool,
        current_row: Vec<Vec<Inline>>,
        current_cell: Vec<Inline>,
    },
    Emphasis(Vec<Inline>),
    Strong(Vec<Inline>),
    Strikethrough(Vec<Inline>),
    Link {
        label: Vec<Inline>,
        destination: String,
    },
    Image {
        alt: String,
        destination: String,
    },
}

fn heading_level_num(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn map_alignment(alignment: Alignment) -> TableAlignment {
    match alignment {
        Alignment::None => TableAlignment::None,
        Alignment::Left => TableAlignment::Left,
        Alignment::Center => TableAlignment::Center,
        Alignment::Right => TableAlignment::Right,
    }
}

fn push_inline(stack: &mut [Container], inline: Inline) {
    for frame in stack.iter_mut().rev() {
        match frame {
            Container::Image { alt, .. } => {
                flatten_inline_into_string(&inline, alt);
                return;
            }
            Container::Heading { inlines, .. }
            | Container::Paragraph(inlines)
            | Container::Emphasis(inlines)
            | Container::Strong(inlines)
            | Container::Strikethrough(inlines)
            | Container::Link { label: inlines, .. } => {
                inlines.push(inline);
                return;
            }
            Container::Table { current_cell, .. } => {
                current_cell.push(inline);
                return;
            }
            Container::ListItem { inline_buf, .. } => {
                inline_buf.push(inline);
                return;
            }
            Container::CodeBlock { content, .. } => {
                flatten_inline_into_string(&inline, content);
                return;
            }
            Container::Document(blocks) | Container::BlockQuote(blocks) => {
                if let Some(Block::Paragraph(inlines)) = blocks.last_mut() {
                    inlines.push(inline);
                } else {
                    blocks.push(Block::Paragraph(vec![inline]));
                }
                return;
            }
            Container::List { .. } => {}
        }
    }
}

fn push_block(stack: &mut [Container], block: Block, truncated: &mut bool) {
    for frame in stack.iter_mut().rev() {
        match frame {
            Container::Document(blocks) => {
                if blocks.len() >= MAX_BLOCKS {
                    *truncated = true;
                } else {
                    blocks.push(block);
                }
                return;
            }
            Container::BlockQuote(blocks) => {
                blocks.push(block);
                return;
            }
            Container::ListItem {
                blocks, inline_buf, ..
            } => {
                if !inline_buf.is_empty() {
                    blocks.push(Block::Paragraph(std::mem::take(inline_buf)));
                }
                blocks.push(block);
                return;
            }
            Container::List { items, .. } => {
                if let Some(last) = items.last_mut() {
                    last.blocks.push(block);
                    return;
                }
            }
            _ => {}
        }
    }
}

fn flatten_inline_into_string(inline: &Inline, out: &mut String) {
    match inline {
        Inline::Text(text) | Inline::Code(text) | Inline::RawHtml(text) => out.push_str(text),
        Inline::Emphasis(children)
        | Inline::Strong(children)
        | Inline::Strikethrough(children)
        | Inline::Link {
            label: children, ..
        } => {
            for child in children {
                flatten_inline_into_string(child, out);
            }
        }
        Inline::Image { alt, .. } => out.push_str(alt),
        Inline::SoftBreak => out.push(' '),
        Inline::HardBreak => out.push('\n'),
    }
}

fn nesting_depth(stack: &[Container]) -> usize {
    stack
        .iter()
        .filter(|frame| matches!(frame, Container::BlockQuote(_) | Container::List { .. }))
        .count()
}

/// Parse Markdown into an owned, sanitized, bounded [`MarkdownDocument`].
///
/// Smart punctuation stays disabled; only GFM tables, strikethrough, and task
/// lists are enabled (`ADR-0075`).
#[must_use]
#[allow(clippy::too_many_lines)] // One event loop keeps start/end tag invariants auditable in one place.
pub fn parse_markdown(input: &str) -> MarkdownDocument {
    let (bounded, mut truncated) = truncate_utf8(input, MAX_INPUT_BYTES);
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let mut stack = vec![Container::Document(Vec::new())];
    let mut skipped_depth = 0usize;

    for event in Parser::new_ext(bounded, options) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => stack.push(Container::Heading {
                    level: heading_level_num(level),
                    inlines: Vec::new(),
                }),
                Tag::Paragraph => stack.push(Container::Paragraph(Vec::new())),
                Tag::CodeBlock(kind) => {
                    let language = match kind {
                        CodeBlockKind::Indented => None,
                        CodeBlockKind::Fenced(info) => {
                            let token = info.split_whitespace().next().unwrap_or("");
                            if token.is_empty() {
                                None
                            } else {
                                Some(sanitize_text(token))
                            }
                        }
                    };
                    stack.push(Container::CodeBlock {
                        language,
                        content: String::new(),
                    });
                }
                Tag::List(start) => {
                    if nesting_depth(&stack) >= MAX_NESTING_DEPTH {
                        skipped_depth += 1;
                    } else {
                        stack.push(Container::List {
                            ordered_start: start,
                            items: Vec::new(),
                        });
                    }
                }
                Tag::Item => {
                    if skipped_depth == 0 {
                        stack.push(Container::ListItem {
                            task_checked: None,
                            blocks: Vec::new(),
                            inline_buf: Vec::new(),
                        });
                    }
                }
                Tag::BlockQuote(_) => {
                    if nesting_depth(&stack) >= MAX_NESTING_DEPTH {
                        skipped_depth += 1;
                    } else {
                        stack.push(Container::BlockQuote(Vec::new()));
                    }
                }
                Tag::Table(alignments) => stack.push(Container::Table {
                    alignments: alignments.into_iter().map(map_alignment).collect(),
                    headers: Vec::new(),
                    rows: Vec::new(),
                    in_head: false,
                    current_row: Vec::new(),
                    current_cell: Vec::new(),
                }),
                Tag::TableHead => {
                    if let Some(Container::Table {
                        in_head,
                        current_row,
                        ..
                    }) = stack.last_mut()
                    {
                        *in_head = true;
                        current_row.clear();
                    }
                }
                Tag::TableRow => {
                    if let Some(Container::Table { current_row, .. }) = stack.last_mut() {
                        current_row.clear();
                    }
                }
                Tag::TableCell => {
                    if let Some(Container::Table { current_cell, .. }) = stack.last_mut() {
                        current_cell.clear();
                    }
                }
                Tag::Emphasis => stack.push(Container::Emphasis(Vec::new())),
                Tag::Strong => stack.push(Container::Strong(Vec::new())),
                Tag::Strikethrough => stack.push(Container::Strikethrough(Vec::new())),
                Tag::Link { dest_url, .. } => stack.push(Container::Link {
                    label: Vec::new(),
                    destination: sanitize_text(&dest_url),
                }),
                Tag::Image { dest_url, .. } => stack.push(Container::Image {
                    alt: String::new(),
                    destination: sanitize_text(&dest_url),
                }),
                Tag::HtmlBlock
                | Tag::FootnoteDefinition(_)
                | Tag::MetadataBlock(_)
                | Tag::DefinitionList
                | Tag::DefinitionListTitle
                | Tag::DefinitionListDefinition
                | Tag::Superscript
                | Tag::Subscript => {}
            },
            Event::End(end) => match end {
                TagEnd::Heading(_) => {
                    if let Some(Container::Heading { level, inlines }) = stack.pop() {
                        push_block(
                            &mut stack,
                            Block::Heading { level, inlines },
                            &mut truncated,
                        );
                    }
                }
                TagEnd::Paragraph => {
                    if let Some(Container::Paragraph(inlines)) = stack.pop() {
                        push_block(&mut stack, Block::Paragraph(inlines), &mut truncated);
                    }
                }
                TagEnd::CodeBlock => {
                    if let Some(Container::CodeBlock { language, content }) = stack.pop() {
                        push_block(
                            &mut stack,
                            Block::CodeBlock { language, content },
                            &mut truncated,
                        );
                    }
                }
                TagEnd::List(_) => {
                    if skipped_depth > 0 {
                        skipped_depth -= 1;
                    } else if let Some(Container::List {
                        ordered_start,
                        items,
                    }) = stack.pop()
                    {
                        push_block(
                            &mut stack,
                            Block::List {
                                ordered_start,
                                items,
                            },
                            &mut truncated,
                        );
                    }
                }
                TagEnd::Item => {
                    if skipped_depth == 0
                        && let Some(Container::ListItem {
                            task_checked,
                            mut blocks,
                            inline_buf,
                        }) = stack.pop()
                    {
                        if !inline_buf.is_empty() {
                            blocks.push(Block::Paragraph(inline_buf));
                        }
                        if let Some(Container::List { items, .. }) = stack.last_mut() {
                            items.push(ListItem {
                                task_checked,
                                blocks,
                            });
                        } else {
                            for block in blocks {
                                push_block(&mut stack, block, &mut truncated);
                            }
                        }
                    }
                }
                TagEnd::BlockQuote(_) => {
                    if skipped_depth > 0 {
                        skipped_depth -= 1;
                    } else if let Some(Container::BlockQuote(blocks)) = stack.pop() {
                        push_block(&mut stack, Block::BlockQuote(blocks), &mut truncated);
                    }
                }
                TagEnd::Table => {
                    if let Some(Container::Table {
                        alignments,
                        headers,
                        rows,
                        ..
                    }) = stack.pop()
                    {
                        push_block(
                            &mut stack,
                            Block::Table {
                                alignments,
                                headers,
                                rows,
                            },
                            &mut truncated,
                        );
                    }
                }
                TagEnd::TableHead => {
                    if let Some(Container::Table {
                        in_head,
                        headers,
                        current_row,
                        ..
                    }) = stack.last_mut()
                    {
                        *in_head = false;
                        *headers = std::mem::take(current_row);
                    }
                }
                TagEnd::TableRow => {
                    if let Some(Container::Table {
                        rows, current_row, ..
                    }) = stack.last_mut()
                    {
                        rows.push(std::mem::take(current_row));
                    }
                }
                TagEnd::TableCell => {
                    if let Some(Container::Table {
                        current_row,
                        current_cell,
                        ..
                    }) = stack.last_mut()
                    {
                        current_row.push(std::mem::take(current_cell));
                    }
                }
                TagEnd::Emphasis => {
                    if let Some(Container::Emphasis(children)) = stack.pop() {
                        push_inline(&mut stack, Inline::Emphasis(children));
                    }
                }
                TagEnd::Strong => {
                    if let Some(Container::Strong(children)) = stack.pop() {
                        push_inline(&mut stack, Inline::Strong(children));
                    }
                }
                TagEnd::Strikethrough => {
                    if let Some(Container::Strikethrough(children)) = stack.pop() {
                        push_inline(&mut stack, Inline::Strikethrough(children));
                    }
                }
                TagEnd::Link => {
                    if let Some(Container::Link { label, destination }) = stack.pop() {
                        push_inline(&mut stack, Inline::Link { label, destination });
                    }
                }
                TagEnd::Image => {
                    if let Some(Container::Image { alt, destination }) = stack.pop() {
                        push_inline(&mut stack, Inline::Image { alt, destination });
                    }
                }
                TagEnd::HtmlBlock
                | TagEnd::FootnoteDefinition
                | TagEnd::MetadataBlock(_)
                | TagEnd::DefinitionList
                | TagEnd::DefinitionListTitle
                | TagEnd::DefinitionListDefinition
                | TagEnd::Superscript
                | TagEnd::Subscript => {}
            },
            Event::Text(text) => {
                let clean = sanitize_text(&text);
                if let Some(Container::CodeBlock { content, .. }) = stack.last_mut() {
                    content.push_str(&clean);
                } else {
                    push_inline(&mut stack, Inline::Text(clean));
                }
            }
            Event::Code(code) => push_inline(&mut stack, Inline::Code(sanitize_text(&code))),
            Event::Html(html) => {
                let clean = sanitize_text(html.trim_end_matches(['\r', '\n']));
                if !clean.is_empty() {
                    push_block(&mut stack, Block::RawHtml(clean), &mut truncated);
                }
            }
            Event::InlineHtml(html) => {
                push_inline(&mut stack, Inline::RawHtml(sanitize_text(&html)));
            }
            Event::InlineMath(math) | Event::DisplayMath(math) => {
                push_inline(&mut stack, Inline::Text(sanitize_text(&math)));
            }
            Event::FootnoteReference(label) => {
                push_inline(
                    &mut stack,
                    Inline::Text(format!("[^{}]", sanitize_text(&label))),
                );
            }
            Event::SoftBreak => push_inline(&mut stack, Inline::SoftBreak),
            Event::HardBreak => push_inline(&mut stack, Inline::HardBreak),
            Event::Rule => push_block(&mut stack, Block::ThematicBreak, &mut truncated),
            Event::TaskListMarker(checked) => {
                if let Some(Container::ListItem { task_checked, .. }) = stack.last_mut() {
                    *task_checked = Some(checked);
                }
            }
        }
    }

    let blocks = match stack.into_iter().next() {
        Some(Container::Document(blocks)) => blocks,
        _ => Vec::new(),
    };
    MarkdownDocument { blocks, truncated }
}

#[derive(Debug, Clone)]
enum Token {
    Break,
    Span(RenderedSpan),
}

fn flatten_inlines(
    inlines: &[Inline],
    base_role: SemanticRole,
    caps: &RenderCapabilities,
    out: &mut Vec<Token>,
) {
    for inline in inlines {
        match inline {
            Inline::Text(text) => {
                if !text.is_empty() {
                    out.push(Token::Span(RenderedSpan {
                        text: text.clone(),
                        role: base_role,
                    }));
                }
            }
            Inline::Emphasis(children) => {
                flatten_inlines(children, SemanticRole::Emphasis, caps, out);
            }
            Inline::Strong(children) => {
                flatten_inlines(children, SemanticRole::Strong, caps, out);
            }
            Inline::Strikethrough(children) => {
                flatten_inlines(children, SemanticRole::Strikethrough, caps, out);
            }
            Inline::Code(code) => {
                out.push(Token::Span(RenderedSpan {
                    text: format!("`{code}`"),
                    role: SemanticRole::Code,
                }));
            }
            Inline::Link { label, destination } => {
                let mut label_text = String::new();
                for child in label {
                    flatten_inline_into_string(child, &mut label_text);
                }
                if label.is_empty() {
                    out.push(Token::Span(RenderedSpan {
                        text: destination.clone(),
                        role: SemanticRole::LinkText,
                    }));
                } else {
                    flatten_inlines(label, SemanticRole::LinkText, caps, out);
                    if caps.links == LinkDisplay::InlineDestination
                        && !destination.is_empty()
                        && destination != label_text.trim()
                    {
                        out.push(Token::Span(RenderedSpan {
                            text: format!(" ({destination})"),
                            role: SemanticRole::LinkDestination,
                        }));
                    }
                }
            }
            Inline::Image { alt, destination } => {
                let desc = if alt.is_empty() && destination.is_empty() {
                    "[image]".to_string()
                } else if alt.is_empty() {
                    format!("[image: {destination}]")
                } else if caps.links == LinkDisplay::InlineDestination && !destination.is_empty() {
                    format!("[image: {alt} ({destination})]")
                } else {
                    format!("[image: {alt}]")
                };
                out.push(Token::Span(RenderedSpan {
                    text: desc,
                    role: SemanticRole::ImageAlt,
                }));
            }
            Inline::SoftBreak => out.push(Token::Span(RenderedSpan {
                text: " ".to_string(),
                role: base_role,
            })),
            Inline::HardBreak => out.push(Token::Break),
            Inline::RawHtml(html) => out.push(Token::Span(RenderedSpan {
                text: html.clone(),
                role: SemanticRole::RawHtml,
            })),
        }
    }
}

fn push_coalesced(spans: &mut Vec<RenderedSpan>, text: &str, role: SemanticRole) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut()
        && last.role == role
    {
        last.text.push_str(text);
        return;
    }
    spans.push(RenderedSpan {
        text: text.to_string(),
        role,
    });
}

fn split_word_segments(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut in_space = None;
    for (idx, ch) in text.char_indices() {
        let ws = ch == ' ' || ch == '\t';
        match in_space {
            None => in_space = Some(ws),
            Some(prev) if prev != ws => {
                parts.push(&text[start..idx]);
                start = idx;
                in_space = Some(ws);
            }
            Some(_) => {}
        }
    }
    if start < text.len() {
        parts.push(&text[start..]);
    }
    parts
}

/// Wrap semantic tokens onto lines bounded by `max_width` display columns using
/// extended Unicode grapheme cluster boundaries (`unicode_segmentation`).
fn wrap_tokens(
    tokens: &[Token],
    first_prefix: &[RenderedSpan],
    cont_prefix: &[RenderedSpan],
    max_width: usize,
    policy: WidthPolicy,
) -> Vec<RenderedLine> {
    let limit = max_width.max(8);
    let mut lines: Vec<RenderedLine> = Vec::new();
    let mut current: Vec<RenderedSpan> = first_prefix.to_vec();
    let mut current_width: usize = current.iter().map(|s| text_width(&s.text, policy)).sum();
    let cont_width: usize = cont_prefix
        .iter()
        .map(|s| text_width(&s.text, policy))
        .sum();
    let body_cap = limit.saturating_sub(cont_width).max(1);

    let start_next_line = |lines: &mut Vec<RenderedLine>,
                           current: &mut Vec<RenderedSpan>,
                           current_width: &mut usize| {
        lines.push(RenderedLine {
            spans: std::mem::take(current),
        });
        current.extend(cont_prefix.iter().cloned());
        *current_width = cont_width;
    };

    for token in tokens {
        match token {
            Token::Break => {
                start_next_line(&mut lines, &mut current, &mut current_width);
            }
            Token::Span(span) => {
                for segment in split_word_segments(&span.text) {
                    let is_space = segment.chars().all(|c| c == ' ' || c == '\t');
                    let normalized = if is_space { " " } else { segment };
                    let seg_w = text_width(normalized, policy);

                    if is_space {
                        if current_width == cont_width && !lines.is_empty() {
                            continue;
                        }
                        if current_width + seg_w <= limit {
                            push_coalesced(&mut current, normalized, span.role);
                            current_width += seg_w;
                        } else {
                            start_next_line(&mut lines, &mut current, &mut current_width);
                        }
                        continue;
                    }

                    if current_width + seg_w <= limit {
                        push_coalesced(&mut current, normalized, span.role);
                        current_width += seg_w;
                        continue;
                    }

                    // If the word fits on a fresh line, wrap before it.
                    let at_line_start = (lines.is_empty()
                        && current_width
                            == first_prefix
                                .iter()
                                .map(|s| text_width(&s.text, policy))
                                .sum::<usize>())
                        || (!lines.is_empty() && current_width == cont_width);
                    if !at_line_start && seg_w <= body_cap {
                        start_next_line(&mut lines, &mut current, &mut current_width);
                        push_coalesced(&mut current, normalized, span.role);
                        current_width += seg_w;
                        continue;
                    }

                    // Long unbreakable token: break strictly on extended
                    // grapheme cluster boundaries.
                    if !at_line_start && current_width >= limit {
                        start_next_line(&mut lines, &mut current, &mut current_width);
                    }
                    for grapheme in normalized.graphemes(true) {
                        let gw = text_width(grapheme, policy);
                        if current_width + gw > limit && current_width > cont_width {
                            start_next_line(&mut lines, &mut current, &mut current_width);
                        }
                        push_coalesced(&mut current, grapheme, span.role);
                        current_width += gw;
                    }
                }
            }
        }
    }

    lines.push(RenderedLine { spans: current });
    lines
}

fn plain_inlines(inlines: &[Inline], caps: &RenderCapabilities) -> String {
    let mut tokens = Vec::new();
    flatten_inlines(inlines, SemanticRole::Plain, caps, &mut tokens);
    let mut out = String::new();
    for token in tokens {
        match token {
            Token::Break => out.push(' '),
            Token::Span(span) => out.push_str(&span.text),
        }
    }
    out
}

fn pad_cell(text: &str, target: usize, align: TableAlignment, policy: WidthPolicy) -> String {
    let w = text_width(text, policy);
    if w >= target {
        return text.to_string();
    }
    let pad = target - w;
    match align {
        TableAlignment::Right => format!("{}{text}", " ".repeat(pad)),
        TableAlignment::Center => {
            let left = pad / 2;
            let right = pad - left;
            format!("{}{text}{}", " ".repeat(left), " ".repeat(right))
        }
        TableAlignment::None | TableAlignment::Left => format!("{text}{}", " ".repeat(pad)),
    }
}

#[allow(clippy::too_many_lines)] // Table grid + narrow card fallback share column widths and prefixing.
fn layout_table(
    alignments: &[TableAlignment],
    headers: &[Vec<Inline>],
    rows: &[Vec<Vec<Inline>>],
    prefix: &[RenderedSpan],
    caps: &RenderCapabilities,
    out: &mut Vec<RenderedLine>,
) {
    let prefix_w: usize = prefix
        .iter()
        .map(|s| text_width(&s.text, caps.width_policy))
        .sum();
    let avail = caps.width.max(8).saturating_sub(prefix_w);
    let col_count = headers
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(0));
    if col_count == 0 {
        return;
    }

    let header_texts: Vec<String> = (0..col_count)
        .map(|i| {
            headers
                .get(i)
                .map_or_else(String::new, |cell| plain_inlines(cell, caps))
        })
        .collect();
    let row_texts: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            (0..col_count)
                .map(|i| {
                    row.get(i)
                        .map_or_else(String::new, |cell| plain_inlines(cell, caps))
                })
                .collect()
        })
        .collect();

    let mut widths = vec![1usize; col_count];
    for (i, text) in header_texts.iter().enumerate() {
        widths[i] = widths[i].max(text_width(text, caps.width_policy));
    }
    for row in &row_texts {
        for (i, text) in row.iter().enumerate() {
            widths[i] = widths[i].max(text_width(text, caps.width_policy));
        }
    }

    let sep = if caps.ascii { " | " } else { " \u{2502} " };
    let sep_w = text_width(sep, caps.width_policy);
    let total_w: usize = widths.iter().sum::<usize>() + sep_w * col_count.saturating_sub(1);

    if total_w <= avail {
        // Wide grid table fits within available display columns.
        let mut head_spans = prefix.to_vec();
        for (i, text) in header_texts.iter().enumerate() {
            if i > 0 {
                push_coalesced(&mut head_spans, sep, SemanticRole::Rule);
            }
            let align = alignments.get(i).copied().unwrap_or(TableAlignment::None);
            push_coalesced(
                &mut head_spans,
                &pad_cell(text, widths[i], align, caps.width_policy),
                SemanticRole::TableHeader,
            );
        }
        out.push(RenderedLine { spans: head_spans });

        let mut rule_spans = prefix.to_vec();
        let dash = if caps.ascii { "-" } else { "\u{2500}" };
        let cross = if caps.ascii {
            "-+-"
        } else {
            "\u{2500}\u{253c}\u{2500}"
        };
        for (i, &w) in widths.iter().enumerate() {
            if i > 0 {
                push_coalesced(&mut rule_spans, cross, SemanticRole::Rule);
            }
            push_coalesced(&mut rule_spans, &dash.repeat(w), SemanticRole::Rule);
        }
        out.push(RenderedLine { spans: rule_spans });

        for row in &row_texts {
            let mut spans = prefix.to_vec();
            for (i, text) in row.iter().enumerate() {
                if i > 0 {
                    push_coalesced(&mut spans, sep, SemanticRole::Rule);
                }
                let align = alignments.get(i).copied().unwrap_or(TableAlignment::None);
                push_coalesced(
                    &mut spans,
                    &pad_cell(text, widths[i], align, caps.width_policy),
                    SemanticRole::Plain,
                );
            }
            out.push(RenderedLine { spans });
        }
        return;
    }

    // Narrow fallback: labelled vertical card form so no columns clip silently.
    for (row_idx, row) in row_texts.iter().enumerate() {
        if row_idx > 0 {
            let mut divider = prefix.to_vec();
            let dash = if caps.ascii { "-" } else { "\u{2500}" };
            push_coalesced(
                &mut divider,
                &dash.repeat(avail.clamp(3, 16)),
                SemanticRole::Rule,
            );
            out.push(RenderedLine { spans: divider });
        }
        for (col_idx, cell) in row.iter().enumerate() {
            let label = header_texts
                .get(col_idx)
                .filter(|h| !h.is_empty())
                .cloned()
                .unwrap_or_else(|| format!("col {}", col_idx + 1));
            let tokens = vec![
                Token::Span(RenderedSpan {
                    text: format!("{label}: "),
                    role: SemanticRole::TableHeader,
                }),
                Token::Span(RenderedSpan {
                    text: cell.clone(),
                    role: SemanticRole::Plain,
                }),
            ];
            let mut cont = prefix.to_vec();
            push_coalesced(&mut cont, "  ", SemanticRole::Plain);
            out.extend(wrap_tokens(
                &tokens,
                prefix,
                &cont,
                caps.width,
                caps.width_policy,
            ));
        }
    }
}

#[allow(clippy::too_many_lines)] // Exhaustive block match keeps spacing and prefix inheritance coherent.
fn layout_blocks(
    blocks: &[Block],
    prefix: &[RenderedSpan],
    caps: &RenderCapabilities,
    out: &mut Vec<RenderedLine>,
) {
    let limit = caps.width.max(8);
    for (idx, block) in blocks.iter().enumerate() {
        if idx > 0 {
            out.push(RenderedLine {
                spans: prefix.to_vec(),
            });
        }
        match block {
            Block::Heading { level, inlines } => {
                let marker = "#".repeat(usize::from((*level).clamp(1, 6)));
                let mut first = prefix.to_vec();
                push_coalesced(&mut first, &format!("{marker} "), SemanticRole::Heading);
                let mut cont = prefix.to_vec();
                push_coalesced(&mut cont, "  ", SemanticRole::Plain);
                let mut tokens = Vec::new();
                flatten_inlines(inlines, SemanticRole::Heading, caps, &mut tokens);
                out.extend(wrap_tokens(
                    &tokens,
                    &first,
                    &cont,
                    limit,
                    caps.width_policy,
                ));
            }
            Block::Paragraph(inlines) => {
                let mut tokens = Vec::new();
                flatten_inlines(inlines, SemanticRole::Plain, caps, &mut tokens);
                out.extend(wrap_tokens(
                    &tokens,
                    prefix,
                    prefix,
                    limit,
                    caps.width_policy,
                ));
            }
            Block::CodeBlock { language, content } => {
                let mut label_spans = prefix.to_vec();
                let label = match language.as_deref() {
                    Some(lang) => format!("[code: {lang}]"),
                    None => "[code]".to_string(),
                };
                push_coalesced(&mut label_spans, &label, SemanticRole::CodeLabel);
                out.push(RenderedLine { spans: label_spans });

                let border = if caps.ascii { "| " } else { "\u{2502} " };
                let mut code_prefix = prefix.to_vec();
                push_coalesced(&mut code_prefix, border, SemanticRole::QuoteMarker);
                let trimmed = content.strip_suffix('\n').unwrap_or(content);
                if trimmed.is_empty() {
                    out.push(RenderedLine { spans: code_prefix });
                } else {
                    for raw_line in trimmed.split('\n') {
                        let clean_line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
                        let mut spans = code_prefix.clone();
                        push_coalesced(&mut spans, clean_line, SemanticRole::Code);
                        out.push(RenderedLine { spans });
                    }
                }
            }
            Block::List {
                ordered_start,
                items,
            } => {
                for (item_idx, item) in items.iter().enumerate() {
                    let bullet = match ordered_start {
                        Some(start) => format!("{}. ", start.saturating_add(item_idx as u64)),
                        None if caps.ascii => "- ".to_string(),
                        None => "\u{2022} ".to_string(),
                    };
                    let check = match item.task_checked {
                        Some(true) => "[x] ",
                        Some(false) => "[ ] ",
                        None => "",
                    };
                    let marker = format!("{bullet}{check}");
                    let marker_w = text_width(&marker, caps.width_policy);
                    let mut first = prefix.to_vec();
                    push_coalesced(&mut first, &marker, SemanticRole::ListMarker);
                    let mut cont = prefix.to_vec();
                    push_coalesced(&mut cont, &" ".repeat(marker_w), SemanticRole::Plain);

                    if let Some((first_block, rest)) = item.blocks.split_first() {
                        match first_block {
                            Block::Paragraph(inlines) => {
                                let mut tokens = Vec::new();
                                flatten_inlines(inlines, SemanticRole::Plain, caps, &mut tokens);
                                out.extend(wrap_tokens(
                                    &tokens,
                                    &first,
                                    &cont,
                                    limit,
                                    caps.width_policy,
                                ));
                            }
                            other => {
                                out.push(RenderedLine { spans: first });
                                layout_blocks(std::slice::from_ref(other), &cont, caps, out);
                            }
                        }
                        if !rest.is_empty() {
                            layout_blocks(rest, &cont, caps, out);
                        }
                    } else {
                        out.push(RenderedLine { spans: first });
                    }
                }
            }
            Block::BlockQuote(children) => {
                let marker = if caps.ascii { "> " } else { "\u{2502} " };
                let mut child_prefix = prefix.to_vec();
                push_coalesced(&mut child_prefix, marker, SemanticRole::QuoteMarker);
                layout_blocks(children, &child_prefix, caps, out);
            }
            Block::Table {
                alignments,
                headers,
                rows,
            } => layout_table(alignments, headers, rows, prefix, caps, out),
            Block::ThematicBreak => {
                let prefix_w: usize = prefix
                    .iter()
                    .map(|s| text_width(&s.text, caps.width_policy))
                    .sum();
                let rule_w = limit.saturating_sub(prefix_w).clamp(3, 40);
                let ch = if caps.ascii { "-" } else { "\u{2500}" };
                let mut spans = prefix.to_vec();
                push_coalesced(&mut spans, &ch.repeat(rule_w), SemanticRole::Rule);
                out.push(RenderedLine { spans });
            }
            Block::RawHtml(html) => {
                let tokens = vec![Token::Span(RenderedSpan {
                    text: html.clone(),
                    role: SemanticRole::RawHtml,
                })];
                out.extend(wrap_tokens(
                    &tokens,
                    prefix,
                    prefix,
                    limit,
                    caps.width_policy,
                ));
            }
        }
    }
}

/// Lay out a parsed [`MarkdownDocument`] into an I/O-free [`RenderedDocument`]
/// of semantically tagged lines and spans.
#[must_use]
pub fn layout_markdown(doc: &MarkdownDocument, caps: &RenderCapabilities) -> RenderedDocument {
    let mut lines = Vec::new();
    layout_blocks(&doc.blocks, &[], caps, &mut lines);
    if doc.truncated {
        lines.push(RenderedLine {
            spans: vec![RenderedSpan {
                text: "[markdown content truncated to safety limit]".to_string(),
                role: SemanticRole::Muted,
            }],
        });
    }
    RenderedDocument { lines }
}

/// Serialize a [`MarkdownDocument`] to terminal text using `anstyle` when
/// `caps.color` is `true`, or pure plain text when `false`.
///
/// This function performs zero I/O and is strictly deterministic.
#[must_use]
pub fn render_terminal(doc: &MarkdownDocument, caps: &RenderCapabilities) -> String {
    let rendered = layout_markdown(doc, caps);
    let mut out = String::new();
    for (idx, line) in rendered.lines.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        for span in &line.spans {
            if caps.color {
                let st = caps.theme.style_for(span.role);
                if st == style::PLAIN {
                    out.push_str(&span.text);
                } else {
                    let _ = write!(out, "{st}{}{st:#}", span.text);
                }
            } else {
                out.push_str(&span.text);
            }
        }
    }
    out
}

/// Convenience helper: parse and render Markdown directly to terminal text.
#[must_use]
pub fn render_markdown_to_terminal(input: &str, caps: &RenderCapabilities) -> String {
    let doc = parse_markdown(input);
    render_terminal(&doc, caps)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip_ansi(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let mut chars = input.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\x1b' && chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    /// Exhaustive audit over `SemanticMember::ALL` (`A2-114`): every semantic
    /// member must parse, appear in the semantic AST, produce non-empty laid-out
    /// spans, and render identically after stripping ANSI escapes.
    #[test]
    #[allow(clippy::too_many_lines)]
    fn every_semantic_member_parses_lays_out_and_renders_with_color_invariance() {
        for &member in SemanticMember::ALL {
            let (md, expected_text, expected_role) = match member {
                SemanticMember::Heading => (
                    "## Section Title",
                    "## Section Title",
                    SemanticRole::Heading,
                ),
                SemanticMember::Paragraph => {
                    ("Plain paragraph.", "Plain paragraph.", SemanticRole::Plain)
                }
                SemanticMember::CodeBlock => (
                    "```rust\nlet x = 1;\n```",
                    "[code: rust]",
                    SemanticRole::CodeLabel,
                ),
                SemanticMember::List => ("- item one", "- item one", SemanticRole::ListMarker),
                SemanticMember::BlockQuote => {
                    ("> quoted line", "> quoted line", SemanticRole::QuoteMarker)
                }
                SemanticMember::Table => (
                    "| H1 | H2 |\n|---|---|\n| v1 | v2 |",
                    "H1 | H2",
                    SemanticRole::TableHeader,
                ),
                SemanticMember::ThematicBreak => ("---", "---", SemanticRole::Rule),
                SemanticMember::RawHtml => {
                    ("<div>raw</div>", "<div>raw</div>", SemanticRole::RawHtml)
                }
                SemanticMember::Text => ("hello text", "hello text", SemanticRole::Plain),
                SemanticMember::Emphasis => ("*italic*", "italic", SemanticRole::Emphasis),
                SemanticMember::Strong => ("**bold**", "bold", SemanticRole::Strong),
                SemanticMember::Strikethrough => ("~~old~~", "old", SemanticRole::Strikethrough),
                SemanticMember::InlineCode => ("`tok`", "`tok`", SemanticRole::Code),
                SemanticMember::Link => (
                    "[docs](https://example.invalid)",
                    "docs (https://example.invalid)",
                    SemanticRole::LinkText,
                ),
                SemanticMember::Image => (
                    "![diagram](https://example.invalid/a.png)",
                    "[image: diagram (https://example.invalid/a.png)]",
                    SemanticRole::ImageAlt,
                ),
                SemanticMember::SoftBreak => ("a\nb", "a b", SemanticRole::Plain),
                SemanticMember::HardBreak => ("a  \nb", "a\nb", SemanticRole::Plain),
                SemanticMember::TaskCheckbox => {
                    ("- [x] done", "- [x] done", SemanticRole::ListMarker)
                }
                SemanticMember::InlineHtml => ("x <span>y</span>", "<span>", SemanticRole::RawHtml),
            };

            let doc = parse_markdown(md);
            assert!(
                !doc.blocks.is_empty(),
                "member {member:?} produced no semantic blocks"
            );

            let plain_caps = RenderCapabilities {
                width: 80,
                ascii: true,
                color: false,
                ..RenderCapabilities::default()
            };
            let color_caps = RenderCapabilities {
                color: true,
                ..plain_caps
            };

            let laid_out = layout_markdown(&doc, &plain_caps);
            assert!(
                laid_out
                    .lines
                    .iter()
                    .flat_map(|l| &l.spans)
                    .any(|s| s.role == expected_role),
                "member {member:?} did not emit expected role {expected_role:?}: {laid_out:?}"
            );

            let plain = render_terminal(&doc, &plain_caps);
            let colored = render_terminal(&doc, &color_caps);
            assert!(
                plain.contains(expected_text),
                "member {member:?} missing `{expected_text}` in:\n{plain}"
            );
            assert_eq!(
                strip_ansi(&colored),
                plain,
                "member {member:?} broke color invariance"
            );
        }

        // Exhaustive check that every SemanticRole resolves cleanly in MarkdownTheme.
        let theme = MarkdownTheme::default();
        for &role in SemanticRole::ALL {
            let _ = theme.style_for(role);
        }
    }

    #[test]
    fn tables_render_aligned_when_wide_and_labelled_cards_when_narrow() {
        let md = "| Task | Effort | Owner |\n|---|---:|---|\n| A2-114 | 4.0d | alice |\n| A2-115 | 2.0d | bob |\n";
        let doc = parse_markdown(md);

        let wide = render_terminal(
            &doc,
            &RenderCapabilities {
                width: 80,
                ascii: true,
                ..RenderCapabilities::default()
            },
        );
        assert!(wide.contains("Task   | Effort | Owner"), "{wide}");
        assert!(wide.contains("A2-114 |   4.0d | alice"), "{wide}");

        let narrow = render_terminal(
            &doc,
            &RenderCapabilities {
                width: 20,
                ascii: true,
                ..RenderCapabilities::default()
            },
        );
        assert!(narrow.contains("Task: A2-114"), "{narrow}");
        assert!(narrow.contains("Effort: 4.0d"), "{narrow}");
        assert!(narrow.contains("Owner: bob"), "{narrow}");
    }

    #[test]
    fn grapheme_safe_wrapping_respects_combining_zwj_variation_and_width_policy() {
        // Combining accent (e + U+0301), ZWJ family emoji, variation selector heart, and ambiguous char.
        let input = "cafe\u{0301} \u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466} \u{2764}\u{fe0f} \u{00a7}\u{00a7}\u{00a7}\u{00a7}\u{00a7}\u{00a7}\u{00a7}\u{00a7} supercalifragilisticexpialidocious";
        let doc = parse_markdown(input);

        for policy in [WidthPolicy::Standard, WidthPolicy::WideAmbiguous] {
            let caps = RenderCapabilities {
                width: 14,
                ascii: true,
                width_policy: policy,
                ..RenderCapabilities::default()
            };
            let laid_out = layout_markdown(&doc, &caps);
            for line in &laid_out.lines {
                let text = line.plain_text();
                // Every line stays within width and never splits combining/ZWJ sequences.
                assert!(
                    line.display_width(policy) <= 14,
                    "line exceeded 14 cols under {policy:?}: `{text}` (w={})",
                    line.display_width(policy)
                );
                assert!(
                    !text.starts_with('\u{0301}')
                        && !text.starts_with('\u{200d}')
                        && !text.starts_with('\u{fe0f}'),
                    "grapheme split at start of line: `{text}`"
                );
            }
            let joined: String = laid_out
                .lines
                .iter()
                .map(RenderedLine::plain_text)
                .collect();
            assert!(joined.contains("cafe\u{0301}"));
            assert!(
                joined.contains("\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}")
            );
            assert!(joined.contains("\u{2764}\u{fe0f}"));
        }

        // Verify WidthPolicy::WideAmbiguous counts East Asian Ambiguous `§` as 2 columns vs 1.
        assert_eq!(text_width("\u{00a7}", WidthPolicy::Standard), 1);
        assert_eq!(text_width("\u{00a7}", WidthPolicy::WideAmbiguous), 2);
    }

    #[test]
    fn adversarial_escapes_bidi_html_deep_nesting_and_oversized_inputs_are_safe() {
        // CSI, OSC-8, C1 CSI (U+009B), bidi RLO (U+202E), raw HTML, smart-punctuation preservation.
        let hostile = "Hello \x1b[31mred\x1b[0m \x1b]8;;https://evil.invalid\x07click\x1b]8;;\x07 \
                       \u{009b}2J safe\u{202e}gnirts \"straight\" -- 'quotes' \
                       [bad](https://evil.invalid/\x1b[2J) <script>alert(1)</script>";
        let doc = parse_markdown(hostile);
        let out = render_terminal(
            &doc,
            &RenderCapabilities {
                width: 80,
                ascii: true,
                color: false,
                ..RenderCapabilities::default()
            },
        );
        assert!(!out.contains('\x1b'), "raw ESC leaked: {out:?}");
        assert!(!out.contains('\u{009b}'), "C1 CSI leaked: {out:?}");
        assert!(!out.contains('\u{202e}'), "bidi RLO leaked: {out:?}");
        assert!(out.contains(sanitize::MARKER), "marker missing: {out:?}");
        assert!(
            out.contains("\"straight\" -- 'quotes'"),
            "smart punctuation altered chars: {out:?}"
        );
        assert!(
            out.replace('\n', "").contains("<script>alert(1)</script>"),
            "raw HTML vanished: {out:?}"
        );

        // Deep nesting (200 nested blockquotes) stays bounded without stack overflow.
        let deep = format!("{}deep text", "> ".repeat(200));
        let deep_doc = parse_markdown(&deep);
        let deep_out = render_terminal(&deep_doc, &RenderCapabilities::default());
        assert!(deep_out.contains("deep text"));

        // Oversized input truncates cleanly with a visible notice.
        let huge = "a ".repeat(MAX_INPUT_BYTES);
        let huge_doc = parse_markdown(&huge);
        assert!(huge_doc.truncated);
        let huge_out = render_terminal(&huge_doc, &RenderCapabilities::default());
        assert!(huge_out.contains("[markdown content truncated to safety limit]"));
    }

    #[test]
    fn unsupported_constructs_preserve_visible_text_and_never_mutate_source_characters() {
        let md = "Text with footnote[^1] and -- double-dash and ... ellipsis.\n\n[^1]: note body\n";
        let out = render_markdown_to_terminal(md, &RenderCapabilities::default());
        assert!(out.contains("footnote"), "{out}");
        assert!(out.contains("-- double-dash"), "{out}");
        assert!(out.contains("... ellipsis"), "{out}");
    }
}
