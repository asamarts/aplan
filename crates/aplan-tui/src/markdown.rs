//! TUI Ratatui sink and incremental viewport cache for task Markdown bodies
//! (`A2-120`, ADR-0075).
//!
//! Consumes [`aplan_output::markdown::MarkdownDocument`] and
//! [`aplan_output::markdown::RenderedLine`] directly without reparsing on
//! scroll/resize, without exposing parser-crate types, and without inventing a
//! second theme vocabulary outside [`crate::shell::Palette`].

use aplan_output::markdown::{
    IncrementalLayout, LinkDisplay, MarkdownDocument, MarkdownTheme, RenderCapabilities,
    RenderedLine, SemanticRole, WidthPolicy, parse_markdown,
};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::shell::Palette;

/// Map every exhaustive [`SemanticRole`] into a Ratatui [`Style`] using only
/// the existing TUI [`Palette`] tokens and terminal modifiers.
pub(crate) fn style_for_role(role: SemanticRole, styles: Palette) -> Style {
    match role {
        SemanticRole::Plain => styles.normal,
        SemanticRole::Heading | SemanticRole::TableHeader => {
            styles.accent.add_modifier(Modifier::BOLD)
        }
        SemanticRole::Strong => styles.normal.add_modifier(Modifier::BOLD),
        SemanticRole::Emphasis => styles.normal.add_modifier(Modifier::ITALIC),
        SemanticRole::Strikethrough => styles.muted.add_modifier(Modifier::CROSSED_OUT),
        SemanticRole::Code => styles.good,
        SemanticRole::CodeLabel
        | SemanticRole::QuoteMarker
        | SemanticRole::Rule
        | SemanticRole::LinkDestination
        | SemanticRole::Muted => styles.muted,
        SemanticRole::ListMarker => styles.accent,
        SemanticRole::LinkText => styles.accent.add_modifier(Modifier::UNDERLINED),
        SemanticRole::ImageAlt => styles.muted.add_modifier(Modifier::ITALIC),
        SemanticRole::RawHtml => styles.warning,
    }
}

/// Convert one laid-out [`RenderedLine`] into a styled Ratatui [`Line`].
pub(crate) fn rendered_line_to_ratatui(line: &RenderedLine, styles: Palette) -> Line<'static> {
    if line.spans.is_empty() {
        return Line::raw("");
    }
    let spans = line
        .spans
        .iter()
        .map(|span| Span::styled(span.text.clone(), style_for_role(span.role, styles)))
        .collect::<Vec<_>>();
    Line::from(spans)
}

/// Cached parsed Markdown document and incremental viewport layout for the TUI
/// task inspector `Body` section.
///
/// - Scrolling, focus changes, and theme switches reuse the active
///   [`IncrementalLayout`] without reparsing or re-laying out earlier blocks.
/// - Terminal width or ASCII/Unicode changes reuse the parsed
///   [`MarkdownDocument`] without reparsing.
/// - Large bodies materialize only the prefix of blocks required to cover the
///   current viewport plus lookahead.
#[derive(Debug, Default)]
pub(crate) struct InspectorBodyCache {
    initialized: bool,
    source: String,
    doc: Option<MarkdownDocument>,
    layout: Option<IncrementalLayout>,
    layout_width: usize,
    layout_ascii: bool,
    parse_count: usize,
    relayout_count: usize,
}

impl InspectorBodyCache {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn ensure_prepared(&mut self, body: &str, width: usize, ascii: bool, target_lines: usize) {
        let effective_width = width.max(8);
        if !self.initialized || self.source != body {
            self.source.clear();
            self.source.push_str(body);
            self.doc = Some(parse_markdown(body));
            self.layout = None;
            self.initialized = true;
            self.parse_count += 1;
        }
        if self.layout.is_none()
            || self.layout_width != effective_width
            || self.layout_ascii != ascii
        {
            let doc = self
                .doc
                .clone()
                .expect("parsed document exists after initialization");
            let caps = RenderCapabilities {
                width: effective_width,
                ascii,
                color: false,
                links: LinkDisplay::InlineDestination,
                width_policy: WidthPolicy::Standard,
                theme: MarkdownTheme::default(),
            };
            self.layout = Some(IncrementalLayout::new(doc, caps));
            self.layout_width = effective_width;
            self.layout_ascii = ascii;
            self.relayout_count += 1;
        }
        if let Some(layout) = self.layout.as_mut() {
            layout.ensure_lines(target_lines);
        }
    }

