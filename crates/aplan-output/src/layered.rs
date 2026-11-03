//! Static terminal painting for validated layered graph cells.
//!
//! The application layer owns projection, ranking, routing, accounting and
//! every semantic identity. This module owns only terminal glyphs, safe text,
//! palette roles and complete-scene width validation. Its input is an adapter
//! record rather than `aplan-app`'s scene type so the output crate remains a
//! leaf and the dependency graph stays acyclic.

use std::fmt::Write as _;

use anstyle::Style;
use aplan_core::Status;
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

const MAX_CANVAS_CELLS: usize = 8_000_000;

/// Concrete direction already selected by the layout layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayeredDirection {
    LeftToRight,
    TopToBottom,
}

/// Card detail already used to measure the shared scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayeredDetail {
    Compact,
    Comfortable,
}

/// One terminal-cell coordinate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct LayeredPoint {
    pub x: i32,
    pub y: i32,
}

/// One non-empty terminal-cell rectangle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LayeredRect {
    pub x: i32,
    pub y: i32,
    pub width: u16,
    pub height: u16,
}

/// Aggregate facts painted inside a comfortable card.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LayeredFacts {
    pub tasks: usize,
    pub done: usize,
    pub ready: usize,
}

/// One already-placed semantic node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayeredNode {
    pub id: String,
    /// Canonical task ids represented by this card at the active semantic level.
    pub represented_ids: Vec<String>,
    /// Dependency rank assigned by the shared cell scene.
    pub rank: Option<i32>,
    /// Stable peer order assigned inside the rank.
    pub order: u32,
    pub title: String,
    pub status: Status,
    pub bounds: LayeredRect,
    pub facts: LayeredFacts,
    pub selected: bool,
    pub related: bool,
    pub critical: bool,
}

/// One already-routed semantic edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayeredEdge {
    pub from: String,
    pub to: String,
    /// Stable ordinal among relationships with the same visible endpoints.
    pub ordinal: u32,
    /// Canonical task dependencies represented by this visible relationship.
    pub provenance: Vec<(String, String)>,
    pub points: Vec<LayeredPoint>,
    pub source_port: Option<LayeredPoint>,
    pub target_port: Option<LayeredPoint>,
    pub arrowhead: Option<LayeredPoint>,
    pub feedback: bool,
    pub bundled: bool,
    pub related: bool,
    /// Typed layout degradations, empty when the connector is complete.
    pub degradations: Vec<String>,
}

/// One already-placed phase grouping band.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayeredBand {
    pub phase: u32,
    /// Typed semantic identities contained by this phase band.
    pub nodes: Vec<String>,
    pub bounds: Option<LayeredRect>,
    pub label_anchor: Option<LayeredPoint>,
}

/// The complete validated scene plus human notes derived by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayeredTextScene {
    pub direction: LayeredDirection,
    pub bounds: LayeredRect,
    pub nodes: Vec<LayeredNode>,
    pub edges: Vec<LayeredEdge>,
    pub bands: Vec<LayeredBand>,
    pub crossings: Vec<LayeredPoint>,
    pub junctions: Vec<LayeredPoint>,
    /// Exact semantic omissions and dependency accounting retained from the
    /// application projection. Painting does not reinterpret these fields.
    pub omissions: LayeredOmissions,
    pub notes: Vec<String>,
}

/// Exact omissions/accounting carried across the application/output boundary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayeredOmissions {
    pub admitted_task_ids: Vec<String>,
    pub level_task_ids: Vec<String>,
    pub visible_task_ids: Vec<String>,
    pub level_omitted_task_ids: Vec<String>,
    pub window_omitted_task_ids: Vec<String>,
    pub admitted_dependency_edges: usize,
    pub represented_dependency_edges: usize,
    pub boundary_dependency_edges: usize,
    pub boundary_dependencies: Vec<(String, String)>,
}

/// Static paint choices which never change graph meaning or geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayeredRenderOptions {
    pub width: usize,
    pub ascii: bool,
    pub detail: LayeredDetail,
}

/// Standard output terminal width without the prose renderer's readability cap.
#[must_use]
pub fn attended_width() -> Option<usize> {
    terminal_size::terminal_size_of(std::io::stdout()).map(|(width, _)| usize::from(width.0))
}