    /// Materialize enough lines to cover `target_lines`, returning the total
    /// materialized line count and whether the entire document is laid out.
    pub(crate) fn measure(
        &mut self,
        body: &str,
        width: usize,
        ascii: bool,
        target_lines: usize,
    ) -> (usize, bool) {
        self.ensure_prepared(body, width, ascii, target_lines);
        let layout = self.layout.as_ref().expect("layout initialized");
        (layout.lines().len(), layout.is_complete())
    }

    /// Materialize and convert ONLY the visible slice `start..start+count` into
    /// styled Ratatui lines, returning `(visible_lines, materialized_total, is_complete)`.
    pub(crate) fn render_window(
        &mut self,
        body: &str,
        width: usize,
        ascii: bool,
        start: usize,
        count: usize,
        styles: Palette,
    ) -> (Vec<Line<'static>>, usize, bool) {
        let target = start.saturating_add(count).saturating_add(1);
        self.ensure_prepared(body, width, ascii, target);
        let layout = self.layout.as_ref().expect("layout initialized");
        let lines = layout.lines();
        let visible = lines
            .iter()
            .skip(start)
            .take(count)
            .map(|line| rendered_line_to_ratatui(line, styles))
            .collect();
        (visible, lines.len(), layout.is_complete())
    }

    #[cfg(test)]
    pub(crate) fn stats(&self) -> (usize, usize, usize, usize) {
        let (laid_out, total) = self
            .layout
            .as_ref()
            .map_or((0, 0), |l| (l.laid_out_blocks(), l.total_blocks()));
        (self.parse_count, self.relayout_count, laid_out, total)
    }
}

#[cfg(test)]
mod tests {
    use aplan_output::markdown::{
        RenderCapabilities, SemanticMember, SemanticRole, layout_markdown, parse_markdown,
        render_terminal,
    };

    use super::*;
    use crate::model::{Action, Focus, Glyphs, InspectorSection, Motion, Route, Theme};
    use crate::shell::{palette, tests::buffer as render_buffer_for_test};
    use crate::testing::ModelBuilder;

    #[test]
    fn every_semantic_role_maps_through_every_tui_theme() {
        for theme in [
            Theme::Dark,
            Theme::Light,
            Theme::LowColor,
            Theme::Monochrome,
        ] {
            let styles = palette(theme);
            for &role in SemanticRole::ALL {
                let style = style_for_role(role, styles);
                let line = rendered_line_to_ratatui(
                    &RenderedLine {
                        spans: vec![aplan_output::markdown::RenderedSpan {
                            text: "sample".to_string(),
                            role,
                        }],
                    },
                    styles,
                );
                assert_eq!(line.spans.len(), 1);
                assert_eq!(line.spans[0].style, style);
            }
        }
    }

    #[test]
    fn every_semantic_member_drives_a_tui_disposition_and_matches_cli_semantic_text() {
        let corpus = "\
# Heading One

Paragraph with *emphasis*, **strong**, ~~strike~~, `inline_code`, [docs](https://example.invalid/doc), and ![diagram](https://example.invalid/img.png).
Line with soft break
and hard break  \nnext line.

> Quoted rationale

- [x] Done item
- [ ] Open item

```rust
let x = 1;
```

| Col A | Col B |
|---|---:|
| alpha | 42 |

---

<div>raw html</div> and <span>inline html</span>.
";
        let doc = parse_markdown(corpus);
        assert!(
            !doc.blocks.is_empty(),
            "corpus must produce semantic blocks"
        );

        for (width, ascii, theme) in [
            (80, false, Theme::Dark),
            (80, true, Theme::Monochrome),
            (28, true, Theme::LowColor),
        ] {
            let caps = RenderCapabilities {
                width,
                ascii,
                color: false,
                ..RenderCapabilities::default()
            };
            let cli_plain = render_terminal(&doc, &caps);
            let laid_out = layout_markdown(&doc, &caps);
            let styles = palette(theme);
            let tui_plain = laid_out
                .lines
                .iter()
                .map(|line| {
                    let rline = rendered_line_to_ratatui(line, styles);
                    rline
                        .spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");

            assert_eq!(
                tui_plain, cli_plain,
                "CLI and TUI must consume the exact same semantic layout for width={width} ascii={ascii}"
            );
        }

        // Verify every SemanticMember::ALL construct appears visibly in the rendered TUI output.
        let mut cache = InspectorBodyCache::new();
        let (lines, _, complete) =
            cache.render_window(corpus, 160, false, 0, 200, palette(Theme::Monochrome));
        assert!(complete);
        let full = lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        for &member in SemanticMember::ALL {
            let expected = match member {
                SemanticMember::Heading => "# Heading One",
                SemanticMember::Paragraph | SemanticMember::Text => "Paragraph with",
                SemanticMember::Emphasis => "emphasis",
                SemanticMember::Strong => "strong",
                SemanticMember::Strikethrough => "strike",
                SemanticMember::InlineCode => "`inline_code`",
                SemanticMember::Link => "docs (https://example.invalid/doc)",
                SemanticMember::Image => "[image: diagram (https://example.invalid/img.png)]",
                SemanticMember::SoftBreak => "soft break and hard break",
                SemanticMember::HardBreak => "next line.",
                SemanticMember::BlockQuote => "Quoted rationale",
                SemanticMember::List | SemanticMember::TaskCheckbox => "[x] Done item",
                SemanticMember::CodeBlock => "[code: rust]",
                SemanticMember::Table => "Col A",
                SemanticMember::ThematicBreak => "\u{2500}",
                SemanticMember::RawHtml => "<div>raw html</div>",
                SemanticMember::InlineHtml => "<span>inline html</span>",
            };
            assert!(
                full.contains(expected),
                "missing TUI rendering for SemanticMember::{member:?} (expected {expected:?}) in:\n{full}"
            );
        }
    }

    #[test]
    fn large_body_scrolls_incrementally_without_reparse_or_redundant_redraw() {
        use std::fmt::Write as _;
        let mut large_body = String::new();
        for i in 0..200 {
            let _ = writeln!(
                large_body,
                "## Section {i}\n\nParagraph {i} with `code_{i}`.\n"
            );
        }

        let mut model = ModelBuilder::default().build();
        model.theme = Theme::Dark;
        model.glyphs = Glyphs::Unicode;
        model.motion = Motion::Reduced;
        model.snapshot.inspectors[0].body = Some(large_body);
        model.update(Action::Resize {
            columns: 100,
            rows: 24,
        });
        model.update(Action::Route(Route::Tasks));
        model.update(Action::Focus(Focus::Detail));
        model.inspector_section = InspectorSection::Body;
        model.inspector_scroll = Some(0);

        let initial = render_buffer_for_test(&mut model, 100, 24);
        assert!(initial.contains("Section 0"), "{initial}");
        assert!(
            initial.contains("aplan show --raw-body"),
            "body inspector should advertise the CLI raw-body route:\n{initial}"
        );

        let (parses, relayouts, laid_out, total) = model.inspector_body_cache.borrow().stats();
        assert_eq!(parses, 1, "body must be parsed once");
        assert_eq!(relayouts, 1, "body must be laid out once");
        assert!(
            laid_out < total,
            "large body must not eagerly lay out all {total} blocks on first paint (laid_out={laid_out})"
        );

        // Redundant scroll to the current offset must not mark the model dirty.
        let _ = model.take_dirty();
        model.update(Action::SetInspectorScroll(0));
        assert!(
            !model.take_dirty(),
            "no-op scroll at boundary must not schedule a redraw"
        );

        // Scroll down: no reparse, no relayout, laid_out blocks grows incrementally.
        model.update(Action::SetInspectorScroll(20));
        assert!(model.take_dirty());
        let scrolled = render_buffer_for_test(&mut model, 100, 24);
        assert!(!scrolled.contains("## Section 0"), "{scrolled}");
        let (parses2, relayouts2, laid_out2, _) = model.inspector_body_cache.borrow().stats();
        assert_eq!(parses2, 1, "scrolling must never reparse");
        assert_eq!(relayouts2, 1, "scrolling must never reset layout");
        assert!(
            laid_out2 > laid_out,
            "scrolling down must advance incremental block layout"
        );

        // Theme switch and focus toggle: neither reparses nor re-lays out.
        model.update(Action::CycleTheme);
        model.update(Action::Focus(Focus::Primary));
        model.update(Action::Focus(Focus::Detail));
        let _ = render_buffer_for_test(&mut model, 100, 24);
        let (parses3, relayouts3, _, _) = model.inspector_body_cache.borrow().stats();
        assert_eq!(parses3, 1);
        assert_eq!(relayouts3, 1);

        // Resize width: re-lays out for new width WITHOUT reparsing.
        model.update(Action::Resize {
            columns: 72,
            rows: 24,
        });
        let _ = render_buffer_for_test(&mut model, 72, 24);
        let (parses4, relayouts4, _, _) = model.inspector_body_cache.borrow().stats();
        assert_eq!(parses4, 1, "resize must reuse parsed document");
        assert_eq!(relayouts4, 2, "resize must relayout for new width");
    }

    #[test]
    fn hostile_malformed_unicode_and_deep_nesting_stay_inert_and_bounded() {
        let mut hostile = String::from(
            "Hostile \x1b[31mred\x1b[0m \x1b]8;;https://evil.invalid\x07click\x1b]8;;\x07 \
             safe\u{202e}gnirts <script>alert(1)</script>\n\n\
             Unicode: \u{1f469}\u{200d}\u{1f52c} \u{6f22}\u{5b57} e\u{0301} \
             supercalifragilisticexpialidocious_unbroken_identifier_token_1234567890\n\n\
             [link](file:///etc/passwd) ![img](https://evil.invalid/x.png)\n\n\
             **unclosed bold `unclosed code\n\n\
             | Column One | Column Two | Column Three |\n\
             |---|---|---|\n\
             | alpha | beta | gamma |\n\n",
        );
        for _ in 0..40 {
            hostile.push_str("> ");
        }
        hostile.push_str("deeply nested\n");

        let mut model = ModelBuilder::default().build();
        model.theme = Theme::Monochrome;
        model.glyphs = Glyphs::Ascii;
        model.motion = Motion::Reduced;
        model.snapshot.inspectors[0].body = Some(hostile);
        model.update(Action::Resize {
            columns: 80,
            rows: 30,
        });
        model.update(Action::Route(Route::Tasks));
        model.update(Action::Focus(Focus::Detail));
        model.inspector_section = InspectorSection::Body;
        model.inspector_scroll = Some(0);

        let view = render_buffer_for_test(&mut model, 80, 30);
        assert!(!view.contains('\x1b'), "no raw ESC may reach TUI cells");
        assert!(
            !view.contains('\u{202e}'),
            "no raw bidi override may reach TUI cells"
        );
        assert!(view.contains("<script>alert(1)"), "{view}");
        assert!(view.contains("Column One: alpha"), "{view}");
        assert!(view.contains("**unclosed bold"), "{view}");

        // Links/images/HTML inside Body must remain inert text (Enter cannot open/spawn/fetch).
        assert!(
            !model.inspector_item_has_link(),
            "Body section must not expose executable links"
        );
        model.update(Action::OpenDetail);
        assert_eq!(model.focus, Focus::Detail);
        assert_eq!(model.inspector_section, InspectorSection::Body);
    }
}