/// Why a complete static scene cannot be represented honestly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayeredRenderError {
    TooWide {
        required: usize,
        available: usize,
    },
    TooLarge {
        columns: usize,
        rows: usize,
    },
    InvalidBounds,
    IncompleteEdge {
        from: String,
        to: String,
        ordinal: u32,
        reasons: Vec<String>,
    },
}

impl std::fmt::Display for LayeredRenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooWide {
                required,
                available,
            } => write!(
                f,
                "the layered graph needs {required} columns but `--width {available}` allows fewer. Increase `--width`, use `--detail compact` or `--orientation top-to-bottom`, or narrow the graph with `--cut`, `--tag` or `--reach`"
            ),
            Self::TooLarge { columns, rows } => write!(
                f,
                "the layered scene is {columns} by {rows} cells, which exceeds the static painter's bounded canvas. Narrow the graph with `--cut`, `--tag` or `--reach`"
            ),
            Self::InvalidBounds => write!(
                f,
                "the layered scene has invalid terminal-cell bounds; use `--layout lanes` and report this layout defect"
            ),
            Self::IncompleteEdge {
                from,
                to,
                ordinal,
                reasons,
            } => write!(
                f,
                "the layered scene cannot draw dependency `{from}` -> `{to}` (edge {ordinal}): {}. Use `--layout lanes` and report this layout defect",
                reasons.join(", ")
            ),
        }
    }
}

impl std::error::Error for LayeredRenderError {}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum PaintRole {
    #[default]
    Normal,
    Muted,
    Id,
    Status(Status),
    Accent,
    Selected,
    Warning,
}

impl PaintRole {
    fn style(self) -> Style {
        match self {
            Self::Normal => Style::new(),
            Self::Muted => crate::style::DIM,
            Self::Id | Self::Accent => crate::style::ID,
            Self::Status(status) => crate::style::status(status.as_str()),
            Self::Selected => crate::style::HEADING,
            Self::Warning => crate::style::WARNING,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Cell {
    symbol: String,
    role: PaintRole,
    continuation: bool,
}

struct Canvas {
    origin: LayeredPoint,
    columns: usize,
    rows: usize,
    cells: Vec<Cell>,
    ascii: bool,
}

impl Canvas {
    fn new(scene: &LayeredTextScene, ascii: bool) -> Result<Self, LayeredRenderError> {
        let columns = usize::from(scene.bounds.width);
        let rows = usize::from(scene.bounds.height);
        if columns == 0 || rows == 0 {
            return Err(LayeredRenderError::InvalidBounds);
        }
        let cells = columns
            .checked_mul(rows)
            .filter(|cells| *cells <= MAX_CANVAS_CELLS)
            .ok_or(LayeredRenderError::TooLarge { columns, rows })?;
        Ok(Self {
            origin: LayeredPoint {
                x: scene.bounds.x,
                y: scene.bounds.y,
            },
            columns,
            rows,
            cells: vec![Cell::default(); cells],
            ascii,
        })
    }

    fn index(&self, point: LayeredPoint) -> Option<usize> {
        let x = usize::try_from(point.x.checked_sub(self.origin.x)?).ok()?;
        let y = usize::try_from(point.y.checked_sub(self.origin.y)?).ok()?;
        (x < self.columns && y < self.rows).then_some(y * self.columns + x)
    }

    fn put(&mut self, point: LayeredPoint, symbol: &str, role: PaintRole) {
        let Some(index) = self.index(point) else {
            return;
        };
        self.cells[index] = Cell {
            symbol: symbol.to_string(),
            role,
            continuation: false,
        };
    }

    fn edge(&mut self, point: LayeredPoint, symbol: &str, role: PaintRole) {
        let Some(index) = self.index(point) else {
            return;
        };
        let occupied = !self.cells[index].symbol.is_empty();
        let crossing = if self.ascii { "+" } else { "\u{253c}" };
        self.cells[index] = Cell {
            symbol: if occupied {
                crossing.to_string()
            } else {
                symbol.to_string()
            },
            role,
            continuation: false,
        };
    }

    fn text(&mut self, start: LayeredPoint, value: &str, width: usize, role: PaintRole) {
        if width == 0 {
            return;
        }
        let value = safe_one_line(value);
        let value = fit_text(&value, width, self.ascii);
        let mut column = 0usize;
        for grapheme in value.graphemes(true) {
            let cells = grapheme.width();
            if cells == 0 || column.saturating_add(cells) > width {
                continue;
            }
            let Some(x) = start
                .x
                .checked_add(i32::try_from(column).unwrap_or(i32::MAX))
            else {
                break;
            };
            let point = LayeredPoint { x, y: start.y };
            let Some(index) = self.index(point) else {
                column = column.saturating_add(cells);
                continue;
            };
            self.cells[index] = Cell {
                symbol: grapheme.to_string(),
                role,
                continuation: false,
            };
            for offset in 1..cells {
                if let Some(index) = self.index(LayeredPoint {
                    x: x.saturating_add(i32::try_from(offset).unwrap_or(i32::MAX)),
                    y: start.y,
                }) {
                    self.cells[index] = Cell {
                        continuation: true,
                        ..Cell::default()
                    };
                }
            }
            column = column.saturating_add(cells);
        }
    }

    fn outline(&mut self, rect: LayeredRect, role: PaintRole, selected: bool) {
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        let glyphs = if self.ascii && selected {
            ("=", "!", "#", "#", "#", "#")
        } else if self.ascii {
            ("-", "|", "+", "+", "+", "+")
        } else if selected {
            (
                "\u{2550}", "\u{2551}", "\u{2554}", "\u{2557}", "\u{255a}", "\u{255d}",
            )
        } else {
            (
                "\u{2500}", "\u{2502}", "\u{250c}", "\u{2510}", "\u{2514}", "\u{2518}",
            )
        };
        let right = rect
            .x
            .saturating_add(i32::from(rect.width))
            .saturating_sub(1);
        let bottom = rect
            .y
            .saturating_add(i32::from(rect.height))
            .saturating_sub(1);
        for x in rect.x..=right {
            self.put(LayeredPoint { x, y: rect.y }, glyphs.0, role);
            self.put(LayeredPoint { x, y: bottom }, glyphs.0, role);
        }
        for y in rect.y..=bottom {
            self.put(LayeredPoint { x: rect.x, y }, glyphs.1, role);
            self.put(LayeredPoint { x: right, y }, glyphs.1, role);
        }
        self.put(
            LayeredPoint {
                x: rect.x,
                y: rect.y,
            },
            glyphs.2,
            role,
        );
        self.put(
            LayeredPoint {
                x: right,
                y: rect.y,
            },
            glyphs.3,
            role,
        );
        self.put(
            LayeredPoint {
                x: rect.x,
                y: bottom,
            },
            glyphs.4,
            role,
        );
        self.put(
            LayeredPoint {
                x: right,
                y: bottom,
            },
            glyphs.5,
            role,
        );
    }

    fn render(&self) -> String {
        let mut out = String::new();
        for row in 0..self.rows {
            let start = row * self.columns;
            let cells = &self.cells[start..start + self.columns];
            let last = cells
                .iter()
                .rposition(|cell| !cell.symbol.is_empty() || cell.continuation);
            let Some(last) = last else {
                out.push('\n');
                continue;
            };
            let mut active = PaintRole::Normal;
            for cell in &cells[..=last] {
                if cell.continuation {
                    continue;
                }
                let role = cell.role;
                if role != active {
                    let reset = active.style();
                    if active != PaintRole::Normal {
                        let _ = write!(out, "{reset:#}");
                    }
                    let style = role.style();
                    if role != PaintRole::Normal {
                        let _ = write!(out, "{style}");
                    }
                    active = role;
                }
                if cell.symbol.is_empty() {
                    out.push(' ');
                } else {
                    out.push_str(&cell.symbol);
                }
            }
            if active != PaintRole::Normal {
                let style = active.style();
                let _ = write!(out, "{style:#}");
            }
            out.push('\n');
        }
        out
    }
}

/// Paint one complete validated scene, refusing rather than clipping width.
///
/// # Errors
///
/// Returns a typed refusal when the supplied width cannot contain the scene,
/// the bounds are invalid, or the public adapter record exceeds its defensive
/// allocation limit.
pub fn text_of_layered_scene(
    scene: &LayeredTextScene,
    options: LayeredRenderOptions,
) -> Result<String, LayeredRenderError> {
    if let Some((edge, reasons)) = scene.edges.iter().find_map(|edge| {
        let reasons = incomplete_edge_reasons(edge);
        (!reasons.is_empty()).then_some((edge, reasons))
    }) {
        return Err(LayeredRenderError::IncompleteEdge {
            from: edge.from.clone(),
            to: edge.to.clone(),
            ordinal: edge.ordinal,
            reasons,
        });
    }
    let required = usize::from(scene.bounds.width);
    if required > options.width {
        return Err(LayeredRenderError::TooWide {
            required,
            available: options.width,
        });
    }
    let mut canvas = Canvas::new(scene, options.ascii)?;
    for band in &scene.bands {
        if let Some(bounds) = band.bounds {
            canvas.outline(bounds, PaintRole::Muted, false);
        }
        if let Some(anchor) = band.label_anchor {
            canvas.text(
                anchor,
                &format!(" phase {} ", band.phase),
                required,
                PaintRole::Accent,
            );
        }
    }
    for edge in &scene.edges {
        let role = if !edge.degradations.is_empty() || edge.feedback {
            PaintRole::Warning
        } else if edge.related {
            PaintRole::Accent
        } else {
            PaintRole::Muted
        };
        for pair in edge.points.windows(2) {
            paint_segment(&mut canvas, pair[0], pair[1], edge, role);
        }
    }
    for point in &scene.crossings {
        canvas.put(
            *point,
            if options.ascii { "+" } else { "\u{253c}" },
            PaintRole::Warning,
        );
    }
    for point in &scene.junctions {
        canvas.put(
            *point,
            if options.ascii { "*" } else { "\u{25c6}" },
            PaintRole::Accent,
        );
    }
    for node in &scene.nodes {
        paint_node(&mut canvas, node, options.detail, options.ascii);
    }
    // Cards deliberately repaint over routes. Restore typed ports and target
    // arrows last so a card border cannot erase edge direction.
    for edge in &scene.edges {
        let role = if !edge.degradations.is_empty() || edge.feedback {
            PaintRole::Warning
        } else if edge.related {
            PaintRole::Accent
        } else {
            PaintRole::Muted
        };
        if let Some(point) = edge.source_port {
            canvas.put(point, if options.ascii { "o" } else { "\u{25cb}" }, role);
        }
        if let Some(point) = edge.target_port.or(edge.arrowhead) {
            canvas.put(point, arrow_glyph(edge, options.ascii), role);
        }
    }
    let mut out = canvas.render();
    out.push('\n');
    let legend = if options.ascii {
        "legend: > selected; + related; C critical; o source port; >/<^v target arrow; : feedback; = bundled; * junction; + crossing"
    } else {
        "legend: \u{25b6} selected; \u{2022} related; C critical; \u{25cb} source port; \u{25b6}/\u{25c0}/\u{25b2}/\u{25bc} target arrow; \u{2504} feedback; \u{2550} bundled; \u{25c6} junction; \u{253c} crossing"
    };
    for note in std::iter::once(legend.to_string()).chain(scene.notes.iter().cloned()) {
        out.push_str(&crate::fold::fold(&safe_one_line(&note), options.width, 0));
        out.push('\n');
    }
    Ok(out)
}

fn incomplete_edge_reasons(edge: &LayeredEdge) -> Vec<String> {
    let mut reasons = edge.degradations.clone();
    let mut add = |reason: &str| {
        if !reasons.iter().any(|existing| existing == reason) {
            reasons.push(reason.to_string());
        }
    };
    if edge.points.len() < 2 {
        add("collapsed_route");
    }
    if edge.source_port != edge.points.first().copied() {
        add("invalid_source_port");
    }
    if edge.target_port != edge.points.last().copied() {
        add("invalid_target_port");
    }
    if edge.arrowhead != edge.points.last().copied() {
        add("invalid_arrowhead");
    }
    if edge
        .points
        .windows(2)
        .any(|pair| pair[0].x != pair[1].x && pair[0].y != pair[1].y)
    {
        add("non_orthogonal_segment");
    }
    reasons
}

fn paint_segment(
    canvas: &mut Canvas,
    from: LayeredPoint,
    to: LayeredPoint,
    edge: &LayeredEdge,
    role: PaintRole,
) {
    let dx = (to.x - from.x).signum();
    let dy = (to.y - from.y).signum();
    if dx != 0 && dy != 0 {
        return;
    }
    let length = to.x.abs_diff(from.x).saturating_add(to.y.abs_diff(from.y));
    let glyph = if edge.feedback {
        if canvas.ascii { ":" } else { "\u{2504}" }
    } else if edge.bundled {
        if canvas.ascii { "=" } else { "\u{2550}" }
    } else if dx == 0 {
        if canvas.ascii { "|" } else { "\u{2502}" }
    } else if canvas.ascii {
        "-"
    } else {
        "\u{2500}"
    };
    let mut point = from;
    for _ in 0..=length {
        canvas.edge(point, glyph, role);
        point.x = point.x.saturating_add(dx);
        point.y = point.y.saturating_add(dy);
    }
}

fn arrow_glyph(edge: &LayeredEdge, ascii: bool) -> &'static str {
    let Some(arrow) = edge.target_port.or(edge.arrowhead) else {
        return if ascii { ">" } else { "\u{25b6}" };
    };
    let Some(previous) = edge.points.iter().rev().find(|point| **point != arrow) else {
        return if ascii { ">" } else { "\u{25b6}" };
    };
    if arrow.x > previous.x {
        if ascii { ">" } else { "\u{25b6}" }
    } else if arrow.x < previous.x {
        if ascii { "<" } else { "\u{25c0}" }
    } else if arrow.y > previous.y {
        if ascii { "v" } else { "\u{25bc}" }
    } else if ascii {
        "^"
    } else {
        "\u{25b2}"
    }
}

fn paint_node(canvas: &mut Canvas, node: &LayeredNode, detail: LayeredDetail, ascii: bool) {
    let role = if node.selected {
        PaintRole::Selected
    } else if node.related || node.critical {
        PaintRole::Accent
    } else {
        PaintRole::Normal
    };
    canvas.outline(node.bounds, role, node.selected);
    let width = usize::from(node.bounds.width.saturating_sub(2));
    if width == 0 || node.bounds.height < 2 {
        return;
    }
    let marker = if node.selected {
        Some(if ascii { ">" } else { "\u{25b6}" })
    } else if node.related {
        Some(if ascii { "+" } else { "\u{2022}" })
    } else if node.critical {
        Some("C")
    } else {
        None
    };
    if let Some(marker) = marker {
        canvas.put(
            LayeredPoint {
                x: node.bounds.x.saturating_add(1),
                y: node.bounds.y,
            },
            marker,
            role,
        );
    }
    // A critical-only card already uses `C` as its primary marker. Preserve a
    // second cue only when selection/relationship took that primary slot.
    if node.critical && (node.selected || node.related) && node.bounds.width >= 4 {
        canvas.put(
            LayeredPoint {
                x: node.bounds.x.saturating_add(2),
                y: node.bounds.y,
            },
            "C",
            PaintRole::Accent,
        );
    }
    canvas.text(
        LayeredPoint {
            x: node.bounds.x.saturating_add(1),
            y: node.bounds.y.saturating_add(1),
        },
        &node.id,
        width,
        PaintRole::Id,
    );
    if node.bounds.height >= 3 {
        let status = status_label(node.status);
        let line = if detail == LayeredDetail::Comfortable {
            format!("{status} {}", node.title)
        } else {
            status.to_string()
        };
        canvas.text(
            LayeredPoint {
                x: node.bounds.x.saturating_add(1),
                y: node.bounds.y.saturating_add(2),
            },
            &line,
            width,
            PaintRole::Status(node.status),
        );
    }
    if detail == LayeredDetail::Comfortable && node.bounds.height >= 5 && node.facts.tasks > 0 {
        canvas.text(
            LayeredPoint {
                x: node.bounds.x.saturating_add(1),
                y: node.bounds.y.saturating_add(3),
            },
            &format!(
                "{}/{} done | {} ready",
                node.facts.done, node.facts.tasks, node.facts.ready
            ),
            width,
            PaintRole::Muted,
        );
    }
}

const fn status_label(status: Status) -> &'static str {
    match status {
        Status::Todo => "TODO",
        Status::InProgress => "DOING",
        Status::Blocked => "BLOCKED",
        Status::Done => "DONE",
        Status::Dropped => "DROPPED",
    }
}

fn safe_one_line(value: &str) -> String {
    crate::sanitize::visible(value)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn fit_text(value: &str, width: usize, ascii: bool) -> String {
    if value.width() <= width {
        return value.to_string();
    }
    let marker = if ascii { "~" } else { "\u{2026}" };
    let marker_width = marker.width();
    if width < marker_width {
        return String::new();
    }
    let target = width.saturating_sub(marker_width);
    let mut out = String::new();
    let mut used = 0usize;
    for grapheme in value.graphemes(true) {
        let cells = grapheme.width();
        if used.saturating_add(cells) > target {
            break;
        }
        out.push_str(grapheme);
        used = used.saturating_add(cells);
    }
    out.push_str(marker);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> LayeredTextScene {
        LayeredTextScene {
            direction: LayeredDirection::LeftToRight,
            bounds: LayeredRect {
                x: 10,
                y: 20,
                width: 34,
                height: 8,
            },
            nodes: vec![
                LayeredNode {
                    id: "A".to_string(),
                    represented_ids: vec!["A".to_string()],
                    rank: Some(0),
                    order: 0,
                    title: "first".to_string(),
                    status: Status::Todo,
                    bounds: LayeredRect {
                        x: 11,
                        y: 21,
                        width: 10,
                        height: 4,
                    },
                    facts: LayeredFacts::default(),
                    selected: true,
                    related: false,
                    critical: false,
                },
                LayeredNode {
                    id: "B".to_string(),
                    represented_ids: vec!["B".to_string()],
                    rank: Some(1),
                    order: 0,
                    title: "second".to_string(),
                    status: Status::Done,
                    bounds: LayeredRect {
                        x: 31,
                        y: 21,
                        width: 11,
                        height: 4,
                    },
                    facts: LayeredFacts::default(),
                    selected: false,
                    related: true,
                    critical: true,
                },
            ],
            edges: vec![LayeredEdge {
                from: "A".to_string(),
                to: "B".to_string(),
                ordinal: 0,
                provenance: vec![("A".to_string(), "B".to_string())],
                points: vec![LayeredPoint { x: 21, y: 23 }, LayeredPoint { x: 30, y: 23 }],
                source_port: Some(LayeredPoint { x: 21, y: 23 }),
                target_port: Some(LayeredPoint { x: 30, y: 23 }),
                arrowhead: Some(LayeredPoint { x: 30, y: 23 }),
                feedback: false,
                bundled: false,
                related: true,
                degradations: Vec::new(),
            }],
            bands: Vec::new(),
            crossings: Vec::new(),
            junctions: Vec::new(),
            omissions: LayeredOmissions::default(),
            notes: vec!["level: plan; 2/2 nodes".to_string()],
        }
    }

    #[test]
    fn static_layered_scene_paints_complete_ascii_geometry_and_notes() {
        let rendered = text_of_layered_scene(
            &scene(),
            LayeredRenderOptions {
                width: 34,
                ascii: true,
                detail: LayeredDetail::Compact,
            },
        )
        .expect("scene fits");
        let plain = anstream::adapter::strip_str(&rendered).to_string();
        assert!(plain.contains("#>=======#"));
        assert!(plain.contains('A'));
        assert!(plain.contains('B'));
        assert!(plain.contains("o-------->"));
        assert!(plain.contains("level: plan; 2/2 nodes"));
    }

    #[test]
    fn static_layered_scene_refuses_width_instead_of_clipping() {
        assert_eq!(
            text_of_layered_scene(
                &scene(),
                LayeredRenderOptions {
                    width: 33,
                    ascii: true,
                    detail: LayeredDetail::Compact,
                },
            ),
            Err(LayeredRenderError::TooWide {
                required: 34,
                available: 33,
            })
        );
    }

    #[test]
    fn static_layered_scene_refuses_an_oversized_canvas_before_allocation() {
        let mut oversized = scene();
        oversized.bounds.width = u16::MAX;
        oversized.bounds.height = u16::MAX;
        assert_eq!(
            text_of_layered_scene(
                &oversized,
                LayeredRenderOptions {
                    width: usize::from(u16::MAX),
                    ascii: true,
                    detail: LayeredDetail::Compact,
                },
            ),
            Err(LayeredRenderError::TooLarge {
                columns: usize::from(u16::MAX),
                rows: usize::from(u16::MAX),
            })
        );
    }

    #[test]
    /// Proof for `graph.layered_scene_is_complete_or_fallback`.
    fn static_layered_scene_refuses_an_edge_with_missing_geometry() {
        let mut incomplete = scene();
        incomplete.edges[0].points.clear();
        incomplete.edges[0].source_port = None;
        incomplete.edges[0].target_port = None;
        incomplete.edges[0].arrowhead = None;
        incomplete.edges[0].degradations.clear();
        let error = text_of_layered_scene(
            &incomplete,
            LayeredRenderOptions {
                width: 34,
                ascii: true,
                detail: LayeredDetail::Compact,
            },
        )
        .expect_err("an edge without validated geometry cannot look complete");
        assert_eq!(
            error,
            LayeredRenderError::IncompleteEdge {
                from: "A".to_string(),
                to: "B".to_string(),
                ordinal: 0,
                reasons: vec!["collapsed_route".to_string()],
            }
        );
        assert!(error.to_string().contains("`A` -> `B`"));
        assert!(error.to_string().contains("--layout lanes"));
    }

    #[test]
    fn static_layered_scene_names_every_invalid_edge_geometry_condition_once() {
        let mut incomplete = scene();
        incomplete.edges[0].points = vec![
            LayeredPoint { x: 21, y: 23 },
            LayeredPoint { x: 25, y: 24 },
            LayeredPoint { x: 30, y: 23 },
        ];
        incomplete.edges[0].source_port = Some(LayeredPoint { x: 20, y: 23 });
        incomplete.edges[0].target_port = Some(LayeredPoint { x: 29, y: 23 });
        incomplete.edges[0].arrowhead = Some(LayeredPoint { x: 29, y: 23 });
        incomplete.edges[0].degradations = vec!["invalid_source_port".to_string()];

        let error = text_of_layered_scene(
            &incomplete,
            LayeredRenderOptions {
                width: 34,
                ascii: true,
                detail: LayeredDetail::Compact,
            },
        )
        .expect_err("invalid edge geometry cannot look complete");
        assert_eq!(
            error,
            LayeredRenderError::IncompleteEdge {
                from: "A".to_string(),
                to: "B".to_string(),
                ordinal: 0,
                reasons: vec![
                    "invalid_source_port".to_string(),
                    "invalid_target_port".to_string(),
                    "invalid_arrowhead".to_string(),
                    "non_orthogonal_segment".to_string(),
                ],
            }
        );
    }

    #[test]
    fn ordinary_cards_keep_a_complete_top_border() {
        let mut ordinary = scene();
        ordinary.nodes[1].related = false;
        ordinary.nodes[1].critical = false;
        let rendered = text_of_layered_scene(
            &ordinary,
            LayeredRenderOptions {
                width: 34,
                ascii: true,
                detail: LayeredDetail::Compact,
            },
        )
        .expect("scene fits");
        assert!(rendered.contains("+---------+"), "{rendered}");
    }

    #[test]
    fn a_critical_only_card_has_one_non_colour_marker() {
        let mut critical = scene();
        critical.nodes[1].related = false;
        let rendered = text_of_layered_scene(
            &critical,
            LayeredRenderOptions {
                width: 34,
                ascii: true,
                detail: LayeredDetail::Compact,
            },
        )
        .expect("scene fits");
        let plain = anstream::adapter::strip_str(&rendered).to_string();
        assert!(plain.contains("+C--------+"), "{plain}");
        assert!(!plain.contains("+CC-------+"), "{plain}");
    }

    #[test]
    fn hostile_labels_are_safe_and_display_column_bounded() {
        let mut hostile = scene();
        hostile.nodes[0].id = "x\u{1b}[".to_string();
        hostile.nodes[0].title = "wide \u{754c}\u{301}".to_string();
        let rendered = text_of_layered_scene(
            &hostile,
            LayeredRenderOptions {
                width: 34,
                ascii: false,
                detail: LayeredDetail::Comfortable,
            },
        )
        .expect("scene fits");
        let plain = anstream::adapter::strip_str(&rendered).to_string();
        assert!(!plain.contains('\u{1b}'));
        assert!(plain.contains(crate::sanitize::MARKER));
        for line in plain.lines() {
            assert!(line.width() <= 34, "{line:?}");
        }
    }

    #[test]
    fn defensive_bounds_refusals_are_typed_and_actionable() {
        let mut invalid = scene();
        invalid.bounds.width = 0;
        let error = text_of_layered_scene(
            &invalid,
            LayeredRenderOptions {
                width: 34,
                ascii: true,
                detail: LayeredDetail::Compact,
            },
        )
        .expect_err("a zero-width canvas is invalid");
        assert_eq!(error, LayeredRenderError::InvalidBounds);
        assert!(error.to_string().contains("invalid terminal-cell bounds"));

        let too_large = LayeredRenderError::TooLarge {
            columns: 90_000,
            rows: 90_000,
        };
        assert!(too_large.to_string().contains("90000 by 90000"));
        assert!(
            LayeredRenderError::TooWide {
                required: 120,
                available: 80,
            }
            .to_string()
            .contains("--width 80")
        );
    }

    #[test]
    fn static_painter_marks_bands_crossings_junctions_feedback_and_vertical_edges() {
        let mut decorated = scene();
        decorated.bands.push(LayeredBand {
            phase: 7,
            nodes: vec!["A".to_string(), "B".to_string()],
            bounds: Some(LayeredRect {
                x: 10,
                y: 20,
                width: 34,
                height: 8,
            }),
            label_anchor: Some(LayeredPoint { x: 12, y: 20 }),
        });
        decorated.edges[0].feedback = true;
        decorated.edges[0].bundled = true;
        decorated.edges[0].points =
            vec![LayeredPoint { x: 25, y: 21 }, LayeredPoint { x: 25, y: 26 }];
        decorated.edges[0].source_port = Some(LayeredPoint { x: 25, y: 21 });
        decorated.edges[0].target_port = Some(LayeredPoint { x: 25, y: 26 });
        decorated.edges[0].arrowhead = Some(LayeredPoint { x: 25, y: 26 });
        decorated.crossings.push(LayeredPoint { x: 26, y: 24 });
        decorated.junctions.push(LayeredPoint { x: 27, y: 24 });
        decorated.nodes[1].bounds.height = 5;
        decorated.nodes[1].facts = LayeredFacts {
            tasks: 4,
            done: 2,
            ready: 1,
        };

        let rendered = text_of_layered_scene(
            &decorated,
            LayeredRenderOptions {
                width: 34,
                ascii: false,
                detail: LayeredDetail::Comfortable,
            },
        )
        .expect("decorated scene fits");
        let plain = anstream::adapter::strip_str(&rendered).to_string();
        assert!(plain.contains("phase 7"), "{plain}");
        assert!(plain.contains('\u{253c}'), "{plain}");
        assert!(plain.contains('\u{25c6}'), "{plain}");
        assert!(plain.contains("2/4 done"), "{plain}");
        assert!(
            plain.contains('\u{25b2}') || plain.contains('\u{25bc}'),
            "{plain}"
        );
    }

    #[test]
    fn canvas_clips_degenerate_and_outside_operations_without_corrupting_rows() {
        let base = scene();
        let mut canvas = Canvas::new(&base, true).expect("canvas");
        canvas.outline(
            LayeredRect {
                x: 10,
                y: 20,
                width: 0,
                height: 4,
            },
            PaintRole::Normal,
            false,
        );
        canvas.put(LayeredPoint { x: 0, y: 0 }, "x", PaintRole::Normal);
        canvas.edge(LayeredPoint { x: 0, y: 0 }, "-", PaintRole::Muted);
        canvas.text(
            LayeredPoint { x: 0, y: 20 },
            "0123456789visible",
            17,
            PaintRole::Id,
        );
        canvas.text(
            LayeredPoint { x: 10, y: 20 },
            "ignored",
            0,
            PaintRole::Normal,
        );
        let rendered = canvas.render();
        assert!(
            rendered
                .lines()
                .next()
                .is_some_and(|line| line.contains("visible"))
        );
        assert_eq!(rendered.lines().count(), 8);
    }
}
