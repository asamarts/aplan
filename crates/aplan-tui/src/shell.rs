use std::fmt::Write as _;
use std::time::Instant;

use ratatui::Frame;
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Constraint, Layout, Margin, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::discovery;
use crate::form::{Activation, FormAction, FormFocus, MutationForm};
use crate::graph_navigation::SpatialDirection;
use crate::graph_widget::{
    self, GraphHit, GraphStyles, GraphViewport as LayeredViewport, LayeredGraph, LayeredGraphState,
};
use crate::model::{
    Action, DiagnosticGroup, EffectKind, Focus, Glyphs, GraphJump, GraphRole, GraphViewportMode,
    InspectorPresentation, InspectorSection, InspectorTone, Modal, Model, MutationState,
    OverviewMetric, Route, SnapshotFreshness, Theme,
};
use crate::mutations::MutationAction;
use crate::text;

pub(crate) const MIN_COLUMNS: u16 = 48;
pub(crate) const MIN_ROWS: u16 = 14;

const ASCII_BORDER: border::Set<'static> = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayoutClass {
    Wide,
    Normal,
    Narrow,
    TooSmall,
}

/// Vertical shell density is independent of horizontal pane composition.
///
/// A short terminal can still be wide, but giving bordered header, tabs and
/// footer ten of its fourteen rows makes that width useless. Compact chrome
/// keeps the same semantic regions and pointer targets while yielding rows to
/// the active route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChromeDensity {
    Full,
    Compact,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Target<T> {
    area: Rect,
    value: T,
}

#[derive(Debug, Clone, PartialEq)]
struct OverlayControl {
    label: String,
    action: Option<Action>,
}

impl OverlayControl {
    fn new(label: impl Into<String>, action: Action) -> Self {
        Self {
            label: label.into(),
            action: Some(action),
        }
    }

    fn inert(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            action: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct OverlayRowTarget {
    first: usize,
    last: usize,
    action: Action,
}

impl OverlayRowTarget {
    const fn one(row: usize, action: Action) -> Self {
        Self {
            first: row,
            last: row + 1,
            action,
        }
    }
}

struct ModalContent {
    title: &'static str,
    lines: Vec<Line<'static>>,
    cursor_column: Option<usize>,
    targets: Vec<OverlayRowTarget>,
}

impl ModalContent {
    fn inert(title: &'static str, lines: Vec<Line<'static>>) -> Self {
        Self {
            title,
            lines,
            cursor_column: None,
            targets: Vec::new(),
        }
    }

    fn targeted(
        title: &'static str,
        lines: Vec<Line<'static>>,
        targets: Vec<OverlayRowTarget>,
    ) -> Self {
        Self {
            title,
            lines,
            cursor_column: None,
            targets,
        }
    }

    fn editor(title: &'static str, lines: Vec<Line<'static>>, cursor_column: usize) -> Self {
        Self {
            title,
            lines,
            cursor_column: Some(cursor_column),
            targets: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OverlayLayout {
    outer: Rect,
    content: Rect,
    controls: Rect,
}

#[derive(Debug, Clone, PartialEq)]
struct PackedOverlayControl {
    x: u16,
    label: String,
    action: Option<Action>,
}

fn label_target<T>(x: u16, y: u16, label: &str, value: T) -> Target<T> {
    let width = u16::try_from(text::cells(&text::safe(label))).unwrap_or(u16::MAX);
    Target {
        area: Rect::new(x, y, width, 1),
        value,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShellLayout {
    class: LayoutClass,
    density: ChromeDensity,
    area: Rect,
    header: Rect,
    navigation: Rect,
    primary: Rect,
    detail: Option<Rect>,
    footer: Rect,
    route_targets: Vec<Target<Route>>,
    footer_targets: Vec<Target<CommandId>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TaskViewport {
    content: Rect,
    intro_rows: u16,
    start: usize,
    end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GraphViewport {
    content: Rect,
    intro_rows: u16,
    row_start: usize,
    row_end: usize,
    column_start: usize,
    column_end: usize,
}

fn graph_viewport(area: Rect, _compact: bool, model: &Model) -> GraphViewport {
    let content = area.inner(Margin::new(1, 1));
    let intro_rows = if content.height <= 2 { 1 } else { 3 }.min(content.height);
    let height = usize::from(content.height.saturating_sub(intro_rows));
    let width = usize::from(content.width);
    let selected = model.graph_cell_scene().map_or_else(
        || {
            model
                .graph_explorer
                .selection
                .as_ref()
                .and_then(|selection| {
                    model
                        .graph_explorer
                        .graph
                        .nodes
                        .iter()
                        .position(|node| node.identity == selection.represented_by)
                        .map(|row| (row.saturating_add(1), 0))
                })
        },
        |scene| {
            scene.focus.as_ref().map(|focus| {
                let center = focus.bounds.center();
                (
                    usize::try_from(center.y.saturating_sub(scene.bounds.y)).unwrap_or_default(),
                    usize::try_from(center.x.saturating_sub(scene.bounds.x)).unwrap_or_default(),
                )
            })
        },
    );
    let (rows, columns) = model.graph_content_extent();
    let max_row = rows.saturating_sub(height);
    let max_column = columns.saturating_sub(width);
    let row_start = if model.graph_viewport_mode == GraphViewportMode::FollowSelection {
        selected.map_or(model.graph_scroll_row, |point| {
            point.0.saturating_sub(height / 2)
        })
    } else {
        model.graph_scroll_row
    }
    .min(max_row);
    let column_start = if model.graph_viewport_mode == GraphViewportMode::FollowSelection {
        selected.map_or(model.graph_scroll_column, |point| {
            point.1.saturating_sub(width / 2)
        })
    } else {
        model.graph_scroll_column
    }
    .min(max_column);
    GraphViewport {
        content,
        intro_rows,
        row_start,
        row_end: row_start.saturating_add(height).min(rows),
        column_start,
        column_end: column_start.saturating_add(width).min(columns),
    }
}

fn graph_control_specs(model: &Model) -> Vec<(String, Action)> {
    let mut controls = Vec::new();
    controls.push((
        format!("g {}", graph_scope_control_label(model)),
        Action::CycleGraphScope,
    ));
    if model.graph_continuation_count() > 0 {
        controls.push(("n open".to_string(), Action::OpenGraphContinuation));
    }
    controls.push((
        if model.graph_request.membership {
            "m dep+member".to_string()
        } else {
            "m dep".to_string()
        },
        Action::ToggleGraphMembership,
    ));
    controls.push((
        format!(
            "v {}",
            if matches!(model.graph_density, graph_widget::GraphDensity::Auto) {
                "auto".to_string()
            } else {
                model.graph_density.label().to_string()
            }
        ),
        Action::CycleGraphDensity,
    ));
    controls.push((
        if model.graph_edge_presentation.is_relations_only() {
            "o all edges".to_string()
        } else {
            "o relations".to_string()
        },
        Action::ToggleGraphRelationsOnly,
    ));
    if model.graph_relation_count(true) > 0 {
        controls.push(("[ blocker".to_string(), Action::FollowGraphBlocker));
    }
    if model.graph_relation_count(false) > 0 {
        controls.push(("] dependent".to_string(), Action::FollowGraphDependent));
    }
    if model.graph_continuation_count() > 1 {
        controls.push((", prev".to_string(), Action::PreviousGraphContinuation));
        controls.push((". next".to_string(), Action::NextGraphContinuation));
    }
    if matches!(
        model.graph_request.scope,
        aplan_app::GraphScope::Neighbourhood {
            depth: Some(value),
            ..
        } if value > 1
    ) {
        controls.push(("- depth".to_string(), Action::ContractGraphDepth));
    }
    if !matches!(
        model.graph_request.scope,
        aplan_app::GraphScope::Neighbourhood { depth: None, .. }
    ) {
        controls.push(("+ depth".to_string(), Action::ExpandGraphDepth));
    }
    controls.push(("u overview".to_string(), Action::GraphOverview));
    controls.push((
        format!("s {}", model.graph_ordering.label()),
        Action::CycleGraphOrdering,
    ));
    controls.push((
        format!("d {}", model.graph_orientation_label()),
        Action::CycleGraphOrientation,
    ));
    controls.push(("z fit".to_string(), Action::FitGraph));
    if model.has_graph_window_history() {
        controls.push(("Esc prior".to_string(), Action::PreviousGraphWindow));
    }
    if let Some(context) = &model.graph_explorer.phase_context {
        if let Some(phase) = context.previous {
            controls.push((
                format!("P phase {phase}"),
                Action::OpenGraphLocation(aplan_app::GraphExplorerLocation::Phase(phase)),
            ));
        }
        if let Some(phase) = context.next {
            controls.push((
                format!("N phase {phase}"),
                Action::OpenGraphLocation(aplan_app::GraphExplorerLocation::Phase(phase)),
            ));
        }
    }
    if model.graph_viewport_mode == GraphViewportMode::Manual {
        controls.push(("Home center".to_string(), Action::RecenterGraph));
    }
    controls
}

fn graph_scope_control_label(model: &Model) -> String {
    let scope = &model.graph_request.scope;
    let mut label = match scope {
        aplan_app::GraphScope::Whole => "whole".to_string(),
        aplan_app::GraphScope::Upstream(id) => format!("up:{id}"),
        aplan_app::GraphScope::Downstream(id) => format!("down:{id}"),
        aplan_app::GraphScope::Between(from, to) => format!("between:{from}>{to}"),
        aplan_app::GraphScope::Unreachable => "unreachable".to_string(),
        aplan_app::GraphScope::Neighbourhood { task, depth, live } => format!(
            "near:{task}:{}{}",
            depth.map_or_else(|| "all".to_string(), |depth| depth.to_string()),
            if *live { ":live" } else { "" }
        ),
        aplan_app::GraphScope::CriticalPath => "critical".to_string(),
    };
    if let Some(edges) = scope.walked_edges(model.graph_request.membership) {
        label.push('/');
        label.push_str(match edges {
            aplan_core::reach::Edges::Dependency => "dep",
            aplan_core::reach::Edges::Membership => "member",
            aplan_core::reach::Edges::Any => "all",
        });
    } else if matches!(scope, aplan_app::GraphScope::CriticalPath) {
        label.push_str("/dep+contain");
    }
    let structural = usize::from(!matches!(scope, aplan_app::GraphScope::Whole));
    let filters = model
        .graph_explorer
        .accounting
        .source_scope
        .len()
        .saturating_sub(structural);
    if filters > 0 {
        let _ = write!(label, "+{filters}F");
    }
    label
}

fn graph_control_targets(viewport: GraphViewport, model: &Model) -> Vec<Target<Action>> {
    if viewport.intro_rows < 2 {
        return Vec::new();
    }
    let mut x = viewport.content.x;
    let y = viewport
        .content
        .y
        .saturating_add(viewport.intro_rows.saturating_sub(1));
    let right = viewport.content.right();
    let mut targets = Vec::new();
    for (label, action) in graph_control_specs(model) {
        let width = u16::try_from(text::cells(&label)).unwrap_or(u16::MAX);
        if x.saturating_add(width) > right {
            continue;
        }
        targets.push(label_target(x, y, &label, action));
        x = x.saturating_add(width).saturating_add(3);
    }
    targets
}

fn graph_pan_action(model: &Model, rows: i16, columns: i16) -> Action {
    let area = Rect::new(0, 0, model.columns, model.rows);
    let layout = ShellLayout::new(area, model);
    let viewport = graph_viewport(layout.primary, layout.class == LayoutClass::Narrow, model);
    let adjust = |value: usize, delta: i16| {
        if delta.is_negative() {
            value.saturating_sub(usize::from(delta.unsigned_abs()))
        } else {
            value.saturating_add(usize::from(delta.unsigned_abs()))
        }
    };
    Action::SetGraphViewport {
        row: adjust(viewport.row_start, rows),
        column: adjust(viewport.column_start, columns),
    }
}

fn task_viewport(area: Rect, compact: bool, selected: usize, task_count: usize) -> TaskViewport {
    let content = area.inner(Margin::new(1, 1));
    let intro_rows = if compact { 1 } else { 2 }.min(content.height);
    let capacity = usize::from(content.height.saturating_sub(intro_rows));
    let end = task_count.min(selected.saturating_add(1).max(capacity).min(task_count));
    let start = end.saturating_sub(capacity);
    TaskViewport {
        content,
        intro_rows,
        start,
        end,
    }
}

fn item_viewport(area: Rect, selected: usize, count: usize) -> TaskViewport {
    let content = area.inner(Margin::new(1, 1));
    let capacity = usize::from(content.height);
    let end = count.min(selected.saturating_add(1).max(capacity).min(count));
    let start = end.saturating_sub(capacity);
    TaskViewport {
        content,
        intro_rows: 0,
        start,
        end,
    }
}

impl ShellLayout {
    fn new(area: Rect, model: &Model) -> Self {
        let class = classify(area);
        if class == LayoutClass::TooSmall {
            return Self {
                class,
                density: ChromeDensity::Compact,
                area,
                header: Rect::default(),
                navigation: Rect::default(),
                primary: area,
                detail: None,
                footer: Rect::default(),
                route_targets: Vec::new(),
                footer_targets: Vec::new(),
            };
        }

        let density = chrome_density(area);
        let (header_height, footer_height) = match density {
            ChromeDensity::Full => (3, 4),
            ChromeDensity::Compact => (1, 2),
        };
        let [header, middle, footer] = Layout::vertical([
            Constraint::Length(header_height),
            Constraint::Min(1),
            Constraint::Length(footer_height),
        ])
        .areas(area);

        let (navigation, primary, detail) = match class {
            LayoutClass::Wide => {
                let [navigation, primary, detail] = Layout::horizontal([
                    Constraint::Length(22),
                    Constraint::Percentage(46),
                    Constraint::Min(28),
                ])
                .areas(middle);
                (navigation, primary, Some(detail))
            }
            LayoutClass::Normal => {
                let navigation_height = match density {
                    ChromeDensity::Full => 3,
                    ChromeDensity::Compact => 1,
                };
                let [navigation, content] =
                    Layout::vertical([Constraint::Length(navigation_height), Constraint::Min(1)])
                        .areas(middle);
                let [primary, detail] =
                    Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
                        .areas(content);
                (navigation, primary, Some(detail))
            }
            LayoutClass::Narrow => {
                let navigation_height = match density {
                    ChromeDensity::Full => 3,
                    ChromeDensity::Compact => 1,
                };
                let [navigation, primary] =
                    Layout::vertical([Constraint::Length(navigation_height), Constraint::Min(1)])
                        .areas(middle);
                (navigation, primary, None)
            }
            LayoutClass::TooSmall => unreachable!("handled above"),
        };

        let route_targets = route_targets(navigation, class, density);
        let footer_targets = footer_targets(footer, model, density);
        Self {
            class,
            density,
            area,
            header,
            navigation,
            primary,
            detail,
            footer,
            route_targets,
            footer_targets,
        }
    }
}

const fn chrome_density(area: Rect) -> ChromeDensity {
    if area.height >= 22 {
        ChromeDensity::Full
    } else {
        ChromeDensity::Compact
    }
}

pub(crate) const fn classify(area: Rect) -> LayoutClass {
    if area.width < MIN_COLUMNS || area.height < MIN_ROWS {
        LayoutClass::TooSmall
    } else if area.width >= 120 && area.height >= 24 {
        LayoutClass::Wide
    } else if area.width >= 80 && area.height >= 18 {
        LayoutClass::Normal
    } else {
        LayoutClass::Narrow
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeyChord {
    code: KeyCode,
    modifiers: KeyModifiers,
}

impl KeyChord {
    const fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }
    }

    fn matches(self, key: KeyEvent) -> bool {
        self.code == key.code && self.modifiers == key.modifiers
    }
}

/// One declaration of both what help paints and what dispatch accepts.
///
/// Keeping the lifecycle policy here matters too: a binding advertised as a
/// navigation accelerator may accept terminal key repeat, while an action
/// that opens, cancels or mutates something accepts only a fresh press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeyBinding {
    label: &'static str,
    chords: BindingChords,
    repeatable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindingChords {
    None,
    One(KeyChord),
    Two(KeyChord, KeyChord),
    Three(KeyChord, KeyChord, KeyChord),
}

impl KeyBinding {
    const fn press_only(label: &'static str, chord: KeyChord) -> Self {
        Self {
            label,
            chords: BindingChords::One(chord),
            repeatable: false,
        }
    }

    const fn press_only2(label: &'static str, first: KeyChord, second: KeyChord) -> Self {
        Self {
            label,
            chords: BindingChords::Two(first, second),
            repeatable: false,
        }
    }

    const fn press_only3(
        label: &'static str,
        first: KeyChord,
        second: KeyChord,
        third: KeyChord,
    ) -> Self {
        Self {
            label,
            chords: BindingChords::Three(first, second, third),
            repeatable: false,
        }
    }

    const fn repeatable2(label: &'static str, first: KeyChord, second: KeyChord) -> Self {
        Self {
            label,
            chords: BindingChords::Two(first, second),
            repeatable: true,
        }
    }

    const fn repeatable(label: &'static str, chord: KeyChord) -> Self {
        Self {
            label,
            chords: BindingChords::One(chord),
            repeatable: true,
        }
    }

    const fn palette_only() -> Self {
        Self {
            label: "--",
            chords: BindingChords::None,
            repeatable: false,
        }
    }

    fn matches(self, key: KeyEvent) -> bool {
        match self.chords {
            BindingChords::None => false,
            BindingChords::One(first) => first.matches(key),
            BindingChords::Two(first, second) => first.matches(key) || second.matches(key),
            BindingChords::Three(first, second, third) => {
                first.matches(key) || second.matches(key) || third.matches(key)
            }
        }
    }

    fn accepts(self, key: KeyEvent) -> bool {
        self.matches(key)
            && (key.kind == KeyEventKind::Press
                || self.repeatable && key.kind == KeyEventKind::Repeat)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandId {
    Back,
    Quit,
    About,
    Overview,
    Tasks,
    Diagnostics,
    Graph,
    Reports,
    Inbox,
    QuickCapture,
    FocusNext,
    FocusPrevious,
    MoveNext,
    MovePrevious,
    Open,
    Search,
    AsOfDate,
    Filters,
    Sort,
    WhyTraversal,
    ClearNarrowing,
    PreviousSection,
    NextSection,
    ScrollUp,
    ScrollDown,
    Palette,
    Help,
    Refresh,
    Verify,
    CancelQueued,
    CycleDiagnosticScope,
    AdvancedCheck,
    ExplainRule,
    FindRule,
    OpenFindingTask,
    GraphScope,
    GraphBetween,
    GraphMembership,
    GraphDensity,
    GraphRelationsOnly,
    GraphPreviousContinuation,
    GraphNextContinuation,
    GraphOpenContinuation,
    GraphContractDepth,
    GraphExpandDepth,
    GraphOverview,
    GraphFit,
    GraphMoveLeft,
    GraphMoveRight,
    GraphPanUp,
    GraphPanDown,
    GraphPreviousLayer,
    GraphNextLayer,
    GraphSource,
    GraphSink,
    GraphCritical,
    GraphMilestone,
    GraphPreviousPhase,
    GraphNextPhase,
    GraphFinding,
    ShowInGraph,
    PanLeft,
    PanRight,
    RecenterGraph,
    CycleTheme,
    ToggleGlyphs,
    TestCatalogue,
}

impl CommandId {
    const ALL: [Self; 67] = [
        Self::Back,
        Self::Quit,
        Self::About,
        Self::Overview,
        Self::Tasks,
        Self::Diagnostics,
        Self::Graph,
        Self::Reports,
        Self::Inbox,
        Self::QuickCapture,
        Self::FocusNext,
        Self::FocusPrevious,
        Self::MoveNext,
        Self::MovePrevious,
        Self::Open,
        Self::Search,
        Self::AsOfDate,
        Self::Filters,
        Self::Sort,
        Self::WhyTraversal,
        Self::ClearNarrowing,
        Self::PreviousSection,
        Self::NextSection,
        Self::ScrollUp,
        Self::ScrollDown,
        Self::Palette,
        Self::Help,
        Self::Refresh,
        Self::Verify,
        Self::CancelQueued,
        Self::CycleDiagnosticScope,
        Self::AdvancedCheck,
        Self::ExplainRule,
        Self::FindRule,
        Self::OpenFindingTask,
        Self::GraphScope,
        Self::GraphBetween,
        Self::GraphMembership,
        Self::GraphDensity,
        Self::GraphRelationsOnly,
        Self::GraphPreviousContinuation,
        Self::GraphNextContinuation,
        Self::GraphOpenContinuation,
        Self::GraphContractDepth,
        Self::GraphExpandDepth,
        Self::GraphOverview,
        Self::GraphFit,
        Self::GraphMoveLeft,
        Self::GraphMoveRight,
        Self::GraphPanUp,
        Self::GraphPanDown,
        Self::GraphPreviousLayer,
        Self::GraphNextLayer,
        Self::GraphSource,
        Self::GraphSink,
        Self::GraphCritical,
        Self::GraphMilestone,
        Self::GraphPreviousPhase,
        Self::GraphNextPhase,
        Self::GraphFinding,
        Self::ShowInGraph,
        Self::PanLeft,
        Self::PanRight,
        Self::RecenterGraph,
        Self::CycleTheme,
        Self::ToggleGlyphs,
        Self::TestCatalogue,
    ];

    #[allow(clippy::too_many_lines)] // One exhaustive match keeps every command's label, chords and lifecycle inseparable.
    const fn binding(self) -> KeyBinding {
        match self {
            Self::Back => {
                KeyBinding::press_only("Esc", KeyChord::new(KeyCode::Esc, KeyModifiers::NONE))
            }
            Self::Quit => KeyBinding::press_only2(
                "q/C-c",
                KeyChord::new(KeyCode::Char('q'), KeyModifiers::NONE),
                KeyChord::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            ),
            Self::About => {
                KeyBinding::press_only("i", KeyChord::new(KeyCode::Char('i'), KeyModifiers::NONE))
            }
            Self::Overview => {
                KeyBinding::press_only("1", KeyChord::new(KeyCode::Char('1'), KeyModifiers::NONE))
            }
            Self::Tasks => {
                KeyBinding::press_only("2", KeyChord::new(KeyCode::Char('2'), KeyModifiers::NONE))
            }
            Self::Diagnostics => {
                KeyBinding::press_only("3", KeyChord::new(KeyCode::Char('3'), KeyModifiers::NONE))
            }
            Self::Graph => {
                KeyBinding::press_only("4", KeyChord::new(KeyCode::Char('4'), KeyModifiers::NONE))
            }
            Self::Reports => {
                KeyBinding::press_only("5", KeyChord::new(KeyCode::Char('5'), KeyModifiers::NONE))
            }
            Self::Inbox => {
                KeyBinding::press_only("6", KeyChord::new(KeyCode::Char('6'), KeyModifiers::NONE))
            }
            Self::QuickCapture => KeyBinding::press_only(
                "C-n",
                KeyChord::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
            ),
            Self::FocusNext => {
                KeyBinding::press_only("Tab", KeyChord::new(KeyCode::Tab, KeyModifiers::NONE))
            }
            Self::FocusPrevious => KeyBinding::press_only3(
                "S-Tab",
                KeyChord::new(KeyCode::BackTab, KeyModifiers::NONE),
                KeyChord::new(KeyCode::BackTab, KeyModifiers::SHIFT),
                KeyChord::new(KeyCode::Tab, KeyModifiers::SHIFT),
            ),
            Self::MoveNext => KeyBinding::repeatable2(
                "down/j",
                KeyChord::new(KeyCode::Down, KeyModifiers::NONE),
                KeyChord::new(KeyCode::Char('j'), KeyModifiers::NONE),
            ),
            Self::MovePrevious => KeyBinding::repeatable2(
                "up/k",
                KeyChord::new(KeyCode::Up, KeyModifiers::NONE),
                KeyChord::new(KeyCode::Char('k'), KeyModifiers::NONE),
            ),
            Self::Open => {
                KeyBinding::press_only("Enter", KeyChord::new(KeyCode::Enter, KeyModifiers::NONE))
            }
            Self::Search => {
                KeyBinding::press_only("/", KeyChord::new(KeyCode::Char('/'), KeyModifiers::NONE))
            }
            Self::AsOfDate => {
                KeyBinding::press_only("d", KeyChord::new(KeyCode::Char('d'), KeyModifiers::NONE))
            }
            Self::Filters => {
                KeyBinding::press_only("f", KeyChord::new(KeyCode::Char('f'), KeyModifiers::NONE))
            }
            Self::Sort => {
                KeyBinding::press_only("s", KeyChord::new(KeyCode::Char('s'), KeyModifiers::NONE))
            }
            Self::WhyTraversal => {
                KeyBinding::press_only("w", KeyChord::new(KeyCode::Char('w'), KeyModifiers::NONE))
            }
            Self::ClearNarrowing => {
                KeyBinding::press_only("x", KeyChord::new(KeyCode::Char('x'), KeyModifiers::NONE))
            }
            Self::PreviousSection => {
                KeyBinding::press_only("[", KeyChord::new(KeyCode::Char('['), KeyModifiers::NONE))
            }
            Self::NextSection => {
                KeyBinding::press_only("]", KeyChord::new(KeyCode::Char(']'), KeyModifiers::NONE))
            }
            Self::ScrollUp => {
                KeyBinding::press_only("PgUp", KeyChord::new(KeyCode::PageUp, KeyModifiers::NONE))
            }
            Self::ScrollDown => {
                KeyBinding::press_only("PgDn", KeyChord::new(KeyCode::PageDown, KeyModifiers::NONE))
            }
            Self::Palette => KeyBinding::press_only(
                "C-p",
                KeyChord::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
            ),
            Self::Help => KeyBinding::press_only2(
                "?",
                KeyChord::new(KeyCode::Char('?'), KeyModifiers::NONE),
                KeyChord::new(KeyCode::Char('?'), KeyModifiers::SHIFT),
            ),
            Self::Refresh => {
                KeyBinding::press_only("r", KeyChord::new(KeyCode::Char('r'), KeyModifiers::NONE))
            }
            Self::Verify => {
                KeyBinding::press_only("V", KeyChord::new(KeyCode::Char('V'), KeyModifiers::SHIFT))
            }
            Self::CancelQueued => {
                KeyBinding::press_only("c", KeyChord::new(KeyCode::Char('c'), KeyModifiers::NONE))
            }
            Self::CycleDiagnosticScope | Self::GraphDensity => {
                KeyBinding::press_only("v", KeyChord::new(KeyCode::Char('v'), KeyModifiers::NONE))
            }
            Self::AdvancedCheck => {
                KeyBinding::press_only("a", KeyChord::new(KeyCode::Char('a'), KeyModifiers::NONE))
            }
            Self::ExplainRule => {
                KeyBinding::press_only("e", KeyChord::new(KeyCode::Char('e'), KeyModifiers::NONE))
            }
            Self::FindRule => {
                KeyBinding::press_only("E", KeyChord::new(KeyCode::Char('E'), KeyModifiers::SHIFT))
            }
            Self::OpenFindingTask | Self::GraphRelationsOnly => {
                KeyBinding::press_only("o", KeyChord::new(KeyCode::Char('o'), KeyModifiers::NONE))
            }
            Self::GraphScope => {
                KeyBinding::press_only("g", KeyChord::new(KeyCode::Char('g'), KeyModifiers::NONE))
            }
            Self::GraphBetween => {
                KeyBinding::press_only("b", KeyChord::new(KeyCode::Char('b'), KeyModifiers::NONE))
            }
            Self::GraphMembership => {
                KeyBinding::press_only("m", KeyChord::new(KeyCode::Char('m'), KeyModifiers::NONE))
            }
            Self::GraphPreviousContinuation => {
                KeyBinding::press_only(",", KeyChord::new(KeyCode::Char(','), KeyModifiers::NONE))
            }
            Self::GraphNextContinuation => {
                KeyBinding::press_only(".", KeyChord::new(KeyCode::Char('.'), KeyModifiers::NONE))
            }
            Self::GraphOpenContinuation => {
                KeyBinding::press_only("n", KeyChord::new(KeyCode::Char('n'), KeyModifiers::NONE))
            }
            Self::GraphContractDepth => {
                KeyBinding::press_only("-", KeyChord::new(KeyCode::Char('-'), KeyModifiers::NONE))
            }
            Self::GraphExpandDepth => KeyBinding::press_only3(
                "+/=",
                KeyChord::new(KeyCode::Char('+'), KeyModifiers::SHIFT),
                KeyChord::new(KeyCode::Char('+'), KeyModifiers::NONE),
                KeyChord::new(KeyCode::Char('='), KeyModifiers::NONE),
            ),
            Self::GraphOverview | Self::ToggleGlyphs => {
                KeyBinding::press_only("u", KeyChord::new(KeyCode::Char('u'), KeyModifiers::NONE))
            }
            Self::GraphFit => {
                KeyBinding::press_only("z", KeyChord::new(KeyCode::Char('z'), KeyModifiers::NONE))
            }
            Self::GraphMoveLeft => KeyBinding::repeatable2(
                "left/h",
                KeyChord::new(KeyCode::Left, KeyModifiers::NONE),
                KeyChord::new(KeyCode::Char('h'), KeyModifiers::NONE),
            ),
            Self::GraphMoveRight => KeyBinding::repeatable2(
                "right/l",
                KeyChord::new(KeyCode::Right, KeyModifiers::NONE),
                KeyChord::new(KeyCode::Char('l'), KeyModifiers::NONE),
            ),
            Self::GraphPanUp => {
                KeyBinding::repeatable("K", KeyChord::new(KeyCode::Char('K'), KeyModifiers::SHIFT))
            }
            Self::GraphPanDown => {
                KeyBinding::repeatable("J", KeyChord::new(KeyCode::Char('J'), KeyModifiers::SHIFT))
            }
            Self::GraphPreviousLayer => {
                KeyBinding::press_only("{", KeyChord::new(KeyCode::Char('{'), KeyModifiers::SHIFT))
            }
            Self::GraphNextLayer => {
                KeyBinding::press_only("}", KeyChord::new(KeyCode::Char('}'), KeyModifiers::SHIFT))
            }
            Self::GraphSource => {
                KeyBinding::press_only("^", KeyChord::new(KeyCode::Char('^'), KeyModifiers::SHIFT))
            }
            Self::GraphSink => {
                KeyBinding::press_only("$", KeyChord::new(KeyCode::Char('$'), KeyModifiers::SHIFT))
            }
            Self::GraphCritical => {
                KeyBinding::press_only("C", KeyChord::new(KeyCode::Char('C'), KeyModifiers::SHIFT))
            }
            Self::GraphMilestone => {
                KeyBinding::press_only("M", KeyChord::new(KeyCode::Char('M'), KeyModifiers::SHIFT))
            }
            Self::GraphPreviousPhase => {
                KeyBinding::press_only("P", KeyChord::new(KeyCode::Char('P'), KeyModifiers::SHIFT))
            }
            Self::GraphNextPhase => {
                KeyBinding::press_only("N", KeyChord::new(KeyCode::Char('N'), KeyModifiers::SHIFT))
            }
            Self::GraphFinding => {
                KeyBinding::press_only("!", KeyChord::new(KeyCode::Char('!'), KeyModifiers::SHIFT))
            }
            Self::ShowInGraph => {
                KeyBinding::press_only("G", KeyChord::new(KeyCode::Char('G'), KeyModifiers::SHIFT))
            }
            Self::PanLeft => {
                KeyBinding::repeatable("H", KeyChord::new(KeyCode::Char('H'), KeyModifiers::SHIFT))
            }
            Self::PanRight => {
                KeyBinding::repeatable("L", KeyChord::new(KeyCode::Char('L'), KeyModifiers::SHIFT))
            }
            Self::RecenterGraph => KeyBinding::press_only2(
                "Home/0",
                KeyChord::new(KeyCode::Home, KeyModifiers::NONE),
                KeyChord::new(KeyCode::Char('0'), KeyModifiers::NONE),
            ),
            Self::CycleTheme => {
                KeyBinding::press_only("t", KeyChord::new(KeyCode::Char('t'), KeyModifiers::NONE))
            }
            Self::TestCatalogue => KeyBinding::palette_only(),
        }
    }

    /// Stable user-facing wording for the generated reference.
    ///
    /// Live help is deliberately more specific: it can say which scope,
    /// theme, ordering or continuation is selected. The generated document
    /// names the same action independent of one transient model state.
    #[allow(clippy::too_many_lines)] // Exhaustiveness is the drift guard for the reference.
    const fn reference(self) -> (&'static str, &'static str) {
        match self {
            Self::Back => (
                "Back or cancel",
                "global; reverses the current layer or history",
            ),
            Self::Quit => ("Quit safely", "global base shell"),
            Self::About => ("Build identity and capabilities", "global"),
            Self::Overview => ("Open Overview", "global destination"),
            Self::Tasks => ("Open Tasks", "global destination"),
            Self::Diagnostics => ("Open Diagnostics", "global destination"),
            Self::Graph => ("Open Graph", "global destination"),
            Self::Reports => ("Open Reports", "global destination"),
            Self::Inbox => ("Open Inbox", "global destination"),
            Self::QuickCapture => ("Quick capture", "global loaded shell"),
            Self::FocusNext => ("Focus next region", "loaded shell"),
            Self::FocusPrevious => ("Focus previous region", "loaded shell"),
            Self::MoveNext => ("Move to next item or down", "lists, overlays and Graph"),
            Self::MovePrevious => ("Move to previous item or up", "lists, overlays and Graph"),
            Self::Open => ("Open or drill into selection", "focused selectable content"),
            Self::Search => (
                "Search current destination",
                "Tasks, Inbox, Diagnostics, Graph or Reports",
            ),
            Self::AsOfDate => (
                "Edit clock or cycle graph direction",
                "Due/Stale contexts or Graph",
            ),
            Self::Filters => (
                "Edit structured filters",
                "Tasks, Inbox, Diagnostics or Graph",
            ),
            Self::Sort => ("Cycle ordering or grouping", "Tasks, Diagnostics or Graph"),
            Self::WhyTraversal => ("Toggle blocking explanation depth", "task Work inspector"),
            Self::ClearNarrowing => ("Clear active narrowing", "a narrowed destination"),
            Self::PreviousSection => (
                "Previous section or direct blocker",
                "task inspector or Graph",
            ),
            Self::NextSection => (
                "Next section or direct dependent",
                "task inspector or Graph",
            ),
            Self::ScrollUp => ("Scroll detail or pan up", "detail, onboarding or Graph"),
            Self::ScrollDown => ("Scroll detail or pan down", "detail, onboarding or Graph"),
            Self::Palette => ("Open command palette", "global"),
            Self::Help => ("Open contextual help", "global"),
            Self::Refresh => ("Refresh or retry loading", "global"),
            Self::Verify => ("Run evidence verification", "loaded plan"),
            Self::CancelQueued => ("Cancel queued work", "when an effect is queued"),
            Self::CycleDiagnosticScope => ("Cycle diagnostic scope", "Diagnostics"),
            Self::AdvancedCheck => (
                "Configure one structural check",
                "Diagnostics / structural scope",
            ),
            Self::ExplainRule => ("Explain selected finding rule", "Diagnostics finding"),
            Self::FindRule => ("Find and explain a rule code", "loaded plan"),
            Self::OpenFindingTask => ("Open finding task", "Diagnostics finding with a task"),
            Self::GraphScope => ("Cycle structural graph scope", "Graph"),
            Self::GraphBetween => ("Start or finish between scope", "Graph"),
            Self::GraphMembership => ("Toggle cut-membership edges", "Graph"),
            Self::GraphDensity => ("Cycle graph density", "Graph"),
            Self::GraphRelationsOnly => ("Toggle unrelated edge paint", "Graph"),
            Self::GraphPreviousContinuation => ("Select previous continuation", "windowed Graph"),
            Self::GraphNextContinuation => ("Select next continuation", "windowed Graph"),
            Self::GraphOpenContinuation => ("Open selected continuation", "windowed Graph"),
            Self::GraphContractDepth => ("Contract neighbourhood depth", "neighbourhood Graph"),
            Self::GraphExpandDepth => ("Expand neighbourhood depth", "neighbourhood Graph"),
            Self::GraphOverview => ("Open graph Overview level", "Graph"),
            Self::GraphFit => ("Fit graph origin", "Graph"),
            Self::GraphMoveLeft => ("Move left through geometry", "Graph"),
            Self::GraphMoveRight => ("Move right through geometry", "Graph"),
            Self::GraphPanUp => ("Pan viewport up", "Graph"),
            Self::GraphPanDown => ("Pan viewport down", "Graph"),
            Self::GraphPreviousLayer => ("Jump to previous dependency layer", "Graph"),
            Self::GraphNextLayer => ("Jump to next dependency layer", "Graph"),
            Self::GraphSource => ("Jump to next source node", "Graph"),
            Self::GraphSink => ("Jump to next sink node", "Graph"),
            Self::GraphCritical => ("Jump to next critical-route node", "Graph"),
            Self::GraphMilestone => ("Jump to next milestone", "Graph"),
            Self::GraphPreviousPhase => ("Open previous phase band", "Graph"),
            Self::GraphNextPhase => ("Open next phase band", "Graph"),
            Self::GraphFinding => ("Jump to next node with a finding", "Graph"),
            Self::ShowInGraph => ("Show selected task in Graph", "task context outside Graph"),
            Self::PanLeft => ("Pan viewport left", "Graph"),
            Self::PanRight => ("Pan viewport right", "Graph"),
            Self::RecenterGraph => ("Recenter on selection", "Graph"),
            Self::CycleTheme => ("Cycle theme", "global unless NO_COLOR fixes monochrome"),
            Self::ToggleGlyphs => ("Toggle Unicode/ASCII glyphs", "global outside Graph"),
            Self::TestCatalogue => ("Search configured test evidence", "command palette only"),
        }
    }

    /// CLI names whose interactive question is reached by this action.
    const fn discovery_aliases(self) -> &'static [&'static str] {
        match self {
            Self::About => &["facts", "capabilities", "identity"],
            Self::Tasks => &["tasks", "ready", "due"],
            Self::Diagnostics => &["check", "check-docs"],
            Self::Graph => &["graph", "tree", "critical-path"],
            Self::Reports => &[
                "version",
                "paths",
                "vacuity",
                "stale",
                "deprecations",
                "summary",
                "roadmap",
            ],
            Self::Inbox => &["inbox"],
            Self::QuickCapture => &["capture", "inbox", "intake"],
            Self::Open => &["show"],
            Self::WhyTraversal => &["why"],
            Self::Verify => &["verify"],
            Self::FindRule | Self::ExplainRule => &["explain"],
            Self::TestCatalogue => &["test", "test list", "evidence"],
            _ => &[],
        }
    }

    #[allow(clippy::too_many_lines)] // One exhaustive match keeps command identity and wording together.
    fn label(self, model: &Model) -> String {
        match self {
            Self::Back if model.modal.is_some() => "cancel modal".to_string(),
            Self::Back
                if model.route == Route::Graph
                    && model.focus == Focus::Primary
                    && model.has_graph_window_history() =>
            {
                "return to prior graph window".to_string()
            }
            Self::Back if model.focus == Focus::Detail && model.inspector_has_history() => {
                "back in inspector".to_string()
            }
            Self::Back if model.focus == Focus::Detail => "back to primary".to_string(),
            Self::Back if model.route != Route::Overview => "back to overview".to_string(),
            Self::Back | Self::Quit => "quit".to_string(),
            Self::About if model.onboarding.is_some() => "About".to_string(),
            Self::About => "build capabilities".to_string(),
            Self::Overview => "open Overview".to_string(),
            Self::Tasks => "open Tasks".to_string(),
            Self::Diagnostics => "open Diagnostics".to_string(),
            Self::Graph => "open Graph".to_string(),
            Self::Reports => "open Reports".to_string(),
            Self::Inbox => "open Inbox".to_string(),
            Self::QuickCapture => "quick capture".to_string(),
            Self::FocusNext => "next region".to_string(),
            Self::FocusPrevious => "previous region".to_string(),
            Self::MoveNext if model.route == Route::Graph => {
                "move down through graph geometry".to_string()
            }
            Self::MovePrevious if model.route == Route::Graph => {
                "move up through graph geometry".to_string()
            }
            Self::MoveNext => "next item".to_string(),
            Self::MovePrevious => "previous item".to_string(),
            Self::Open if model.route == Route::Graph => {
                "drill into selected graph level".to_string()
            }
            Self::Open => "open selected".to_string(),
            Self::Search if model.route == Route::Graph => {
                "find a node without changing graph scope".to_string()
            }
            Self::Search if model.route == Route::Reports && model.focus == Focus::Detail => {
                "search current report rows".to_string()
            }
            Self::Search if model.route == Route::Reports => "search report names".to_string(),
            Self::Search => format!("search {}", model.route.label()),
            Self::AsOfDate if model.route == Route::Graph => format!(
                "cycle graph direction (currently {})",
                model.graph_orientation_label()
            ),
            Self::AsOfDate if work_date_context(model) => work_date_label(model),
            Self::AsOfDate => "edit Stale work as-of date".to_string(),
            Self::Filters if model.route == Route::Diagnostics => "filter findings".to_string(),
            Self::Filters if model.route == Route::Inbox => "filter captures".to_string(),
            Self::Filters => "filter tasks".to_string(),
            Self::Sort if model.route == Route::Graph => format!(
                "cycle peer order ({}; {})",
                model.graph_ordering.label(),
                model.graph_ordering.layout_cost()
            ),
            Self::Sort if model.route == Route::Diagnostics => format!(
                "group findings (currently {})",
                model.diagnostic_group.label()
            ),
            Self::Sort => format!("sort tasks (currently {})", model.task_sort.label()),
            Self::WhyTraversal => why_traversal_label(model),
            Self::ClearNarrowing if model.route == Route::Diagnostics => {
                "clear diagnostic search and filters".to_string()
            }
            Self::ClearNarrowing if model.route == Route::Graph => {
                "clear graph search, filters, scope and membership".to_string()
            }
            Self::ClearNarrowing if model.route == Route::Reports => {
                "clear report searches, filters and as-of date".to_string()
            }
            Self::ClearNarrowing if model.route == Route::Inbox => {
                "clear capture search and filters".to_string()
            }
            Self::ClearNarrowing => "clear task search and filters".to_string(),
            Self::PreviousSection
                if model.route == Route::Graph && model.focus == Focus::Primary =>
            {
                "follow a direct blocker".to_string()
            }
            Self::PreviousSection => "previous inspector section".to_string(),
            Self::NextSection if model.route == Route::Graph && model.focus == Focus::Primary => {
                "follow a direct dependent".to_string()
            }
            Self::NextSection => "next inspector section".to_string(),
            Self::ScrollUp if model.route == Route::Graph && model.focus == Focus::Primary => {
                "pan graph up".to_string()
            }
            Self::ScrollUp if model.onboarding.is_some() => "earlier".to_string(),
            Self::ScrollUp => "scroll inspector up".to_string(),
            Self::ScrollDown if model.route == Route::Graph && model.focus == Focus::Primary => {
                "pan graph down".to_string()
            }
            Self::ScrollDown if model.onboarding.is_some() => "more".to_string(),
            Self::ScrollDown => "scroll inspector down".to_string(),
            Self::Palette => "command palette".to_string(),
            Self::Help => "contextual help".to_string(),
            Self::Refresh if model.onboarding.is_some() => "retry".to_string(),
            Self::Refresh => "refresh".to_string(),
            Self::Verify => "verify evidence".to_string(),
            Self::CancelQueued => "cancel queued work".to_string(),
            Self::CycleDiagnosticScope => format!(
                "switch diagnostic scope (currently {})",
                model.diagnostic_scope.label()
            ),
            Self::AdvancedCheck => "advanced one-run check".to_string(),
            Self::ExplainRule => "explain selected finding's rule".to_string(),
            Self::FindRule => "explain an exact rule code".to_string(),
            Self::OpenFindingTask => "open finding's task".to_string(),
            Self::GraphScope
            | Self::GraphBetween
            | Self::GraphMembership
            | Self::GraphDensity
            | Self::GraphRelationsOnly
            | Self::GraphPreviousContinuation
            | Self::GraphNextContinuation
            | Self::GraphOpenContinuation
            | Self::GraphContractDepth
            | Self::GraphExpandDepth => graph_command_label(self, model),
            Self::GraphOverview => "open explicit graph Overview".to_string(),
            Self::GraphFit => "fit graph origin in viewport".to_string(),
            Self::GraphMoveLeft => "move left through graph geometry".to_string(),
            Self::GraphMoveRight => "move right through graph geometry".to_string(),
            Self::GraphPanUp => "pan graph up".to_string(),
            Self::GraphPanDown => "pan graph down".to_string(),
            Self::GraphPreviousLayer => "jump to previous dependency layer".to_string(),
            Self::GraphNextLayer => "jump to next dependency layer".to_string(),
            Self::GraphSource => "jump to next source node".to_string(),
            Self::GraphSink => "jump to next sink node".to_string(),
            Self::GraphCritical => "jump to next critical-route node".to_string(),
            Self::GraphMilestone => "jump to next milestone".to_string(),
            Self::GraphPreviousPhase => "open previous phase band".to_string(),
            Self::GraphNextPhase => "open next phase band".to_string(),
            Self::GraphFinding => "jump to next diagnostic-bearing node".to_string(),
            Self::ShowInGraph => "show selected task in Graph with a return breadcrumb".to_string(),
            Self::PanLeft => "pan graph left".to_string(),
            Self::PanRight => "pan graph right".to_string(),
            Self::RecenterGraph => "recenter graph on selection".to_string(),
            Self::CycleTheme => format!("theme ({})", model.theme.label()),
            Self::ToggleGlyphs => format!("glyphs ({})", model.glyphs.label()),
            Self::TestCatalogue => "search configured test evidence".to_string(),
        }
    }

    const fn route(self) -> Option<Route> {
        match self {
            Self::Overview => Some(Route::Overview),
            Self::Tasks => Some(Route::Tasks),
            Self::Diagnostics => Some(Route::Diagnostics),
            Self::Graph => Some(Route::Graph),
            Self::Reports => Some(Route::Reports),
            Self::Inbox => Some(Route::Inbox),
            _ => None,
        }
    }

    const fn available_during_onboarding(self) -> bool {
        matches!(
            self,
            Self::Back
                | Self::Quit
                | Self::About
                | Self::MoveNext
                | Self::MovePrevious
                | Self::ScrollUp
                | Self::ScrollDown
                | Self::Palette
                | Self::Help
                | Self::Refresh
                | Self::CancelQueued
                | Self::CycleTheme
                | Self::ToggleGlyphs
        )
    }

    fn unavailable(self, model: &Model) -> Option<String> {
        if model.onboarding.is_some() && !self.available_during_onboarding() {
            return Some("unavailable until a plan loads; press r to retry".to_string());
        }
        if model.onboarding.is_some() && matches!(self, Self::ScrollUp | Self::ScrollDown) {
            return None;
        }
        if let Some(reason) = terminal_size_unavailable(self, model) {
            return Some(reason);
        }
        if let Some(reason) = self.diagnostic_unavailable(model) {
            return Some(reason);
        }
        if let Some(reason) = self.graph_unavailable(model) {
            return Some(reason);
        }
        if let Some(reason) = self.layer_unavailable(model) {
            return Some(reason);
        }
        if let Some(reason) = self.discovery_unavailable(model) {
            return Some(reason);
        }
        if let Some(reason) = inbox_command_unavailable(self, model) {
            return Some(reason);
        }
        if let Some(reason) = task_command_unavailable(self, model) {
            return Some(reason);
        }
        match self {
            Self::ClearNarrowing if no_narrowing(model) => {
                Some("no search or filters are active".to_string())
            }
            Self::PreviousSection | Self::NextSection
                if !(model.route == Route::Graph && model.focus == Focus::Primary)
                    && (model.focus != Focus::Detail
                        || matches!(
                            model.route,
                            Route::Overview | Route::Diagnostics | Route::Reports | Route::Inbox
                        )) =>
            {
                Some("focus the task inspector first".to_string())
            }
            Self::ScrollUp | Self::ScrollDown
                if model.route == Route::Reports
                    || model.focus != Focus::Detail
                        && !(model.route == Route::Graph && model.focus == Focus::Primary) =>
            {
                Some("focus the task inspector first".to_string())
            }
            _ => None,
        }
    }

    fn discovery_unavailable(self, model: &Model) -> Option<String> {
        match self {
            Self::CycleTheme if !model.can_cycle_theme() => {
                Some("NO_COLOR fixes the session to monochrome".to_string())
            }
            Self::TestCatalogue if model.pending_effects() >= crate::model::MAX_PENDING_EFFECTS => {
                Some(format!(
                    "effect queue is full ({} running or queued)",
                    crate::model::MAX_PENDING_EFFECTS
                ))
            }
            Self::TestCatalogue if model.has_pending_kind(EffectKind::TestCatalogue) => {
                Some("configured test discovery is already pending".to_string())
            }
            Self::AsOfDate if model.route == Route::Graph => None,
            Self::AsOfDate
                if !work_date_context(model)
                    && (model.route != Route::Reports
                        || model.report_kind != aplan_app::ReportKind::Stale) =>
            {
                Some(
                    "available on Overview's Due row, the Work inspector, or the Stale report"
                        .to_string(),
                )
            }
            Self::WhyTraversal if !work_inspector_context(model) => {
                Some("available in the task inspector's Work section".to_string())
            }
            Self::Search | Self::Filters
                if !matches!(
                    model.route,
                    Route::Tasks
                        | Route::Diagnostics
                        | Route::Graph
                        | Route::Reports
                        | Route::Inbox
                ) =>
            {
                Some("available in Tasks, Inbox, Diagnostics, Graph and Reports".to_string())
            }
            Self::Sort
                if !matches!(
                    model.route,
                    Route::Tasks | Route::Diagnostics | Route::Graph
                ) =>
            {
                Some("available in the Tasks, Diagnostics and Graph destinations".to_string())
            }
            _ => None,
        }
    }

    fn layer_unavailable(self, model: &Model) -> Option<String> {
        match self {
            Self::Palette if model.modal == Some(Modal::Palette) => {
                Some("the command palette is already open".to_string())
            }
            Self::Help if model.modal == Some(Modal::Help) => {
                Some("contextual help is already open".to_string())
            }
            Self::About if model.modal == Some(Modal::About) => {
                Some("build capabilities are already open".to_string())
            }
            Self::TestCatalogue if model.modal == Some(Modal::TestCatalogue) => {
                Some("the configured test selector is already open".to_string())
            }
            Self::CancelQueued if !model.has_queued_effect() => Some(
                "no queued effect can be cancelled; running verification is not interruptible and quit waits"
                    .to_string(),
            ),
            _ => None,
        }
    }

    fn graph_unavailable(self, model: &Model) -> Option<String> {
        match self {
            Self::Open
                if model.route == Route::Graph
                    && model.focus == Focus::Primary
                    && model
                        .selected_id
                        .as_deref()
                        .is_none_or(|id| !model.graph_projection.contains(id)) =>
            {
                Some(if model.graph_node_ids().is_empty() {
                    "no task is available to open".to_string()
                } else {
                    "the selected task is hidden or was removed; select a visible task first"
                        .to_string()
                })
            }
            Self::MoveNext | Self::MovePrevious
                if model.modal.is_none()
                    && model.route == Route::Graph
                    && model.focus == Focus::Primary
                    && model.graph_node_ids().is_empty() =>
            {
                Some("the current list is empty".to_string())
            }
            Self::GraphScope
            | Self::GraphBetween
            | Self::GraphMembership
            | Self::GraphDensity
            | Self::GraphRelationsOnly
            | Self::GraphPreviousContinuation
            | Self::GraphNextContinuation
            | Self::GraphOpenContinuation
            | Self::GraphContractDepth
            | Self::GraphExpandDepth
            | Self::GraphOverview
            | Self::GraphFit
            | Self::GraphMoveLeft
            | Self::GraphMoveRight
            | Self::GraphPanUp
            | Self::GraphPanDown
            | Self::GraphPreviousLayer
            | Self::GraphNextLayer
            | Self::GraphSource
            | Self::GraphSink
            | Self::GraphCritical
            | Self::GraphMilestone
            | Self::GraphPreviousPhase
            | Self::GraphNextPhase
            | Self::GraphFinding
            | Self::PanLeft
            | Self::PanRight
            | Self::RecenterGraph
                if model.route != Route::Graph || model.focus != Focus::Primary =>
            {
                Some("focus the Graph destination first".to_string())
            }
            Self::ShowInGraph if model.route == Route::Graph => {
                Some("the selected subject is already in Graph".to_string())
            }
            Self::ShowInGraph if model.selected_id.is_none() => {
                Some("select a task before opening it in Graph".to_string())
            }
            Self::GraphPreviousContinuation
            | Self::GraphNextContinuation
            | Self::GraphOpenContinuation
                if model.graph_continuation_count() == 0 =>
            {
                Some("the current graph window has no omitted continuation".to_string())
            }
            Self::GraphContractDepth
                if !matches!(
                    model.graph_request.scope,
                    aplan_app::GraphScope::Neighbourhood {
                        depth: Some(value),
                        ..
                    } if value > 1
                ) =>
            {
                Some("contract depth after selecting a neighbourhood deeper than 1".to_string())
            }
            Self::GraphExpandDepth
                if matches!(
                    model.graph_request.scope,
                    aplan_app::GraphScope::Neighbourhood { depth: None, .. }
                ) =>
            {
                Some("the neighbourhood already includes every depth".to_string())
            }
            _ => None,
        }
    }

    fn diagnostic_unavailable(self, model: &Model) -> Option<String> {
        match self {
            Self::Open
                if model.focus == Focus::Primary
                    && model.route == Route::Diagnostics
                    && model.diagnostic_findings().is_empty() =>
            {
                Some("no finding is available to open".to_string())
            }
            Self::CycleDiagnosticScope
            | Self::AdvancedCheck
            | Self::ExplainRule
            | Self::OpenFindingTask
                if model.route != Route::Diagnostics =>
            {
                Some("available in the Diagnostics destination".to_string())
            }
            Self::AdvancedCheck if model.diagnostic_scope != aplan_app::DiagnosticScope::Check => {
                Some("available for the structural check scope; press v to switch".to_string())
            }
            Self::ExplainRule
                if model
                    .selected_finding()
                    .is_none_or(|finding| aplan_app::explain_rule(&finding.code).is_err()) =>
            {
                Some("the selected finding does not name a catalogue rule".to_string())
            }
            Self::OpenFindingTask
                if model
                    .selected_finding()
                    .is_none_or(|finding| finding.task.is_none()) =>
            {
                Some("the selected finding has no task subject".to_string())
            }
            Self::MoveNext | Self::MovePrevious
                if model.modal.is_none()
                    && model.focus == Focus::Primary
                    && model.route == Route::Diagnostics
                    && model.diagnostic_findings().is_empty() =>
            {
                Some("the current list is empty".to_string())
            }
            _ => None,
        }
    }

    fn action(self, model: &Model) -> Option<Action> {
        if self.unavailable(model).is_some() {
            return None;
        }
        if let Some(route) = self.route() {
            return Some(Action::Route(route));
        }
        if let Some(action) = graph_command_action(self, model) {
            return Some(action);
        }
        self.non_graph_action(model)
    }

    fn non_graph_action(self, model: &Model) -> Option<Action> {
        match self {
            Self::Back
                if model.route == Route::Graph
                    && model.focus == Focus::Primary
                    && model.has_graph_window_history() =>
            {
                Some(Action::PreviousGraphWindow)
            }
            Self::Back => Some(Action::CloseLayer),
            Self::Quit => Some(Action::Quit),
            Self::About => Some(Action::OpenAbout),
            Self::QuickCapture => Some(Action::OpenMutation(MutationAction::CaptureCreate)),
            Self::FocusNext => Some(Action::Focus(next_focus(model.focus, true))),
            Self::FocusPrevious => Some(Action::Focus(next_focus(model.focus, false))),
            Self::MoveNext => move_action(model, true),
            Self::MovePrevious => move_action(model, false),
            Self::Open => Some(Action::OpenDetail),
            Self::Search => Some(Action::OpenSearch),
            Self::AsOfDate if model.route == Route::Graph => Some(Action::CycleGraphOrientation),
            Self::AsOfDate if work_date_context(model) => Some(Action::OpenWorkDate),
            Self::AsOfDate => Some(Action::OpenReportDate),
            Self::Filters => Some(Action::OpenFilters),
            Self::Sort if model.route == Route::Graph => Some(Action::CycleGraphOrdering),
            Self::Sort => Some(Action::CycleTaskSort),
            Self::WhyTraversal => Some(Action::ToggleWhyTraversal),
            Self::ClearNarrowing => Some(Action::ClearNarrowing),
            Self::PreviousSection
                if model.route == Route::Graph && model.focus == Focus::Primary =>
            {
                Some(Action::FollowGraphBlocker)
            }
            Self::PreviousSection => Some(Action::PreviousInspectorSection),
            Self::NextSection if model.route == Route::Graph && model.focus == Focus::Primary => {
                Some(Action::FollowGraphDependent)
            }
            Self::NextSection => Some(Action::NextInspectorSection),
            Self::ScrollUp if model.route == Route::Graph && model.focus == Focus::Primary => {
                Some(graph_pan_action(model, -5, 0))
            }
            Self::ScrollUp if model.onboarding.is_some() => Some(Action::AdjustModalScroll(-5)),
            Self::ScrollUp => inspector_scroll_target(model, false).map(Action::SetInspectorScroll),
            Self::ScrollDown if model.route == Route::Graph && model.focus == Focus::Primary => {
                Some(graph_pan_action(model, 5, 0))
            }
            Self::ScrollDown if model.onboarding.is_some() => Some(Action::AdjustModalScroll(5)),
            Self::ScrollDown => {
                inspector_scroll_target(model, true).map(Action::SetInspectorScroll)
            }
            Self::Palette => Some(Action::OpenPalette),
            Self::Help => Some(Action::OpenHelp),
            Self::Refresh if model.onboarding.is_some() => Some(Action::Retry),
            Self::Refresh => Some(Action::Refresh),
            Self::Verify => Some(Action::OpenVerifyRunSheet),
            Self::CancelQueued => Some(Action::CancelQueued),
            Self::CycleDiagnosticScope => Some(Action::CycleDiagnosticScope),
            Self::AdvancedCheck => Some(Action::OpenRunSheet),
            Self::ExplainRule => Some(Action::OpenContextualRule),
            Self::FindRule => Some(Action::OpenRuleSearch),
            Self::OpenFindingTask => Some(Action::OpenFindingTask),
            Self::ShowInGraph => model.selected_id.clone().map(Action::OpenGraphAt),
            Self::CycleTheme => Some(Action::CycleTheme),
            Self::ToggleGlyphs => Some(Action::ToggleGlyphs),
            Self::TestCatalogue => Some(Action::OpenTestCatalogue),
            Self::Overview
            | Self::Tasks
            | Self::Diagnostics
            | Self::Graph
            | Self::Reports
            | Self::Inbox
            | Self::GraphScope
            | Self::GraphBetween
            | Self::GraphMembership
            | Self::GraphDensity
            | Self::GraphRelationsOnly
            | Self::GraphPreviousContinuation
            | Self::GraphNextContinuation
            | Self::GraphOpenContinuation
            | Self::GraphContractDepth
            | Self::GraphExpandDepth
            | Self::GraphOverview
            | Self::GraphFit
            | Self::GraphMoveLeft
            | Self::GraphMoveRight
            | Self::GraphPanUp
            | Self::GraphPanDown
            | Self::GraphPreviousLayer
            | Self::GraphNextLayer
            | Self::GraphSource
            | Self::GraphSink
            | Self::GraphCritical
            | Self::GraphMilestone
            | Self::GraphPreviousPhase
            | Self::GraphNextPhase
            | Self::GraphFinding
            | Self::PanLeft
            | Self::PanRight
            | Self::RecenterGraph => {
                unreachable!("routes and graph commands returned above")
            }
        }
    }

    const fn footer(self) -> bool {
        matches!(
            self,
            Self::Back
                | Self::Quit
                | Self::About
                | Self::FocusNext
                | Self::Open
                | Self::Search
                | Self::AsOfDate
                | Self::Filters
                | Self::Sort
                | Self::WhyTraversal
                | Self::PreviousSection
                | Self::NextSection
                | Self::ScrollUp
                | Self::ScrollDown
                | Self::Palette
                | Self::Help
                | Self::Refresh
                | Self::Verify
                | Self::CancelQueued
                | Self::CycleDiagnosticScope
                | Self::AdvancedCheck
                | Self::ExplainRule
                | Self::OpenFindingTask
                | Self::GraphScope
                | Self::GraphBetween
                | Self::GraphMembership
                | Self::GraphDensity
                | Self::GraphRelationsOnly
                | Self::GraphPreviousContinuation
                | Self::GraphNextContinuation
                | Self::GraphOpenContinuation
                | Self::GraphContractDepth
                | Self::GraphExpandDepth
                | Self::PanLeft
                | Self::PanRight
                | Self::RecenterGraph
        )
    }

    fn matches(self, key: KeyEvent) -> bool {
        self.binding().matches(key)
    }
}

pub(crate) fn generated_action_reference_markdown() -> String {
    use std::fmt::Write as _;

    let mut output = String::from(
        "<!-- @generated by `cargo xtask tui-reference`; edit the executable catalogues, not this file. -->\n\
         # TUI action reference\n\n\
         This file is generated from the action, mutation and command-disposition catalogues that\n\
         drive the live TUI. Run `cargo xtask tui-reference` after changing one of those catalogues.\n\n\
         Live `?` help uses the same action membership and key bindings, then adds the current\n\
         context, availability and exact reason for a disabled action. `Ctrl-P` searches these\n\
         actions, every mutation below and the explicit CLI boundaries. Key repeats are accepted\n\
         only for navigation actions; confirmation and mutation actions require a fresh press.\n\n\
         ## Shell actions\n\n\
         | Keys | Action | Context |\n\
         |---|---|---|\n",
    );
    for command in CommandId::ALL {
        let (action, context) = command.reference();
        writeln!(
            output,
            "| `{}` | {} | {} |",
            command.binding().label,
            action,
            context
        )
        .expect("writing to String cannot fail");
    }

    output.push_str(
        "\n## Mutation actions\n\n\
         Mutations are found by title or CLI name in `Ctrl-P`. Each opens a typed form or preview;\n\
         nothing is published until the guarded confirmation step succeeds.\n\n\
         | Palette action | CLI semantics |\n\
         |---|---|\n",
    );
    for action in MutationAction::ALL {
        output.push_str("| ");
        output.push_str(action.title());
        output.push_str(" | `aplan");
        output.push(' ');
        output.push_str(action.cli());
        output.push_str("` |\n");
    }

    output.push_str(
        "\n## Guided and CLI-only boundaries\n\n\
         These entries are deliberately inert in the TUI. Searching `Ctrl-P` or `?` shows the\n\
         same reason and exact copyable command rather than implying that the full-screen surface\n\
         performed a batch, transport, alternate-root or automation operation.\n\n",
    );
    for command in discovery::COMMANDS {
        if let Some((reason, example)) = command.disposition.boundary() {
            writeln!(
                output,
                "### `{}` ({})\n",
                command.name,
                command.disposition.label()
            )
            .expect("writing to String cannot fail");
            writeln!(output, "{reason}\n\n```sh\n{example}\n```\n")
                .expect("writing to String cannot fail");
        }
        for boundary in command.boundaries {
            writeln!(output, "### `{}` (CLI-only)\n", boundary.name)
                .expect("writing to String cannot fail");
            writeln!(
                output,
                "{}\n\n```sh\n{}\n```\n",
                boundary.reason, boundary.example
            )
            .expect("writing to String cannot fail");
        }
    }
    output.truncate(output.trim_end_matches('\n').len());
    output.push('\n');
    output
}

fn graph_command_action(command: CommandId, model: &Model) -> Option<Action> {
    Some(match command {
        CommandId::GraphScope => Action::CycleGraphScope,
        CommandId::GraphBetween => Action::SetGraphBetween,
        CommandId::GraphMembership => Action::ToggleGraphMembership,
        CommandId::GraphDensity => Action::CycleGraphDensity,
        CommandId::GraphRelationsOnly => Action::ToggleGraphRelationsOnly,
        CommandId::GraphPreviousContinuation => Action::PreviousGraphContinuation,
        CommandId::GraphNextContinuation => Action::NextGraphContinuation,
        CommandId::GraphOpenContinuation => Action::OpenGraphContinuation,
        CommandId::GraphContractDepth => Action::ContractGraphDepth,
        CommandId::GraphExpandDepth => Action::ExpandGraphDepth,
        CommandId::GraphOverview => Action::GraphOverview,
        CommandId::GraphFit => Action::FitGraph,
        CommandId::GraphMoveLeft => Action::MoveGraph(SpatialDirection::Left),
        CommandId::GraphMoveRight => Action::MoveGraph(SpatialDirection::Right),
        CommandId::GraphPanUp => graph_pan_action(model, -3, 0),
        CommandId::GraphPanDown => graph_pan_action(model, 3, 0),
        CommandId::GraphPreviousLayer => Action::JumpGraph(GraphJump::PreviousLayer),
        CommandId::GraphNextLayer => Action::JumpGraph(GraphJump::NextLayer),
        CommandId::GraphSource => Action::JumpGraph(GraphJump::Source),
        CommandId::GraphSink => Action::JumpGraph(GraphJump::Sink),
        CommandId::GraphCritical => Action::JumpGraph(GraphJump::Critical),
        CommandId::GraphMilestone => Action::JumpGraph(GraphJump::Milestone),
        CommandId::GraphPreviousPhase => Action::JumpGraph(GraphJump::PreviousPhase),
        CommandId::GraphNextPhase => Action::JumpGraph(GraphJump::NextPhase),
        CommandId::GraphFinding => Action::JumpGraph(GraphJump::Finding),
        CommandId::PanLeft => graph_pan_action(model, 0, -4),
        CommandId::PanRight => graph_pan_action(model, 0, 4),
        CommandId::RecenterGraph => Action::RecenterGraph,
        _ => return None,
    })
}

fn inbox_command_unavailable(command: CommandId, model: &Model) -> Option<String> {
    if model.route != Route::Inbox {
        return None;
    }
    match command {
        CommandId::Open if model.focus == Focus::Primary && model.selected_capture().is_none() => {
            Some("no capture is available to inspect".to_string())
        }
        CommandId::Open
            if model.focus == Focus::Detail
                && model
                    .selected_capture()
                    .is_none_or(|capture| capture.promoted_to.is_none()) =>
        {
            Some("the selected capture has no promoted task link".to_string())
        }
        CommandId::MoveNext | CommandId::MovePrevious
            if model.modal.is_none()
                && model.focus == Focus::Primary
                && model.inbox_rows().is_empty() =>
        {
            Some("the current inbox list is empty".to_string())
        }
        _ => None,
    }
}

fn task_command_unavailable(command: CommandId, model: &Model) -> Option<String> {
    match command {
        CommandId::Open
            if model.focus == Focus::Primary
                && !matches!(
                    model.route,
                    Route::Overview
                        | Route::Diagnostics
                        | Route::Graph
                        | Route::Reports
                        | Route::Inbox
                )
                && model.selected_task().is_none() =>
        {
            Some(if model.task_browser.rows.is_empty() {
                "no task is available to open".to_string()
            } else {
                "the selected task is hidden or was removed; select a visible task first"
                    .to_string()
            })
        }
        CommandId::Open if model.focus == Focus::Navigation => {
            Some("focus the primary list or inspector first".to_string())
        }
        CommandId::Open
            if model.focus == Focus::Detail
                && matches!(model.route, Route::Overview | Route::Diagnostics) =>
        {
            Some("the current detail panel has no linked destination".to_string())
        }
        CommandId::Open
            if model.focus == Focus::Detail
                && !matches!(model.route, Route::Reports | Route::Inbox)
                && !model.inspector_item_has_link() =>
        {
            Some("the selected inspector item has no linked destination".to_string())
        }
        CommandId::MoveNext | CommandId::MovePrevious
            if model.modal.is_none()
                && model.focus == Focus::Detail
                && matches!(model.route, Route::Overview | Route::Diagnostics) =>
        {
            Some("the current detail panel has no selectable items".to_string())
        }
        CommandId::MoveNext | CommandId::MovePrevious
            if model.modal.is_none()
                && model.focus == Focus::Detail
                && model.route != Route::Reports
                && model.inspector_items().is_empty() =>
        {
            Some("the current inspector section is empty".to_string())
        }
        CommandId::MoveNext | CommandId::MovePrevious
            if model.modal.is_none()
                && model.focus == Focus::Primary
                && !matches!(
                    model.route,
                    Route::Overview
                        | Route::Diagnostics
                        | Route::Graph
                        | Route::Reports
                        | Route::Inbox
                )
                && model.task_browser.rows.is_empty() =>
        {
            Some("the current list is empty".to_string())
        }
        _ => None,
    }
}

fn terminal_size_unavailable(command: CommandId, model: &Model) -> Option<String> {
    (classify(Rect::new(0, 0, model.columns, model.rows)) == LayoutClass::TooSmall
        && !matches!(
            command,
            CommandId::Back | CommandId::Quit | CommandId::Refresh | CommandId::CancelQueued
        ))
    .then(|| format!("terminal must be at least {MIN_COLUMNS}x{MIN_ROWS}"))
}

fn graph_command_label(command: CommandId, model: &Model) -> String {
    match command {
        CommandId::GraphScope => {
            format!("cycle graph scope ({})", model.graph_request.scope.label())
        }
        CommandId::GraphBetween => model.graph_between_anchor.as_ref().map_or_else(
            || "start a between scope".to_string(),
            |id| format!("finish between scope from {id}"),
        ),
        CommandId::GraphMembership => format!(
            "{} cut-membership edges",
            if model.graph_request.membership {
                "hide"
            } else {
                "show"
            }
        ),
        CommandId::GraphDensity => format!(
            "cycle graph density ({} -> {})",
            model.graph_density.label(),
            model.graph_resolved_density.label()
        ),
        CommandId::GraphRelationsOnly => format!(
            "{} unrelated edge paint",
            if model.graph_edge_presentation.is_relations_only() {
                "restore"
            } else {
                "suppress"
            }
        ),
        CommandId::GraphPreviousContinuation => "select previous continuation".to_string(),
        CommandId::GraphNextContinuation => "select next continuation".to_string(),
        CommandId::GraphOpenContinuation => model.selected_graph_continuation().map_or_else(
            || "open graph continuation".to_string(),
            |continuation| {
                format!(
                    "open {} continuation at {}",
                    Model::graph_continuation_direction_label(continuation.cursor.key.direction),
                    continuation.cursor.key.boundary
                )
            },
        ),
        CommandId::GraphContractDepth => "contract neighbourhood depth".to_string(),
        CommandId::GraphExpandDepth => "expand neighbourhood depth".to_string(),
        _ => unreachable!("only graph-labelled commands are routed here"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandState {
    id: CommandId,
    keys: &'static str,
    label: String,
    unavailable: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaletteTarget {
    Command(CommandId),
    Mutation(MutationAction),
    Boundary(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PaletteState {
    target: PaletteTarget,
    keys: &'static str,
    label: String,
    unavailable: Option<String>,
    aliases: &'static [&'static str],
    search_hint: &'static str,
}

fn command_states(model: &Model) -> Vec<CommandState> {
    CommandId::ALL
        .into_iter()
        .map(|id| CommandState {
            id,
            keys: id.binding().label,
            label: id.label(model),
            unavailable: id.unavailable(model),
        })
        .collect()
}

fn mutation_unavailable(model: &Model, action: MutationAction) -> Option<String> {
    if model.onboarding.is_some() {
        return Some("unavailable until a plan loads; press r to retry".to_string());
    }
    if terminal_size_unavailable(CommandId::Palette, model).is_some() {
        return Some(format!(
            "terminal must be at least {MIN_COLUMNS}x{MIN_ROWS}"
        ));
    }
    if model.pending_effects() >= crate::model::MAX_PENDING_EFFECTS {
        return Some(format!(
            "effect queue is full ({} running or queued)",
            crate::model::MAX_PENDING_EFFECTS
        ));
    }
    if action.needs_selected_task()
        && model.selected_id.as_deref().is_none_or(|id| {
            !model
                .snapshot
                .inspectors
                .iter()
                .any(|inspector| inspector.id == id)
        })
    {
        return Some("select a loaded task in Tasks or Graph first".to_string());
    }
    if action.needs_selected_capture() {
        if model.route != Route::Inbox {
            return Some("select the capture in the Inbox destination first".to_string());
        }
        let Some(capture) = model.selected_capture() else {
            return Some("select a loaded capture in Inbox first".to_string());
        };
        let admitted = match action {
            MutationAction::CaptureRestore => {
                capture.state == aplan_core::capture::CaptureState::Discarded
            }
            MutationAction::CaptureAmend
            | MutationAction::CaptureDiscard
            | MutationAction::CapturePromote => {
                capture.state == aplan_core::capture::CaptureState::Open
            }
            _ => true,
        };
        if !admitted {
            return Some(format!(
                "{} is unavailable for a {} capture",
                action.title(),
                crate::model::capture_state_name(capture.state)
            ));
        }
    }
    None
}

fn action_states(model: &Model) -> Vec<PaletteState> {
    command_states(model)
        .into_iter()
        .map(|state| PaletteState {
            target: PaletteTarget::Command(state.id),
            keys: state.keys,
            label: state.label,
            unavailable: state.unavailable,
            aliases: state.id.discovery_aliases(),
            search_hint: "",
        })
        .chain(MutationAction::ALL.into_iter().map(|action| PaletteState {
            target: PaletteTarget::Mutation(action),
            keys: "--",
            label: format!("{} (CLI: {} {})", action.title(), "aplan", action.cli()),
            unavailable: mutation_unavailable(model, action),
            aliases: action.discovery_aliases(),
            search_hint: "",
        }))
        .chain(discovery::COMMANDS.iter().filter_map(|command| {
            let (reason, example) = command.disposition.boundary()?;
            // About is the invokable guided `facts` action. Keeping a second
            // disabled row would make one command appear to have two answers.
            if command.name == "facts" {
                return None;
            }
            let unavailable = format!(
                "{}: {reason}; exact command (shown only): `{example}`",
                command.disposition.label()
            );
            Some(PaletteState {
                target: PaletteTarget::Boundary(command.name),
                keys: "--",
                label: format!("{}: {example}", command.disposition.label()),
                unavailable: Some(unavailable),
                aliases: std::slice::from_ref(&command.name),
                search_hint: command.title,
            })
        }))
        .chain(discovery::COMMANDS.iter().flat_map(|command| {
            command.boundaries.iter().map(|boundary| PaletteState {
                target: PaletteTarget::Boundary(boundary.name),
                keys: "--",
                label: format!("CLI-only: {}", boundary.example),
                unavailable: Some(format!(
                    "CLI-only: {}; exact command (shown only): `{}`",
                    boundary.reason, boundary.example
                )),
                aliases: boundary.aliases,
                search_hint: boundary.reason,
            })
        }))
        .collect()
}

fn palette_states(model: &Model) -> Vec<PaletteState> {
    let query = model.palette_query.to_lowercase();
    action_states(model)
        .into_iter()
        .filter(|state| {
            query.is_empty()
                || state.label.to_lowercase().contains(&query)
                || state.keys.to_lowercase().contains(&query)
                || state.search_hint.to_lowercase().contains(&query)
                || state
                    .aliases
                    .iter()
                    .any(|alias| alias.to_lowercase().contains(&query))
        })
        .collect()
}

fn no_narrowing(model: &Model) -> bool {
    match model.route {
        Route::Diagnostics => !model.has_diagnostic_narrowing(),
        Route::Graph => !model.has_graph_narrowing(),
        Route::Reports => !model.has_report_narrowing(),
        Route::Inbox => !model.has_inbox_narrowing(),
        Route::Overview | Route::Tasks => !model.has_task_narrowing(),
    }
}

fn work_inspector_context(model: &Model) -> bool {
    matches!(model.route, Route::Tasks | Route::Graph)
        && model.focus == Focus::Detail
        && model.inspector_section == InspectorSection::Work
        && model.work_projection().is_some()
}

fn work_date_context(model: &Model) -> bool {
    work_inspector_context(model)
        || (model.route == Route::Overview
            && model.focus == Focus::Primary
            && OverviewMetric::ALL.get(model.overview_selected) == Some(&OverviewMetric::Due))
}

fn work_date_label(model: &Model) -> String {
    format!(
        "edit due-work as-of instant (currently {})",
        model.effective_work_as_of_epoch_seconds()
    )
}

fn why_traversal_label(model: &Model) -> String {
    format!(
        "show {} why traversal",
        model.why_traversal.toggled().label()
    )
}

fn next_focus(focus: Focus, forward: bool) -> Focus {
    let index = Focus::ALL
        .iter()
        .position(|candidate| *candidate == focus)
        .unwrap_or(0);
    let next = if forward {
        (index + 1) % Focus::ALL.len()
    } else {
        index.checked_sub(1).unwrap_or(Focus::ALL.len() - 1)
    };
    Focus::ALL[next]
}

fn next_route(route: Route, forward: bool) -> Route {
    let index = Route::ALL
        .iter()
        .position(|candidate| *candidate == route)
        .unwrap_or(0);
    let next = if forward {
        (index + 1) % Route::ALL.len()
    } else {
        index.checked_sub(1).unwrap_or(Route::ALL.len() - 1)
    };
    Route::ALL[next]
}

fn move_action(model: &Model, forward: bool) -> Option<Action> {
    if let Some(modal) = model.modal {
        let count = modal_item_count(model, modal);
        if count == 0 {
            return None;
        }
        let index = if forward {
            (model.modal_selected + 1) % count
        } else {
            model.modal_selected.checked_sub(1).unwrap_or(count - 1)
        };
        return Some(Action::SelectModal(index));
    }
    if model.focus == Focus::Navigation {
        return Some(Action::Route(next_route(model.route, forward)));
    }
    if forward {
        Some(Action::SelectNext)
    } else {
        Some(Action::SelectPrevious)
    }
}

fn decode_search_key(key: KeyEvent) -> Option<Action> {
    match (key.kind, key.code, key.modifiers) {
        (KeyEventKind::Press, KeyCode::Esc | KeyCode::Enter, KeyModifiers::NONE) => {
            Some(Action::CloseLayer)
        }
        (KeyEventKind::Press | KeyEventKind::Repeat, KeyCode::Backspace, KeyModifiers::NONE) => {
            Some(Action::SearchBackspace)
        }
        (KeyEventKind::Press | KeyEventKind::Repeat, KeyCode::Delete, KeyModifiers::NONE) => {
            Some(Action::SearchDelete)
        }
        (KeyEventKind::Press | KeyEventKind::Repeat, KeyCode::Left, KeyModifiers::NONE) => {
            Some(Action::SearchLeft)
        }
        (KeyEventKind::Press | KeyEventKind::Repeat, KeyCode::Right, KeyModifiers::NONE) => {
            Some(Action::SearchRight)
        }
        (KeyEventKind::Press | KeyEventKind::Repeat, KeyCode::Home, KeyModifiers::NONE) => {
            Some(Action::SearchHome)
        }
        (KeyEventKind::Press | KeyEventKind::Repeat, KeyCode::End, KeyModifiers::NONE) => {
            Some(Action::SearchEnd)
        }
        (KeyEventKind::Press | KeyEventKind::Repeat, KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            Some(Action::SearchClear)
        }
        (
            KeyEventKind::Press | KeyEventKind::Repeat,
            KeyCode::Char(value),
            KeyModifiers::NONE | KeyModifiers::SHIFT,
        ) => Some(Action::SearchInput(value.to_string())),
        _ => None,
    }
}

pub(crate) fn decode_event(event: Event, model: &Model) -> Option<Action> {
    match event {
        Event::Key(key) => decode_key(key, model),
        Event::Resize(columns, rows) => Some(Action::Resize { columns, rows }),
        Event::Paste(value) if model.startup.is_none() => Some(Action::Paste(value)),
        Event::FocusGained => Some(Action::FocusGained),
        Event::FocusLost => Some(Action::FocusLost),
        Event::Mouse(mouse) if model.startup.is_none() => decode_mouse(mouse, model),
        Event::Paste(_) | Event::Mouse(_) => None,
    }
}

fn decode_key(key: KeyEvent, model: &Model) -> Option<Action> {
    let navigation = matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat);
    if !navigation {
        return None;
    }
    if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('c') {
        return Some(Action::Quit);
    }
    if model.startup.is_some() {
        return (key.kind == KeyEventKind::Press
            && key.modifiers == KeyModifiers::NONE
            && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc))
        .then_some(Action::Quit);
    }
    if classify(Rect::new(0, 0, model.columns, model.rows)) == LayoutClass::TooSmall
        && key.kind == KeyEventKind::Press
        && CommandId::Quit.matches(key)
    {
        return Some(Action::Quit);
    }

    if model.mutation.is_some() {
        return decode_mutation_key(key, model);
    }

    if let Some(modal) = model.modal {
        return decode_modal_key(key, model, modal);
    }

    for id in CommandId::ALL {
        if id.binding().accepts(key) {
            return id
                .action(model)
                .or_else(|| id.unavailable(model).map(Action::Notify));
        }
    }
    None
}

fn decode_mutation_key(key: KeyEvent, model: &Model) -> Option<Action> {
    let layer = model.mutation.as_ref()?;
    if layer.committing {
        return None;
    }
    if key.kind == KeyEventKind::Press
        && key.code == KeyCode::Esc
        && key.modifiers == KeyModifiers::NONE
    {
        return Some(Action::CloseLayer);
    }
    match &layer.state {
        MutationState::Loading => None,
        MutationState::QuickReview { .. } | MutationState::Result { .. } => {
            if key.kind != KeyEventKind::Press || key.modifiers != KeyModifiers::NONE {
                return None;
            }
            match key.code {
                KeyCode::PageUp => Some(Action::MutationScroll(-5)),
                KeyCode::PageDown => Some(Action::MutationScroll(5)),
                KeyCode::Enter if matches!(layer.state, MutationState::QuickReview { .. }) => {
                    Some(Action::ConfirmMutation)
                }
                _ => None,
            }
        }
        MutationState::Form(form) => {
            let action = match (key.code, key.modifiers) {
                (KeyCode::Char('e'), KeyModifiers::CONTROL)
                    if key.kind == KeyEventKind::Press
                        && layer.action == crate::mutations::MutationAction::Add =>
                {
                    return Some(Action::OpenExternalEditor);
                }
                (KeyCode::Tab, KeyModifiers::NONE) => FormAction::NextFocus,
                (KeyCode::BackTab, _) | (KeyCode::Tab, KeyModifiers::SHIFT) => {
                    FormAction::PreviousFocus
                }
                (KeyCode::Up, KeyModifiers::NONE) => FormAction::PreviousItem,
                (KeyCode::Down, KeyModifiers::NONE) => FormAction::NextItem,
                (KeyCode::PageUp, KeyModifiers::NONE) => {
                    mutation_form_scroll_action(model, form, -5)
                }
                (KeyCode::PageDown, KeyModifiers::NONE) => {
                    mutation_form_scroll_action(model, form, 5)
                }
                (KeyCode::Left, KeyModifiers::NONE) => FormAction::CursorLeft,
                (KeyCode::Right, KeyModifiers::NONE) => FormAction::CursorRight,
                (KeyCode::Home, KeyModifiers::NONE) => FormAction::CursorHome,
                (KeyCode::End, KeyModifiers::NONE) => FormAction::CursorEnd,
                (KeyCode::Backspace, KeyModifiers::NONE) => FormAction::Backspace,
                (KeyCode::Delete, KeyModifiers::NONE) => FormAction::Delete,
                (KeyCode::Char('n'), KeyModifiers::CONTROL) if key.kind == KeyEventKind::Press => {
                    FormAction::AddItem
                }
                (KeyCode::Char('d'), KeyModifiers::CONTROL) if key.kind == KeyEventKind::Press => {
                    FormAction::RemoveItem
                }
                (KeyCode::Up, KeyModifiers::ALT) if key.kind == KeyEventKind::Press => {
                    FormAction::MoveItemUp
                }
                (KeyCode::Down, KeyModifiers::ALT) if key.kind == KeyEventKind::Press => {
                    FormAction::MoveItemDown
                }
                (KeyCode::Enter, KeyModifiers::NONE) if key.kind == KeyEventKind::Press => {
                    match form.focus() {
                        FormFocus::Field(_) => FormAction::NextFocus,
                        FormFocus::Cancel => FormAction::Cancel,
                        FormFocus::Review => FormAction::Review,
                        FormFocus::Recompute => FormAction::Recompute,
                        FormFocus::Confirm => {
                            FormAction::Confirm(Activation::from_key_kind(key.kind))
                        }
                    }
                }
                (KeyCode::Char(value), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                    FormAction::Insert(value.to_string())
                }
                _ => return None,
            };
            Some(Action::MutationForm(action))
        }
    }
}

fn mutation_form_scroll_action(model: &Model, form: &MutationForm, delta: i16) -> FormAction {
    let shell = Rect::new(0, 0, model.columns, model.rows);
    let controls = mutation_controls(model);
    let width = usize::from(modal_area(shell).width.saturating_sub(2));
    let packed = pack_overlay_controls(&controls, width);
    let layout = overlay_layout(shell, packed.len());
    FormAction::Scroll {
        delta,
        from: form.resolved_scroll(
            usize::from(layout.content.width),
            usize::from(layout.content.height),
        ),
    }
}

fn decode_modal_key(key: KeyEvent, model: &Model, overlay: Modal) -> Option<Action> {
    match overlay {
        Modal::Palette => decode_palette_key(key, model),
        Modal::Search | Modal::ReportDate => decode_search_key(key),
        Modal::WorkDate => {
            if key.kind == KeyEventKind::Press
                && key.code == KeyCode::Enter
                && key.modifiers == KeyModifiers::NONE
            {
                Some(Action::SubmitWorkDate)
            } else {
                decode_search_key(key)
            }
        }
        Modal::RuleSearch => {
            if key.kind == KeyEventKind::Press
                && key.code == KeyCode::Enter
                && key.modifiers == KeyModifiers::NONE
            {
                Some(Action::SubmitRuleSearch)
            } else {
                decode_search_key(key)
            }
        }
        Modal::TestCatalogue => decode_test_catalogue_key(key, model),
        Modal::RuleExplanation => {
            if key.kind == KeyEventKind::Press && key.modifiers == KeyModifiers::NONE {
                match key.code {
                    KeyCode::Esc => return Some(Action::CloseLayer),
                    KeyCode::PageUp => return Some(Action::AdjustModalScroll(-5)),
                    KeyCode::PageDown => return Some(Action::AdjustModalScroll(5)),
                    _ => {}
                }
            }
            None
        }
        Modal::RunSheet => {
            if key.kind == KeyEventKind::Press {
                match (key.code, key.modifiers) {
                    (KeyCode::Esc, KeyModifiers::NONE) => return Some(Action::CloseLayer),
                    (KeyCode::Enter | KeyCode::Char(' '), KeyModifiers::NONE) => {
                        return Some(Action::ToggleRunSheet(model.modal_selected));
                    }
                    _ => {}
                }
            }
            decode_modal_navigation(key, model)
        }
        Modal::VerifyRunSheet => {
            if key.kind == KeyEventKind::Press {
                match (key.code, key.modifiers) {
                    (KeyCode::Esc, KeyModifiers::NONE) => return Some(Action::CloseLayer),
                    (KeyCode::PageUp, KeyModifiers::NONE) => {
                        return Some(Action::AdjustModalScroll(-5));
                    }
                    (KeyCode::PageDown, KeyModifiers::NONE) => {
                        return Some(Action::AdjustModalScroll(5));
                    }
                    (KeyCode::Enter | KeyCode::Char(' '), KeyModifiers::NONE) => {
                        return Some(Action::ToggleVerifyRunSheet(model.modal_selected));
                    }
                    _ => {}
                }
            }
            decode_modal_navigation(key, model)
        }
        Modal::About | Modal::Verification => {
            if key.kind == KeyEventKind::Press && key.modifiers == KeyModifiers::NONE {
                match key.code {
                    KeyCode::Esc => return Some(Action::CloseLayer),
                    KeyCode::PageUp => return Some(Action::AdjustModalScroll(-5)),
                    KeyCode::PageDown => return Some(Action::AdjustModalScroll(5)),
                    _ => {}
                }
            }
            decode_modal_navigation(key, model)
        }
        Modal::Filters => {
            if key.kind == KeyEventKind::Press {
                match (key.code, key.modifiers) {
                    (KeyCode::Esc, KeyModifiers::NONE) => return Some(Action::CloseLayer),
                    (KeyCode::Enter | KeyCode::Char(' '), KeyModifiers::NONE) => {
                        return Some(Action::ToggleFilter(model.modal_selected));
                    }
                    (KeyCode::Char('c'), KeyModifiers::NONE) => return Some(Action::ClearFilters),
                    _ => {}
                }
            }
            decode_modal_navigation(key, model)
        }
        Modal::Help => {
            if key.kind == KeyEventKind::Press
                && key.code == KeyCode::Esc
                && key.modifiers == KeyModifiers::NONE
            {
                Some(Action::CloseLayer)
            } else {
                decode_modal_navigation(key, model)
            }
        }
    }
}

fn decode_test_catalogue_key(key: KeyEvent, model: &Model) -> Option<Action> {
    if key.kind == KeyEventKind::Press {
        match (key.code, key.modifiers) {
            (KeyCode::Esc, KeyModifiers::NONE) => return Some(Action::CloseLayer),
            (KeyCode::Enter, KeyModifiers::NONE) => return Some(Action::UseTestEvidence),
            _ => {}
        }
    }
    decode_modal_navigation(key, model).or_else(|| decode_search_key(key))
}

fn decode_palette_key(key: KeyEvent, model: &Model) -> Option<Action> {
    if key.kind == KeyEventKind::Press {
        match (key.code, key.modifiers) {
            (KeyCode::Esc, KeyModifiers::NONE) => return Some(Action::CloseLayer),
            (KeyCode::Enter, KeyModifiers::NONE) => {
                return palette_selected_action(model);
            }
            (KeyCode::Backspace, KeyModifiers::NONE) => {
                return Some(Action::PaletteBackspace);
            }
            (KeyCode::Char(value), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                return Some(Action::PaletteInput(value.to_string()));
            }
            _ => {}
        }
    }
    decode_modal_navigation(key, model)
}

fn palette_selected_action(model: &Model) -> Option<Action> {
    let states = palette_states(model);
    let state = states.get(model.modal_selected)?;
    if let Some(reason) = &state.unavailable {
        return Some(Action::Notify(reason.clone()));
    }
    match state.target {
        PaletteTarget::Mutation(action) => Some(Action::OpenMutation(action)),
        PaletteTarget::Command(id) => {
            let action = id.action(model)?;
            Some(if id == CommandId::Back {
                action
            } else {
                Action::Invoke(Box::new(action))
            })
        }
        PaletteTarget::Boundary(_) => {
            unreachable!("boundary rows always carry a disabled reason")
        }
    }
}

fn decode_modal_navigation(key: KeyEvent, model: &Model) -> Option<Action> {
    for id in [
        CommandId::MoveNext,
        CommandId::MovePrevious,
        CommandId::FocusNext,
        CommandId::FocusPrevious,
    ] {
        if id.binding().accepts(key) {
            return match id {
                CommandId::MoveNext | CommandId::FocusNext => move_action(model, true),
                CommandId::MovePrevious | CommandId::FocusPrevious => move_action(model, false),
                _ => unreachable!("modal navigation enumerates only movement commands"),
            };
        }
    }
    None
}

fn decode_mouse(mouse: MouseEvent, model: &Model) -> Option<Action> {
    if mouse.modifiers != KeyModifiers::NONE {
        return None;
    }
    let area = Rect::new(0, 0, model.columns, model.rows);
    let layout = ShellLayout::new(area, model);
    let position = Position::new(mouse.column, mouse.row);
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => hit_test(&layout, model, position),
        MouseEventKind::Drag(MouseButton::Left)
            if model.modal.is_none() && model.mutation.is_none() && model.onboarding.is_none() =>
        {
            drag_hit_test(&layout, model, position)
        }
        MouseEventKind::ScrollDown => wheel_event_action(&layout, model, position, true),
        MouseEventKind::ScrollUp => wheel_event_action(&layout, model, position, false),
        _ => None,
    }
}

fn wheel_event_action(
    layout: &ShellLayout,
    model: &Model,
    position: Position,
    forward: bool,
) -> Option<Action> {
    let delta: i8 = if forward { 3 } else { -3 };
    if let Some(layer) = model.mutation.as_ref() {
        let controls = mutation_controls(model);
        let width = usize::from(modal_area(layout.area).width.saturating_sub(2));
        let packed = pack_overlay_controls(&controls, width);
        let overlay = overlay_layout(layout.area, packed.len());
        if layer.committing || !overlay.content.contains(position) {
            return None;
        }
        return match &layer.state {
            MutationState::Form(form) => Some(Action::MutationForm(FormAction::Scroll {
                delta: i16::from(delta),
                from: form.resolved_scroll(
                    usize::from(overlay.content.width),
                    usize::from(overlay.content.height),
                ),
            })),
            MutationState::QuickReview { .. } | MutationState::Result { .. } => {
                Some(Action::MutationScroll(delta))
            }
            MutationState::Loading => None,
        };
    }
    if let Some(modal) = model.modal {
        let controls = modal_controls(model, modal);
        let width = usize::from(modal_area(layout.area).width.saturating_sub(2));
        let packed = pack_overlay_controls(&controls, width);
        let overlay = overlay_layout(layout.area, packed.len());
        return overlay
            .content
            .contains(position)
            .then(|| modal_wheel_action(model, modal, forward))
            .flatten();
    }
    if model.onboarding.is_some() {
        return (position.y >= layout.header.bottom() && position.y < layout.footer.y)
            .then_some(Action::AdjustModalScroll(delta));
    }

    let primary_content = layout.primary.inner(Margin::new(1, 1));
    let detail = if layout.class == LayoutClass::Narrow && model.focus == Focus::Detail {
        Some(primary_content)
    } else {
        layout.detail.map(|area| area.inner(Margin::new(1, 1)))
    };
    if detail.is_some_and(|content| content.contains(position)) {
        return detail_wheel_action(model, forward);
    }
    if primary_content.contains(position) {
        return wheel_action(model, forward);
    }
    None
}

fn modal_item_count(model: &Model, layer: Modal) -> usize {
    match layer {
        Modal::Palette => palette_states(model).len(),
        Modal::Filters if model.route == Route::Diagnostics => {
            model.diagnostic_filter_choices().len()
        }
        Modal::Filters if model.route == Route::Inbox => model.inbox_filter_choices().len(),
        Modal::Filters => model.filter_choices().len(),
        Modal::Help => action_states(model).len(),
        Modal::RunSheet => 4,
        Modal::TestCatalogue => model.filtered_test_choices().len(),
        Modal::VerifyRunSheet => 2,
        Modal::Verification => model.verification.as_deref().map_or(0, |verification| {
            verification.run.findings.len()
                + verification.citations.len()
                + verification.costs.len()
        }),
        Modal::About
        | Modal::Search
        | Modal::ReportDate
        | Modal::WorkDate
        | Modal::RuleSearch
        | Modal::RuleExplanation => 0,
    }
}

fn modal_wheel_action(model: &Model, layer: Modal, forward: bool) -> Option<Action> {
    if matches!(
        layer,
        Modal::About | Modal::VerifyRunSheet | Modal::Verification | Modal::RuleExplanation
    ) {
        return Some(Action::AdjustModalScroll(if forward { 3 } else { -3 }));
    }
    let count = modal_item_count(model, layer);
    let last = count.checked_sub(1)?;
    let selected = model.modal_selected.min(last);
    let next = if forward {
        selected.saturating_add(1).min(last)
    } else {
        selected.saturating_sub(1)
    };
    (next != selected).then_some(Action::SelectModal(next))
}

fn detail_wheel_action(model: &Model, forward: bool) -> Option<Action> {
    if model.route == Route::Reports {
        let count = model.report_rows().len();
        let last = count.checked_sub(1)?;
        let selected = model.report_selected_row.min(last);
        let next = if forward {
            selected.saturating_add(1).min(last)
        } else {
            selected.saturating_sub(1)
        };
        return (next != selected).then_some(Action::SelectReportRow(next));
    }
    let (current, maximum) = inspector_scroll_bounds(model)?;
    let next = if forward {
        current.saturating_add(3).min(maximum)
    } else {
        current.saturating_sub(3)
    };
    (next != current).then_some(Action::SetInspectorScroll(next))
}

fn wheel_action(model: &Model, forward: bool) -> Option<Action> {
    if model.route == Route::Graph {
        return Some(graph_pan_action(model, if forward { 3 } else { -3 }, 0));
    }
    if model.route == Route::Reports {
        let kinds = model.report_kinds();
        let selected = kinds
            .iter()
            .position(|kind| *kind == model.report_kind)
            .unwrap_or_default();
        let last = kinds.len().checked_sub(1)?;
        let next = if forward {
            selected.saturating_add(1).min(last)
        } else {
            selected.saturating_sub(1)
        };
        return (next != selected).then_some(Action::SelectIndex(next));
    }
    let count = match model.route {
        Route::Overview => OverviewMetric::ALL.len(),
        Route::Diagnostics => model.diagnostic_findings().len(),
        Route::Tasks => model.task_browser.rows.len(),
        Route::Inbox => model.inbox_rows().len(),
        Route::Graph | Route::Reports => unreachable!("handled above"),
    };
    let last = count.checked_sub(1)?;
    let selected = match model.route {
        Route::Overview => model.overview_selected,
        Route::Diagnostics => model.selected,
        Route::Tasks => model.selected_visible_index().unwrap_or(0),
        Route::Inbox => model.selected_capture_visible_index().unwrap_or(0),
        Route::Graph | Route::Reports => unreachable!("handled above"),
    }
    .min(last);
    let next = if forward {
        selected.saturating_add(1).min(last)
    } else {
        selected.saturating_sub(1)
    };
    (next != selected).then_some(Action::SelectIndex(next))
}

fn drag_hit_test(layout: &ShellLayout, model: &Model, position: Position) -> Option<Action> {
    hit_test(layout, model, position).filter(|action| {
        matches!(
            action,
            Action::SelectIndex(_) | Action::SelectTask(_) | Action::Focus(_)
        )
    })
}

fn hit_test(layout: &ShellLayout, model: &Model, position: Position) -> Option<Action> {
    if layout.class == LayoutClass::TooSmall {
        return None;
    }
    if let Some(overlay) = model.modal {
        return modal_hit_test(layout.area, model, overlay, position);
    }
    if model.mutation.is_some() {
        return mutation_hit_test(layout.area, model, position);
    }
    if model.onboarding.is_some() {
        return layout
            .footer_targets
            .iter()
            .find(|target| target.area.contains(position))
            .and_then(|target| target.value.action(model));
    }
    if let Some(target) = layout
        .route_targets
        .iter()
        .find(|target| target.area.contains(position))
    {
        return Some(Action::Route(target.value));
    }
    if let Some(target) = layout
        .footer_targets
        .iter()
        .find(|target| target.area.contains(position))
    {
        return target.value.action(model);
    }
    let compact = layout.class == LayoutClass::Narrow;
    if model.route == Route::Graph {
        return graph_hit_test(layout, model, position, compact);
    }
    let viewport = match model.route {
        Route::Overview => item_viewport(
            layout.primary,
            model.overview_selected,
            OverviewMetric::ALL.len(),
        ),
        Route::Diagnostics => item_viewport(
            layout.primary,
            model.selected,
            model.diagnostic_findings().len(),
        ),
        Route::Reports => item_viewport(
            layout.primary,
            model
                .report_kinds()
                .iter()
                .position(|kind| *kind == model.report_kind)
                .unwrap_or_default(),
            model.report_kinds().len(),
        ),
        Route::Inbox => item_viewport(
            layout.primary,
            model.selected_capture_visible_index().unwrap_or_default(),
            model.inbox_rows().len(),
        ),
        Route::Tasks => task_viewport(
            layout.primary,
            compact,
            model.selected,
            model.task_browser.rows.len(),
        ),
        Route::Graph => unreachable!("graph hit testing returned above"),
    };
    if viewport.content.contains(position) {
        if layout.class == LayoutClass::Narrow && model.focus == Focus::Detail {
            return Some(Action::Focus(Focus::Detail));
        }
        let first_row = viewport.content.y.saturating_add(viewport.intro_rows);
        let offset = usize::from(position.y.saturating_sub(first_row));
        let index = viewport.start.saturating_add(offset);
        return if position.y >= first_row && index < viewport.end {
            Some(Action::SelectIndex(index))
        } else {
            Some(Action::Focus(Focus::Primary))
        };
    }
    if layout
        .detail
        .is_some_and(|area| area.inner(Margin::new(1, 1)).contains(position))
    {
        return Some(Action::Focus(Focus::Detail));
    }
    None
}

fn graph_hit_test(
    layout: &ShellLayout,
    model: &Model,
    position: Position,
    compact: bool,
) -> Option<Action> {
    let viewport = graph_viewport(layout.primary, compact, model);
    if viewport.content.contains(position) {
        if layout.class == LayoutClass::Narrow && model.focus == Focus::Detail {
            return Some(Action::Focus(Focus::Detail));
        }
        if let Some(target) = graph_control_targets(viewport, model)
            .into_iter()
            .find(|target| target.area.contains(position))
        {
            return Some(target.value);
        }
        let first_row = viewport.content.y.saturating_add(viewport.intro_rows);
        if position.y < first_row {
            return Some(Action::Focus(Focus::Primary));
        }
        let graph_area = Rect::new(
            viewport.content.x,
            first_row,
            viewport.content.width,
            viewport.content.bottom().saturating_sub(first_row),
        );
        let hit = model.graph_cell_scene().and_then(|scene| {
            graph_widget::hit_test(
                scene,
                graph_area,
                LayeredViewport {
                    row: viewport.row_start,
                    column: viewport.column_start,
                },
                position,
                model.graph_edge_presentation,
            )
        });
        if hit.is_none() && model.graph_scene_fallback().is_some() {
            let row = viewport
                .row_start
                .saturating_add(usize::from(position.y.saturating_sub(first_row)));
            if row > 0
                && let Some(node) = model.graph_explorer.graph.nodes.get(row - 1)
            {
                return Some(Action::SelectGraphIdentity(node.identity.clone()));
            }
        }
        return match hit {
            Some(GraphHit::Node(identity)) => Some(Action::SelectGraphIdentity(identity)),
            Some(GraphHit::Edge(edge)) => Some(Action::SelectGraphIdentity(edge.to)),
            Some(GraphHit::Phase(phase)) => Some(Action::OpenGraphLocation(
                aplan_app::GraphExplorerLocation::Phase(phase),
            )),
            Some(GraphHit::VerticalScrollbar | GraphHit::HorizontalScrollbar) | None => {
                Some(Action::Focus(Focus::Primary))
            }
        };
    }
    layout
        .detail
        .filter(|area| area.inner(Margin::new(1, 1)).contains(position))
        .map(|_| Action::Focus(Focus::Detail))
}

pub(crate) fn render(frame: &mut Frame<'_>, model: &Model) {
    if model.startup.is_some() {
        render_startup(frame, model);
        return;
    }
    let layout = ShellLayout::new(frame.area(), model);
    if layout.class == LayoutClass::TooSmall {
        render_too_small(frame, model, layout.area);
        return;
    }

    render_header(frame, model, layout.header, layout.density);
    if model.onboarding.is_some() {
        let body = Rect::new(
            layout.area.x,
            layout.header.bottom(),
            layout.area.width,
            layout.footer.y.saturating_sub(layout.header.bottom()),
        );
        render_onboarding(frame, model, body);
    } else {
        render_navigation(frame, model, &layout);
        match (layout.class, model.focus) {
            (LayoutClass::Narrow, Focus::Detail) => {
                render_detail(frame, model, layout.primary);
            }
            _ => render_primary(
                frame,
                model,
                layout.primary,
                layout.class == LayoutClass::Narrow,
            ),
        }
        if let Some(area) = layout.detail {
            render_detail(frame, model, area);
        }
    }
    render_footer(frame, model, &layout);
    if let Some(modal) = model.modal {
        render_modal(frame, model, modal, layout.area);
    }
    if model.mutation.is_some() {
        render_mutation(frame, model, layout.area);
    }
}

struct StartupPresentation {
    marker: &'static str,
    elapsed: String,
    phase: &'static str,
    measure: Option<aplan_app::ProgressMeasure>,
    lifecycle: &'static str,
}

fn startup_presentation(model: &Model) -> StartupPresentation {
    startup_presentation_at(model, Instant::now())
}

fn startup_presentation_at(model: &Model, now: Instant) -> StartupPresentation {
    let startup = model
        .startup
        .as_ref()
        .expect("startup renderer is selected only for startup state");
    let marker = if model.motion == crate::model::Motion::Reduced {
        "*"
    } else if model.glyphs == Glyphs::Ascii {
        ["|", "/", "-", "\\"][usize::from(model.animation_frame)]
    } else {
        ["\u{25d0}", "\u{25d3}", "\u{25d1}", "\u{25d2}"][usize::from(model.animation_frame)]
    };
    let elapsed = now.saturating_duration_since(startup.started);
    let elapsed = if elapsed.as_secs() == 0 {
        format!("{} ms", elapsed.as_millis())
    } else {
        format!("{:.1} s", elapsed.as_secs_f64())
    };
    let (phase, measure, lifecycle) =
        startup
            .latest
            .map_or(("preparing the loading worker", None, "queued"), |event| {
                let phase = match event.subject {
                    aplan_app::ProgressSubject::Operation => event.kind.label(),
                    aplan_app::ProgressSubject::Phase { phase, .. } => phase.label(),
                };
                let lifecycle = match event.lifecycle {
                    aplan_app::ProgressLifecycle::Queued => "queued",
                    aplan_app::ProgressLifecycle::Started => "started",
                    aplan_app::ProgressLifecycle::Advanced => "in progress",
                    aplan_app::ProgressLifecycle::Completed => "complete",
                    aplan_app::ProgressLifecycle::Failed => "failed",
                    aplan_app::ProgressLifecycle::Cancelled => "cancelled",
                };
                (phase, Some(event.measure), lifecycle)
            });
    StartupPresentation {
        marker,
        elapsed,
        phase,
        measure,
        lifecycle,
    }
}

fn render_startup(frame: &mut Frame<'_>, model: &Model) {
    let area = frame.area();
    let styles = palette(model.theme);
    let StartupPresentation {
        marker,
        elapsed,
        phase,
        measure,
        lifecycle,
    } = startup_presentation(model);
    let shell = (classify(area) != LayoutClass::TooSmall).then(|| ShellLayout::new(area, model));
    let compact = shell
        .as_ref()
        .map_or(area.width < 48 || area.height < 12, |layout| {
            layout.class == LayoutClass::Narrow
        });
    let panel = if let Some(layout) = &shell {
        render_startup_chrome(frame, model, layout, &elapsed, measure, styles);
        layout.primary
    } else if compact {
        area
    } else {
        let width = area.width.min(72);
        let height = area.height.min(13);
        Rect::new(
            area.x + area.width.saturating_sub(width) / 2,
            area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        )
    };
    let mut block = Block::default()
        .title(" aplan - loading identified inputs ")
        .borders(Borders::ALL)
        .border_style(styles.accent);
    if model.glyphs == Glyphs::Ascii {
        block = block.border_set(ASCII_BORDER);
    }
    let inner = block.inner(panel);
    frame.render_widget(Clear, panel);
    frame.render_widget(block, panel);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let mut lines = Vec::new();
    if inner.height >= 7 {
        lines.push(Line::styled(
            "A status is a claim, not a proof.",
            styles.accent,
        ));
        lines.push(Line::raw(""));
    }
    lines.push(Line::from(vec![
        Span::styled(format!("{marker} "), styles.accent),
        Span::styled(phase, styles.normal),
    ]));
    let constrained = inner.height <= 3;
    if constrained
        && let Some(aplan_app::ProgressMeasure::Bounded {
            completed,
            total,
            unit,
        }) = measure
    {
        lines.push(Line::styled(
            format!("{completed}/{total} {}", progress_unit_label(unit)),
            styles.muted,
        ));
    } else {
        lines.push(Line::styled(
            format!("{lifecycle}  |  elapsed {elapsed}"),
            styles.muted,
        ));
        append_startup_measure(&mut lines, model, styles, compact, inner.width, measure);
    }
    if inner.height >= 8 {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "Resize is safe. Press q, Esc, or Ctrl-C to cancel.",
            styles.muted,
        ));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

fn render_startup_chrome(
    frame: &mut Frame<'_>,
    model: &Model,
    layout: &ShellLayout,
    elapsed: &str,
    measure: Option<aplan_app::ProgressMeasure>,
    styles: Palette,
) {
    let line = if layout.density == ChromeDensity::Compact {
        Line::from(vec![
            Span::styled("aplan ", styles.accent),
            Span::styled("* LOADING", styles.warning),
            Span::styled(
                text::fit(
                    &format!("  elapsed {elapsed}"),
                    usize::from(layout.header.width.saturating_sub(15)),
                    model.glyphs == Glyphs::Ascii,
                ),
                styles.muted,
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled(" aplan ", styles.accent),
            Span::styled("* LOADING", styles.warning),
            Span::styled(format!("  elapsed {elapsed}"), styles.muted),
        ])
    };
    let header = Paragraph::new(line).style(styles.normal);
    if layout.density == ChromeDensity::Compact {
        frame.render_widget(header, layout.header);
    } else {
        frame.render_widget(
            header.block(block(model, " Plan health ", false)),
            layout.header,
        );
    }

    let navigation = if layout.class == LayoutClass::Wide {
        route_labels(layout.class)
            .into_iter()
            .map(|(_, label)| Line::styled(label, styles.muted))
            .collect::<Vec<_>>()
    } else {
        vec![Line::from(
            route_labels(layout.class)
                .into_iter()
                .flat_map(|(_, label)| [Span::styled(label, styles.muted), Span::raw(" ")])
                .collect::<Vec<_>>(),
        )]
    };
    let navigation = Paragraph::new(navigation);
    if layout.density == ChromeDensity::Compact && layout.class != LayoutClass::Wide {
        frame.render_widget(navigation, layout.navigation);
    } else {
        frame.render_widget(
            navigation.block(block(model, " Destinations ", false)),
            layout.navigation,
        );
    }
    if let Some(detail) = layout.detail {
        frame.render_widget(
            Paragraph::new("One identified snapshot will populate this pane.")
                .style(styles.muted)
                .wrap(Wrap { trim: true })
                .block(block(model, " Inspector ", false)),
            detail,
        );
    }
    let footer = Paragraph::new(vec![
        Line::styled("q / Esc / Ctrl-C  cancel", styles.normal),
        Line::styled(
            text::fit(
                if matches!(measure, Some(aplan_app::ProgressMeasure::Bounded { .. })) {
                    "stable phase total available"
                } else {
                    "indeterminate phase; no fake ETA"
                },
                usize::from(layout.footer.width),
                model.glyphs == Glyphs::Ascii,
            ),
            styles.muted,
        ),
    ]);
    if layout.density == ChromeDensity::Compact {
        frame.render_widget(footer, layout.footer);
    } else {
        frame.render_widget(
            footer.block(block(model, " Commands ", false)),
            layout.footer,
        );
    }
}

fn append_startup_measure(
    lines: &mut Vec<Line<'static>>,
    model: &Model,
    styles: Palette,
    compact: bool,
    inner_width: u16,
    measure: Option<aplan_app::ProgressMeasure>,
) {
    if let Some(aplan_app::ProgressMeasure::Bounded {
        completed,
        total,
        unit,
    }) = measure
    {
        let label = progress_unit_label(unit);
        let bar_width = usize::from(inner_width.saturating_sub(4)).min(48);
        let filled = completed
            .saturating_mul(bar_width as u64)
            .checked_div(total)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(bar_width)
            .min(bar_width);
        let (full, empty) = if model.glyphs == Glyphs::Ascii {
            ("#", "-")
        } else {
            ("\u{2588}", "\u{2591}")
        };
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            format!(
                "{}{}",
                full.repeat(filled),
                empty.repeat(bar_width - filled)
            ),
            styles.good,
        ));
        lines.push(Line::styled(
            format!("{completed}/{total} {label}"),
            styles.muted,
        ));
    } else if !compact {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "This phase has no honest fixed total; no percentage or ETA is invented.",
            styles.muted,
        ));
    }
}

const fn progress_unit_label(unit: aplan_app::ProgressUnit) -> &'static str {
    match unit {
        aplan_app::ProgressUnit::Sources => "sources",
        aplan_app::ProgressUnit::Providers => "providers",
        aplan_app::ProgressUnit::Items => "items",
    }
}

#[derive(Debug, Clone, Copy)]
struct Palette {
    normal: Style,
    muted: Style,
    accent: Style,
    selected: Style,
    good: Style,
    warning: Style,
    error: Style,
}

fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::Dark => Palette {
            normal: Style::new().fg(Color::White),
            muted: Style::new().fg(Color::DarkGray),
            accent: Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            selected: Style::new()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            good: Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
            warning: Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            error: Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
        },
        Theme::Light => Palette {
            normal: Style::new().fg(Color::Black),
            muted: Style::new().fg(Color::Gray),
            accent: Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD),
            selected: Style::new()
                .fg(Color::White)
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD),
            good: Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
            warning: Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD),
            error: Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
        },
        Theme::LowColor => Palette {
            normal: Style::new(),
            muted: Style::new().add_modifier(Modifier::DIM),
            accent: Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            selected: Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD),
            good: Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
            warning: Style::new()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            error: Style::new()
                .fg(Color::Red)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED),
        },
        Theme::Monochrome => Palette {
            normal: Style::new(),
            muted: Style::new().add_modifier(Modifier::DIM),
            accent: Style::new().add_modifier(Modifier::BOLD),
            selected: Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD),
            good: Style::new().add_modifier(Modifier::BOLD),
            warning: Style::new().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            error: Style::new().add_modifier(Modifier::BOLD | Modifier::REVERSED),
        },
    }
}

fn block(model: &Model, title: impl Into<String>, focused: bool) -> Block<'static> {
    let styles = palette(model.theme);
    let title = title.into();
    let title = if focused {
        format!("> {} <", title.trim())
    } else {
        title
    };
    let mut block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(if focused { styles.accent } else { styles.muted });
    if model.glyphs == Glyphs::Ascii {
        block = block.border_set(ASCII_BORDER);
    }
    block
}

fn status_token(model: &Model) -> (&'static str, &'static str, Style) {
    let styles = palette(model.theme);
    if model.snapshot.overview.errors > 0 {
        (
            "ERROR",
            if model.glyphs == Glyphs::Ascii {
                "x"
            } else {
                "\u{2715}"
            },
            styles.error,
        )
    } else if model.snapshot.overview.warnings > 0 {
        (
            "WARN",
            if model.glyphs == Glyphs::Ascii {
                "!"
            } else {
                "\u{25b2}"
            },
            styles.warning,
        )
    } else {
        (
            "OK",
            if model.glyphs == Glyphs::Ascii {
                "+"
            } else {
                "\u{25cf}"
            },
            styles.good,
        )
    }
}

fn render_header(frame: &mut Frame<'_>, model: &Model, area: Rect, density: ChromeDensity) {
    let styles = palette(model.theme);
    let (status, shape, status_style) = status_token(model);
    let ascii = model.glyphs == Glyphs::Ascii;
    let brand = " aplan ";
    let status = format!("{shape} {status}");
    let inactive = if model.active { "" } else { "  INACTIVE" };
    let summary = format!(
        "  {}  {}  {}",
        counted(model.snapshot.overview.loaded_tasks, "task"),
        counted(model.snapshot.overview.errors, "error"),
        counted(model.snapshot.overview.warnings, "warning")
    );
    let capacity = usize::from(match density {
        ChromeDensity::Full => area.width.saturating_sub(2),
        ChromeDensity::Compact => area.width,
    });
    let fixed = text::cells(brand) + text::cells(&status) + text::cells(inactive);
    let remaining = capacity.saturating_sub(fixed);
    let root = model
        .snapshot
        .overview
        .input
        .plan_root
        .display()
        .to_string();
    let root_reserve = if root.is_empty() {
        0
    } else {
        remaining.min(12)
    };
    let summary_budget = if text::cells(&summary)
        .saturating_add(root_reserve)
        .saturating_add(2)
        <= remaining
    {
        text::cells(&summary)
    } else {
        remaining.saturating_sub(root_reserve.saturating_add(2))
    };
    let summary = text::fit(&summary, summary_budget, ascii);
    let remaining = remaining.saturating_sub(text::cells(&summary));
    let root = if remaining > 2 {
        let fitted = text::fit(&root, remaining - 2, ascii);
        format!("  {fitted}")
    } else {
        String::new()
    };
    let line = Line::from(vec![
        Span::styled(brand, styles.accent),
        Span::styled(status, status_style),
        Span::styled(inactive, styles.warning),
        Span::raw(summary),
        Span::styled(root, styles.muted),
    ]);
    let paragraph = Paragraph::new(line).style(styles.normal);
    if density == ChromeDensity::Compact {
        frame.render_widget(paragraph, area);
    } else {
        frame.render_widget(paragraph.block(block(model, " Plan health ", false)), area);
    }
}

fn counted(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

fn route_labels(class: LayoutClass) -> Vec<(Route, String)> {
    Route::ALL
        .into_iter()
        .map(|route| {
            let label = if class == LayoutClass::Narrow {
                match route {
                    Route::Overview => "O",
                    Route::Tasks => "T",
                    Route::Diagnostics => "D",
                    Route::Graph => "G",
                    Route::Reports => "R",
                    Route::Inbox => "I",
                }
            } else {
                route.label()
            };
            (route, format!("[{} {label}]", route.number()))
        })
        .collect()
}

fn route_targets(area: Rect, class: LayoutClass, density: ChromeDensity) -> Vec<Target<Route>> {
    let labels = route_labels(class);
    if class == LayoutClass::Wide {
        return labels
            .into_iter()
            .enumerate()
            .map(|(index, (route, label))| {
                label_target(
                    area.x.saturating_add(1),
                    area.y
                        .saturating_add(1 + u16::try_from(index).unwrap_or(u16::MAX)),
                    &label,
                    route,
                )
            })
            .collect();
    }

    let inset = u16::from(density == ChromeDensity::Full);
    let mut x = area.x.saturating_add(inset);
    labels
        .into_iter()
        .map(|(route, label)| {
            let width = u16::try_from(text::cells(&label)).unwrap_or(u16::MAX);
            let target = label_target(x, area.y.saturating_add(inset), &label, route);
            x = x.saturating_add(width).saturating_add(1);
            target
        })
        .collect()
}

fn render_navigation(frame: &mut Frame<'_>, model: &Model, layout: &ShellLayout) {
    let styles = palette(model.theme);
    let labels = route_labels(layout.class);
    let lines = if layout.class == LayoutClass::Wide {
        labels
            .into_iter()
            .map(|(route, label)| {
                Line::styled(
                    label,
                    if route == model.route {
                        styles.selected
                    } else {
                        styles.normal
                    },
                )
            })
            .collect()
    } else {
        vec![Line::from(
            labels
                .into_iter()
                .flat_map(|(route, label)| {
                    [
                        Span::styled(
                            label,
                            if route == model.route {
                                styles.selected
                            } else {
                                styles.normal
                            },
                        ),
                        Span::raw(" "),
                    ]
                })
                .collect::<Vec<_>>(),
        )]
    };
    let paragraph = Paragraph::new(lines).style(styles.normal);
    if layout.density == ChromeDensity::Compact && layout.class != LayoutClass::Wide {
        frame.render_widget(paragraph, layout.navigation);
    } else {
        frame.render_widget(
            paragraph.block(block(
                model,
                " Destinations ",
                model.focus == Focus::Navigation,
            )),
            layout.navigation,
        );
    }
}

fn primary_title(model: &Model) -> String {
    format!(" {} - {} ", model.route.label(), model.route_question())
}

fn detail_title(model: &Model, area: Rect, subject: &str) -> String {
    let raw = if classify(Rect::new(0, 0, model.columns, model.rows)) == LayoutClass::Narrow {
        let back = match model.route {
            Route::Overview => "metrics",
            Route::Tasks => {
                if model.inspector_has_history() {
                    "back"
                } else {
                    "tasks"
                }
            }
            Route::Diagnostics => "findings",
            Route::Graph => {
                if model.inspector_has_history() {
                    "back"
                } else {
                    "graph"
                }
            }
            Route::Reports => "reports",
            Route::Inbox => "inbox",
        };
        format!(" Esc {back} | {} > {subject} ", model.route.label())
    } else {
        format!(" {subject} ")
    };
    text::fit(
        &raw,
        usize::from(area.width.saturating_sub(4)),
        model.glyphs == Glyphs::Ascii,
    )
}

fn matched_field(
    value: &str,
    matches: &[aplan_app::MatchSpan],
    width: usize,
    ascii: bool,
    base: Style,
    match_style: Style,
) -> Vec<Span<'static>> {
    let fitted = text::fit(value, width, ascii);
    if matches.is_empty() || text::safe(value) != value {
        return vec![Span::styled(fitted, base)];
    }
    let mut spans = Vec::new();
    let mut at = 0;
    for range in matches {
        if range.start < at
            || range.end > fitted.len()
            || !fitted.is_char_boundary(range.start)
            || !fitted.is_char_boundary(range.end)
        {
            continue;
        }
        if range.start > at {
            spans.push(Span::styled(fitted[at..range.start].to_string(), base));
        }
        spans.push(Span::styled(
            fitted[range.start..range.end].to_string(),
            match_style,
        ));
        at = range.end;
    }
    if at < fitted.len() {
        spans.push(Span::styled(fitted[at..].to_string(), base));
    }
    spans
}

fn task_sort_metadata(row: &aplan_app::TaskBrowserMatch, sort: aplan_app::TaskSort) -> String {
    match sort {
        aplan_app::TaskSort::ReadyRank => {
            let rank = row
                .task
                .ready_rank
                .map_or_else(|| "--".to_string(), |rank| format!("#{rank}"));
            row.task.rank_reason.as_ref().map_or_else(
                || format!(" [{rank} {}]", row.task.status),
                |reason| format!(" [{rank} {}; {reason}]", row.task.status),
            )
        }
        aplan_app::TaskSort::Id | aplan_app::TaskSort::Title | aplan_app::TaskSort::Status => {
            format!(" [{}]", row.task.status)
        }
        aplan_app::TaskSort::Cut => {
            format!(" [cut:{}]", row.task.cut.as_deref().unwrap_or("none"))
        }
        aplan_app::TaskSort::Phase => format!(" [phase:{}]", row.task.phase),
        aplan_app::TaskSort::Owner => {
            format!(" [owner:{}]", row.task.owner.as_deref().unwrap_or("nobody"))
        }
    }
}

fn task_semantic_style(row: &aplan_app::TaskBrowserMatch, styles: Palette) -> Style {
    match row.task.status {
        aplan_app::Status::Done | aplan_app::Status::Dropped => styles.muted,
        aplan_app::Status::Blocked => styles.warning,
        aplan_app::Status::InProgress => styles.accent,
        aplan_app::Status::Todo if row.task.ready_rank.is_some() => styles.good,
        aplan_app::Status::Todo => styles.normal,
    }
}

fn task_line(
    model: &Model,
    styles: Palette,
    width: usize,
    index: usize,
    row: &aplan_app::TaskBrowserMatch,
) -> Line<'static> {
    let field_budget = width.saturating_sub(4);
    let mut metadata = if model.route == Route::Tasks {
        task_sort_metadata(row, model.task_sort)
    } else {
        String::new()
    };
    if row.body_matches {
        metadata.push_str(" [body]");
    }
    if let Some(claim) = &row.task.claim {
        let _ = write!(
            metadata,
            " [claim:{}]",
            aplan_output::sanitize::visible(&claim.holder)
        );
    }
    metadata = text::fit(&metadata, field_budget / 2, model.glyphs == Glyphs::Ascii);
    let metadata_width = text::cells(&metadata).min(field_budget);
    let content_budget = field_budget.saturating_sub(metadata_width);
    let id_budget = if row.task.title.is_empty() {
        content_budget
    } else {
        content_budget.div_ceil(3).max(6).min(content_budget)
    };
    let id = text::fit(&row.task.id, id_budget, model.glyphs == Glyphs::Ascii);
    let title_budget = content_budget.saturating_sub(text::cells(&id));
    let selected = model.selected_visible_index() == Some(index);
    let row_style = if selected {
        styles.selected
    } else {
        task_semantic_style(row, styles)
    };
    let match_style = row_style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    let mut spans = vec![Span::styled(
        if selected { "> " } else { "  " },
        if selected {
            styles.accent
        } else {
            styles.normal
        },
    )];
    spans.extend(matched_field(
        &row.task.id,
        &row.id_matches,
        id_budget,
        model.glyphs == Glyphs::Ascii,
        styles.accent,
        styles
            .accent
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    ));
    spans.push(Span::raw("  "));
    spans.extend(matched_field(
        &row.task.title,
        &row.title_matches,
        title_budget,
        model.glyphs == Glyphs::Ascii,
        row_style,
        match_style,
    ));
    spans.push(Span::styled(metadata, styles.muted));
    Line::from(spans)
}

fn capture_deferred(model: &Model, capture: &aplan_core::capture::Capture) -> bool {
    model.snapshot.inbox.as_ref().is_ok_and(|workspace| {
        capture.state == aplan_core::capture::CaptureState::Open
            && capture
                .revisit_at
                .as_deref()
                .is_some_and(|instant| instant > workspace.evaluated_at.as_str())
    })
}

fn capture_line(
    model: &Model,
    styles: Palette,
    width: usize,
    index: usize,
    capture: &aplan_core::capture::Capture,
) -> Line<'static> {
    let selected = model.selected_capture_visible_index() == Some(index);
    let deferred = capture_deferred(model, capture);
    let state = if deferred {
        "DEFERRED"
    } else {
        crate::model::capture_state_name(capture.state)
    };
    let profile = capture
        .profile
        .as_deref()
        .map_or(String::new(), |profile| format!(" profile:{profile}"));
    let source = capture.source.as_ref().map_or(String::new(), |source| {
        format!(" source:{}", source.namespace)
    });
    let metadata = text::fit(
        &format!(
            " [{state}; {}; {}{profile}{source}]",
            capture.kind, capture.captured_at
        ),
        width.saturating_div(2),
        model.glyphs == Glyphs::Ascii,
    );
    let prefix = if selected { "> " } else { "  " };
    let content = text::fit(
        &format!("{}  {}", capture.id, capture.title),
        width.saturating_sub(text::cells(prefix) + text::cells(&metadata)),
        model.glyphs == Glyphs::Ascii,
    );
    let semantic = if selected {
        styles.selected
    } else {
        match capture.state {
            aplan_core::capture::CaptureState::Open if deferred => styles.warning,
            aplan_core::capture::CaptureState::Open => styles.normal,
            aplan_core::capture::CaptureState::Promoted => styles.good,
            aplan_core::capture::CaptureState::Discarded => styles.muted,
        }
    };
    Line::from(vec![
        Span::styled(
            prefix,
            if selected {
                styles.accent
            } else {
                styles.normal
            },
        ),
        Span::styled(content, semantic),
        Span::styled(
            metadata,
            if selected {
                styles.selected
            } else {
                styles.muted
            },
        ),
    ])
}

fn render_inbox(frame: &mut Frame<'_>, model: &Model, area: Rect, compact: bool) {
    let styles = palette(model.theme);
    let rows = model.inbox_rows();
    let viewport = task_viewport(area, compact, model.selected, rows.len());
    let mut lines = route_intro(model, styles, compact, usize::from(viewport.content.width));
    match &model.snapshot.inbox {
        Err(error) => lines.push(Line::styled(
            text::fit(
                &format!("Inbox unavailable: {error}"),
                usize::from(viewport.content.width),
                model.glyphs == Glyphs::Ascii,
            ),
            styles.error,
        )),
        Ok(_) if rows.is_empty() => lines.push(Line::styled(
            if model.has_inbox_narrowing() {
                "No captures match the active search and filters."
            } else {
                "No actionable open captures. Deferred and terminal records remain available through Filters."
            },
            styles.muted,
        )),
        Ok(_) => lines.extend(
            rows.iter()
                .enumerate()
                .skip(viewport.start)
                .take(viewport.end.saturating_sub(viewport.start))
                .map(|(index, capture)| {
                    capture_line(
                        model,
                        styles,
                        usize::from(viewport.content.width),
                        index,
                        capture,
                    )
                }),
        ),
    }
    frame.render_widget(
        Paragraph::new(lines).style(styles.normal).block(block(
            model,
            text::fit(
                &primary_title(model),
                usize::from(area.width.saturating_sub(2)),
                model.glyphs == Glyphs::Ascii,
            ),
            model.focus == Focus::Primary,
        )),
        area,
    );
}

fn inbox_detail_lines(model: &Model, width: usize, styles: Palette) -> Vec<Line<'static>> {
    let Some(capture) = model.selected_capture() else {
        return vec![Line::styled("No capture selected.", styles.muted)];
    };
    let mut values = vec![
        format!("ID: {}", capture.id),
        format!("Title: {}", capture.title),
        format!(
            "State: {}{}",
            crate::model::capture_state_name(capture.state),
            if capture_deferred(model, capture) {
                " (deferred from the actionable view)"
            } else {
                ""
            }
        ),
        format!("Kind: {}", capture.kind),
        format!("Captured at: {}", capture.captured_at),
        format!("Origin: {:?}", capture.origin),
        format!(
            "Labels: {}",
            if capture.labels.is_empty() {
                "none".to_string()
            } else {
                capture.labels.join(", ")
            }
        ),
        format!(
            "Revisit at: {}",
            capture.revisit_at.as_deref().unwrap_or("none")
        ),
        format!("Profile: {}", capture.profile.as_deref().unwrap_or("none")),
    ];
    if let Some(source) = &capture.source {
        values.push(format!("Source namespace: {}", source.namespace));
        values.push(format!("Source key (focused only): {}", source.key));
    } else {
        values.push("Source: none".to_string());
    }
    if let Some(defaults) = &capture.promotion_defaults {
        values.push(format!(
            "Promotion defaults: phase {:?}; cut {}; effort {:?}; owner {}; paths [{}]; tags [{}]",
            defaults.phase,
            defaults.cut.as_deref().unwrap_or("none"),
            defaults.effort_days,
            defaults.owner.as_deref().unwrap_or("none"),
            defaults.paths.join(", "),
            defaults.tags.join(", "),
        ));
    } else {
        values.push("Promotion defaults: none".to_string());
    }
    values.push(format!(
        "Disposed at: {}",
        capture.disposed_at.as_deref().unwrap_or("none")
    ));
    values.push(format!(
        "Reason: {}",
        capture.reason.as_deref().unwrap_or("none")
    ));
    if let Some(task) = &capture.promoted_to {
        values.push(format!(
            "Promoted task: {task} (Enter opens the loaded task)"
        ));
    }
    values.push("Body:".to_string());
    values.push(
        capture
            .body
            .as_deref()
            .unwrap_or("No body was captured.")
            .to_string(),
    );
    values
        .into_iter()
        .flat_map(|value| {
            text::wrap(&value, width)
                .into_iter()
                .map(|line| Line::styled(line, styles.normal))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn render_inbox_detail(frame: &mut Frame<'_>, model: &Model, area: Rect) {
    let styles = palette(model.theme);
    let content = area.inner(Margin::new(1, 1));
    let lines = inbox_detail_lines(model, usize::from(content.width), styles);
    let capacity = usize::from(content.height);
    let (scroll, _) = resolved_inspector_scroll(model, lines.len(), None, capacity);
    let subject = model
        .selected_capture()
        .map_or("Capture inspector", |capture| capture.id.as_str());
    frame.render_widget(
        Paragraph::new(
            lines
                .into_iter()
                .skip(scroll)
                .take(capacity)
                .collect::<Vec<_>>(),
        )
        .style(styles.normal)
        .block(block(
            model,
            detail_title(model, area, subject),
            model.focus == Focus::Detail,
        )),
        area,
    );
}

fn render_primary(frame: &mut Frame<'_>, model: &Model, area: Rect, compact: bool) {
    match model.route {
        Route::Overview => {
            render_overview(frame, model, area);
            return;
        }
        Route::Diagnostics => {
            render_diagnostics(frame, model, area);
            return;
        }
        Route::Graph => {
            render_graph(frame, model, area, compact);
            return;
        }
        Route::Reports => {
            render_reports(frame, model, area);
            return;
        }
        Route::Inbox => {
            render_inbox(frame, model, area, compact);
            return;
        }
        Route::Tasks => {}
    }
    let styles = palette(model.theme);
    let viewport = task_viewport(area, compact, model.selected, model.task_browser.rows.len());
    let mut lines = route_intro(model, styles, compact, usize::from(viewport.content.width));
    if model.task_browser.rows.is_empty() {
        if usize::from(viewport.content.height) > lines.len() {
            lines.push(Line::styled(
                model
                    .task_browser
                    .no_results
                    .map_or("No tasks loaded.", aplan_app::NoTaskResults::message),
                styles.muted,
            ));
        }
    } else {
        lines.extend(
            model
                .task_browser
                .rows
                .iter()
                .enumerate()
                .skip(viewport.start)
                .take(viewport.end.saturating_sub(viewport.start))
                .map(|(index, row)| {
                    task_line(
                        model,
                        styles,
                        usize::from(viewport.content.width),
                        index,
                        row,
                    )
                }),
        );
    }
    frame.render_widget(
        Paragraph::new(lines).style(styles.normal).block(block(
            model,
            text::fit(
                &primary_title(model),
                usize::from(area.width.saturating_sub(2)),
                model.glyphs == Glyphs::Ascii,
            ),
            model.focus == Focus::Primary,
        )),
        area,
    );
}

fn report_description(kind: aplan_app::ReportKind) -> &'static str {
    match kind {
        aplan_app::ReportKind::Roadmap => "cuts, effort, duration and critical paths",
        aplan_app::ReportKind::Summary => "selected work rolled up by phase or configured family",
        aplan_app::ReportKind::Version => "owed impacts, sources and contributors",
        aplan_app::ReportKind::Paths => "recent-history breadth of declared paths",
        aplan_app::ReportKind::Vacuity => "what each rule could actually have checked",
        aplan_app::ReportKind::Stale => "advisory inactivity measured from Git history",
        aplan_app::ReportKind::Deprecations => "announcement and removal runways",
    }
}

fn render_reports(frame: &mut Frame<'_>, model: &Model, area: Rect) {
    let styles = palette(model.theme);
    let kinds = model.report_kinds();
    let selected = kinds
        .iter()
        .position(|kind| *kind == model.report_kind)
        .unwrap_or_default();
    let viewport = item_viewport(area, selected, kinds.len());
    let width = usize::from(viewport.content.width);
    let ascii = model.glyphs == Glyphs::Ascii;
    let mut lines = kinds
        .iter()
        .copied()
        .enumerate()
        .skip(viewport.start)
        .take(viewport.end.saturating_sub(viewport.start))
        .map(|(index, kind)| {
            let selected = index == selected;
            Line::styled(
                text::fit(
                    &format!(
                        "{}{:14}  {}",
                        if selected { "> " } else { "  " },
                        kind.label(),
                        report_description(kind)
                    ),
                    width,
                    ascii,
                ),
                if selected {
                    styles.selected
                } else {
                    styles.normal
                },
            )
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        lines.push(Line::styled(
            text::fit(
                &format!(
                    "No report name matches `{}`; the reports still exist. Press x to clear.",
                    text::safe(&model.report_kind_query)
                ),
                width,
                ascii,
            ),
            styles.muted,
        ));
    }
    frame.render_widget(
        Paragraph::new(lines).style(styles.normal).block(block(
            model,
            primary_title(model),
            model.focus == Focus::Primary,
        )),
        area,
    );
}

fn graph_role_style(role: GraphRole, styles: Palette) -> Style {
    match role {
        GraphRole::Selected | GraphRole::SelectedCritical => styles.selected,
        GraphRole::Blocker | GraphRole::BlockerCritical | GraphRole::BlockerDependent => {
            styles.warning
        }
        GraphRole::Dependent | GraphRole::DependentCritical => styles.good,
        GraphRole::Member | GraphRole::MemberCritical | GraphRole::Critical => styles.accent,
        GraphRole::Unrelated => styles.normal,
    }
}

fn graph_state_label(projection: &aplan_app::GraphExplorerProjection) -> String {
    let windowed = !projection.accounting.window_omitted_task_ids.is_empty();
    match &projection.state {
        aplan_app::GraphExplorerState::Complete if windowed => "WINDOWED".to_string(),
        aplan_app::GraphExplorerState::Complete
            if projection.accounting.represented_dependency_edges == 0 =>
        {
            "COMPLETE NO EDGES".to_string()
        }
        aplan_app::GraphExplorerState::Complete => "COMPLETE".to_string(),
        aplan_app::GraphExplorerState::Empty => "EMPTY".to_string(),
        aplan_app::GraphExplorerState::InvalidTaskCycle if windowed => {
            "WINDOWED INVALID CYCLIC TASK GRAPH".to_string()
        }
        aplan_app::GraphExplorerState::InvalidTaskCycle => "INVALID CYCLIC TASK GRAPH".to_string(),
        aplan_app::GraphExplorerState::SourceUnavailable(state) => {
            format!("SOURCE UNAVAILABLE: {state:?}")
        }
        aplan_app::GraphExplorerState::UnknownLocation(location) => {
            format!("UNKNOWN LOCATION: {location:?}")
        }
    }
}

fn graph_controls_line(model: &Model, width: usize, styles: Palette) -> Line<'static> {
    let mut spans = Vec::new();
    let mut used = 0;
    for (label, _) in graph_control_specs(model) {
        let separator = usize::from(!spans.is_empty()) * 3;
        let cells = text::cells(&label);
        if used + separator + cells > width {
            continue;
        }
        if !spans.is_empty() {
            spans.push(Span::styled(" | ", styles.muted));
        }
        spans.push(Span::styled(label, styles.accent));
        used += separator + cells;
    }
    if spans.is_empty() {
        Line::styled(
            text::fit(
                "No omitted continuation; +/- selects neighbourhood depth.",
                width,
                true,
            ),
            styles.muted,
        )
    } else {
        Line::from(spans)
    }
}

fn graph_context_identity(model: &Model, width: usize) -> String {
    let selection = model.graph_explorer.selection.as_ref();
    let selected = selection.map_or("none", |selection| selection.represented_by.id());
    let semantic = selection.and_then(|selection| {
        model
            .graph_explorer
            .graph
            .nodes
            .iter()
            .find(|node| node.identity == selection.represented_by)
    });
    let rank = model.graph_cell_scene().and_then(|scene| {
        scene
            .nodes
            .iter()
            .find(|node| node.identity.id() == selected)
            .and_then(|node| node.rank)
    });
    let mut identity = if model.task_query.is_empty() {
        String::new()
    } else {
        format!("find={} | ", text::safe(&model.task_query))
    };
    if width < 64 {
        let _ = write!(
            identity,
            "{} | {} {}/{}N {}/{}E omit {}M/{}G",
            model.graph_breadcrumb(),
            graph_state_label(&model.graph_explorer),
            model.graph_explorer.accounting.visible_task_ids.len(),
            model.graph_explorer.accounting.admitted_task_ids.len(),
            model.graph_explorer.accounting.represented_dependency_edges,
            model.graph_explorer.accounting.admitted_dependency_edges,
            model.graph_explorer.accounting.grouping.edges,
            model.graph_explorer.accounting.grouping.tasks,
        );
    } else {
        let _ = write!(
            identity,
            "{} | {} {}/{}N {}/{}E omit {}M/{}G | {selected} P{}/R{}",
            model.graph_breadcrumb(),
            graph_state_label(&model.graph_explorer),
            model.graph_explorer.accounting.visible_task_ids.len(),
            model.graph_explorer.accounting.admitted_task_ids.len(),
            model.graph_explorer.accounting.represented_dependency_edges,
            model.graph_explorer.accounting.admitted_dependency_edges,
            model.graph_explorer.accounting.grouping.edges,
            model.graph_explorer.accounting.grouping.tasks,
            semantic.map_or_else(|| "-".to_string(), |node| node.phase.to_string()),
            rank.map_or_else(|| "-".to_string(), |rank| rank.to_string()),
        );
    }
    if model.graph_scene_fallback().is_some() {
        identity = format!("FALLBACK | {identity}");
    }
    if let Some((incoming, index, count)) = model.graph_relation_progress() {
        let _ = write!(
            identity,
            " | {} {index}/{count}",
            if incoming { "blocker" } else { "dependent" }
        );
    }
    identity
}

fn graph_context_lines(
    model: &Model,
    viewport: GraphViewport,
    compact: bool,
    width: usize,
    ascii: bool,
    styles: Palette,
) -> Vec<Line<'static>> {
    let (total_rows, total_columns) = model.graph_content_extent();
    let identity = graph_context_identity(model, width);
    let up = viewport.row_start;
    let down = total_rows.saturating_sub(viewport.row_end);
    let left = viewport.column_start;
    let right = total_columns.saturating_sub(viewport.column_end);
    let extent = format!(
        "r{}:{}/{} c{}:{}/{} | U{up} D{down} L{left} R{right} | {} ({}) | {} | {} | {}",
        usize::from(viewport.row_end > viewport.row_start) * (viewport.row_start + 1),
        viewport.row_end,
        total_rows,
        usize::from(viewport.column_end > viewport.column_start) * (viewport.column_start + 1),
        viewport.column_end,
        total_columns,
        model.graph_ordering.label(),
        model.graph_ordering.layout_cost(),
        model.graph_orientation_label(),
        if model.graph_edge_presentation.is_relations_only() {
            "relations only"
        } else {
            "all edges"
        },
        model.graph_movement_label(),
    );
    let mut lines = vec![Line::styled(
        text::fit(&identity, width, ascii),
        if !model
            .graph_explorer
            .accounting
            .window_omitted_task_ids
            .is_empty()
            || model.graph_scene_fallback().is_some()
        {
            styles.warning
        } else {
            styles.accent
        },
    )];
    if viewport.intro_rows > 1 {
        lines.push(Line::styled(text::fit(&extent, width, ascii), styles.muted));
    }
    if viewport.intro_rows > 2 {
        lines.push(graph_controls_line(model, width, styles));
    }
    if compact && viewport.intro_rows == 2 {
        lines[1] = graph_controls_line(model, width, styles);
    }
    lines
}

fn graph_fallback_lines(
    model: &Model,
    viewport: GraphViewport,
    width: usize,
    styles: Palette,
) -> Vec<Line<'static>> {
    let visible_rows = viewport.row_end.saturating_sub(viewport.row_start);
    let mut lines = Vec::with_capacity(visible_rows);
    if viewport.row_start == 0 && viewport.row_end > 0 {
        let message = model.graph_scene_fallback().map_or_else(
            || graph_widget::FALLBACK_MESSAGE.to_string(),
            graph_widget::fallback_message,
        );
        lines.push(Line::styled(
            text::slice_cells(&message, viewport.column_start, width),
            styles.warning,
        ));
    }

    // Conceptual row zero is the fallback explanation. Translate the visible
    // rows after it directly into a node slice so painting a viewport never
    // formats nodes above or below that viewport.
    let nodes = &model.graph_explorer.graph.nodes;
    let node_start = viewport.row_start.max(1).saturating_sub(1).min(nodes.len());
    let node_end = viewport.row_end.saturating_sub(1).min(nodes.len());
    lines.extend(nodes[node_start..node_end].iter().map(|node| {
        let selected = model
            .graph_explorer
            .selection
            .as_ref()
            .is_some_and(|selection| selection.represented_by == node.identity);
        Line::styled(
            text::slice_cells(
                &graph_widget::fallback_node_text(node, selected),
                viewport.column_start,
                width,
            ),
            if selected {
                styles.selected
            } else {
                styles.normal
            },
        )
    }));
    lines
}

fn render_graph(frame: &mut Frame<'_>, model: &Model, area: Rect, compact: bool) {
    let styles = palette(model.theme);
    let viewport = graph_viewport(area, compact, model);
    let width = usize::from(viewport.content.width);
    let ascii = model.glyphs == Glyphs::Ascii;
    frame.render_widget(
        block(
            model,
            text::fit(
                &primary_title(model),
                usize::from(area.width.saturating_sub(2)),
                ascii,
            ),
            model.focus == Focus::Primary,
        ),
        area,
    );
    let context = graph_context_lines(model, viewport, compact, width, ascii, styles);
    let context_area = Rect::new(
        viewport.content.x,
        viewport.content.y,
        viewport.content.width,
        viewport.intro_rows,
    );
    frame.render_widget(Paragraph::new(context).style(styles.normal), context_area);
    let graph_area = Rect::new(
        viewport.content.x,
        viewport.content.y.saturating_add(viewport.intro_rows),
        viewport.content.width,
        viewport.content.height.saturating_sub(viewport.intro_rows),
    );
    if let Some(scene) = model.graph_cell_scene() {
        let mut state = LayeredGraphState::default();
        frame.render_stateful_widget(
            LayeredGraph {
                scene,
                projection: &model.graph_explorer,
                viewport: LayeredViewport {
                    row: viewport.row_start,
                    column: viewport.column_start,
                },
                density: model.graph_resolved_density,
                ascii,
                edge_presentation: model.graph_edge_presentation,
                styles: GraphStyles {
                    normal: styles.normal,
                    muted: styles.muted,
                    accent: styles.accent,
                    selected: styles.selected,
                    warning: styles.warning,
                },
            },
            graph_area,
            &mut state,
        );
    } else {
        frame.render_widget(
            Paragraph::new(graph_fallback_lines(model, viewport, width, styles))
                .style(styles.normal),
            graph_area,
        );
    }
}

const OVERVIEW_METER_CELLS: usize = 8;

fn ratio_meter(value: usize, total: usize, filled: char) -> String {
    let rounded = value
        .min(total)
        .saturating_mul(OVERVIEW_METER_CELLS)
        .saturating_add(total / 2)
        .checked_div(total)
        .unwrap_or(0);
    let filled_cells = rounded
        .max(usize::from(value > 0 && total > 0))
        .min(OVERVIEW_METER_CELLS);
    format!(
        "[{}{}]",
        filled.to_string().repeat(filled_cells),
        ".".repeat(OVERVIEW_METER_CELLS.saturating_sub(filled_cells))
    )
}

fn finding_meter(errors: usize, warnings: usize) -> String {
    let total = errors.saturating_add(warnings);
    if total == 0 {
        return "[........]".to_string();
    }
    let occupied = total.min(OVERVIEW_METER_CELLS);
    let mut error_cells = if errors == 0 {
        0
    } else {
        errors
            .saturating_mul(occupied)
            .div_ceil(total)
            .min(occupied)
    };
    if warnings > 0 {
        error_cells = error_cells.min(occupied.saturating_sub(1));
    }
    let warning_cells = occupied.saturating_sub(error_cells);
    format!(
        "[{}{}{}]",
        "!".repeat(error_cells),
        "?".repeat(warning_cells),
        ".".repeat(OVERVIEW_METER_CELLS.saturating_sub(occupied))
    )
}

fn append_meter(exact: String, meter: &str, width: usize) -> String {
    let candidate = format!("{exact} | {meter}");
    if text::cells(&candidate) <= width {
        candidate
    } else {
        exact
    }
}

fn live_work_count(model: &Model) -> usize {
    model
        .snapshot
        .task_rows
        .iter()
        .filter(|task| {
            !task.status.is_terminal() && !task.tags.iter().any(|tag| tag == "milestone")
        })
        .count()
}

fn overview_metric_text(model: &Model, metric: OverviewMetric, width: usize) -> String {
    let overview = &model.snapshot.overview;
    match metric {
        OverviewMetric::Cuts => {
            let cuts = overview.cuts.iter().filter(|cut| !cut.borrows).count();
            append_meter(
                format!(
                    "CUTS {cuts} | DONE {}/{}",
                    overview.done_cut_work, overview.cut_work
                ),
                &ratio_meter(overview.done_cut_work, overview.cut_work, '#'),
                width,
            )
        }
        OverviewMetric::Effort => {
            let qualifier = match overview.effort_state {
                aplan_app::OverviewEffortState::Complete => "complete estimates".to_string(),
                aplan_app::OverviewEffortState::Partial => format!(
                    "{} missing estimate(s)",
                    overview.missing_effort_task_ids.len()
                ),
                aplan_app::OverviewEffortState::None => "no effort estimates".to_string(),
                aplan_app::OverviewEffortState::NoLiveWork => "no live work".to_string(),
            };
            format!(
                "EFFORT {:.1}/{:.1}d | {qualifier}",
                overview.done_effort_days, overview.live_effort_days
            )
        }
        OverviewMetric::Ready => {
            let live = live_work_count(model);
            append_meter(
                format!("READY {}", overview.ready.len()),
                &ratio_meter(overview.ready.len(), live, '+'),
                width,
            )
        }
        OverviewMetric::Due => {
            let revisits = model.work_revisits();
            format!(
                "REVISIT DUE {} | LATER {} | AS-OF {}",
                revisits.due.len(),
                revisits.waiting.len(),
                model.effective_work_as_of_epoch_seconds()
            )
        }
        OverviewMetric::Findings => append_meter(
            format!(
                "FINDINGS E:{} W:{} | {}",
                overview.errors,
                overview.warnings,
                if overview.denied { "DENIED" } else { "PASS" }
            ),
            &finding_meter(overview.errors, overview.warnings),
            width,
        ),
        OverviewMetric::CriticalPath => overview.critical_path.as_ref().map_or_else(
            || "CRITICAL unavailable: dependency cycle".to_string(),
            |(days, tasks)| {
                let live = live_work_count(model);
                append_meter(
                    format!("CRITICAL {days:.1}d | TASKS {}", tasks.len()),
                    &ratio_meter(tasks.len(), live, '*'),
                    width,
                )
            },
        ),
    }
}

fn overview_metric_style(model: &Model, metric: OverviewMetric, styles: Palette) -> Style {
    match metric {
        OverviewMetric::Cuts | OverviewMetric::Effort | OverviewMetric::CriticalPath => {
            styles.accent
        }
        OverviewMetric::Due => styles.warning,
        OverviewMetric::Findings if model.snapshot.overview.errors > 0 => styles.error,
        OverviewMetric::Findings if model.snapshot.overview.warnings > 0 => styles.warning,
        OverviewMetric::Ready | OverviewMetric::Findings => styles.good,
    }
}

fn render_overview(frame: &mut Frame<'_>, model: &Model, area: Rect) {
    let styles = palette(model.theme);
    let viewport = item_viewport(area, model.overview_selected, OverviewMetric::ALL.len());
    let width = usize::from(viewport.content.width);
    let ascii = model.glyphs == Glyphs::Ascii;
    let lines = OverviewMetric::ALL
        .iter()
        .copied()
        .enumerate()
        .skip(viewport.start)
        .take(viewport.end.saturating_sub(viewport.start))
        .map(|(index, metric)| {
            let selected = model.overview_selected == index;
            let text = format!(
                "{}{}",
                if selected { "> " } else { "  " },
                overview_metric_text(model, metric, width.saturating_sub(2))
            );
            Line::styled(
                text::fit(&text, width, ascii),
                if selected {
                    styles.selected
                } else {
                    overview_metric_style(model, metric, styles)
                },
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines).style(styles.normal).block(block(
            model,
            primary_title(model),
            model.focus == Focus::Primary,
        )),
        area,
    );
    if viewport.content.height > 0 && viewport.start > 0 {
        frame.render_widget(
            Paragraph::new(if ascii { "^" } else { "\u{2191}" }).style(styles.warning),
            Rect::new(
                viewport.content.right().saturating_sub(1),
                viewport.content.y,
                1,
                1,
            ),
        );
    }
    if viewport.content.height > 0 && viewport.end < OverviewMetric::ALL.len() {
        frame.render_widget(
            Paragraph::new(if ascii { ">" } else { "\u{2026}" }).style(styles.warning),
            Rect::new(
                viewport.content.right().saturating_sub(1),
                viewport.content.bottom().saturating_sub(1),
                1,
                1,
            ),
        );
    }
}

fn render_diagnostics(frame: &mut Frame<'_>, model: &Model, area: Rect) {
    let styles = palette(model.theme);
    let run = model.diagnostic_run();
    let findings = model.diagnostic_findings();
    let viewport = item_viewport(area, model.selected, findings.len());
    let width = usize::from(viewport.content.width);
    let ascii = model.glyphs == Glyphs::Ascii;
    let lines = diagnostic_lines(
        model,
        &findings,
        viewport.start,
        viewport.end,
        width,
        styles,
        ascii,
    );
    let title = diagnostic_title(model, run, area.width, ascii);
    frame.render_widget(
        Paragraph::new(lines).style(styles.normal).block(block(
            model,
            title,
            model.focus == Focus::Primary,
        )),
        area,
    );
    if viewport.content.height > 0 && viewport.start > 0 {
        frame.render_widget(
            Paragraph::new(if ascii { "^" } else { "\u{2191}" }).style(styles.warning),
            Rect::new(
                viewport.content.right().saturating_sub(1),
                viewport.content.y,
                1,
                1,
            ),
        );
    }
    if viewport.content.height > 0 && viewport.end < findings.len() {
        frame.render_widget(
            Paragraph::new(if ascii { ">" } else { "\u{2026}" }).style(styles.warning),
            Rect::new(
                viewport.content.right().saturating_sub(1),
                viewport.content.bottom().saturating_sub(1),
                1,
                1,
            ),
        );
    }
}

fn diagnostic_lines(
    model: &Model,
    findings: &[&aplan_app::OverviewFinding],
    start: usize,
    end: usize,
    width: usize,
    styles: Palette,
    ascii: bool,
) -> Vec<Line<'static>> {
    if findings.is_empty() {
        let run = model.diagnostic_run();
        if model.has_diagnostic_narrowing() && !run.findings.is_empty() {
            return vec![
                Line::styled(
                    text::fit(
                        "No findings match the active search or filters.",
                        width,
                        ascii,
                    ),
                    styles.warning,
                ),
                Line::styled(
                    text::fit(
                        &format!(
                            "Whole run remains {}: E{} W{}.",
                            diagnostic_outcome(run),
                            run.errors,
                            run.warnings
                        ),
                        width,
                        ascii,
                    ),
                    if run.denied {
                        styles.error
                    } else {
                        styles.warning
                    },
                ),
            ];
        }
        let (message, style) = match &run.coverage {
            aplan_app::DiagnosticCoverage::Complete { subjects } => (
                format!("Clean complete run across {subjects} subject(s)."),
                styles.good,
            ),
            aplan_app::DiagnosticCoverage::NoSubjects(reason)
            | aplan_app::DiagnosticCoverage::Unavailable(reason) => {
                (format!("Not a clean verdict: {reason}."), styles.warning)
            }
            aplan_app::DiagnosticCoverage::Partial {
                subjects,
                unavailable,
            } => (
                format!("Partial run: {subjects} subject(s) checked, {unavailable} unavailable."),
                styles.error,
            ),
        };
        return vec![Line::styled(text::fit(&message, width, ascii), style)];
    }
    findings
        .iter()
        .enumerate()
        .skip(start)
        .take(end.saturating_sub(start))
        .map(|(index, finding)| {
            let selected = model.selected == index;
            let group = diagnostic_group_value(model.diagnostic_group, finding);
            let previous_group = index.checked_sub(1).and_then(|previous| {
                findings
                    .get(previous)
                    .map(|finding| diagnostic_group_value(model.diagnostic_group, finding))
            });
            let group_prefix = if model.diagnostic_group == DiagnosticGroup::Canonical
                || (index != start && previous_group.as_deref() == Some(group.as_str()))
            {
                String::new()
            } else {
                format!("[{group}] ")
            };
            let value = format!(
                "{}{}{} {} {}: {}",
                if selected { "> " } else { "  " },
                group_prefix,
                finding.severity,
                finding.code,
                finding.subject,
                finding.message
            );
            Line::styled(
                text::fit(&value, width, ascii),
                if selected {
                    styles.selected
                } else if finding.severity == aplan_app::Severity::Error {
                    styles.error
                } else {
                    styles.warning
                },
            )
        })
        .collect()
}

fn diagnostic_title(
    model: &Model,
    run: &aplan_app::DiagnosticRun,
    area_width: u16,
    ascii: bool,
) -> String {
    let narrowed = model.has_diagnostic_narrowing();
    let relaxed = model.diagnostic_scope == aplan_app::DiagnosticScope::Check
        && model.snapshot.diagnostics.run_sheet.relaxed;
    let outcome = diagnostic_outcome(run);
    text::fit(
        &format!(
            " {}{}{} {} E{} W{} ",
            run.scope.label(),
            if narrowed { " | FILTERED" } else { "" },
            if relaxed { " | ONE-RUN RELAXED" } else { "" },
            outcome,
            run.errors,
            run.warnings,
        ),
        usize::from(area_width.saturating_sub(2)),
        ascii,
    )
}

fn diagnostic_outcome(run: &aplan_app::DiagnosticRun) -> String {
    match &run.coverage {
        aplan_app::DiagnosticCoverage::Complete { .. } => {
            if run.denied { "DENIED" } else { "PASS" }.to_string()
        }
        aplan_app::DiagnosticCoverage::Partial { .. } => {
            format!("PARTIAL{}", if run.denied { "/DENIED" } else { "" })
        }
        aplan_app::DiagnosticCoverage::NoSubjects(_) => {
            format!("NO-SUBJECTS{}", if run.denied { "/REFUSED" } else { "" })
        }
        aplan_app::DiagnosticCoverage::Unavailable(_) => {
            format!("UNAVAILABLE{}", if run.denied { "/REFUSED" } else { "" })
        }
    }
}

fn diagnostic_group_value(group: DiagnosticGroup, finding: &aplan_app::OverviewFinding) -> String {
    match group {
        DiagnosticGroup::Canonical => String::new(),
        DiagnosticGroup::Severity => finding.severity.to_string(),
        DiagnosticGroup::Rule => finding.code.clone(),
        DiagnosticGroup::Task => finding
            .task
            .clone()
            .unwrap_or_else(|| "whole plan".to_string()),
        DiagnosticGroup::File => finding
            .file
            .as_ref()
            .map_or_else(|| "no file".to_string(), |path| path.display().to_string()),
    }
}

impl Model {
    fn route_question(&self) -> &'static str {
        match self.route {
            Route::Overview => "where attention goes next",
            Route::Tasks => "what is true about this work",
            Route::Diagnostics => "why the plan warns or fails",
            Route::Graph => "how work is connected",
            Route::Reports => "which aggregate needs a decision",
            Route::Inbox => "which captured intake needs review",
        }
    }
}

fn inbox_route_intro(model: &Model, styles: Palette, width: usize) -> Vec<Line<'static>> {
    let (loaded, total, deferred, truncated) =
        model
            .snapshot
            .inbox
            .as_ref()
            .map_or((0, 0, 0, false), |workspace| {
                (
                    workspace.captures.len(),
                    workspace.total(),
                    workspace.deferred_open,
                    workspace.is_truncated(),
                )
            });
    vec![
        Line::styled(
            text::fit("CAPTURE INBOX", width, model.glyphs == Glyphs::Ascii),
            styles.accent,
        ),
        Line::from(text::fit(
            &format!(
                "{loaded}/{total} retained record(s) loaded{}; {deferred} deferred open; {}/{} shown.",
                if truncated { " (bounded)" } else { "" },
                model.inbox_rows().len(),
                loaded,
            ),
            width,
            model.glyphs == Glyphs::Ascii,
        )),
    ]
}

fn route_intro(model: &Model, styles: Palette, compact: bool, width: usize) -> Vec<Line<'static>> {
    let ascii = model.glyphs == Glyphs::Ascii;
    let line = |text: String| Line::from(text::fit(&text, width, ascii));
    let heading = |text: &'static str| Line::styled(text::fit(text, width, ascii), styles.accent);
    let lines = match model.route {
        Route::Overview => vec![
            heading("PLAN OVERVIEW"),
            line(format!(
                "{}; {} need attention.",
                counted(model.snapshot.overview.loaded_tasks, "task"),
                counted(
                    model.snapshot.overview.errors + model.snapshot.overview.warnings,
                    "finding"
                )
            )),
        ],
        Route::Tasks => {
            let mut narrowing = Vec::new();
            if !model.task_query.is_empty() {
                narrowing.push(format!("search `{}`", model.task_query));
            }
            narrowing.extend(model.task_browser.active_filters.iter().cloned());
            let narrowed = if narrowing.is_empty() {
                "all tasks".to_string()
            } else {
                narrowing.join("; ")
            };
            vec![
                heading("TASK BROWSER"),
                line(format!(
                    "{}; {}/{} shown; sort: {}.",
                    narrowed,
                    model.task_browser.rows.len(),
                    model.task_browser.total,
                    model.task_sort.label()
                )),
            ]
        }
        Route::Diagnostics => {
            let run = model.diagnostic_run();
            vec![
                heading("DIAGNOSTIC WORKBENCH"),
                line(format!(
                    "{}: ERROR {} WARN {}; {}/{} shown; group: {}. Whole-run verdict retained.",
                    run.scope.label(),
                    run.errors,
                    run.warnings,
                    model.diagnostic_findings().len(),
                    run.findings.len(),
                    model.diagnostic_group.label(),
                )),
            ]
        }
        Route::Graph => vec![
            heading("DEPENDENCY EXPLORER"),
            line("Layered node-link geometry; edges run beneath semantic cards.".to_string()),
        ],
        Route::Reports => vec![
            heading("REPORTS"),
            line(format!(
                "{} selected; {}/{} row(s) shown{}. Open for provenance and drill-down.",
                model.report_kind.label(),
                model.report_rows().len(),
                model.report_projection.rows.len(),
                if model.report_row_query.is_empty() {
                    String::new()
                } else {
                    format!(" for `{}`", text::safe(&model.report_row_query))
                }
            )),
        ],
        Route::Inbox => inbox_route_intro(model, styles, width),
    };
    if compact && model.route == Route::Tasks {
        lines.into_iter().skip(1).take(1).collect()
    } else if compact {
        lines.into_iter().take(1).collect()
    } else {
        lines
    }
}

fn inspector_lines(
    model: &Model,
    width: usize,
    ascii: bool,
    styles: Palette,
) -> (Vec<Line<'static>>, Option<(usize, usize)>) {
    let mut lines = Vec::new();
    let tabs = InspectorSection::ALL
        .iter()
        .map(|section| {
            if *section == model.inspector_section {
                format!("[{}]", section.label())
            } else {
                section.label().to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    lines.push(Line::styled(text::fit(&tabs, width, ascii), styles.accent));
    if model.route == Route::Graph {
        if let Some(selection) = &model.graph_explorer.selection {
            lines.push(Line::styled(
                text::fit(
                    &format!(
                        "Graph anchor: {} ({}, presentation only; not plan priority or evidence).",
                        selection.represented_by.id(),
                        graph_widget::anchor_reason(&selection.reason)
                    ),
                    width,
                    ascii,
                ),
                styles.muted,
            ));
        }
        if let Some(reason) = model.graph_selection_exclusion() {
            lines.push(Line::styled(
                text::fit(
                    &format!(
                        "Selected task is outside this graph: {reason}. Press x to reveal it."
                    ),
                    width,
                    ascii,
                ),
                styles.warning,
            ));
        }
    } else if model.selected_task().is_none() {
        lines.push(Line::styled(
            text::fit(
                "Selected task is hidden by search or filters; x reveals it in the list.",
                width,
                ascii,
            ),
            styles.warning,
        ));
    }

    let items = model.inspector_items();
    let label_width = inspector_label_width(&items, width);
    let mut selected_range = None;
    let mut previous_group = None;
    for (index, item) in items.iter().enumerate() {
        if item.group != previous_group {
            if !lines.is_empty() {
                lines.push(Line::raw(""));
            }
            if let Some(group) = item.group {
                lines.push(Line::styled(
                    text::fit(&format!("-- {group} --"), width, ascii),
                    styles.accent.add_modifier(Modifier::UNDERLINED),
                ));
            }
            previous_group = item.group;
        }
        let selected = index == model.inspector_selected;
        let start = lines.len();
        render_inspector_item(
            &mut lines,
            item,
            selected,
            width,
            label_width,
            ascii,
            model,
            styles,
        );
        if selected {
            selected_range = Some((start, lines.len()));
        }
    }
    (lines, selected_range)
}

fn inspector_label_width(items: &[crate::model::InspectorItem], width: usize) -> usize {
    if width < 38 {
        return 0;
    }
    items
        .iter()
        .filter_map(|item| match &item.presentation {
            InspectorPresentation::Labelled { label, .. }
            | InspectorPresentation::Badge { label, .. }
            | InspectorPresentation::Metric { label, .. }
            | InspectorPresentation::Callout { label, .. } => Some(text::cells(label)),
            InspectorPresentation::Identity { .. } | InspectorPresentation::Text(_) => None,
        })
        .max()
        .unwrap_or_default()
        .clamp(8, 18)
        .min(width.saturating_div(3))
}

fn inspector_tone_style(tone: InspectorTone, styles: Palette) -> Style {
    match tone {
        InspectorTone::Normal => styles.normal,
        InspectorTone::Accent => styles.accent,
        InspectorTone::Good => styles.good,
        InspectorTone::Warning => styles.warning,
        InspectorTone::Error => styles.error,
        InspectorTone::Muted => styles.muted,
    }
}

fn inspector_tone_cue(tone: InspectorTone) -> &'static str {
    match tone {
        InspectorTone::Normal => "i",
        InspectorTone::Accent => "*",
        InspectorTone::Good => "+",
        InspectorTone::Warning => "!",
        InspectorTone::Error => "x",
        InspectorTone::Muted => "-",
    }
}

fn inspector_linked_task_style(
    model: &Model,
    item: &crate::model::InspectorItem,
    styles: Palette,
) -> Option<Style> {
    if model.inspector_section != InspectorSection::Relationships {
        return None;
    }
    match &item.link {
        Some(crate::model::InspectorLink::Task(id)) => {
            Some(graph_role_style(model.graph_role(id), styles))
        }
        Some(crate::model::InspectorLink::Section(_)) | None => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn render_inspector_item(
    lines: &mut Vec<Line<'static>>,
    item: &crate::model::InspectorItem,
    selected: bool,
    width: usize,
    label_width: usize,
    ascii: bool,
    model: &Model,
    styles: Palette,
) {
    let marker = if selected { "> " } else { "  " };
    let selected_style = |style| if selected { styles.selected } else { style };
    let link_suffix = if item.link.is_some() { "  [Enter]" } else { "" };
    let linked_task_style = inspector_linked_task_style(model, item, styles);
    match &item.presentation {
        InspectorPresentation::Identity { id, title } => {
            let id = text::safe(id);
            let first_budget = width.saturating_sub(2);
            lines.push(Line::from(vec![
                Span::styled(marker, selected_style(styles.accent)),
                Span::styled(
                    text::fit(&id, first_budget, ascii),
                    selected_style(styles.accent.add_modifier(Modifier::BOLD)),
                ),
            ]));
            for part in text::wrap(title, width.saturating_sub(4)) {
                lines.push(Line::styled(
                    format!("    {part}"),
                    selected_style(styles.normal.add_modifier(Modifier::BOLD)),
                ));
            }
        }
        InspectorPresentation::Text(value) => {
            let mut value = value.clone();
            value.push_str(link_suffix);
            for (part, value) in text::wrap(&value, width.saturating_sub(2))
                .into_iter()
                .enumerate()
            {
                lines.push(Line::styled(
                    format!("{}{value}", if part == 0 { marker } else { "  " }),
                    selected_style(styles.normal),
                ));
            }
        }
        InspectorPresentation::Labelled { label, value } => render_inspector_value(
            lines,
            marker,
            label,
            value,
            link_suffix,
            selected_style(linked_task_style.unwrap_or(styles.normal)),
            selected_style(styles.muted),
            width,
            label_width,
            false,
        ),
        InspectorPresentation::Badge { label, value, tone } => render_inspector_value(
            lines,
            marker,
            label,
            &format!("[{value}]"),
            link_suffix,
            selected_style(inspector_tone_style(*tone, styles)),
            selected_style(styles.muted),
            width,
            label_width,
            false,
        ),
        InspectorPresentation::Metric {
            label,
            value,
            detail,
        } => {
            let value = detail
                .as_ref()
                .map_or_else(|| value.clone(), |detail| format!("{value} - {detail}"));
            render_inspector_value(
                lines,
                marker,
                label,
                &value,
                link_suffix,
                selected_style(styles.accent),
                selected_style(styles.muted),
                width,
                label_width,
                false,
            );
        }
        InspectorPresentation::Callout { label, value, tone } => {
            let cue = inspector_tone_cue(*tone);
            render_inspector_value(
                lines,
                marker,
                &format!("[{cue}] {label}"),
                value,
                link_suffix,
                selected_style(inspector_tone_style(*tone, styles)),
                selected_style(inspector_tone_style(*tone, styles)),
                width,
                label_width,
                true,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_inspector_value(
    lines: &mut Vec<Line<'static>>,
    marker: &str,
    label: &str,
    value: &str,
    link_suffix: &str,
    value_style: Style,
    label_style: Style,
    width: usize,
    label_width: usize,
    callout: bool,
) {
    let value = format!("{value}{link_suffix}");
    if label_width == 0 || text::cells(&text::safe(label)) > label_width {
        for (part, label) in text::wrap(label, width.saturating_sub(2))
            .into_iter()
            .enumerate()
        {
            lines.push(Line::from(vec![
                Span::styled(
                    if part == 0 { marker } else { "  " }.to_string(),
                    label_style,
                ),
                Span::styled(label, label_style.add_modifier(Modifier::BOLD)),
            ]));
        }
        for part in text::wrap(&value, width.saturating_sub(4)) {
            lines.push(Line::styled(format!("    {part}"), value_style));
        }
        return;
    }

    let value_width = width.saturating_sub(label_width.saturating_add(4));
    let wrapped = text::wrap(&value, value_width);
    for (part, value) in wrapped.into_iter().enumerate() {
        let prefix = if part == 0 { marker } else { "  " };
        let label = if part == 0 {
            let safe = text::fit(label, label_width, true);
            format!("{safe:<label_width$}")
        } else {
            " ".repeat(label_width)
        };
        lines.push(Line::from(vec![
            Span::styled(prefix.to_string(), label_style),
            Span::styled(
                label,
                if callout {
                    label_style.add_modifier(Modifier::BOLD)
                } else {
                    label_style
                },
            ),
            Span::styled("  ", label_style),
            Span::styled(value, value_style),
        ]));
    }
}

fn resolved_inspector_scroll(
    model: &Model,
    line_count: usize,
    selected_range: Option<(usize, usize)>,
    capacity: usize,
) -> (usize, usize) {
    let max_scroll = line_count.saturating_sub(capacity);
    let scroll = model.inspector_scroll.map_or_else(
        || {
            selected_range.map_or(0, |(start, end)| {
                if end.saturating_sub(start) >= capacity {
                    start
                } else {
                    end.saturating_sub(capacity)
                }
            })
        },
        |scroll| scroll.min(max_scroll),
    );
    (scroll, max_scroll)
}

/// Resolve the model's automatic selection-following offset against the exact
/// pane the current responsive layout will paint. Page commands use this same
/// result as rendering, so they cannot accumulate an invisible offset past the
/// end or mistake an explicit top offset for automatic following.
fn inspector_scroll_bounds(model: &Model) -> Option<(usize, usize)> {
    let layout = ShellLayout::new(Rect::new(0, 0, model.columns, model.rows), model);
    let area = match layout.class {
        LayoutClass::Narrow => layout.primary,
        LayoutClass::Normal | LayoutClass::Wide => layout.detail?,
        LayoutClass::TooSmall => return None,
    };
    let content = area.inner(Margin::new(1, 1));
    if model.route == Route::Overview {
        let maximum = 8usize.saturating_sub(usize::from(content.height));
        return Some((model.inspector_scroll.unwrap_or(0).min(maximum), maximum));
    }
    if model.route == Route::Diagnostics {
        let lines =
            diagnostic_detail_lines(model, usize::from(content.width), palette(model.theme));
        return Some(resolved_inspector_scroll(
            model,
            lines.len(),
            None,
            usize::from(content.height),
        ));
    }
    if model.route == Route::Inbox {
        let lines = inbox_detail_lines(model, usize::from(content.width), palette(model.theme));
        return Some(resolved_inspector_scroll(
            model,
            lines.len(),
            None,
            usize::from(content.height),
        ));
    }
    let (lines, selected_range) = inspector_lines(
        model,
        usize::from(content.width),
        model.glyphs == Glyphs::Ascii,
        palette(model.theme),
    );
    Some(resolved_inspector_scroll(
        model,
        lines.len(),
        selected_range,
        usize::from(content.height),
    ))
}

fn inspector_scroll_target(model: &Model, forward: bool) -> Option<usize> {
    let (current, maximum) = inspector_scroll_bounds(model)?;
    Some(if forward {
        current.saturating_add(5).min(maximum)
    } else {
        current.saturating_sub(5)
    })
}

fn report_value_summary(model: &Model) -> String {
    let Some(body) = &model.report_projection.body else {
        return match &model.report_projection.state {
            aplan_app::ReportState::Ready => "ready".to_string(),
            aplan_app::ReportState::Unavailable(reason) => format!("unavailable: {reason}"),
            aplan_app::ReportState::Invalid(reason) => format!("invalid parameters: {reason}"),
        };
    };
    match body {
        aplan_app::ReportBody::Roadmap(report) => format!(
            "{} cut(s); {:.1}d total; throughput {}",
            report.cuts.len(),
            report.total_effort_days(),
            report.effort_days_per_week.map_or_else(
                || "not declared".to_string(),
                |days| format!("{days:.1}d/week")
            )
        ),
        aplan_app::ReportBody::Summary(summary) => {
            let (tasks, done, effort, ready, weeks, remaining) = match summary {
                aplan_app::SummaryReport::Phases(report) => (
                    report.total_tasks(),
                    report.total_done(),
                    report.total_effort_days(),
                    report.ready,
                    report.total_weeks(),
                    report.remaining_weeks(),
                ),
                aplan_app::SummaryReport::Groups(report) => (
                    report.total_tasks(),
                    report.total_done(),
                    report.total_effort_days(),
                    report.ready(),
                    report.total_weeks(),
                    report.remaining_weeks(),
                ),
            };
            format!(
                "{done}/{tasks} done; {ready} ready; {effort:.1}d{}",
                weeks.map_or_else(String::new, |all| format!(
                    " / {all:.1}w ({:.1}w left)",
                    remaining.unwrap_or_default()
                ))
            )
        }
        aplan_app::ReportBody::Version(report) => match &report.current_source {
            aplan_app::VersionCurrentSource::Configured(command) => {
                format!("current source argv: {command:?}")
            }
            aplan_app::VersionCurrentSource::Missing => {
                "current source missing; showing plan-only owed impacts".to_string()
            }
        },
        aplan_app::ReportBody::Paths(report) => format!(
            "sampled {}/{} commits; {} measured row(s); {} without paths; {} unmeasurable",
            report.sampled,
            report.asked_for,
            report.tasks.len(),
            report.without_paths,
            report.unmeasurable
        ),
        aplan_app::ReportBody::Vacuity(report) => {
            format!(
                "{} rule reach row(s); zero subjects is evidence, not a pass",
                report.len()
            )
        }
        aplan_app::ReportBody::Stale(report) => format!(
            "advisory; older than {}d; as of {}; {} stale, {} fresh, {} untracked",
            report.window_days,
            report.as_of.as_deref().unwrap_or("HEAD commit time"),
            report.tasks.len(),
            report.fresh,
            report.untracked
        ),
        aplan_app::ReportBody::Deprecations(report) => format!(
            "{} runway(s); {} unannounced removal(s)",
            report.runways.len(),
            report.unannounced.len()
        ),
    }
}

fn render_report_detail(frame: &mut Frame<'_>, model: &Model, area: Rect) {
    let styles = palette(model.theme);
    let content = area.inner(Margin::new(1, 1));
    let width = usize::from(content.width);
    let ascii = model.glyphs == Glyphs::Ascii;
    let mut lines = text::wrap(&report_value_summary(model), width)
        .into_iter()
        .map(|line| Line::styled(line, styles.accent))
        .collect::<Vec<_>>();
    let scope = format!(
        "{}{}",
        model.report_projection.scope,
        if model.report_projection.advisory {
            "; advisory - never a verdict"
        } else {
            ""
        }
    );
    lines.extend(
        text::wrap(&scope, width)
            .into_iter()
            .map(|line| Line::styled(line, styles.muted)),
    );
    let rows = model.report_rows();
    if rows.is_empty() {
        let empty =
            if !model.report_row_query.is_empty() && !model.report_projection.rows.is_empty() {
                "No report row matches the search; the report result itself is not empty."
            } else if matches!(model.report_projection.state, aplan_app::ReportState::Ready) {
                "No rows. This is an empty result, not an unavailable measurement."
            } else {
                "No report rows are available."
            };
        lines.extend(
            text::wrap(empty, width)
                .into_iter()
                .map(|line| Line::styled(line, styles.muted)),
        );
    } else {
        let selected_row = &rows[model.report_selected_row.min(rows.len() - 1)];
        lines.extend(
            text::wrap(
                &format!("Selected {}: {}", selected_row.label, selected_row.summary),
                width,
            )
            .into_iter()
            .map(|line| Line::styled(line, styles.accent)),
        );
        let capacity = usize::from(content.height).saturating_sub(lines.len());
        let start = model
            .report_selected_row
            .saturating_sub(capacity.saturating_sub(1) / 2)
            .min(rows.len().saturating_sub(capacity));
        lines.extend(
            rows.iter()
                .enumerate()
                .skip(start)
                .take(capacity)
                .map(|(index, row)| {
                    let selected = index == model.report_selected_row;
                    let provenance = match row.provenance {
                        aplan_app::ReportRowProvenance::Tasks => "[Enter: tasks]",
                        aplan_app::ReportRowProvenance::Configuration => "[configuration]",
                        aplan_app::ReportRowProvenance::NoSubjects => "[no subjects]",
                    };
                    Line::styled(
                        text::fit(
                            &format!(
                                "{}{provenance} {}",
                                if selected { "> " } else { "  " },
                                row.label
                            ),
                            width,
                            ascii,
                        ),
                        if selected {
                            styles.selected
                        } else {
                            styles.normal
                        },
                    )
                }),
        );
    }
    frame.render_widget(
        Paragraph::new(lines).style(styles.normal).block(block(
            model,
            detail_title(
                model,
                area,
                &format!("{} report", model.report_kind.label()),
            ),
            model.focus == Focus::Detail,
        )),
        area,
    );
}

fn render_detail(frame: &mut Frame<'_>, model: &Model, area: Rect) {
    if model.route == Route::Overview {
        render_overview_identity(frame, model, area);
        return;
    }
    if model.route == Route::Diagnostics {
        render_diagnostic_detail(frame, model, area);
        return;
    }
    if model.route == Route::Reports {
        render_report_detail(frame, model, area);
        return;
    }
    if model.route == Route::Inbox {
        render_inbox_detail(frame, model, area);
        return;
    }
    let styles = palette(model.theme);
    let content = area.inner(Margin::new(1, 1));
    let width = usize::from(content.width);
    let ascii = model.glyphs == Glyphs::Ascii;
    let Some(show) = model.selected_inspector() else {
        let message = model.selected_id.as_ref().map_or_else(
            || "No task selected.".to_string(),
            |id| format!("Selected task {id} was removed during refresh."),
        );
        frame.render_widget(
            Paragraph::new(Line::styled(
                text::fit(&message, width, ascii),
                styles.muted,
            ))
            .style(styles.normal)
            .block(block(
                model,
                detail_title(model, area, "Task inspector"),
                model.focus == Focus::Detail,
            )),
            area,
        );
        return;
    };

    let (lines, selected_range) = inspector_lines(model, width, ascii, styles);

    let capacity = usize::from(content.height);
    let (scroll, _max_scroll) =
        resolved_inspector_scroll(model, lines.len(), selected_range, capacity);
    let title = detail_title(
        model,
        area,
        &format!("{} / {}", show.id, model.inspector_section.label()),
    );
    let line_count = lines.len();
    let visible_lines = lines
        .into_iter()
        .skip(scroll)
        .take(capacity)
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(visible_lines)
            .style(styles.normal)
            .block(block(model, &title, model.focus == Focus::Detail)),
        area,
    );
    if capacity > 0 && scroll > 0 {
        let marker = if ascii { "^" } else { "\u{2191}" };
        frame.render_widget(
            Paragraph::new(marker).style(styles.warning),
            Rect::new(content.right().saturating_sub(1), content.y, 1, 1),
        );
    }
    if capacity > 0 && scroll.saturating_add(capacity) < line_count {
        // The exact wrapped-line count is `lines.len()` above; keep overflow
        // visible even when a narrow pane has room for only one content row.
        let marker = if ascii { ">" } else { "\u{2026}" };
        frame.render_widget(
            Paragraph::new(marker).style(styles.warning),
            Rect::new(
                content.right().saturating_sub(1),
                content.bottom().saturating_sub(1),
                1,
                1,
            ),
        );
    }
}

fn diagnostic_detail_lines(model: &Model, width: usize, styles: Palette) -> Vec<Line<'static>> {
    let Some(finding) = model.selected_finding() else {
        return vec![Line::styled("No finding selected.", styles.muted)];
    };
    let location = diagnostic_location(finding);
    let mut lines = vec![
        diagnostic_context_line(model, width, styles),
        Line::styled(
            text::fit(
                &format!("{} {}", finding.severity, finding.code),
                width,
                model.glyphs == Glyphs::Ascii,
            ),
            if finding.severity == aplan_app::Severity::Error {
                styles.error
            } else {
                styles.warning
            },
        ),
        Line::from(text::fit(
            &format!("scope: {}", finding.scope.label()),
            width,
            model.glyphs == Glyphs::Ascii,
        )),
        Line::from(text::fit(
            &format!("subject: {}", finding.subject),
            width,
            model.glyphs == Glyphs::Ascii,
        )),
        Line::from(text::fit(
            &format!("location: {location}"),
            width,
            model.glyphs == Glyphs::Ascii,
        )),
        Line::from(text::fit(
            &format!("verdict: {}", finding.contribution.label()),
            width,
            model.glyphs == Glyphs::Ascii,
        )),
    ];
    lines.extend(
        text::wrap(&format!("message: {}", finding.message), width)
            .into_iter()
            .map(Line::from),
    );
    if let Some(instruction) = &finding.agent_instruction {
        lines.extend(
            text::wrap(&format!("agent instruction: {instruction}"), width)
                .into_iter()
                .map(Line::from),
        );
    } else {
        lines.push(Line::styled("agent instruction: none", styles.muted));
    }
    for detail in &finding.detail {
        lines.extend(
            text::wrap(&format!("detail: {detail}"), width)
                .into_iter()
                .map(Line::from),
        );
    }
    if let Some(source_line) = &finding.source_line {
        lines.extend(
            text::wrap(&format!("source: {source_line}"), width)
                .into_iter()
                .map(|line| Line::styled(line, styles.muted)),
        );
    }
    if let Ok(explanation) = aplan_app::explain_rule(&finding.code) {
        lines.extend(
            text::wrap(
                &format!(
                    "rule: {} {} - {} (press e for explanation)",
                    explanation.severity, explanation.code, explanation.title
                ),
                width,
            )
            .into_iter()
            .map(|line| Line::styled(line, styles.accent)),
        );
    }
    if finding.task.is_some() {
        lines.push(Line::styled(
            "o: open task; Esc returns here",
            styles.accent,
        ));
    }
    if model.diagnostic_scope == aplan_app::DiagnosticScope::Check {
        lines.push(diagnostic_run_line(model, width, styles));
    }
    lines
}

fn diagnostic_context_line(model: &Model, width: usize, styles: Palette) -> Line<'static> {
    let run = model.diagnostic_run();
    let narrowed = model.has_diagnostic_narrowing();
    let relaxed = model.diagnostic_scope == aplan_app::DiagnosticScope::Check
        && model.snapshot.diagnostics.run_sheet.relaxed;
    Line::styled(
        text::fit(
            &format!(
                "{}{}{} {} E{} W{}",
                run.scope.label(),
                if narrowed { " | FILTERED" } else { "" },
                if relaxed { " | ONE-RUN RELAXED" } else { "" },
                diagnostic_outcome(run),
                run.errors,
                run.warnings,
            ),
            width,
            model.glyphs == Glyphs::Ascii,
        ),
        if run.denied {
            styles.error
        } else if narrowed || relaxed {
            styles.warning
        } else {
            styles.muted
        },
    )
}

fn diagnostic_location(finding: &aplan_app::OverviewFinding) -> String {
    finding.file.as_ref().map_or_else(
        || "no source location".to_string(),
        |file| {
            finding.line.map_or_else(
                || file.display().to_string(),
                |line| {
                    finding.column.map_or_else(
                        || format!("{}:{line}", file.display()),
                        |column| format!("{}:{line}:{column}", file.display()),
                    )
                },
            )
        },
    )
}

fn diagnostic_run_line(model: &Model, width: usize, styles: Palette) -> Line<'static> {
    let sheet = &model.snapshot.diagnostics.run_sheet;
    let traces = sheet
        .effective
        .traces_root
        .as_ref()
        .map_or_else(|| "disabled".to_string(), |path| path.display().to_string());
    let value = format!(
        "run:{} effort={} traces={} warnings={}",
        if sheet.relaxed {
            " [ONE-RUN RELAXED]"
        } else {
            ""
        },
        if sheet.effective.require_effort {
            "required"
        } else {
            "optional"
        },
        traces,
        if sheet.effective.deny_warnings {
            "deny"
        } else {
            "allow"
        },
    );
    Line::styled(
        text::fit(&value, width, model.glyphs == Glyphs::Ascii),
        if sheet.relaxed {
            styles.warning
        } else {
            styles.muted
        },
    )
}

fn render_diagnostic_detail(frame: &mut Frame<'_>, model: &Model, area: Rect) {
    let styles = palette(model.theme);
    let content = area.inner(Margin::new(1, 1));
    let lines = diagnostic_detail_lines(model, usize::from(content.width), styles);
    let capacity = usize::from(content.height);
    let (scroll, _) = resolved_inspector_scroll(model, lines.len(), None, capacity);
    let line_count = lines.len();
    let relaxed = model.diagnostic_scope == aplan_app::DiagnosticScope::Check
        && model.snapshot.diagnostics.run_sheet.relaxed;
    let title = model.selected_finding().map_or_else(
        || detail_title(model, area, "Finding detail"),
        |finding| {
            detail_title(
                model,
                area,
                &format!(
                    "Finding{} {}",
                    if relaxed { " [ONE-RUN RELAXED]" } else { "" },
                    finding.code
                ),
            )
        },
    );
    frame.render_widget(
        Paragraph::new(
            lines
                .into_iter()
                .skip(scroll)
                .take(capacity)
                .collect::<Vec<_>>(),
        )
        .style(styles.normal)
        .block(block(model, &title, model.focus == Focus::Detail)),
        area,
    );
    if capacity > 0 && scroll > 0 {
        frame.render_widget(
            Paragraph::new(if model.glyphs == Glyphs::Ascii {
                "^"
            } else {
                "\u{2191}"
            })
            .style(styles.warning),
            Rect::new(content.right().saturating_sub(1), content.y, 1, 1),
        );
    }
    if capacity > 0 && scroll.saturating_add(capacity) < line_count {
        frame.render_widget(
            Paragraph::new(if model.glyphs == Glyphs::Ascii {
                ">"
            } else {
                "\u{2026}"
            })
            .style(styles.warning),
            Rect::new(
                content.right().saturating_sub(1),
                content.bottom().saturating_sub(1),
                1,
                1,
            ),
        );
    }
}

fn short_identity(value: &str) -> String {
    if value.is_empty() {
        "unavailable".to_string()
    } else {
        value.chars().take(12).collect()
    }
}

fn repository_identity_label(identity: &aplan_app::RepositoryIdentity) -> String {
    match identity {
        aplan_app::RepositoryIdentity::NotObserved { reason } => {
            format!("not observed: {reason}")
        }
        aplan_app::RepositoryIdentity::Unavailable { reason } => {
            format!("unavailable: {reason}")
        }
        aplan_app::RepositoryIdentity::Observed {
            head,
            modified,
            untracked,
            status_sha256,
        } => format!(
            "HEAD:{} {}; status:{}",
            head.as_deref()
                .map_or_else(|| "unborn".to_string(), short_identity),
            if *modified == 0 && *untracked == 0 {
                "clean".to_string()
            } else {
                format!("dirty {modified} modified, {untracked} untracked")
            },
            short_identity(status_sha256)
        ),
    }
}

fn overview_identity_lines(model: &Model, width: usize, ascii: bool) -> Vec<Line<'static>> {
    let overview = &model.snapshot.overview;
    let config = overview.input.configuration.path.as_ref().map_or_else(
        || "none selected".to_string(),
        |path| path.display().to_string(),
    );
    let repository = repository_identity_label(&overview.input.repository);
    let load = match overview.load_state {
        aplan_app::OverviewLoadState::Complete => "complete".to_string(),
        aplan_app::OverviewLoadState::Empty => "empty plan".to_string(),
        aplan_app::OverviewLoadState::Malformed => "malformed plan".to_string(),
        aplan_app::OverviewLoadState::PartiallyUnloadable => format!(
            "partially unloadable ({} file(s))",
            overview.unloadable.len()
        ),
    };
    let work = match overview.work_state {
        aplan_app::OverviewWorkState::Active => "active work",
        aplan_app::OverviewWorkState::AllDone => "all work done",
        aplan_app::OverviewWorkState::OnlyDropped => "only dropped work",
        aplan_app::OverviewWorkState::NoLoadedWork => "no loaded work",
    };
    let effort = match overview.effort_state {
        aplan_app::OverviewEffortState::Complete => "complete",
        aplan_app::OverviewEffortState::Partial => "partial",
        aplan_app::OverviewEffortState::None => "none estimated",
        aplan_app::OverviewEffortState::NoLiveWork => "no live work",
    };
    let freshness = match model.freshness {
        SnapshotFreshness::Current => "current",
        SnapshotFreshness::Refreshing => "refreshing (stale possible)",
        SnapshotFreshness::Unconfirmed => "unconfirmed (stale possible)",
    };
    let source = if model.snapshot.build.source.is_empty() {
        "source unavailable".to_string()
    } else {
        short_identity(&model.snapshot.build.source)
    };
    let mut lines = vec![
        format!(
            "plan: {} sha:{}",
            overview.input.plan_root.display(),
            short_identity(&overview.input.plan_sha256)
        ),
        format!(
            "config: {config} sha:{}",
            short_identity(&overview.input.configuration.sha256)
        ),
    ];
    if let Some(resolution) = &model.launch_resolution {
        lines.push(format!(
            "root source: {}",
            resolution.root_selection.label()
        ));
        lines.push(format!(
            "configuration source: {}",
            resolution.configuration.trust_label()
        ));
    }
    lines.extend([
        format!("repository: {}", overview.input.repository_root.display()),
        format!("repository state: {repository}"),
        format!("build: {} src:{source}", model.snapshot.build.version),
        format!(
            "facts:{} binary:{}",
            short_identity(&model.snapshot.build.facts_sha256),
            short_identity(&model.snapshot.build.binary_sha256)
        ),
        format!(
            "input: {} {} {:?}",
            short_identity(&overview.input.sha256),
            overview.input.source,
            overview.input.trust
        ),
        format!("snapshot: {freshness}; memory dirty: no"),
        format!("load: {load}"),
        format!("work: {work}; effort: {effort}"),
    ]);
    lines
        .into_iter()
        .map(|line| Line::from(text::fit(&line, width, ascii)))
        .collect()
}

fn render_overview_identity(frame: &mut Frame<'_>, model: &Model, area: Rect) {
    let styles = palette(model.theme);
    let ascii = model.glyphs == Glyphs::Ascii;
    let width = usize::from(area.inner(Margin::new(1, 1)).width);
    let lines = overview_identity_lines(model, width, ascii);
    let capacity = usize::from(area.inner(Margin::new(1, 1)).height);
    let maximum = lines.len().saturating_sub(capacity);
    let scroll = model.inspector_scroll.unwrap_or(0).min(maximum);
    frame.render_widget(
        Paragraph::new(
            lines
                .into_iter()
                .skip(scroll)
                .take(capacity)
                .collect::<Vec<_>>(),
        )
        .style(styles.normal)
        .block(block(
            model,
            detail_title(model, area, "Snapshot identity"),
            model.focus == Focus::Detail,
        )),
        area,
    );
    let content = area.inner(Margin::new(1, 1));
    if capacity > 0 && scroll > 0 {
        frame.render_widget(
            Paragraph::new(if ascii { "^" } else { "\u{2191}" }).style(styles.warning),
            Rect::new(content.right().saturating_sub(1), content.y, 1, 1),
        );
    }
    if capacity > 0 && scroll < maximum {
        frame.render_widget(
            Paragraph::new(if ascii { ">" } else { "\u{2026}" }).style(styles.warning),
            Rect::new(
                content.right().saturating_sub(1),
                content.bottom().saturating_sub(1),
                1,
                1,
            ),
        );
    }
}

fn footer_states(model: &Model) -> Vec<PaletteState> {
    let mut states = action_states(model)
        .into_iter()
        .filter(|state| {
            matches!(state.target, PaletteTarget::Command(id) if id.footer())
                && state.unavailable.is_none()
        })
        .collect::<Vec<_>>();
    states.sort_by_key(|state| match state.target {
        PaletteTarget::Command(id) => footer_priority(model, id),
        PaletteTarget::Mutation(_) | PaletteTarget::Boundary(_) => u8::MAX,
    });
    states
}

const fn footer_priority(model: &Model, id: CommandId) -> u8 {
    if model.onboarding.is_some() {
        return match id {
            CommandId::Back => 0,
            CommandId::Refresh => 1,
            CommandId::Palette => 2,
            CommandId::Help => 3,
            CommandId::ScrollDown | CommandId::ScrollUp => 4,
            CommandId::About => 5,
            CommandId::Quit => 6,
            _ => 7,
        };
    }
    match id {
        CommandId::Back => 0,
        CommandId::Palette => 1,
        CommandId::Help => 2,
        CommandId::Open => 3,
        CommandId::Search
        | CommandId::Filters
        | CommandId::Sort
        | CommandId::WhyTraversal
        | CommandId::GraphOpenContinuation
        | CommandId::OpenFindingTask => 4,
        CommandId::Quit => 5,
        _ => 6,
    }
}

fn compact_command_label(id: CommandId, model: &Model) -> String {
    match id {
        CommandId::Back if model.focus == Focus::Detail && model.inspector_has_history() => {
            "back".to_string()
        }
        CommandId::Back if model.focus == Focus::Detail => match model.route {
            Route::Overview => "metrics",
            Route::Tasks => "tasks",
            Route::Diagnostics => "findings",
            Route::Graph => "graph",
            Route::Reports => "reports",
            Route::Inbox => "inbox",
        }
        .to_string(),
        CommandId::Back
            if model.route == Route::Graph
                && model.focus == Focus::Primary
                && model.has_graph_window_history() =>
        {
            "prior".to_string()
        }
        CommandId::Back if model.route != Route::Overview => "overview".to_string(),
        CommandId::Back | CommandId::Quit => "quit".to_string(),
        CommandId::Open => "open".to_string(),
        CommandId::Palette => "commands".to_string(),
        CommandId::Help => "help".to_string(),
        CommandId::Search => "search".to_string(),
        CommandId::Filters => "filters".to_string(),
        CommandId::Sort => "sort".to_string(),
        CommandId::WhyTraversal => "why".to_string(),
        CommandId::FocusNext => "focus".to_string(),
        CommandId::PreviousSection | CommandId::NextSection => "section".to_string(),
        CommandId::ScrollUp if model.onboarding.is_some() => "earlier".to_string(),
        CommandId::ScrollDown if model.onboarding.is_some() => "more".to_string(),
        CommandId::ScrollUp | CommandId::ScrollDown => "page".to_string(),
        CommandId::About if model.onboarding.is_some() => "About".to_string(),
        CommandId::About => "about".to_string(),
        CommandId::Refresh if model.onboarding.is_some() => "retry".to_string(),
        CommandId::Refresh => "refresh".to_string(),
        CommandId::Verify => "verify".to_string(),
        CommandId::CycleDiagnosticScope | CommandId::GraphScope => "scope".to_string(),
        CommandId::AdvancedCheck => "options".to_string(),
        CommandId::ExplainRule | CommandId::FindRule => "explain".to_string(),
        CommandId::OpenFindingTask => "task".to_string(),
        CommandId::GraphBetween => "between".to_string(),
        CommandId::GraphMembership => "membership".to_string(),
        CommandId::GraphDensity => "density".to_string(),
        CommandId::GraphRelationsOnly => "relations".to_string(),
        CommandId::GraphPreviousContinuation | CommandId::GraphNextContinuation => {
            "continuation".to_string()
        }
        CommandId::GraphOpenContinuation => "continue".to_string(),
        CommandId::GraphContractDepth | CommandId::GraphExpandDepth => "depth".to_string(),
        CommandId::GraphOverview => "overview".to_string(),
        CommandId::GraphFit => "fit".to_string(),
        CommandId::GraphMoveLeft | CommandId::GraphMoveRight => "move".to_string(),
        CommandId::GraphPanUp
        | CommandId::GraphPanDown
        | CommandId::PanLeft
        | CommandId::PanRight => "pan".to_string(),
        CommandId::GraphPreviousLayer | CommandId::GraphNextLayer => "layer".to_string(),
        CommandId::GraphSource => "source".to_string(),
        CommandId::GraphSink => "sink".to_string(),
        CommandId::GraphCritical => "critical".to_string(),
        CommandId::GraphMilestone => "milestone".to_string(),
        CommandId::GraphPreviousPhase | CommandId::GraphNextPhase => "phase".to_string(),
        CommandId::GraphFinding => "finding".to_string(),
        CommandId::ShowInGraph => "graph".to_string(),
        CommandId::RecenterGraph => "center".to_string(),
        CommandId::AsOfDate if model.route == Route::Graph => "direction".to_string(),
        CommandId::AsOfDate => "date".to_string(),
        CommandId::CancelQueued => "cancel".to_string(),
        CommandId::QuickCapture => "capture".to_string(),
        CommandId::Overview
        | CommandId::Tasks
        | CommandId::Diagnostics
        | CommandId::Graph
        | CommandId::Reports
        | CommandId::Inbox
        | CommandId::FocusPrevious
        | CommandId::MoveNext
        | CommandId::MovePrevious
        | CommandId::ClearNarrowing
        | CommandId::CycleTheme
        | CommandId::ToggleGlyphs
        | CommandId::TestCatalogue => id.label(model),
    }
}

fn compact_command_keys(id: CommandId, model: &Model, default: &str) -> String {
    if id == CommandId::Back
        && model.route == Route::Overview
        && model.focus != Focus::Detail
        && !model.has_graph_window_history()
    {
        "Esc/q/C-c".to_string()
    } else {
        default.to_string()
    }
}

fn footer_segments(area: Rect, model: &Model, density: ChromeDensity) -> Vec<(CommandId, String)> {
    let budget = usize::from(match density {
        ChromeDensity::Full => area.width.saturating_sub(2),
        ChromeDensity::Compact => area.width,
    });
    let mut used = 0;
    let mut segments = Vec::new();
    for state in footer_states(model) {
        let PaletteTarget::Command(id) = state.target else {
            unreachable!("footer states contain only shell commands")
        };
        if density == ChromeDensity::Compact && id == CommandId::Quit {
            // `Esc` is always the compact shell's first command and is the
            // contextual quit action at the Overview root. Do not spend the
            // minimum-width budget stating the same exit twice.
            continue;
        }
        let label = if density == ChromeDensity::Compact {
            compact_command_label(id, model)
        } else {
            state.label
        };
        let keys = if density == ChromeDensity::Compact {
            compact_command_keys(id, model, state.keys)
        } else {
            state.keys.to_string()
        };
        let segment = format!("{keys} {label}");
        let width = text::cells(&segment) + usize::from(!segments.is_empty()) * 3;
        if used + width > budget {
            continue;
        }
        used += width;
        segments.push((id, segment));
    }
    segments
}

fn footer_targets(area: Rect, model: &Model, density: ChromeDensity) -> Vec<Target<CommandId>> {
    let inset = u16::from(density == ChromeDensity::Full);
    let mut x = area.x.saturating_add(inset);
    footer_segments(area, model, density)
        .into_iter()
        .map(|(id, segment)| {
            let width = u16::try_from(text::cells(&segment)).unwrap_or(u16::MAX);
            let target = label_target(x, area.y.saturating_add(inset), &segment, id);
            x = x.saturating_add(width).saturating_add(3);
            target
        })
        .collect()
}

fn render_footer(frame: &mut Frame<'_>, model: &Model, layout: &ShellLayout) {
    let styles = palette(model.theme);
    let segments = footer_segments(layout.footer, model, layout.density);
    let mut spans = Vec::new();
    for (index, (_, segment)) in segments.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" | ", styles.muted));
        }
        spans.push(Span::styled(segment, styles.normal));
    }
    let lines = vec![
        Line::from(spans),
        Line::styled(
            text::fit(
                &model.status_line(),
                usize::from(match layout.density {
                    ChromeDensity::Full => layout.footer.width.saturating_sub(2),
                    ChromeDensity::Compact => layout.footer.width,
                }),
                model.glyphs == Glyphs::Ascii,
            ),
            styles.muted,
        ),
    ];
    let footer = Paragraph::new(lines).style(styles.normal);
    if layout.density == ChromeDensity::Compact {
        frame.render_widget(footer, layout.footer);
    } else {
        frame.render_widget(
            footer.block(block(model, " Commands ", false)),
            layout.footer,
        );
    }
}

fn render_too_small(frame: &mut Frame<'_>, model: &Model, area: Rect) {
    let styles = palette(model.theme);
    let lines = vec![
        Line::styled("TERMINAL TOO SMALL", styles.error),
        Line::from(format!(
            "Current: {}x{}. Required: {MIN_COLUMNS}x{MIN_ROWS}.",
            area.width, area.height
        )),
        Line::from("Resize to retry automatically. Press Esc, q or Ctrl-C to quit."),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .style(styles.normal)
            .block(block(model, " aplan ", true))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_onboarding(frame: &mut Frame<'_>, model: &Model, area: Rect) {
    let styles = palette(model.theme);
    let Some(state) = model.onboarding.as_ref() else {
        return;
    };
    let configuration = match &state.configuration {
        aplan_app::ConfigurationSelection::Discover {
            start,
            searched,
            found,
        } => format!(
            "{} from {}; selected {}; searched {} location(s)",
            state.configuration.trust_label(),
            start.display(),
            found
                .as_ref()
                .map_or_else(|| "none".to_string(), |path| path.display().to_string()),
            searched.len()
        ),
        aplan_app::ConfigurationSelection::Explicit { path } => {
            format!("{}: {}", state.configuration.trust_label(), path.display())
        }
        aplan_app::ConfigurationSelection::None => state.configuration.trust_label().to_string(),
    };
    let root_source = match &state.root_selection {
        aplan_app::RootSelection::Configuration { file } => {
            format!("{} ({})", state.root_selection.label(), file.display())
        }
        _ => state.root_selection.label().to_string(),
    };
    let content = area.inner(Margin::new(1, 1));
    let width = usize::from(content.width);
    let mut lines = Vec::new();
    push_wrapped_modal_line(&mut lines, state.cause.label(), width, styles.error);
    lines.push(Line::raw(""));
    push_wrapped_modal_line(
        &mut lines,
        &format!("Plan root: {}", state.plan_root.display()),
        width,
        styles.accent,
    );
    push_wrapped_modal_line(
        &mut lines,
        &format!("Root source: {root_source}"),
        width,
        styles.accent,
    );
    push_wrapped_modal_line(
        &mut lines,
        &format!("Configuration: {configuration}"),
        width,
        styles.accent,
    );
    lines.push(Line::raw(""));
    push_wrapped_modal_line(&mut lines, &state.detail, width, styles.normal);
    if !state.guidance.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::styled("What to do", styles.accent));
        for item in &state.guidance {
            push_wrapped_modal_line(&mut lines, &format!("- {item}"), width, styles.normal);
        }
    }
    if !state.commands.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "Suggested commands (shown only; never run)",
            styles.accent,
        ));
        for command in &state.commands {
            push_wrapped_modal_line(
                &mut lines,
                &format!("- {}", command.purpose),
                width,
                styles.normal,
            );
            push_wrapped_modal_line(
                &mut lines,
                &format!("  {}", command.display),
                width,
                styles.good,
            );
        }
    }
    lines.push(Line::raw(""));
    push_wrapped_modal_line(
        &mut lines,
        "After repairing the input, press r to retry in place. Press i for exact build capabilities.",
        width,
        styles.muted,
    );
    let maximum = lines.len().saturating_sub(usize::from(content.height));
    let scroll = model.onboarding_scroll.min(maximum);
    frame.render_widget(
        Paragraph::new(lines)
            .style(styles.normal)
            .block(block(model, " Setup and repair ", true))
            .scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0)),
        area,
    );
}

fn about_modal(model: &Model, styles: Palette, width: usize) -> Vec<Line<'static>> {
    let view = &model.capabilities;
    let mut lines = Vec::new();
    for value in [
        format!("Version: {}", view.advertised_version),
        format!("Build identity version: {}", view.identity.version),
        format!("Build source: {}", view.identity.source),
        format!("Facts digest: {}", view.identity.facts_sha256),
        format!("Binary digest: {}", view.identity.binary_sha256),
        format!("Facts schema: {}", view.schema_version),
        format!(
            "Exit codes: clean {}, error {}, usage {}, internal {}",
            view.exit_codes.clean,
            view.exit_codes.error,
            view.exit_codes.usage,
            view.exit_codes.internal
        ),
        format!(
            "Counts: {} commands, {} formats, {} rules, {} evidence kinds, {} behaviours, {} MCP tools, {} config keys, {} build capabilities",
            view.counts.commands,
            view.counts.formats,
            view.counts.rules,
            view.counts.evidence_kinds,
            view.counts.behaviours,
            view.counts.mcp_tools,
            view.counts.config_keys,
            view.counts.build_capabilities,
        ),
    ] {
        push_wrapped_modal_line(&mut lines, &value, width, styles.normal);
    }
    for group in &view.groups {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            format!("{} ({})", group.section.label(), group.items.len()),
            styles.accent,
        ));
        for item in &group.items {
            let value = item.detail.as_ref().map_or_else(
                || format!("- {}", item.name),
                |detail| format!("- {}: {detail}", item.name),
            );
            push_wrapped_modal_line(&mut lines, &value, width, styles.normal);
        }
    }
    lines
}

fn modal_area(area: Rect) -> Rect {
    // Short and narrow terminals need the overlay to yield its decorative
    // margin before form fields, review rows or error guidance disappear.
    // Roomy terminals keep a clear shell context around the modal.
    let horizontal_gutter = if area.width < 64 { 2 } else { 8 };
    let vertical_gutter = if area.height < 18 { 2 } else { 4 };
    let width = area.width.saturating_sub(horizontal_gutter).clamp(1, 86);
    let height = area.height.saturating_sub(vertical_gutter).clamp(1, 22);
    let [vertical] = Layout::vertical([Constraint::Length(height)])
        .flex(ratatui::layout::Flex::Center)
        .areas(area);
    let [centered] = Layout::horizontal([Constraint::Length(width)])
        .flex(ratatui::layout::Flex::Center)
        .areas(vertical);
    centered
}

fn search_modal(model: &Model, styles: Palette, inner_width: usize) -> (Vec<Line<'static>>, usize) {
    let prompt = if matches!(model.modal, Some(Modal::ReportDate | Modal::WorkDate)) {
        "As of: "
    } else {
        "Search: "
    };
    let field_width = inner_width.saturating_sub(text::cells(prompt));
    let (query, cursor) = if model.modal == Some(Modal::ReportDate) {
        (&model.report_as_of, model.report_as_of_cursor)
    } else if model.modal == Some(Modal::WorkDate) {
        (&model.work_as_of_draft, model.work_as_of_cursor)
    } else if model.route == Route::Diagnostics {
        (&model.diagnostic_query, model.diagnostic_query_cursor)
    } else if model.route == Route::Reports && model.focus == Focus::Detail {
        (&model.report_row_query, model.report_row_query_cursor)
    } else if model.route == Route::Reports {
        (&model.report_kind_query, model.report_kind_query_cursor)
    } else if model.route == Route::Inbox {
        (&model.inbox_query, model.inbox_query_cursor)
    } else {
        (&model.task_query, model.task_query_cursor)
    };
    let before = &query[..cursor];
    let after = &query[cursor..];
    let visible_before = text::fit_tail(before, field_width, model.glyphs == Glyphs::Ascii);
    let visible_after = text::fit(
        after,
        field_width.saturating_sub(text::cells(&visible_before)),
        model.glyphs == Glyphs::Ascii,
    );
    let count = if model.modal == Some(Modal::ReportDate) {
        "ISO date YYYY-MM-DD; blank uses HEAD commit time; Enter accepts.".to_string()
    } else if model.modal == Some(Modal::WorkDate) {
        if model.work_as_of_draft_epoch().is_some() {
            "RFC 3339 or signed epoch seconds; blank uses snapshot time; Enter applies.".to_string()
        } else {
            "INVALID: enter RFC 3339 or signed epoch seconds; Esc cancels.".to_string()
        }
    } else if model.route == Route::Diagnostics {
        format!(
            "{}/{} findings shown; whole-run verdict is unchanged; Enter accepts.",
            model.diagnostic_findings().len(),
            model.diagnostic_run().findings.len()
        )
    } else if model.route == Route::Reports && model.focus == Focus::Detail {
        format!(
            "{}/{} rows shown; matches key, label and summary; Enter accepts.",
            model.report_rows().len(),
            model.report_projection.rows.len()
        )
    } else if model.route == Route::Reports {
        format!(
            "{}/{} report names shown; Enter accepts.",
            model.report_kinds().len(),
            aplan_app::ReportKind::ALL.len()
        )
    } else if model.route == Route::Inbox {
        let loaded = model
            .snapshot
            .inbox
            .as_ref()
            .map_or(0, |workspace| workspace.captures.len());
        format!(
            "{}/{} bounded capture rows shown; searches id, title and body; Enter accepts.",
            model.inbox_rows().len(),
            loaded
        )
    } else {
        format!(
            "{}/{} tasks shown; Enter accepts; Esc keeps the query and closes.",
            model.task_browser.rows.len(),
            model.task_browser.total
        )
    };
    (
        vec![
            Line::from(vec![
                Span::styled(prompt, styles.accent),
                Span::raw(format!("{visible_before}{visible_after}")),
            ]),
            Line::styled(text::fit(&count, inner_width, false), styles.muted),
        ],
        text::cells(prompt) + text::cells(&visible_before),
    )
}

fn inbox_filter_modal(
    model: &Model,
    styles: Palette,
    inner_width: usize,
    inner_height: usize,
) -> (Vec<Line<'static>>, Vec<OverlayRowTarget>) {
    let choices = model.inbox_filter_choices();
    let list_height = inner_height.saturating_sub(1);
    let start = model
        .modal_selected
        .saturating_sub(list_height.saturating_sub(1));
    let mut lines = vec![Line::styled(
        "Enter/space selects one value per State, Review, Kind or Label group; c resets defaults.",
        styles.muted,
    )];
    let mut targets = Vec::new();
    for (offset, filter) in choices.iter().skip(start).take(list_height).enumerate() {
        let index = start + offset;
        targets.push(OverlayRowTarget::one(
            lines.len(),
            Action::SelectModal(index),
        ));
        let checked = if model.inbox_filter_selected(filter) {
            "[x]"
        } else {
            "[ ]"
        };
        let style = if index == model.modal_selected {
            styles.selected
        } else {
            styles.normal
        };
        lines.push(Line::styled(
            text::fit(
                &format!("{checked} {}", filter.label()),
                inner_width,
                model.glyphs == Glyphs::Ascii,
            ),
            style,
        ));
    }
    (lines, targets)
}

fn filter_modal(
    model: &Model,
    styles: Palette,
    inner_width: usize,
    inner_height: usize,
) -> (Vec<Line<'static>>, Vec<OverlayRowTarget>) {
    if model.route == Route::Inbox {
        return inbox_filter_modal(model, styles, inner_width, inner_height);
    }
    if model.route == Route::Diagnostics {
        let choices = model.diagnostic_filter_choices();
        let list_height = inner_height.saturating_sub(1);
        let start = model
            .modal_selected
            .saturating_sub(list_height.saturating_sub(1));
        let mut lines = vec![Line::styled(
            "Enter/space toggles; c clears; filtering never changes the verdict.",
            styles.muted,
        )];
        let mut targets = Vec::new();
        for (offset, filter) in choices.iter().skip(start).take(list_height).enumerate() {
            let index = start + offset;
            targets.push(OverlayRowTarget::one(
                lines.len(),
                Action::SelectModal(index),
            ));
            let checked = if model.diagnostic_filters.contains(filter) {
                "[x]"
            } else {
                "[ ]"
            };
            let style = if index == model.modal_selected {
                styles.selected
            } else {
                styles.normal
            };
            lines.push(Line::styled(
                text::fit(
                    &format!("{checked} {}", filter.label()),
                    inner_width,
                    model.glyphs == Glyphs::Ascii,
                ),
                style,
            ));
        }
        return (lines, targets);
    }
    let choices = model.filter_choices();
    let list_height = inner_height.saturating_sub(1);
    let start = model
        .modal_selected
        .saturating_sub(list_height.saturating_sub(1));
    let mut lines = vec![Line::styled(
        "Enter/space toggles; c clears; Esc closes.",
        styles.muted,
    )];
    let mut targets = Vec::new();
    for (offset, filter) in choices.iter().skip(start).take(list_height).enumerate() {
        let index = start + offset;
        targets.push(OverlayRowTarget::one(
            lines.len(),
            Action::SelectModal(index),
        ));
        let checked = if filter.selected(&model.task_selector) {
            "[x]"
        } else {
            "[ ]"
        };
        let style = if index == model.modal_selected {
            styles.selected
        } else {
            styles.normal
        };
        lines.push(Line::styled(
            text::fit(
                &format!("{checked} {}", filter.label()),
                inner_width,
                model.glyphs == Glyphs::Ascii,
            ),
            style,
        ));
    }
    (lines, targets)
}

fn modal_controls(model: &Model, overlay: Modal) -> Vec<OverlayControl> {
    let mut controls = vec![OverlayControl::new("Cancel", Action::CloseLayer)];
    match overlay {
        Modal::About | Modal::RuleExplanation => {
            controls.push(OverlayControl::new(
                "Page up",
                Action::AdjustModalScroll(-5),
            ));
            controls.push(OverlayControl::new(
                "Page down",
                Action::AdjustModalScroll(5),
            ));
        }
        Modal::Help => {}
        Modal::Palette => {
            if let Some(action) = palette_selected_action(model) {
                controls.push(OverlayControl::new("Invoke", action));
            }
        }
        Modal::Search | Modal::ReportDate => {
            controls.push(OverlayControl::new("Clear", Action::SearchClear));
            controls.push(OverlayControl::new("Apply", Action::CloseLayer));
        }
        Modal::WorkDate => {
            controls.push(OverlayControl::new("Clear", Action::SearchClear));
            controls.push(OverlayControl::new("Apply", Action::SubmitWorkDate));
        }
        Modal::Filters => {
            controls[0].label = "Close".to_string();
            controls.push(OverlayControl::new("Clear", Action::ClearFilters));
            if modal_item_count(model, overlay) > 0 {
                controls.push(OverlayControl::new(
                    "Toggle",
                    Action::ToggleFilter(model.modal_selected),
                ));
            }
        }
        Modal::RunSheet => {
            controls.push(OverlayControl::new(
                "Activate",
                Action::ToggleRunSheet(model.modal_selected),
            ));
        }
        Modal::VerifyRunSheet => {
            controls.push(OverlayControl::new(
                "Activate",
                Action::ToggleVerifyRunSheet(model.modal_selected),
            ));
        }
        Modal::Verification => {
            controls[0].label = "Close".to_string();
            let count = modal_item_count(model, overlay);
            if count > 1 {
                let selected = model.modal_selected.min(count - 1);
                if selected > 0 {
                    controls.push(OverlayControl::new(
                        "Previous",
                        Action::SelectModal(selected - 1),
                    ));
                }
                if selected + 1 < count {
                    controls.push(OverlayControl::new(
                        "Next",
                        Action::SelectModal(selected + 1),
                    ));
                }
            }
            controls.push(OverlayControl::new(
                "Page up",
                Action::AdjustModalScroll(-5),
            ));
            controls.push(OverlayControl::new(
                "Page down",
                Action::AdjustModalScroll(5),
            ));
        }
        Modal::RuleSearch => {
            controls.push(OverlayControl::new("Clear", Action::SearchClear));
            controls.push(OverlayControl::new("Explain", Action::SubmitRuleSearch));
        }
        Modal::TestCatalogue => {
            if matches!(
                model.test_catalogue,
                Some(crate::model::TestCatalogueLayer::Failed(_))
            ) {
                controls.push(OverlayControl::new("Retry", Action::OpenTestCatalogue));
            }
            if matches!(
                model.test_catalogue,
                Some(crate::model::TestCatalogueLayer::Ready(_))
            ) {
                controls.push(OverlayControl::new("Insert", Action::UseTestEvidence));
            }
        }
    }
    controls
}

fn mutation_controls(model: &Model) -> Vec<OverlayControl> {
    let Some(layer) = model.mutation.as_ref() else {
        return Vec::new();
    };
    if layer.committing {
        return Vec::new();
    }
    match &layer.state {
        MutationState::Loading => vec![OverlayControl::new("Cancel", Action::CloseLayer)],
        MutationState::QuickReview { .. } => vec![
            OverlayControl::new("Cancel", Action::CloseLayer),
            OverlayControl::new("Confirm", Action::ConfirmMutation),
        ],
        MutationState::Result { .. } => {
            vec![OverlayControl::new("Close", Action::CloseLayer)]
        }
        MutationState::Form(form) if form.is_editing() => {
            let mut controls = vec![OverlayControl::new(
                "Cancel",
                Action::MutationForm(FormAction::Cancel),
            )];
            if layer.action == crate::mutations::MutationAction::Add {
                controls.push(OverlayControl::new(
                    "External editor (Ctrl-E)",
                    Action::OpenExternalEditor,
                ));
            }
            controls.push(OverlayControl::new(
                "Review",
                Action::MutationForm(FormAction::Review),
            ));
            controls
        }
        MutationState::Form(form) => {
            let mut controls = vec![
                OverlayControl::new("Cancel", Action::MutationForm(FormAction::Cancel)),
                OverlayControl::new("Recompute", Action::MutationForm(FormAction::Recompute)),
            ];
            if form.can_confirm() {
                let confirm = OverlayControl::new(
                    form.confirm_label(),
                    Action::MutationForm(FormAction::PointerConfirm(Activation::from_key_kind(
                        KeyEventKind::Press,
                    ))),
                );
                match form.broad_confirmation_armed() {
                    Some(false) => controls.insert(0, confirm),
                    Some(true) => {
                        controls.insert(0, OverlayControl::inert("Broad change armed"));
                        controls.push(confirm);
                    }
                    None => controls.push(confirm),
                }
            }
            controls
        }
    }
}

fn pack_overlay_controls(
    controls: &[OverlayControl],
    width: usize,
) -> Vec<Vec<PackedOverlayControl>> {
    if controls.is_empty() || width == 0 {
        return Vec::new();
    }
    let mut rows: Vec<Vec<PackedOverlayControl>> = vec![Vec::new()];
    let mut used = 0;
    for control in controls {
        let label = format!("[{}]", text::safe(&control.label));
        let cells = text::cells(&label);
        if cells > width {
            continue;
        }
        let separator = usize::from(!rows.last().expect("one row").is_empty());
        if used + separator + cells > width {
            rows.push(Vec::new());
            used = 0;
        }
        if used > 0 {
            used += 1;
        }
        rows.last_mut()
            .expect("one row")
            .push(PackedOverlayControl {
                x: u16::try_from(used).unwrap_or(u16::MAX),
                label,
                action: control.action.clone(),
            });
        used += cells;
    }
    rows.retain(|row| !row.is_empty());
    rows
}

fn overlay_layout(shell: Rect, control_rows: usize) -> OverlayLayout {
    let outer = modal_area(shell);
    let inner = outer.inner(Margin::new(1, 1));
    let control_height = u16::try_from(control_rows)
        .unwrap_or(u16::MAX)
        .min(inner.height);
    let content_height = inner.height.saturating_sub(control_height);
    OverlayLayout {
        outer,
        content: Rect::new(inner.x, inner.y, inner.width, content_height),
        controls: Rect::new(
            inner.x,
            inner.y.saturating_add(content_height),
            inner.width,
            control_height,
        ),
    }
}

fn render_overlay_controls(
    frame: &mut Frame<'_>,
    rows: &[Vec<PackedOverlayControl>],
    area: Rect,
    styles: Palette,
) {
    let lines = rows
        .iter()
        .map(|row| {
            let mut spans = Vec::new();
            for (index, control) in row.iter().enumerate() {
                if index > 0 {
                    spans.push(Span::raw(" "));
                }
                spans.push(Span::styled(
                    control.label.clone(),
                    if control.action.is_some() {
                        styles.accent
                    } else {
                        styles.muted
                    },
                ));
            }
            Line::from(spans)
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), area);
}

fn overlay_control_targets(rows: &[Vec<PackedOverlayControl>], area: Rect) -> Vec<Target<Action>> {
    rows.iter()
        .enumerate()
        .flat_map(|(row, controls)| {
            controls.iter().filter_map(move |control| {
                control.action.clone().map(|action| {
                    label_target(
                        area.x.saturating_add(control.x),
                        area.y
                            .saturating_add(u16::try_from(row).unwrap_or(u16::MAX)),
                        &control.label,
                        action,
                    )
                })
            })
        })
        .collect()
}

fn modal_scroll(model: &Model, overlay: Modal, line_count: usize, height: usize) -> usize {
    if matches!(
        overlay,
        Modal::About | Modal::VerifyRunSheet | Modal::Verification | Modal::RuleExplanation
    ) {
        model.modal_scroll.min(line_count.saturating_sub(height))
    } else {
        0
    }
}

fn modal_hit_test(
    shell: Rect,
    model: &Model,
    overlay: Modal,
    position: Position,
) -> Option<Action> {
    let controls = modal_controls(model, overlay);
    let width = usize::from(modal_area(shell).width.saturating_sub(2));
    let packed = pack_overlay_controls(&controls, width);
    let layout = overlay_layout(shell, packed.len());
    if !layout.outer.contains(position) {
        return None;
    }
    if let Some(target) = overlay_control_targets(&packed, layout.controls)
        .into_iter()
        .find(|target| target.area.contains(position))
    {
        return Some(target.value);
    }
    if !layout.content.contains(position) {
        return None;
    }
    let content = modal_content(
        model,
        overlay,
        palette(model.theme),
        usize::from(layout.content.width),
        usize::from(layout.content.height),
    );
    let scroll = modal_scroll(
        model,
        overlay,
        content.lines.len(),
        usize::from(layout.content.height),
    );
    let row = scroll + usize::from(position.y.saturating_sub(layout.content.y));
    content
        .targets
        .into_iter()
        .find(|target| row >= target.first && row < target.last)
        .map(|target| target.action)
}

fn mutation_hit_test(shell: Rect, model: &Model, position: Position) -> Option<Action> {
    let layer = model.mutation.as_ref()?;
    let controls = mutation_controls(model);
    let width = usize::from(modal_area(shell).width.saturating_sub(2));
    let packed = pack_overlay_controls(&controls, width);
    let layout = overlay_layout(shell, packed.len());
    if !layout.outer.contains(position) {
        return None;
    }
    if let Some(target) = overlay_control_targets(&packed, layout.controls)
        .into_iter()
        .find(|target| target.area.contains(position))
    {
        return Some(target.value);
    }
    if layer.committing || !layout.content.contains(position) {
        return None;
    }
    let MutationState::Form(form) = &layer.state else {
        return None;
    };
    let row = usize::from(position.y.saturating_sub(layout.content.y));
    form.pointer_rows(
        usize::from(layout.content.width),
        usize::from(layout.content.height),
    )
    .into_iter()
    .find_map(|(target_row, action)| (target_row == row).then_some(Action::MutationForm(action)))
}

fn render_modal(frame: &mut Frame<'_>, model: &Model, overlay: Modal, shell: Rect) {
    let styles = palette(model.theme);
    let controls = modal_controls(model, overlay);
    let width = usize::from(modal_area(shell).width.saturating_sub(2));
    let packed = pack_overlay_controls(&controls, width);
    let layout = overlay_layout(shell, packed.len());
    frame.render_widget(Clear, layout.outer);
    let mut content = modal_content(
        model,
        overlay,
        styles,
        usize::from(layout.content.width),
        usize::from(layout.content.height),
    );
    if content.lines.is_empty() {
        content
            .lines
            .push(Line::styled("No matching commands.", styles.muted));
    }
    let scroll = modal_scroll(
        model,
        overlay,
        content.lines.len(),
        usize::from(layout.content.height),
    );
    frame.render_widget(block(model, content.title, true), layout.outer);
    frame.render_widget(
        Paragraph::new(content.lines)
            .style(styles.normal)
            .scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0)),
        layout.content,
    );
    render_overlay_controls(frame, &packed, layout.controls, styles);
    if let Some(cursor_column) = content
        .cursor_column
        .filter(|_| layout.content.width > 10 && layout.content.height > 0)
    {
        let max_x = layout.content.right().saturating_sub(1);
        let x = layout
            .content
            .x
            .saturating_add(u16::try_from(cursor_column).unwrap_or(u16::MAX))
            .min(max_x);
        frame.set_cursor_position(Position::new(x, layout.content.y));
    }
}

fn render_mutation(frame: &mut Frame<'_>, model: &Model, shell: Rect) {
    let Some(layer) = &model.mutation else {
        return;
    };
    let styles = palette(model.theme);
    let controls = mutation_controls(model);
    let width = usize::from(modal_area(shell).width.saturating_sub(2));
    let packed = pack_overlay_controls(&controls, width);
    let layout = overlay_layout(shell, packed.len());
    frame.render_widget(Clear, layout.outer);
    let rows = layer.rows(
        usize::from(layout.content.width),
        usize::from(layout.content.height),
    );
    let lines = rows
        .into_iter()
        .map(|row| Line::styled(text::safe(&row), styles.normal))
        .collect::<Vec<_>>();
    let title = if layer.committing {
        format!(" {} - publishing; quit waits ", layer.action.title())
    } else {
        format!(" {} - Esc closes ", layer.action.title())
    };
    frame.render_widget(block(model, &title, true), layout.outer);
    frame.render_widget(Paragraph::new(lines).style(styles.normal), layout.content);
    render_overlay_controls(frame, &packed, layout.controls, styles);
}

fn modal_content(
    model: &Model,
    overlay: Modal,
    styles: Palette,
    width: usize,
    height: usize,
) -> ModalContent {
    match overlay {
        Modal::About => {
            ModalContent::inert(" About this build ", about_modal(model, styles, width))
        }
        Modal::Help => {
            let (lines, targets) = help_modal(model, styles, width, height);
            ModalContent::targeted(" Help - Esc cancels ", lines, targets)
        }
        Modal::Palette => {
            let (title, lines, cursor_column, targets) =
                palette_modal_content(model, styles, width, height);
            ModalContent {
                title,
                lines,
                cursor_column,
                targets,
            }
        }
        Modal::Search | Modal::ReportDate | Modal::WorkDate => {
            let (lines, cursor) = search_modal(model, styles, width);
            let title = if model.modal == Some(Modal::ReportDate) {
                " Stale report as-of date "
            } else if model.modal == Some(Modal::WorkDate) {
                " Work discovery as-of instant "
            } else if model.route == Route::Diagnostics {
                " Search findings "
            } else if model.route == Route::Reports {
                if model.focus == Focus::Detail {
                    " Search report rows "
                } else {
                    " Search report names "
                }
            } else {
                " Search tasks "
            };
            ModalContent::editor(title, lines, cursor)
        }
        Modal::Filters => {
            let (lines, targets) = filter_modal(model, styles, width, height);
            let title = if model.route == Route::Diagnostics {
                " Diagnostic filters "
            } else {
                " Task filters "
            };
            ModalContent::targeted(title, lines, targets)
        }
        Modal::RunSheet => {
            let (lines, targets) = run_sheet_modal(model, styles, width);
            ModalContent::targeted(" Advanced check ", lines, targets)
        }
        Modal::VerifyRunSheet => {
            let (lines, targets) = verify_run_sheet_modal(model, styles, width);
            ModalContent::targeted(" Verify ", lines, targets)
        }
        Modal::Verification => ModalContent::inert(
            " Verification ",
            verification_result_modal(model, styles, width),
        ),
        Modal::RuleSearch => {
            let (lines, cursor) = rule_search_modal(model, styles, width);
            ModalContent::editor(" Explain rule - exact catalogue name ", lines, cursor)
        }
        Modal::RuleExplanation => {
            ModalContent::inert(" Rule explanation ", rule_explanation_modal(model, styles))
        }
        Modal::TestCatalogue => {
            let (lines, cursor, targets) = test_catalogue_modal(model, styles, width, height);
            ModalContent {
                title: " Configured tests ",
                lines,
                cursor_column: Some(cursor),
                targets,
            }
        }
    }
}

fn test_catalogue_modal(
    model: &Model,
    styles: Palette,
    width: usize,
    height: usize,
) -> (Vec<Line<'static>>, usize, Vec<OverlayRowTarget>) {
    let prompt = "Test: ";
    let field_width = width.saturating_sub(text::cells(prompt));
    let before = &model.test_query[..model.test_query_cursor];
    let after = &model.test_query[model.test_query_cursor..];
    let visible_before = text::fit_tail(before, field_width, model.glyphs == Glyphs::Ascii);
    let visible_after = text::fit(
        after,
        field_width.saturating_sub(text::cells(&visible_before)),
        model.glyphs == Glyphs::Ascii,
    );
    let mut lines = vec![Line::from(vec![
        Span::styled(prompt, styles.accent),
        Span::raw(format!("{visible_before}{visible_after}")),
    ])];
    let mut targets = Vec::new();
    match model.test_catalogue.as_ref() {
        Some(crate::model::TestCatalogueLayer::Loading) => {
            lines.push(Line::styled(
                "Listing configured runners on the bounded worker; navigation remains responsive.",
                styles.muted,
            ));
        }
        Some(crate::model::TestCatalogueLayer::Failed(error)) => {
            lines.push(Line::styled("UNAVAILABLE", styles.error));
            push_wrapped_modal_line(&mut lines, error, width, styles.normal);
        }
        Some(crate::model::TestCatalogueLayer::Ready(view)) => {
            let (ready_lines, ready_targets) =
                ready_test_catalogue(model, view, styles, width, height);
            lines.extend(ready_lines);
            targets.extend(ready_targets);
        }
        None => lines.push(Line::styled("Catalogue is not loaded.", styles.error)),
    }
    (
        lines,
        text::cells(prompt) + text::cells(&visible_before),
        targets,
    )
}

fn ready_test_catalogue(
    model: &Model,
    view: &crate::evidence::TestCatalogueView,
    styles: Palette,
    width: usize,
    height: usize,
) -> (Vec<Line<'static>>, Vec<OverlayRowTarget>) {
    let mut lines = Vec::new();
    let mut targets = Vec::new();
    let summary = &view.catalogue.summary;
    lines.push(Line::styled(
        format!(
            "{:?}: {}/{} runner(s) answered, {} unique name(s), {} ambiguous, {} failed",
            view.catalogue.state,
            summary.listed_runners + summary.empty_runners,
            summary.runners,
            summary.unique_names,
            summary.ambiguous_names,
            summary.failed_runners
        ),
        if view.catalogue.denied {
            styles.error
        } else {
            styles.muted
        },
    ));
    if let Some(finding) = view.catalogue.findings.first() {
        push_wrapped_modal_line(
            &mut lines,
            &format!("{} {}: {}", finding.severity, finding.code, finding.message),
            width,
            styles.error,
        );
    }
    let choices = model.filtered_test_choices();
    lines.push(Line::styled(
        format!(
            "{}/{} runner-name pair(s) match; arrows select; ambiguous names cannot be inserted.",
            choices.len(),
            view.choices.len()
        ),
        styles.muted,
    ));
    let prompt_rows = 1;
    let list_height = height.saturating_sub(prompt_rows + lines.len() + 1).max(1);
    let start = model
        .modal_selected
        .saturating_sub(list_height.saturating_sub(1));
    for (offset, choice) in choices.iter().skip(start).take(list_height).enumerate() {
        let index = start + offset;
        targets.push(OverlayRowTarget::one(
            prompt_rows + lines.len(),
            Action::SelectModal(index),
        ));
        let marker = if choice.selectable { "[ ]" } else { "[!]" };
        let style = if index == model.modal_selected {
            styles.selected
        } else if choice.selectable {
            styles.normal
        } else {
            styles.error
        };
        lines.push(Line::styled(
            text::fit(
                &format!("{marker} {}", choice.citation),
                width,
                model.glyphs == Glyphs::Ascii,
            ),
            style,
        ));
    }
    if let Some(choice) = model.selected_test_choice() {
        lines.push(Line::styled(
            text::fit(&choice.advice, width, model.glyphs == Glyphs::Ascii),
            if choice.selectable {
                styles.muted
            } else {
                styles.error
            },
        ));
    }
    (lines, targets)
}

fn palette_modal_content(
    model: &Model,
    styles: Palette,
    width: usize,
    height: usize,
) -> (
    &'static str,
    Vec<Line<'static>>,
    Option<usize>,
    Vec<OverlayRowTarget>,
) {
    let states = palette_states(model);
    let list_height = height.saturating_sub(2);
    let start = model
        .modal_selected
        .saturating_sub(list_height.saturating_sub(1));
    let query = text::fit_tail(
        &model.palette_query,
        width.saturating_sub(text::cells("Action: ")),
        model.glyphs == Glyphs::Ascii,
    );
    let cursor = text::cells("Action: ") + text::cells(&query);
    let mut lines = vec![
        Line::from(vec![
            Span::styled("Action: ", styles.accent),
            Span::raw(query),
        ]),
        Line::styled("Type to filter; Enter invokes; Esc cancels.", styles.muted),
    ];
    let mut targets = Vec::new();
    for (offset, state) in states.into_iter().skip(start).take(list_height).enumerate() {
        let index = start + offset;
        targets.push(OverlayRowTarget::one(
            lines.len(),
            Action::SelectModal(index),
        ));
        lines.push(palette_line(
            &state,
            index == model.modal_selected,
            styles,
            width,
            model.glyphs == Glyphs::Ascii,
        ));
    }
    (" Command palette ", lines, Some(cursor), targets)
}

fn help_modal(
    model: &Model,
    styles: Palette,
    width: usize,
    height: usize,
) -> (Vec<Line<'static>>, Vec<OverlayRowTarget>) {
    let states = action_states(model);
    let start = model
        .modal_selected
        .saturating_sub(height.saturating_sub(1));
    let mut lines = Vec::new();
    let mut targets = Vec::new();
    for (offset, state) in states.into_iter().skip(start).take(height).enumerate() {
        let index = start + offset;
        targets.push(OverlayRowTarget::one(
            lines.len(),
            Action::SelectModal(index),
        ));
        lines.push(palette_line(
            &state,
            index == model.modal_selected,
            styles,
            width,
            model.glyphs == Glyphs::Ascii,
        ));
    }
    (lines, targets)
}

fn run_sheet_modal(
    model: &Model,
    styles: Palette,
    width: usize,
) -> (Vec<Line<'static>>, Vec<OverlayRowTarget>) {
    let pending = &model.pending_check_overrides;
    let baseline = &model.snapshot.diagnostics.run_sheet.repository;
    let warning = match pending.warning_policy {
        aplan_app::WarningPolicyOverride::Inherit => format!(
            "inherit ({})",
            if baseline.deny_warnings {
                "deny"
            } else {
                "allow"
            }
        ),
        aplan_app::WarningPolicyOverride::Deny => "deny".to_string(),
        aplan_app::WarningPolicyOverride::Allow => "allow".to_string(),
    };
    let rows = [
        format!(
            "effort: {}",
            if pending.no_require_effort && baseline.require_effort {
                "optional [relaxed override]"
            } else if pending.no_require_effort {
                "optional (same as inherited)"
            } else if baseline.require_effort {
                "required (inherited)"
            } else {
                "optional (inherited)"
            }
        ),
        format!(
            "traces: {}",
            if pending.no_traces && baseline.traces_root.is_some() {
                "disabled [relaxed override]".to_string()
            } else if pending.no_traces {
                "disabled (same as inherited)".to_string()
            } else {
                baseline.traces_root.as_ref().map_or_else(
                    || "disabled (inherited)".to_string(),
                    |path| format!("{} (inherited)", path.display()),
                )
            }
        ),
        format!("warnings: {warning}"),
        "RUN ONCE (ordinary refresh returns to repository policy)".to_string(),
    ];
    let mut lines = Vec::new();
    let mut targets = Vec::new();
    for (index, row) in rows.into_iter().enumerate() {
        targets.push(OverlayRowTarget::one(
            lines.len(),
            Action::SelectModal(index),
        ));
        lines.push(Line::styled(
            text::fit(
                &format!(
                    "{} {row}",
                    if index == model.modal_selected {
                        ">"
                    } else {
                        " "
                    }
                ),
                width,
                model.glyphs == Glyphs::Ascii,
            ),
            if index == model.modal_selected {
                styles.selected
            } else {
                styles.normal
            },
        ));
    }
    (lines, targets)
}

fn verify_run_sheet_modal(
    model: &Model,
    styles: Palette,
    width: usize,
) -> (Vec<Line<'static>>, Vec<OverlayRowTarget>) {
    let sheet = &model.snapshot.verification_sheet;
    let mut lines = Vec::new();
    let config = sheet.configuration.as_ref().map_or_else(
        || "none selected".to_string(),
        |path| path.display().to_string(),
    );
    let traces = sheet.policy.traces_root.as_ref().map_or_else(
        || "disabled/repository default".to_string(),
        |path| path.display().to_string(),
    );
    let first = lines.len();
    push_selected_modal_line(
        &mut lines,
        model,
        0,
        &format!(
            "execute configured tests: {}{}",
            if model.pending_verification_mode.execute() {
                "ON"
            } else {
                "off"
            },
            if sheet.execute.is_available() {
                " [explicit trust action]"
            } else {
                " [unavailable: no test runner]"
            }
        ),
        width,
        styles,
    );
    let mut targets = vec![OverlayRowTarget {
        first,
        last: lines.len(),
        action: Action::SelectModal(0),
    }];
    let first = lines.len();
    push_selected_modal_line(&mut lines, model, 1, "RUN VERIFY", width, styles);
    targets.push(OverlayRowTarget {
        first,
        last: lines.len(),
        action: Action::SelectModal(1),
    });
    let detail_rows = [
        format!("repository: {}", sheet.repository.display()),
        format!("configuration: {config}"),
        format!(
            "policy: effort={} traces={traces} warnings={} drift={}",
            if sheet.policy.require_effort {
                "required"
            } else {
                "optional"
            },
            if sheet.policy.deny_warnings {
                "deny"
            } else {
                "allow"
            },
            if sheet.policy.detect_drift {
                "on"
            } else {
                "off"
            },
        ),
        format!(
            "trust: {}. providers: {} test runner(s); url.check={}; version.current={}; version.check={}. Resolve-only may run test.list and configured URL/version commands.",
            match sheet.trust {
                aplan_app::Trust::Hermetic => "hermetic",
                aplan_app::Trust::RepositoryRead => "repository read",
                aplan_app::Trust::ConfiguredProcesses => "configured processes",
            },
            sheet.providers.test_runners,
            if sheet.providers.url_check.is_available() {
                "configured"
            } else {
                "unavailable"
            },
            if sheet.providers.version_current.is_available() {
                "configured"
            } else {
                "unavailable"
            },
            if sheet.providers.version_check.is_available() {
                "configured"
            } else {
                "unavailable"
            },
        ),
    ];
    for row in detail_rows {
        push_wrapped_modal_line(&mut lines, &row, width, styles.normal);
    }
    for note in [
        "progress: active phase/provider ordinal and known completed/remaining work; no fake percentage",
        "cancel: queued work only; running work is joined on quit",
        "fix: exact repairs are a separate guarded palette action; this verification run remains read-only",
    ] {
        push_wrapped_modal_line(&mut lines, note, width, styles.muted);
    }
    (lines, targets)
}

fn push_selected_modal_line(
    lines: &mut Vec<Line<'static>>,
    model: &Model,
    index: usize,
    value: &str,
    width: usize,
    styles: Palette,
) {
    let selected = model.modal_selected == index;
    push_wrapped_modal_line(
        lines,
        &format!("{} {value}", if selected { ">" } else { " " }),
        width,
        if selected {
            styles.selected
        } else {
            styles.normal
        },
    );
}

fn push_wrapped_modal_line(
    lines: &mut Vec<Line<'static>>,
    value: &str,
    width: usize,
    style: Style,
) {
    lines.extend(
        text::wrap(value, width)
            .into_iter()
            .map(|part| Line::styled(part, style)),
    );
}

fn verification_result_modal(model: &Model, styles: Palette, width: usize) -> Vec<Line<'static>> {
    let Some(verification) = model.verification.as_deref() else {
        return vec![Line::styled("No verification result.", styles.muted)];
    };
    let mut lines = verification_result_header(verification, model, styles, width);
    if let Some(finding) = verification.run.findings.get(model.modal_selected) {
        append_verification_finding(
            &mut lines,
            finding,
            model.modal_selected,
            verification.run.findings.len(),
            width,
            styles.normal,
        );
        return lines;
    }
    let citation_index = model
        .modal_selected
        .saturating_sub(verification.run.findings.len());
    if let Some(citation) = verification.citations.get(citation_index) {
        append_verification_citation(
            &mut lines,
            verification,
            citation,
            citation_index,
            width,
            model.glyphs == Glyphs::Ascii,
            styles,
        );
        return lines;
    }
    let cost_index = citation_index.saturating_sub(verification.citations.len());
    if let Some(provider) = verification.costs.get(cost_index) {
        push_wrapped_modal_line(
            &mut lines,
            &format!(
                "provider cost {}/{}: {}",
                cost_index + 1,
                verification.costs.len(),
                verification_cost_line(&provider.subject, provider.cost)
            ),
            width,
            styles.normal,
        );
        return lines;
    }
    lines.push(Line::styled(
        "No findings, evidence citations or provider costs were produced by this complete run.",
        styles.muted,
    ));
    if let Some(repair) = verification.repairs.first() {
        push_wrapped_modal_line(
            &mut lines,
            &verification_repair_line(repair, 0, verification.repairs.len(), true),
            width,
            styles.normal,
        );
    }
    lines
}

fn verification_result_header(
    verification: &aplan_app::VerificationWorkbench,
    model: &Model,
    styles: Palette,
    width: usize,
) -> Vec<Line<'static>> {
    let ascii = model.glyphs == Glyphs::Ascii;
    let verdict = if verification.state == aplan_app::VerificationState::Refused {
        "REFUSED"
    } else if verification.run.denied {
        "DENIED"
    } else {
        "PASS"
    };
    let mode = if verification.execute {
        " [EXECUTE]"
    } else {
        " [RESOLVE ONLY]"
    };
    let mut lines = vec![Line::styled(
        text::fit(
            &format!(
                "{verdict}{mode} E{} W{}",
                verification.run.errors, verification.run.warnings
            ),
            width,
            ascii,
        ),
        if verification.run.denied {
            styles.error
        } else {
            styles.good
        },
    )];
    lines.push(Line::from(text::fit(
        &format!(
            "findings: {} citations: {} provider cost(s): {} repairs: {}",
            verification.run.findings.len(),
            verification.citations.len(),
            verification.costs.len(),
            verification.repairs.len()
        ),
        width,
        ascii,
    )));
    push_wrapped_modal_line(
        &mut lines,
        if verification.repairs.is_empty() {
            "fix: this result contains no exact repair to apply"
        } else {
            "fix: choose `Apply verification repairs` in the palette; this result remains read-only"
        },
        width,
        styles.normal,
    );
    push_wrapped_modal_line(
        &mut lines,
        &format!(
            "input: {} repository: {} scope: {} evidence: {} source: {} completeness: {}",
            short_identity(&verification.input.sha256),
            verification.sheet.repository.display(),
            verification.observation.scope.label(),
            verification.observation.state.label(),
            verification.observation.provider_source.label(),
            verification.observation.completeness.label(),
        ),
        width,
        styles.normal,
    );
    lines
}

fn append_verification_finding(
    lines: &mut Vec<Line<'static>>,
    finding: &aplan_app::OverviewFinding,
    index: usize,
    total: usize,
    width: usize,
    style: Style,
) {
    push_wrapped_modal_line(
        lines,
        &format!(
            "finding {}/{}: {} {} - {}",
            index + 1,
            total,
            finding.severity,
            finding.code,
            finding.subject,
        ),
        width,
        style,
    );
    if let Some(file) = &finding.file {
        push_wrapped_modal_line(
            lines,
            &format!(
                "location: {}{}{}",
                file.display(),
                finding
                    .line
                    .map_or_else(String::new, |line| format!(":{line}")),
                finding
                    .column
                    .map_or_else(String::new, |column| format!(":{column}")),
            ),
            width,
            style,
        );
    }
    push_wrapped_modal_line(
        lines,
        &format!("message: {}", finding.message),
        width,
        style,
    );
    if let Some(instruction) = &finding.agent_instruction {
        push_wrapped_modal_line(lines, &format!("action: {instruction}"), width, style);
    }
    for detail in &finding.detail {
        push_wrapped_modal_line(lines, &format!("detail: {detail}"), width, style);
    }
}

fn append_verification_citation(
    lines: &mut Vec<Line<'static>>,
    verification: &aplan_app::VerificationWorkbench,
    citation: &aplan_app::VerificationCitation,
    citation_index: usize,
    width: usize,
    ascii: bool,
    styles: Palette,
) {
    lines.push(Line::from(text::fit(
        &format!(
            "evidence {}/{}: task {}",
            citation_index + 1,
            verification.citations.len(),
            citation.task
        ),
        width,
        ascii,
    )));
    push_wrapped_modal_line(
        lines,
        &format!("kind: {} ref: {}", citation.kind, citation.reference),
        width,
        styles.normal,
    );
    push_wrapped_modal_line(
        lines,
        &format!("resolver: {}", citation.resolver),
        width,
        styles.normal,
    );
    lines.push(Line::styled(
        text::fit(
            &format!(
                "observation: {} strength: {} result: {}",
                citation.observation_state.label(),
                citation.strength.label(),
                citation.outcome.label()
            ),
            width,
            ascii,
        ),
        if citation.outcome == aplan_app::CitationOutcome::Resolved {
            styles.good
        } else if matches!(
            citation.outcome,
            aplan_app::CitationOutcome::TimedOut
                | aplan_app::CitationOutcome::Unavailable
                | aplan_app::CitationOutcome::OutputLimited
        ) {
            styles.warning
        } else {
            styles.error
        },
    ));
    if let Some((index, repair)) = verification.repairs.iter().enumerate().find(|(_, repair)| {
        repair.task.as_deref() == Some(citation.task.as_str())
            && repair.from_kind == citation.kind
            && repair.from_reference == citation.reference
    }) {
        push_wrapped_modal_line(
            lines,
            &verification_repair_line(repair, index, verification.repairs.len(), false),
            width,
            styles.normal,
        );
    }
    if let Some(detail) = &citation.detail {
        push_wrapped_modal_line(lines, &format!("detail: {detail}"), width, styles.normal);
    }
    if let Some(subject) = &citation.cost_subject {
        if let Some(provider) = verification
            .costs
            .iter()
            .find(|provider| &provider.subject == subject)
        {
            push_wrapped_modal_line(
                lines,
                &verification_cost_line(subject, provider.cost),
                width,
                styles.normal,
            );
        }
    } else {
        lines.push(Line::styled("cost: no process was spawned", styles.muted));
    }
}

fn verification_repair_line(
    repair: &aplan_app::VerificationRepair,
    index: usize,
    total: usize,
    include_task: bool,
) -> String {
    let task = if include_task {
        format!(" task {}", repair.task.as_deref().unwrap_or("<plan>"))
    } else {
        String::new()
    };
    format!(
        "repair {}/{}:{task} replace {}:{} with {}:{} [apply unavailable]",
        index + 1,
        total,
        repair.from_kind,
        repair.from_reference,
        repair.to_kind,
        repair.to_reference,
    )
}

fn verification_cost_line(subject: &str, cost: aplan_app::VerificationCost) -> String {
    if !cost.measured {
        return format!("cost ({subject}, shared): measurement unavailable");
    }
    format!(
        "cost ({subject}, shared): {:.3}s, peak {} process(es), {} KiB summed RSS{}{}",
        cost.elapsed.as_secs_f64(),
        cost.peak_processes,
        cost.peak_rss_kb,
        if cost.breach.is_some() {
            ", limit breached"
        } else {
            ""
        },
        if cost.unenforced.is_some() {
            ", declared limits unenforced"
        } else {
            ""
        }
    )
}

fn rule_search_modal(model: &Model, styles: Palette, width: usize) -> (Vec<Line<'static>>, usize) {
    let field_width = width.saturating_sub(text::cells("Exact code: "));
    let before = &model.rule_query[..model.rule_query_cursor];
    let after = &model.rule_query[model.rule_query_cursor..];
    let visible_before = text::fit_tail(before, field_width, model.glyphs == Glyphs::Ascii);
    let visible_after = text::fit(
        after,
        field_width.saturating_sub(text::cells(&visible_before)),
        model.glyphs == Glyphs::Ascii,
    );
    let cursor = text::cells("Exact code: ") + text::cells(&visible_before);
    (
        vec![
            Line::from(vec![
                Span::styled("Exact code: ", styles.accent),
                Span::raw(format!("{visible_before}{visible_after}")),
            ]),
            Line::styled(
                "Enter resolves through the CLI's typed catalogue; unknown names are refused.",
                styles.muted,
            ),
        ],
        cursor,
    )
}

fn rule_explanation_modal(model: &Model, styles: Palette) -> Vec<Line<'static>> {
    model.rule_explanation.as_ref().map_or_else(
        || vec![Line::styled("No rule explanation is loaded.", styles.muted)],
        |explanation| {
            vec![
                Line::styled(
                    format!("{} {}", explanation.severity, explanation.code),
                    styles.accent,
                ),
                Line::from(explanation.title.clone()),
                Line::styled(
                    "Resolved from the same typed catalogue as `aplan explain rule`.",
                    styles.muted,
                ),
                Line::styled("Esc closes.", styles.muted),
            ]
        },
    )
}

fn palette_line(
    state: &PaletteState,
    selected: bool,
    styles: Palette,
    width: usize,
    ascii: bool,
) -> Line<'static> {
    let style = if selected {
        styles.selected
    } else {
        styles.normal
    };
    let availability = state.unavailable.as_ref().map_or_else(
        || "available".to_string(),
        |reason| format!("disabled: {reason}"),
    );
    let line = format!("{:<8}{} - {availability}", state.keys, state.label);
    Line::styled(text::fit(&line, width, ascii), style)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::form::{FieldSpec, MutationForm};
    use crate::model::{EffectCompletion, EffectOutput};
    use crate::mutations::{MutationPayload, MutationTransfer, PreparedMutation};
    use crate::testing::{ModelBuilder, capture_record, graph_task, task_inspector, task_row};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyEventState;

    fn model() -> Model {
        ModelBuilder::default().build()
    }

    #[test]
    fn generated_action_reference_covers_every_executable_action() {
        let reference = generated_action_reference_markdown();
        assert!(reference.ends_with('\n'));
        assert!(!reference.ends_with("\n\n"));
        assert_eq!(
            reference
                .lines()
                .filter(|line| line.starts_with("| `"))
                .count(),
            CommandId::ALL.len()
        );
        for command in CommandId::ALL {
            let (action, context) = command.reference();
            assert!(reference.contains(command.binding().label));
            assert!(reference.contains(action));
            assert!(reference.contains(context));
        }
        for action in MutationAction::ALL {
            assert!(reference.contains(action.title()));
            assert!(reference.contains(&format!("`aplan {}`", action.cli())));
        }
        for command in discovery::COMMANDS {
            if let Some((reason, example)) = command.disposition.boundary() {
                assert!(reference.contains(reason));
                assert!(reference.contains(example));
            }
            for boundary in command.boundaries {
                assert!(reference.contains(boundary.name));
                assert!(reference.contains(boundary.reason));
                assert!(reference.contains(boundary.example));
            }
        }
    }

    fn capability_facts() -> aplan_app::BuildFacts {
        let mut facts = aplan_app::BuildFacts {
            behaviours: vec![aplan_app::BehaviourFact {
                name: "tui.example".to_string(),
                what: "an example behaviour".to_string(),
            }],
            build_capabilities: vec![aplan_app::BuildCapabilityFact {
                name: "build.tui".to_string(),
                enabled: true,
            }],
            commands: vec![aplan_app::CommandFact {
                name: "check".to_string(),
                about: Some("validate a plan".to_string()),
                formats: vec!["human".to_string(), "json".to_string()],
                default_format: Some("human".to_string()),
            }],
            config_keys: vec!["root".to_string()],
            counts: aplan_app::FactCounts::default(),
            evidence_kinds: vec!["test".to_string()],
            exit_codes: aplan_app::ExitCodeFacts {
                clean: 0,
                error: 1,
                usage: 2,
                internal: 3,
            },
            formats: vec!["human".to_string()],
            mcp_tools: vec![aplan_app::McpToolFact {
                name: "aplan_check".to_string(),
                description: "validate a plan".to_string(),
            }],
            rules: vec![aplan_app::RuleFact {
                code: "APLAN001".to_string(),
                severity: "error".to_string(),
                title: "unique ids".to_string(),
            }],
            schema_version: "v1".to_string(),
            version: "0.test".to_string(),
        };
        facts.counts = facts.measured_counts();
        facts
    }

    fn onboarding_model(cause: aplan_app::OnboardingCause) -> Model {
        let build = aplan_app::BuildIdentity {
            version: "0.test".to_string(),
            source: "commit:abc".to_string(),
            facts_sha256: "facts-digest".to_string(),
            binary_sha256: "binary-digest".to_string(),
        };
        let (configuration, root_selection) = match cause {
            aplan_app::OnboardingCause::MissingPlan => (
                aplan_app::ConfigurationSelection::Discover {
                    start: "/repo".into(),
                    searched: vec!["/repo/.aplan.yml".into(), "/.aplan.yml".into()],
                    found: None,
                },
                aplan_app::RootSelection::Default,
            ),
            aplan_app::OnboardingCause::MissingRoot => (
                aplan_app::ConfigurationSelection::Discover {
                    start: "/repo".into(),
                    searched: vec!["/repo/.aplan.yml".into()],
                    found: Some("/repo/.aplan.yml".into()),
                },
                aplan_app::RootSelection::Configuration {
                    file: "/repo/.aplan.yml".into(),
                },
            ),
            _ => (
                aplan_app::ConfigurationSelection::Explicit {
                    path: "/repo/policy.yml".into(),
                },
                aplan_app::RootSelection::Explicit,
            ),
        };
        let onboarding = aplan_app::Onboarding {
            cause,
            plan_root: "/repo/aplan".into(),
            root_selection,
            configuration,
            detail: format!("typed detail for {}", cause.label()),
            guidance: vec!["repair this exact input".to_string()],
            commands: vec![aplan_app::GuidedCommand {
                purpose: "safe repair".to_string(),
                display: "aplan init -- '/repo'".to_string(),
            }],
        };
        let capabilities = capability_facts().capability_view(&build);
        Model::with_launch(
            crate::model::Snapshot::onboarding(&build, &onboarding),
            Some(onboarding.clone()),
            capabilities,
            Some(onboarding.resolution()),
            crate::model::Appearance::default(),
        )
    }

    fn reports_model() -> Model {
        let mut model = ModelBuilder::default().glyphs(Glyphs::Ascii).build();
        model.update(Action::Route(Route::Reports));
        model
    }

    fn open_report(model: &mut Model, index: usize) {
        model.update(Action::Focus(Focus::Primary));
        model.update(Action::SelectIndex(index));
        model.update(Action::Focus(Focus::Detail));
    }

    fn graph_model() -> Model {
        let tasks = vec![
            graph_task("A", "root", &[]),
            graph_task("B", "left \u{754c}", &["A"]),
            graph_task("C", "right e\u{301}", &["A"]),
            graph_task("D", "merge", &["B", "C"]),
        ];
        let mut model = ModelBuilder::default()
            .graph(tasks)
            .glyphs(Glyphs::Ascii)
            .build();
        model.theme = Theme::Monochrome;
        model.update(Action::Route(Route::Graph));
        model
    }

    fn oversized_graph_model() -> Model {
        let mut tasks = vec![graph_task("T-000", "root", &[])];
        tasks.extend((1..=60).map(|index| {
            graph_task(
                &format!("T-{index:03}"),
                &format!("dependent {index}"),
                &["T-000"],
            )
        }));
        let mut model = ModelBuilder::default()
            .graph(tasks)
            .glyphs(Glyphs::Ascii)
            .build();
        model.theme = Theme::Monochrome;
        model.update(Action::Route(Route::Graph));
        model.graph_explorer_request.budget = aplan_app::GraphWindowBudget {
            nodes: 8,
            edges: 8,
            lanes: 4,
        };
        model.update(Action::OpenGraphLocation(
            aplan_app::GraphExplorerLocation::Task("T-000".to_string()),
        ));
        model
    }

    fn aggregate_window_graph_model() -> Model {
        let mut tasks = vec![graph_task("T-000", "root", &[])];
        tasks.extend((1..=70).map(|index| {
            graph_task(
                &format!("T-{index:03}"),
                &format!("dependent {index}"),
                &["T-000"],
            )
        }));
        let mut model = ModelBuilder::default()
            .graph(tasks)
            .glyphs(Glyphs::Ascii)
            .build();
        model.theme = Theme::Monochrome;
        model.update(Action::Route(Route::Graph));
        model
    }

    fn fallback_graph_model() -> Model {
        let mut model = aggregate_window_graph_model();
        model.graph_explorer_request.budget = aplan_app::GraphWindowBudget {
            nodes: 71,
            edges: 70,
            lanes: usize::MAX,
        };
        model.update(Action::OpenGraphLocation(
            aplan_app::GraphExplorerLocation::Task("T-000".to_string()),
        ));
        model
    }

    fn buffer(model: &mut Model, width: u16, height: u16) -> String {
        model.update(Action::Resize {
            columns: width,
            rows: height,
        });
        current_buffer(model, width, height)
    }

    fn fixture_snapshot(snapshot: &str) -> String {
        snapshot.lines().collect::<Vec<_>>().join("\n")
    }

    fn current_buffer(model: &Model, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|frame| render(frame, model)).expect("draw");
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn citation_outcomes() -> [aplan_app::CitationOutcome; 6] {
        [
            aplan_app::CitationOutcome::Resolved,
            aplan_app::CitationOutcome::Failed,
            aplan_app::CitationOutcome::Refused,
            aplan_app::CitationOutcome::TimedOut,
            aplan_app::CitationOutcome::Unavailable,
            aplan_app::CitationOutcome::OutputLimited,
        ]
    }

    fn loading_model(
        theme: Theme,
        glyphs: Glyphs,
        reduced_motion: bool,
        phase: aplan_app::ProgressPhase,
        measure: aplan_app::ProgressMeasure,
    ) -> Model {
        let build = aplan_app::BuildIdentity::default();
        let capabilities = aplan_app::BuildFacts::default().capability_view(&build);
        let (mut model, request) = Model::loading(
            &build,
            capabilities,
            crate::model::Appearance {
                theme,
                glyphs,
                color: if theme == Theme::Monochrome {
                    crate::model::ColorCapability::None
                } else {
                    crate::model::ColorCapability::Extended
                },
                reduced_motion,
            },
        );
        model.update(Action::EffectStarted(request.id));
        for progress in [
            aplan_app::ProgressEvent {
                operation: aplan_app::OperationId::new(request.id.0),
                sequence: 0,
                kind: aplan_app::OperationKind::PlanLoad,
                subject: aplan_app::ProgressSubject::Operation,
                lifecycle: aplan_app::ProgressLifecycle::Queued,
                measure: aplan_app::ProgressMeasure::Indeterminate,
            },
            aplan_app::ProgressEvent {
                operation: aplan_app::OperationId::new(request.id.0),
                sequence: 1,
                kind: aplan_app::OperationKind::PlanLoad,
                subject: aplan_app::ProgressSubject::Operation,
                lifecycle: aplan_app::ProgressLifecycle::Started,
                measure: aplan_app::ProgressMeasure::Indeterminate,
            },
            aplan_app::ProgressEvent {
                operation: aplan_app::OperationId::new(request.id.0),
                sequence: 2,
                kind: aplan_app::OperationKind::PlanLoad,
                subject: aplan_app::ProgressSubject::Phase {
                    phase,
                    child: Some(aplan_app::ProgressChildId::new(0)),
                },
                lifecycle: aplan_app::ProgressLifecycle::Started,
                measure,
            },
        ] {
            model.update(Action::EffectProgress {
                id: request.id,
                progress,
            });
        }
        model
    }

    #[test]
    fn startup_view_is_truthful_responsive_and_available_in_every_appearance() {
        for theme in Theme::ALL {
            for glyphs in Glyphs::ALL {
                for reduced_motion in [false, true] {
                    let mut model = loading_model(
                        theme,
                        glyphs,
                        reduced_motion,
                        aplan_app::ProgressPhase::SourceParsing,
                        aplan_app::ProgressMeasure::bounded(2, 5, aplan_app::ProgressUnit::Sources),
                    );
                    let rendered = buffer(&mut model, 48, 14);
                    assert!(rendered.contains("aplan - loading identified inputs"));
                    assert!(rendered.contains("reading plan sources"));
                    assert!(rendered.contains("2/5 sources"));
                    assert!(rendered.contains("elapsed"));
                    assert!(!rendered.contains('%') && !rendered.contains("ETA"));
                    let stable_shell = buffer(&mut model, 80, 24);
                    for heading in ["Plan health", "Destinations", "Inspector", "Commands"] {
                        assert!(stable_shell.contains(heading), "{heading}: {stable_shell}");
                    }
                    assert!(
                        !stable_shell.contains("0 tasks"),
                        "placeholder data must not masquerade as a loaded plan: {stable_shell}"
                    );
                    let tiny = buffer(&mut model, 32, 8);
                    assert!(tiny.contains("reading plan sources"), "{tiny}");
                }
            }
        }

        let model = loading_model(
            Theme::Dark,
            Glyphs::Unicode,
            true,
            aplan_app::ProgressPhase::SourceParsing,
            aplan_app::ProgressMeasure::Indeterminate,
        );
        let started = model.startup.as_ref().expect("startup state").started;
        let delayed =
            startup_presentation_at(&model, started + std::time::Duration::from_millis(1_260));
        assert_eq!(delayed.elapsed, "1.3 s");
        assert_eq!(delayed.marker, "*");
        assert_eq!(delayed.phase, "reading plan sources");
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &model),
            Some(Action::Quit)
        );
        assert_eq!(
            decode_key(
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
                &model
            ),
            Some(Action::Quit)
        );
        assert_eq!(
            decode_key(
                KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE),
                &model
            ),
            None
        );
    }

    /// Proof for `tui.onboarding_and_capabilities_are_typed`: every classified
    /// first-use/repair cause paints a stable non-plan screen, while About
    /// disposes every typed facts collection and build identity field.
    #[test]
    fn onboarding_causes_and_complete_about_projection_are_rendered() {
        for cause in [
            aplan_app::OnboardingCause::MissingPlan,
            aplan_app::OnboardingCause::MissingConfiguration,
            aplan_app::OnboardingCause::MissingRoot,
            aplan_app::OnboardingCause::UnreadableConfiguration,
            aplan_app::OnboardingCause::InvalidConfiguration,
        ] {
            let mut model = onboarding_model(cause);
            let view = buffer(&mut model, 100, 24);
            assert!(view.contains(cause.label()), "{cause:?}:\n{view}");
            assert!(view.contains("Setup and repair"));
            assert!(view.contains("Plan root: /repo/aplan"));
            assert!(view.contains("Suggested commands (shown only; never run)"));
            assert!(view.contains("aplan init -- '/repo'"));
            assert!(!view.contains("Destinations"));
            assert!(!model.should_probe_external());
            assert_eq!(
                decode_mouse(
                    MouseEvent {
                        kind: MouseEventKind::Down(MouseButton::Left),
                        column: 2,
                        row: 4,
                        modifiers: KeyModifiers::NONE,
                    },
                    &model,
                ),
                None,
                "an invisible destination must not retain a pointer target"
            );
        }

        let mut model = onboarding_model(aplan_app::OnboardingCause::MissingPlan);
        let minimum = buffer(&mut model, 48, 14);
        for expected in ["Esc/q/C-c quit", "r retry", "C-p commands", "? help"] {
            assert!(
                minimum.contains(expected),
                "minimum onboarding omitted `{expected}`:\n{minimum}"
            );
        }
        model.update(Action::OpenHelp);
        let help = buffer(&mut model, 48, 14);
        assert!(help.contains('i') && help.contains("About"), "{help}");
        let more = action_states(&model)
            .iter()
            .position(|state| state.target == PaletteTarget::Command(CommandId::ScrollDown))
            .expect("scroll-down help row");
        model.update(Action::SelectModal(more));
        let help = buffer(&mut model, 48, 14);
        assert!(help.contains("PgDn") && help.contains("more"), "{help}");
        model.update(Action::CloseLayer);
        let scroll = decode_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), &model)
            .expect("minimum onboarding remains scrollable");
        model.update(scroll);
        assert_eq!(model.onboarding_scroll, 5);
        model.update(Action::AdjustModalScroll(100));
        let clamped = buffer(&mut model, 48, 14);
        assert!(
            clamped.contains("After repairing the input")
                && clamped.contains("exact build capabilities"),
            "overscrolling must stop at the final setup guidance:\n{clamped}"
        );
        model.update(Action::OpenAbout);
        let lines = about_modal(&model, palette(model.theme), 200)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.into_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        let text = lines.join("\n");
        for expected in [
            "Version: 0.test",
            "Build identity version: 0.test",
            "Build source: commit:abc",
            "Facts digest: facts-digest",
            "Binary digest: binary-digest",
            "Exit codes: clean 0, error 1, usage 2, internal 3",
            "Counts: 1 commands, 1 formats, 1 rules, 1 evidence kinds, 1 behaviours, 1 MCP tools, 1 config keys, 1 build capabilities",
            "Build capabilities (1)",
            "Commands (1)",
            "- check: validate a plan; formats: human, json; default: human",
            "Formats (1)",
            "Rules (1)",
            "Evidence kinds (1)",
            "Behaviours (1)",
            "MCP tools (1)",
            "Configuration keys (1)",
        ] {
            assert!(
                text.contains(expected),
                "missing `{expected}` from:\n{text}"
            );
        }
        let view = buffer(&mut model, 100, 24);
        assert!(view.contains("About this build"));
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), &model),
            Some(Action::AdjustModalScroll(5))
        );
    }

    fn verification_fixture(
        model: &Model,
        outcomes: &[aplan_app::CitationOutcome],
    ) -> Box<aplan_app::VerificationWorkbench> {
        let mut input = model.snapshot.overview.input.clone();
        input
            .sha256
            .clone_from(&model.snapshot.verification_sheet.input_sha256);
        input.trust = model.snapshot.verification_sheet.trust;
        let citations = outcomes
            .iter()
            .copied()
            .enumerate()
            .map(|(index, outcome)| aplan_app::VerificationCitation {
                task: format!("T-{index}"),
                kind: "test".to_string(),
                reference: format!("suite::{index}"),
                resolver: "test runner `unit`".to_string(),
                strength: if outcome == aplan_app::CitationOutcome::Resolved {
                    aplan_app::CitationStrength::Executed
                } else {
                    aplan_app::CitationStrength::Resolved
                },
                outcome,
                detail: Some(format!("typed outcome {index}")),
                cost_subject: Some("the `unit` test runner".to_string()),
                observation_state: aplan_app::EvidenceObservationState::Current,
            })
            .collect();
        let mut observation = aplan_app::EvidenceObservation::current();
        observation.provider_source = aplan_app::EvidenceProviderSource::Direct;
        Box::new(aplan_app::VerificationWorkbench {
            input,
            sheet: model.snapshot.verification_sheet.clone(),
            execute: true,
            state: aplan_app::VerificationState::Completed,
            observation,
            run: aplan_app::DiagnosticRun {
                scope: aplan_app::DiagnosticScope::Verify,
                coverage: aplan_app::DiagnosticCoverage::Complete { subjects: 6 },
                findings: Vec::new(),
                errors: 1,
                warnings: 2,
                blocking: 1,
                advisory: 0,
                denied: true,
                input_label: "test verification".to_string(),
            },
            citations,
            costs: vec![aplan_app::VerificationProviderCost {
                subject: "the `unit` test runner".to_string(),
                cost: aplan_app::VerificationCost {
                    peak_processes: 2,
                    peak_rss_kb: 512,
                    elapsed: std::time::Duration::from_millis(250),
                    measured: true,
                    ..aplan_app::VerificationCost::default()
                },
            }],
            repairs: vec![aplan_app::VerificationRepair {
                task: Some("T-0".to_string()),
                from_kind: "test".to_string(),
                from_reference: "suite::0".to_string(),
                to_kind: "test".to_string(),
                to_reference: "replacement-0".to_string(),
            }],
        })
    }

    fn assert_receipt_provenance_is_visible(model: &mut Model) {
        let verification = model.verification.as_deref_mut().expect("verification");
        verification.observation.scope = aplan_app::EvidenceObservationScope::WholePlan;
        verification.observation.provider_source = aplan_app::EvidenceProviderSource::Receipt;
        let receipt = buffer(model, 100, 24);
        assert!(receipt.contains("scope: whole_plan"), "{receipt}");
        assert!(
            receipt.contains("source: receipt completeness: complete"),
            "{receipt}"
        );
    }

    fn assert_empty_verification_outcomes_are_explicit(model: &mut Model) {
        let verification = model.verification.as_deref_mut().expect("verification");
        verification.run.findings.clear();
        verification.run.errors = 0;
        verification.run.warnings = 0;
        verification.run.denied = false;
        verification.citations.clear();
        verification.costs.clear();
        verification.execute = false;
        verification.state = aplan_app::VerificationState::Completed;
        let repaired_empty = buffer(model, 100, 24);
        for expected in [
            "PASS [RESOLVE ONLY] E0 W0",
            "No findings, evidence citations or provider costs were produced",
            "replace test:suite::0 with test:replacement-0",
        ] {
            assert!(
                repaired_empty.contains(expected),
                "complete empty result omitted `{expected}`:\n{repaired_empty}"
            );
        }

        model
            .verification
            .as_deref_mut()
            .expect("verification")
            .repairs
            .clear();
        let empty = buffer(model, 100, 24);
        assert!(
            empty.contains("fix: this result contains no exact repair to apply"),
            "{empty}"
        );

        model
            .verification
            .as_deref_mut()
            .expect("verification")
            .state = aplan_app::VerificationState::Refused;
        let refused = buffer(model, 100, 24);
        assert!(
            refused.contains("REFUSED [RESOLVE ONLY] E0 W0"),
            "{refused}"
        );
    }

    /// Proof for `tui.verification_is_typed_and_non_blocking`: the run sheet
    /// exposes trust and policy inputs before work, and every resolver terminal
    /// state renders from typed data with provider cost kept shared.
    /// Proof for `evidence.verification_reports_provider_provenance` (`A1-645`).
    #[test]
    fn verification_run_sheet_and_results_keep_every_state_explicit() {
        let mut model = model();
        model.update(Action::OpenVerifyRunSheet);
        let sheet = buffer(&mut model, 100, 24);
        for expected in [
            "execute configured tests: off [explicit trust action]",
            "RUN VERIFY",
            "policy: effort=required",
            "trust: configured processes",
            "Resolve-only may run test.list",
            "progress: active phase/provider ordinal",
            "cancel: queued work only",
            "fix: exact repairs are a separate guarded palette action",
        ] {
            assert!(
                sheet.contains(expected),
                "run sheet omitted `{expected}`:\n{sheet}"
            );
        }
        model.update(Action::ToggleVerifyRunSheet(0));
        let request = model
            .update(Action::ToggleVerifyRunSheet(1))
            .effect
            .expect("verification request");
        assert!(request.execute, "execute is an explicit request field");

        let outcomes = citation_outcomes();
        model.verification = Some(verification_fixture(&model, &outcomes));
        model.modal = Some(Modal::Verification);
        for (index, outcome) in outcomes.into_iter().enumerate() {
            model.update(Action::SelectModal(index));
            let result = buffer(&mut model, 100, 24);
            assert!(result.contains("DENIED [EXECUTE] E1 W2"));
            assert!(result.contains("scope: selected_tasks"), "{result}");
            assert!(
                result.contains("source: direct completeness: complete"),
                "{result}"
            );
            assert_eq!(
                model
                    .verification
                    .as_deref()
                    .expect("verification")
                    .observation
                    .state,
                aplan_app::EvidenceObservationState::Current
            );
            assert!(result.contains(&format!("result: {}", outcome.label())));
            assert!(result.contains("cost (the `unit` test runner, shared): 0.250s"));
            assert!(result.contains("repairs: 1"));
            assert!(result.contains("choose `Apply verification repairs` in the palette"));
        }
        assert_receipt_provenance_is_visible(&mut model);
        model.update(Action::SelectModal(outcomes.len()));
        let provider = buffer(&mut model, 100, 24);
        assert!(provider.contains("provider cost 1/1: cost (the `unit` test runner, shared)"));
        model.update(Action::SelectModal(0));
        let first = buffer(&mut model, 100, 24);
        assert!(
            first.contains("replace test:suite::0 with test:replacement-0"),
            "{first}"
        );

        model.update(Action::Resize {
            columns: 48,
            rows: 14,
        });
        model.update(Action::AdjustModalScroll(100));
        let scrolled = buffer(&mut model, 48, 14);
        assert!(scrolled.contains("cost (the `unit` test runner"));

        let verification = model.verification.as_deref_mut().expect("verification");
        verification.citations.clear();
        verification.run.findings = vec![aplan_app::OverviewFinding {
            identity: "verify-finding".to_string(),
            scope: aplan_app::DiagnosticScope::Verify,
            code: "APLAN013".to_string(),
            severity: aplan_app::Severity::Error,
            subject: "task T-9".to_string(),
            task: Some("T-9".to_string()),
            file: Some("aplan/tasks/T-9.md".into()),
            line: Some(7),
            column: Some(3),
            message: "the cited test did not resolve".to_string(),
            agent_instruction: Some("cite the exact listed test name".to_string()),
            detail: vec!["resolver returned no matching name".to_string()],
            contribution: aplan_app::VerdictContribution::Blocking,
            source_line: None,
        }];
        model.update(Action::SelectModal(0));
        let finding = buffer(&mut model, 100, 24);
        for expected in [
            "finding 1/1: error APLAN013 - task T-9",
            "location: aplan/tasks/T-9.md:7:3",
            "message: the cited test did not resolve",
            "action: cite the exact listed test name",
            "detail: resolver returned no matching name",
        ] {
            assert!(
                finding.contains(expected),
                "missing `{expected}`:\n{finding}"
            );
        }

        assert_empty_verification_outcomes_are_explicit(&mut model);
    }

    fn select_task(model: &mut Model, id: &str) {
        let index = model
            .task_browser
            .rows
            .iter()
            .position(|row| row.task.id == id)
            .expect("task remains visible");
        model.update(Action::SelectIndex(index));
    }

    /// Proof for `tui.shell_is_responsive_and_contextual`: dispatch, compact
    /// hints and both discovery surfaces enumerate one semantic catalogue.
    #[test]
    fn the_responsive_shell_uses_one_contextual_command_catalogue() {
        let mut model = model();
        model.update(Action::Resize {
            columns: 100,
            rows: 30,
        });
        let catalogue = action_states(&model);
        let boundary_count = discovery::COMMANDS
            .iter()
            .filter(|command| command.name != "facts" && command.disposition.boundary().is_some())
            .count();
        let nested_boundary_count = discovery::COMMANDS
            .iter()
            .map(|command| command.boundaries.len())
            .sum::<usize>();
        assert_eq!(
            catalogue.len(),
            CommandId::ALL.len()
                + MutationAction::ALL.len()
                + boundary_count
                + nested_boundary_count
        );
        for state in &catalogue {
            match state.target {
                PaletteTarget::Command(id) if state.unavailable.is_none() => {
                    assert!(id.action(&model).is_some(), "{id:?}");
                }
                PaletteTarget::Mutation(action) => {
                    assert_eq!(state.aliases, action.discovery_aliases());
                }
                PaletteTarget::Boundary(name) => {
                    let (reason, example) = discovery::boundary(name).expect("catalogued boundary");
                    let refusal = state.unavailable.as_deref().expect("boundary disabled");
                    assert!(refusal.contains(reason));
                    assert!(refusal.contains(example));
                    assert!(state.label.contains(example));
                }
                PaletteTarget::Command(_) => {}
            }
        }
        let footer = footer_states(&model);
        assert!(footer.iter().all(|state| catalogue.contains(state)));

        for (index, state) in catalogue.iter().enumerate() {
            model.update(Action::OpenHelp);
            model.update(Action::SelectModal(index));
            let help = buffer(&mut model, 100, 30);
            let current = action_states(&model);
            let current = current.get(index).expect("help retains the catalogue row");
            assert_eq!(current.target, state.target);
            let rendered = palette_line(
                current,
                true,
                palette(model.theme),
                usize::from(modal_area(Rect::new(0, 0, 100, 30)).width.saturating_sub(2)),
                false,
            )
            .to_string();
            assert!(help.contains(&rendered), "help omitted {:?}", state.target);
            model.update(Action::CloseLayer);
            model.update(Action::OpenPalette);
            model.update(Action::SelectModal(index));
            let palette = buffer(&mut model, 100, 30);
            let current = palette_states(&model);
            let current = current
                .get(index)
                .expect("palette retains the catalogue row");
            assert_eq!(current.target, state.target);
            let rendered = palette_line(
                current,
                true,
                super::palette(model.theme),
                usize::from(modal_area(Rect::new(0, 0, 100, 30)).width.saturating_sub(2)),
                false,
            )
            .to_string();
            assert!(
                palette.contains(&rendered),
                "palette omitted {:?}",
                state.target
            );
            model.update(Action::CloseLayer);
        }
        model.update(Action::OpenPalette);
        assert_eq!(
            decode_key(
                KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE),
                &model
            ),
            Some(Action::PaletteInput("4".to_string())),
            "text input owns printable route keys while the palette is open"
        );
        model.update(Action::CloseLayer);
        assert_eq!(
            decode_key(
                KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE),
                &model
            ),
            Some(Action::Route(Route::Graph))
        );
    }

    /// Proof for `tui.input_accessibility_is_safe_and_discoverable`: the
    /// painted label, accepted chords and repeat policy are one value, layers
    /// restore focus exactly, and non-text paste cannot become a shortcut.
    #[test]
    #[allow(clippy::too_many_lines)] // One lifecycle audit must exercise every binding and every focus-restoring layer.
    fn input_accessibility_uses_one_key_lifecycle_and_restores_focus() {
        let mut shell = model();
        shell.update(Action::Resize {
            columns: 100,
            rows: 24,
        });
        let states = command_states(&shell);
        for id in CommandId::ALL {
            let binding = id.binding();
            let state = states
                .iter()
                .find(|state| state.id == id)
                .expect("every command has a catalogue state");
            assert_eq!(state.keys, binding.label, "{id:?}");
            let chords = match binding.chords {
                BindingChords::None => [None, None, None],
                BindingChords::One(first) => [Some(first), None, None],
                BindingChords::Two(first, second) => [Some(first), Some(second), None],
                BindingChords::Three(first, second, third) => {
                    [Some(first), Some(second), Some(third)]
                }
            };
            for chord in chords.into_iter().flatten() {
                let press = KeyEvent::new(chord.code, chord.modifiers);
                assert!(binding.accepts(press), "{id:?} rejected its painted chord");
                let mut repeat = press;
                repeat.kind = KeyEventKind::Repeat;
                assert_eq!(binding.accepts(repeat), binding.repeatable, "{id:?}");
                let mut release = press;
                release.kind = KeyEventKind::Release;
                assert!(!binding.accepts(release), "{id:?} accepted key release");
            }
        }

        for modifiers in [
            KeyModifiers::SHIFT,
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ] {
            assert_eq!(
                decode_key(KeyEvent::new(KeyCode::Down, modifiers), &shell),
                None,
                "modified arrows are not unadvertised navigation"
            );
        }
        shell.update(Action::OpenHelp);
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL), &shell),
            None,
            "a modified arrow cannot reach through the modal catalogue"
        );
        shell.update(Action::CloseLayer);

        shell.modal = Some(Modal::RuleExplanation);
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), &shell),
            Some(Action::AdjustModalScroll(5))
        );
        shell.update(Action::AdjustModalScroll(5));
        assert_eq!(shell.modal_scroll, 5);
        shell.update(Action::CloseLayer);

        let openers = [
            Action::OpenAbout,
            Action::OpenHelp,
            Action::OpenPalette,
            Action::OpenSearch,
            Action::OpenReportDate,
            Action::OpenFilters,
            Action::OpenRunSheet,
            Action::OpenVerifyRunSheet,
            Action::OpenRuleSearch,
        ];
        for focus in Focus::ALL {
            for opener in &openers {
                let mut layered = model();
                layered.update(Action::Resize {
                    columns: 100,
                    rows: 24,
                });
                layered.update(Action::Focus(focus));
                layered.update(opener.clone());
                assert!(layered.modal.is_some(), "{opener:?} did not open a layer");
                layered.update(Action::CloseLayer);
                assert_eq!(layered.focus, focus, "{opener:?} did not restore focus");
            }

            let mut mutation = model();
            mutation.update(Action::Focus(focus));
            assert!(
                mutation
                    .update(Action::OpenMutation(MutationAction::Add))
                    .effect
                    .is_some()
            );
            mutation.update(Action::CloseLayer);
            assert_eq!(
                mutation.focus, focus,
                "mutation close did not restore focus"
            );
        }

        let route = shell.route;
        let focus = shell.focus;
        shell.update(Action::Paste("q\n\u{1b}[2J".to_string()));
        assert_eq!((shell.route, shell.focus), (route, focus));
        assert!(
            !shell.should_quit(),
            "paste outside an editor cannot invoke quit"
        );
        shell.update(Action::FocusLost);
        shell.update(Action::FocusGained);
        assert_eq!((shell.route, shell.focus), (route, focus));
    }

    /// Proof for `tui.input_accessibility_is_safe_and_discoverable`: pointer
    /// accelerators target the painted layer and region, clamp at boundaries,
    /// and drag can select but never activate a control.
    #[test]
    #[allow(clippy::too_many_lines)] // One layered pointer journey distinguishes selection, scrolling and blocked activation.
    fn pointer_accelerators_are_layered_clamped_and_nondestructive() {
        let pointer = |kind, position: Position, modifiers| MouseEvent {
            kind,
            column: position.x,
            row: position.y,
            modifiers,
        };
        let mut model = model();
        model.update(Action::Resize {
            columns: 100,
            rows: 24,
        });
        model.update(Action::Route(Route::Overview));
        let layout = ShellLayout::new(Rect::new(0, 0, 100, 24), &model);
        let primary = layout.primary.inner(Margin::new(1, 1));
        let row = Position::new(primary.x, primary.y);

        assert_eq!(
            decode_mouse(
                pointer(MouseEventKind::ScrollDown, row, KeyModifiers::NONE),
                &model
            ),
            Some(Action::SelectIndex(1)),
            "Overview wheel bounds come from metrics, not the task count"
        );
        model.update(Action::SelectIndex(OverviewMetric::ALL.len() - 1));
        assert_eq!(
            decode_mouse(
                pointer(MouseEventKind::ScrollDown, row, KeyModifiers::NONE),
                &model
            ),
            None
        );
        assert_eq!(
            decode_mouse(
                pointer(MouseEventKind::ScrollUp, row, KeyModifiers::NONE),
                &model
            ),
            Some(Action::SelectIndex(OverviewMetric::ALL.len() - 2))
        );

        model.update(Action::Route(Route::Tasks));
        let layout = ShellLayout::new(Rect::new(0, 0, 100, 24), &model);
        let tasks = task_viewport(
            layout.primary,
            false,
            model.selected,
            model.task_browser.rows.len(),
        );
        let task_row = Position::new(
            tasks.content.x,
            tasks
                .content
                .y
                .saturating_add(tasks.intro_rows)
                .saturating_add(1),
        );
        assert_eq!(
            decode_mouse(
                pointer(
                    MouseEventKind::Down(MouseButton::Left),
                    task_row,
                    KeyModifiers::NONE
                ),
                &model
            ),
            Some(Action::SelectIndex(1)),
            "a left-button press accelerates the same row selection available by keyboard"
        );
        assert_eq!(
            decode_mouse(
                pointer(
                    MouseEventKind::Drag(MouseButton::Left),
                    task_row,
                    KeyModifiers::NONE
                ),
                &model
            ),
            Some(Action::SelectIndex(1))
        );
        let footer_control = layout
            .footer_targets
            .first()
            .expect("painted footer action")
            .area;
        assert_eq!(
            decode_mouse(
                pointer(
                    MouseEventKind::Drag(MouseButton::Left),
                    Position::new(footer_control.x, footer_control.y),
                    KeyModifiers::NONE
                ),
                &model
            ),
            None,
            "drag cannot activate a footer command"
        );
        for kind in [
            MouseEventKind::Up(MouseButton::Left),
            MouseEventKind::Moved,
            MouseEventKind::Down(MouseButton::Right),
        ] {
            assert_eq!(
                decode_mouse(pointer(kind, task_row, KeyModifiers::NONE), &model),
                None
            );
        }
        assert_eq!(
            decode_mouse(
                pointer(
                    MouseEventKind::Down(MouseButton::Left),
                    task_row,
                    KeyModifiers::ALT
                ),
                &model
            ),
            None,
            "modified pointer events are not hidden accelerators"
        );

        model.update(Action::OpenHelp);
        let overlay = modal_area(Rect::new(0, 0, 100, 24));
        let overlay_row = Position::new(overlay.x.saturating_add(1), overlay.y.saturating_add(1));
        assert_eq!(
            decode_mouse(
                pointer(MouseEventKind::ScrollUp, overlay_row, KeyModifiers::NONE),
                &model
            ),
            None
        );
        assert_eq!(
            decode_mouse(
                pointer(MouseEventKind::ScrollDown, overlay_row, KeyModifiers::NONE),
                &model
            ),
            Some(Action::SelectModal(1))
        );
        assert_eq!(
            decode_mouse(
                pointer(
                    MouseEventKind::Down(MouseButton::Left),
                    task_row,
                    KeyModifiers::NONE
                ),
                &model
            ),
            None,
            "an overlay prevents click-through"
        );
        model.update(Action::CloseLayer);

        let prepare = model
            .update(Action::OpenMutation(MutationAction::Add))
            .effect
            .expect("loading mutation");
        assert_eq!(
            decode_mouse(
                pointer(
                    MouseEventKind::Down(MouseButton::Left),
                    task_row,
                    KeyModifiers::NONE
                ),
                &model
            ),
            None,
            "a mutation layer prevents click-through"
        );
        assert_eq!(
            decode_mouse(
                pointer(MouseEventKind::ScrollDown, task_row, KeyModifiers::NONE),
                &model
            ),
            None,
            "loading has no scroll target and cannot move the hidden list"
        );
        model.update(Action::CloseLayer);
        model.update(Action::EffectStarted(prepare.id));

        let mut reports = reports_model();
        reports.update(Action::Resize {
            columns: 100,
            rows: 24,
        });
        reports.update(Action::Focus(Focus::Detail));
        let report_layout = ShellLayout::new(Rect::new(0, 0, 100, 24), &reports);
        let detail = report_layout
            .detail
            .expect("normal layout detail")
            .inner(Margin::new(1, 1));
        let detail_row = Position::new(detail.x, detail.y);
        if reports.report_rows().len() > 1 {
            assert_eq!(
                decode_mouse(
                    pointer(MouseEventKind::ScrollDown, detail_row, KeyModifiers::NONE),
                    &reports
                ),
                Some(Action::SelectReportRow(1))
            );
            reports.update(Action::SelectReportRow(reports.report_rows().len() - 1));
            assert_eq!(
                decode_mouse(
                    pointer(MouseEventKind::ScrollDown, detail_row, KeyModifiers::NONE),
                    &reports
                ),
                None,
                "detail wheel stops at the final report row"
            );
        }
    }

    /// Proof for `tui.overlay_pointer_activation_is_semantic_and_guarded`:
    /// modal rows, fixed controls and mutation fields all expose typed targets
    /// from their painted geometry, while borders, button rows and unsafe
    /// event shapes remain inert at the minimum supported size.
    #[test]
    #[allow(clippy::too_many_lines)] // One journey compares every overlay state against its shared geometry.
    fn overlay_pointer_targets_share_painted_geometry_and_preserve_guards() {
        let pointer = |kind, position: Position, modifiers| MouseEvent {
            kind,
            column: position.x,
            row: position.y,
            modifiers,
        };
        let left = |position| {
            pointer(
                MouseEventKind::Down(MouseButton::Left),
                position,
                KeyModifiers::NONE,
            )
        };
        let mut model = model();
        model.update(Action::Resize {
            columns: 48,
            rows: 14,
        });
        model.update(Action::Route(Route::Tasks));
        model.update(Action::OpenSearch);
        let view = buffer(&mut model, 48, 14);
        assert!(view.contains("[Cancel] [Clear] [Apply]"), "{view}");
        let controls = modal_controls(&model, Modal::Search);
        let packed = pack_overlay_controls(&controls, 38);
        let layout = overlay_layout(Rect::new(0, 0, 48, 14), packed.len());
        let clear = overlay_control_targets(&packed, layout.controls)
            .into_iter()
            .find(|target| target.value == Action::SearchClear)
            .expect("painted clear control");
        let clear_position = Position::new(clear.area.x, clear.area.y);
        assert_eq!(
            decode_mouse(left(clear_position), &model),
            Some(Action::SearchClear)
        );
        assert_eq!(
            decode_mouse(
                pointer(
                    MouseEventKind::Down(MouseButton::Right),
                    clear_position,
                    KeyModifiers::NONE,
                ),
                &model,
            ),
            None
        );
        assert_eq!(
            decode_mouse(
                pointer(
                    MouseEventKind::Down(MouseButton::Left),
                    clear_position,
                    KeyModifiers::CONTROL,
                ),
                &model,
            ),
            None
        );
        assert_eq!(
            decode_mouse(
                pointer(
                    MouseEventKind::ScrollDown,
                    clear_position,
                    KeyModifiers::NONE
                ),
                &model,
            ),
            None,
            "a fixed control row is not a scrollable region"
        );
        assert_eq!(
            decode_mouse(left(Position::new(layout.outer.x, layout.outer.y)), &model),
            None,
            "an overlay border is not an implicit action"
        );

        model.update(Action::CloseLayer);
        model.update(Action::OpenFilters);
        let controls = modal_controls(&model, Modal::Filters);
        let packed = pack_overlay_controls(&controls, 38);
        let layout = overlay_layout(Rect::new(0, 0, 48, 14), packed.len());
        let content = modal_content(
            &model,
            Modal::Filters,
            palette(model.theme),
            usize::from(layout.content.width),
            usize::from(layout.content.height),
        );
        let row = content.targets.first().expect("visible filter row");
        let row_position = Position::new(
            layout.content.x,
            layout
                .content
                .y
                .saturating_add(u16::try_from(row.first).expect("row fits")),
        );
        assert_eq!(
            decode_mouse(left(row_position), &model),
            Some(row.action.clone())
        );
        assert!(matches!(row.action, Action::SelectModal(_)));
        let toggle = overlay_control_targets(&packed, layout.controls)
            .into_iter()
            .find(|target| target.value == Action::ToggleFilter(model.modal_selected))
            .expect("painted filter toggle control");
        assert_eq!(
            decode_mouse(left(Position::new(toggle.area.x, toggle.area.y)), &model),
            Some(Action::ToggleFilter(model.modal_selected))
        );
        assert_eq!(
            decode_mouse(left(Position::new(0, model.rows.saturating_sub(1))), &model,),
            None,
            "the top overlay prevents click-through to the footer"
        );

        model.update(Action::CloseLayer);
        model.update(Action::Resize {
            columns: 100,
            rows: 30,
        });
        let prepare = model
            .update(Action::OpenMutation(MutationAction::Add))
            .effect
            .expect("prepare mutation");
        let form = MutationForm::new(
            "Pointer form",
            model.snapshot.overview.input.sha256.clone(),
            false,
            vec![
                FieldSpec::new("title", "Title", "required value").required(),
                FieldSpec::new("tags", "Tags", "ordered values")
                    .repeated()
                    .initial_values(["one", "two"]),
            ],
        )
        .expect("pointer form");
        model.update(Action::EffectCompleted(Box::new(EffectCompletion {
            request: prepare,
            outcome: Ok(EffectOutput::Mutation(MutationTransfer::new(
                MutationPayload::Prepared(PreparedMutation::Form(Box::new(form))),
            ))),
        })));
        let controls = mutation_controls(&model);
        let packed = pack_overlay_controls(&controls, 84);
        let layout = overlay_layout(Rect::new(0, 0, 100, 30), packed.len());
        let form = match &model.mutation.as_ref().expect("mutation").state {
            MutationState::Form(form) => form,
            other => panic!("unexpected mutation state: {other:?}"),
        };
        let (item_row, item_action) = form
            .pointer_rows(
                usize::from(layout.content.width),
                usize::from(layout.content.height),
            )
            .into_iter()
            .find(|(_, action)| *action == FormAction::FocusItem { field: 1, item: 1 })
            .expect("second repeated item is visible");
        let item_position = Position::new(
            layout.content.x,
            layout
                .content
                .y
                .saturating_add(u16::try_from(item_row).expect("row fits")),
        );
        assert_eq!(
            decode_mouse(left(item_position), &model),
            Some(Action::MutationForm(item_action.clone()))
        );
        model.update(Action::MutationForm(item_action));
        let form = match &model.mutation.as_ref().expect("mutation").state {
            MutationState::Form(form) => form,
            other => panic!("unexpected mutation state: {other:?}"),
        };
        assert_eq!(form.focus(), crate::form::FormFocus::Field(1));
        assert_eq!(form.fields()[1].selected(), 1);

        let review = overlay_control_targets(&packed, layout.controls)
            .into_iter()
            .find(|target| target.value == Action::MutationForm(FormAction::Review))
            .expect("painted review control");
        assert_eq!(
            decode_mouse(left(Position::new(review.area.x, review.area.y)), &model),
            Some(Action::MutationForm(FormAction::Review))
        );

        let layer = model.mutation.as_mut().expect("mutation layer");
        layer.state = MutationState::QuickReview {
            candidate: None,
            form: None,
            rows: vec!["PREVIEW: pointer confirmation".to_string()],
        };
        let controls = mutation_controls(&model);
        let packed = pack_overlay_controls(&controls, 84);
        let layout = overlay_layout(Rect::new(0, 0, 100, 30), packed.len());
        let confirm = overlay_control_targets(&packed, layout.controls)
            .into_iter()
            .find(|target| target.value == Action::ConfirmMutation)
            .expect("painted quick confirmation");
        assert_eq!(
            decode_mouse(left(Position::new(confirm.area.x, confirm.area.y)), &model),
            Some(Action::ConfirmMutation)
        );

        let layer = model.mutation.as_mut().expect("mutation layer");
        layer.state = MutationState::Result {
            form: None,
            rows: vec!["Verdict: NOT PUBLISHED".to_string()],
        };
        let controls = mutation_controls(&model);
        let packed = pack_overlay_controls(&controls, 84);
        let layout = overlay_layout(Rect::new(0, 0, 100, 30), packed.len());
        let close = overlay_control_targets(&packed, layout.controls)
            .into_iter()
            .find(|target| target.value == Action::CloseLayer)
            .expect("painted result close control");
        assert_eq!(
            decode_mouse(left(Position::new(close.area.x, close.area.y)), &model),
            Some(Action::CloseLayer)
        );

        model.mutation.as_mut().expect("mutation layer").committing = true;
        assert!(mutation_controls(&model).is_empty());
        assert_eq!(
            decode_mouse(
                left(Position::new(layout.content.x, layout.content.y)),
                &model
            ),
            None,
            "publishing exposes no pointer activation target"
        );

        for width in [38, 84] {
            let arm_action = Action::MutationForm(FormAction::PointerConfirm(
                Activation::from_key_kind(KeyEventKind::Press),
            ));
            let unarmed = pack_overlay_controls(
                &[
                    OverlayControl::new("Arm broad change", arm_action.clone()),
                    OverlayControl::new("Cancel", Action::MutationForm(FormAction::Cancel)),
                    OverlayControl::new("Recompute", Action::MutationForm(FormAction::Recompute)),
                ],
                width,
            );
            let armed = pack_overlay_controls(
                &[
                    OverlayControl::inert("Broad change armed"),
                    OverlayControl::new("Cancel", Action::MutationForm(FormAction::Cancel)),
                    OverlayControl::new("Recompute", Action::MutationForm(FormAction::Recompute)),
                    OverlayControl::new("Confirm broad change", arm_action.clone()),
                ],
                width,
            );
            let shell_width = u16::try_from(width + 2).expect("test width");
            let unarmed_layout = overlay_layout(
                Rect::new(0, 0, shell_width.saturating_add(8), 14),
                unarmed.len(),
            );
            let armed_layout = overlay_layout(
                Rect::new(0, 0, shell_width.saturating_add(8), 14),
                armed.len(),
            );
            let old_arm = overlay_control_targets(&unarmed, unarmed_layout.controls)
                .into_iter()
                .find(|target| target.value == arm_action)
                .expect("unarmed target");
            let repeated_position = Position::new(old_arm.area.x, old_arm.area.y);
            assert!(
                overlay_control_targets(&armed, armed_layout.controls)
                    .iter()
                    .all(|target| !target.area.contains(repeated_position)),
                "a rapid second press at the arm coordinate reached an armed action at width {width}"
            );
        }
    }

    /// Proof for `tui.action_discovery_is_closed_and_contextual`: every
    /// product command is searchable through the same rows used by help and
    /// the footer, while inert boundaries return copyable advice rather than
    /// spawning a process.
    #[test]
    #[allow(clippy::too_many_lines)] // One closed catalogue pass keeps each command beside its exact UI target.
    fn every_product_command_resolves_to_one_discoverable_action_or_boundary() {
        let mut model = model();
        model.update(Action::Resize {
            columns: 120,
            rows: 36,
        });
        let states = action_states(&model);
        for command in discovery::COMMANDS {
            let matches = states
                .iter()
                .filter(|state| state.aliases.contains(&command.name))
                .collect::<Vec<_>>();
            match command.disposition {
                crate::discovery::CommandDisposition::Native => {
                    assert!(
                        !matches.is_empty(),
                        "native command `{}` has no discoverable action",
                        command.name
                    );
                    for surface in command.surfaces {
                        let represented = matches.iter().any(|state| match surface {
                            crate::discovery::NativeSurface::Diagnostics(_) => {
                                state.target == PaletteTarget::Command(CommandId::Diagnostics)
                            }
                            crate::discovery::NativeSurface::Verification => {
                                state.target == PaletteTarget::Command(CommandId::Verify)
                            }
                            crate::discovery::NativeSurface::TestCatalogue => {
                                state.target == PaletteTarget::Command(CommandId::TestCatalogue)
                            }
                            crate::discovery::NativeSurface::Work => {
                                state.target
                                    == PaletteTarget::Command(if command.name == "why" {
                                        CommandId::WhyTraversal
                                    } else {
                                        CommandId::Tasks
                                    })
                            }
                            crate::discovery::NativeSurface::Report(_) => {
                                state.target == PaletteTarget::Command(CommandId::Reports)
                            }
                            crate::discovery::NativeSurface::Graph => {
                                state.target == PaletteTarget::Command(CommandId::Graph)
                            }
                            crate::discovery::NativeSurface::Inspector => {
                                state.target == PaletteTarget::Command(CommandId::Open)
                            }
                            crate::discovery::NativeSurface::Explanation => matches!(
                                state.target,
                                PaletteTarget::Command(
                                    CommandId::FindRule | CommandId::ExplainRule
                                )
                            ),
                            crate::discovery::NativeSurface::Tasks => {
                                state.target == PaletteTarget::Command(CommandId::Tasks)
                            }
                            crate::discovery::NativeSurface::Inbox => {
                                state.target == PaletteTarget::Command(CommandId::Inbox)
                            }
                        });
                        assert!(
                            represented,
                            "native command `{}` has no action for {surface:?}",
                            command.name
                        );
                    }
                    for action in command.mutations {
                        assert!(
                            matches.iter().any(|state| {
                                state.target == PaletteTarget::Mutation(*action)
                                    && state.aliases.contains(&command.name)
                            }),
                            "native command `{}` lost mutation {action:?}",
                            command.name
                        );
                    }
                }
                crate::discovery::CommandDisposition::Guided { .. } if command.name == "facts" => {
                    assert_eq!(matches.len(), 1);
                    assert_eq!(matches[0].target, PaletteTarget::Command(CommandId::About));
                }
                crate::discovery::CommandDisposition::Guided { .. }
                | crate::discovery::CommandDisposition::CliOnly { .. }
                | crate::discovery::CommandDisposition::EntryPoint { .. } => {
                    assert_eq!(matches.len(), 1, "boundary `{}`", command.name);
                    assert_eq!(matches[0].target, PaletteTarget::Boundary(command.name));
                }
            }
            for boundary in command.boundaries {
                let matches = states
                    .iter()
                    .filter(|state| state.target == PaletteTarget::Boundary(boundary.name))
                    .collect::<Vec<_>>();
                assert_eq!(matches.len(), 1, "nested boundary `{}`", boundary.name);
                assert!(matches[0].label.contains(boundary.example));
                assert!(
                    matches[0]
                        .unavailable
                        .as_deref()
                        .is_some_and(|message| message.contains(boundary.reason))
                );
            }
        }

        model.update(Action::OpenPalette);
        model.update(Action::PaletteInput("export-roadmap-json".to_string()));
        let matches = palette_states(&model);
        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].target,
            PaletteTarget::Boundary("export-roadmap-json")
        );
        let action = decode_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &model)
            .expect("disabled boundary explains itself");
        let Action::Notify(message) = action else {
            panic!("CLI-only discovery tried to invoke an action")
        };
        assert!(message.contains("aplan export-roadmap-json --check roadmap.json"));
        model.update(Action::CloseLayer);

        model.update(Action::OpenPalette);
        model.update(Action::PaletteInput("agent contract".to_string()));
        let matches = palette_states(&model);
        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].target,
            PaletteTarget::Boundary("export-agents-md")
        );
        model.update(Action::CloseLayer);

        model.update(Action::OpenPalette);
        model.update(Action::PaletteInput("facts".to_string()));
        let matches = palette_states(&model);
        assert_eq!(matches.len(), 2);
        let about = matches
            .iter()
            .position(|state| state.target == PaletteTarget::Command(CommandId::About))
            .expect("facts opens About as well as advertising its assertion boundary");
        model.update(Action::SelectModal(about));
        let action = decode_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &model)
            .expect("guided facts action");
        model.update(action);
        assert_eq!(model.modal, Some(Modal::About));
    }

    /// Proof for `tui.configured_test_selector_is_reachable`.
    #[test]
    fn configured_test_palette_action_has_a_searchable_selectable_modal() {
        let mut model = model();
        model.update(Action::Resize {
            columns: 100,
            rows: 24,
        });
        model.update(Action::OpenPalette);
        model.update(Action::PaletteInput("test list".to_string()));
        let states = palette_states(&model);
        assert_eq!(states.len(), 1, "test discovery must have one live owner");
        assert_eq!(
            states[0].target,
            PaletteTarget::Command(CommandId::TestCatalogue)
        );
        let action = decode_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &model)
            .expect("palette test action");
        let transition = model.update(action);
        assert_eq!(
            transition.effect.as_ref().map(|effect| effect.kind),
            Some(EffectKind::TestCatalogue)
        );
        assert_eq!(model.modal, Some(Modal::TestCatalogue));

        let loading = buffer(&mut model, 100, 24);
        assert!(loading.contains("Listing configured runners"), "{loading}");
        model.test_catalogue = Some(crate::model::TestCatalogueLayer::Failed(
            "runner launch was refused".to_string(),
        ));
        let failed = buffer(&mut model, 100, 24);
        assert!(failed.contains("UNAVAILABLE"), "{failed}");
        assert!(failed.contains("runner launch was refused"), "{failed}");
        model.test_catalogue = None;
        let absent = buffer(&mut model, 100, 24);
        assert!(absent.contains("Catalogue is not loaded"), "{absent}");

        model.test_catalogue = Some(crate::model::TestCatalogueLayer::Ready(Box::new(
            crate::evidence::TestCatalogueView {
                catalogue: aplan_app::TestCatalogue {
                    state: aplan_app::TestCatalogueState::Partial,
                    query: None,
                    summary: aplan_app::TestCatalogueSummary {
                        runners: 2,
                        listed_runners: 1,
                        empty_runners: 0,
                        failed_runners: 1,
                        unique_names: 1,
                        runner_name_pairs: 1,
                        matched_names: 1,
                        matched_pairs: 1,
                        ambiguous_names: 0,
                    },
                    runners: Vec::new(),
                    findings: vec![aplan_app::TestCatalogueFinding {
                        code: "APLAN013".to_string(),
                        severity: "warning".to_string(),
                        file: None,
                        message: "runner `broken` failed".to_string(),
                        instruction: Some("repair the configured runner".to_string()),
                        advisory: false,
                        blocking: true,
                    }],
                    denied: true,
                },
                choices: vec![crate::evidence::TestEvidenceChoice {
                    name: "suite::proof".to_string(),
                    reference: "suite::proof".to_string(),
                    citation: "test:suite::proof".to_string(),
                    selectable: true,
                    advice: "accepted reference: use `suite::proof` unchanged".to_string(),
                }],
            },
        )));
        let view = buffer(&mut model, 100, 24);
        for expected in [
            "Configured tests",
            "Partial: 1/2 runner(s) answered",
            "APLAN013: runner `broken` failed",
            "test:suite::proof",
            "accepted reference",
        ] {
            assert!(
                view.contains(expected),
                "missing `{expected}` from:\n{view}"
            );
        }
        assert_eq!(
            decode_key(
                KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
                &model
            ),
            Some(Action::SearchInput("p".to_string()))
        );
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &model),
            Some(Action::UseTestEvidence)
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)] // Each refusal needs the same complete mutation catalogue as its control case.
    fn mutation_discovery_explains_every_contextual_refusal() {
        let mutation_states = |model: &Model| {
            action_states(model)
                .into_iter()
                .filter(|state| matches!(state.target, PaletteTarget::Mutation(_)))
                .collect::<Vec<_>>()
        };

        let onboarding = onboarding_model(aplan_app::OnboardingCause::MissingPlan);
        assert!(mutation_states(&onboarding).iter().all(|state| {
            state
                .unavailable
                .as_deref()
                .is_some_and(|reason| reason.contains("until a plan loads"))
        }));

        let mut undersized = model();
        undersized.update(Action::Resize {
            columns: MIN_COLUMNS - 1,
            rows: MIN_ROWS - 1,
        });
        assert!(mutation_states(&undersized).iter().all(|state| {
            state
                .unavailable
                .as_deref()
                .is_some_and(|reason| reason.contains("terminal must be at least"))
        }));

        let mut saturated = model();
        saturated.update(Action::Resize {
            columns: 100,
            rows: 30,
        });
        assert!(saturated.update(Action::Refresh).effect.is_some());
        assert!(saturated.update(Action::Verify).effect.is_some());
        assert_eq!(
            saturated.pending_effects(),
            crate::model::MAX_PENDING_EFFECTS
        );
        assert!(mutation_states(&saturated).iter().all(|state| {
            state
                .unavailable
                .as_deref()
                .is_some_and(|reason| reason.contains("effect queue is full"))
        }));

        let mut stale_selection = model();
        stale_selection.update(Action::Resize {
            columns: 100,
            rows: 30,
        });
        stale_selection.selected_id = Some("not-loaded".to_string());
        for state in mutation_states(&stale_selection) {
            let PaletteTarget::Mutation(action) = state.target else {
                unreachable!("filtered to mutation states")
            };
            assert_eq!(
                state.unavailable.as_deref(),
                if action.needs_selected_task() {
                    Some("select a loaded task in Tasks or Graph first")
                } else if action.needs_selected_capture() {
                    Some("select the capture in the Inbox destination first")
                } else {
                    None
                }
            );
        }

        let mut no_selection = model();
        no_selection.update(Action::Resize {
            columns: 100,
            rows: 30,
        });
        no_selection.selected_id = None;
        let no_selection_states = mutation_states(&no_selection);
        for action in MutationAction::ALL {
            let state = no_selection_states
                .iter()
                .find(|state| state.target == PaletteTarget::Mutation(action))
                .expect("every mutation remains discoverable without selection");
            if action.needs_selected_task() {
                assert_eq!(
                    state.unavailable.as_deref(),
                    Some("select a loaded task in Tasks or Graph first")
                );
            } else if action.needs_selected_capture() {
                assert_eq!(
                    state.unavailable.as_deref(),
                    Some("select the capture in the Inbox destination first")
                );
            } else {
                assert!(state.unavailable.is_none());
            }
        }

        let mut available = model();
        available.update(Action::Resize {
            columns: 100,
            rows: 30,
        });
        for state in mutation_states(&available) {
            let PaletteTarget::Mutation(action) = state.target else {
                unreachable!("filtered to mutation states")
            };
            if action.needs_selected_capture() {
                assert_eq!(
                    state.unavailable.as_deref(),
                    Some("select the capture in the Inbox destination first")
                );
            } else {
                assert!(state.unavailable.is_none(), "{action:?}: {state:?}");
            }
        }

        available.palette_query = "Enter".to_string();
        assert!(
            palette_states(&available)
                .iter()
                .all(|state| !matches!(state.target, PaletteTarget::Mutation(_))),
            "palette-only mutations must not advertise the palette's generic Enter key as a shortcut"
        );
        for action in MutationAction::ALL {
            available.palette_query = action.cli().to_string();
            assert!(
                palette_states(&available)
                    .iter()
                    .any(|state| state.target == PaletteTarget::Mutation(action)),
                "{} is not searchable by its CLI semantic path",
                action.cli()
            );
        }

        available.palette_query = "external CI provenance".to_string();
        let reason_matches = palette_states(&available);
        assert_eq!(reason_matches.len(), 1);
        assert_eq!(
            reason_matches[0].target,
            PaletteTarget::Boundary("test receipt")
        );

        available.palette_query = "--no-traces".to_string();
        let verify_policy = palette_states(&available);
        assert_eq!(verify_policy.len(), 1);
        assert_eq!(
            verify_policy[0].target,
            PaletteTarget::Boundary("verify --no-traces")
        );
        assert!(
            verify_policy[0]
                .unavailable
                .as_deref()
                .is_some_and(|reason| reason.contains("aplan verify --no-traces"))
        );

        available.palette_query = "capability assertion".to_string();
        let capability_assertion = palette_states(&available);
        assert_eq!(capability_assertion.len(), 1);
        assert_eq!(
            capability_assertion[0].target,
            PaletteTarget::Boundary("facts --require")
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)] // One stateful keyboard journey keeps focus-sensitive keys ordered.
    fn mutation_layer_keyboard_rendering_and_refusal_are_complete_and_fail_closed() {
        let mut model = model();
        model.update(Action::Resize {
            columns: 100,
            rows: 30,
        });
        let prepare = model
            .update(Action::OpenMutation(MutationAction::Add))
            .effect
            .expect("prepare request");
        let loading = buffer(&mut model, 100, 30);
        assert!(loading.contains("Preparing Add task"), "{loading}");
        assert!(loading.contains("No write occurs"), "{loading}");
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &model),
            Some(Action::CloseLayer)
        );
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &model),
            None,
            "loading must not invent a confirm action"
        );

        let form = MutationForm::new(
            "Test mutation",
            model.snapshot.overview.input.sha256.clone(),
            false,
            vec![
                FieldSpec::new("name", "Name", "required value").required(),
                FieldSpec::new("items", "Items", "ordered values").repeated(),
            ],
        )
        .expect("form");
        model.update(Action::EffectCompleted(Box::new(EffectCompletion {
            request: prepare,
            outcome: Ok(EffectOutput::Mutation(MutationTransfer::new(
                MutationPayload::Prepared(PreparedMutation::Form(Box::new(form))),
            ))),
        })));
        let rendered = buffer(&mut model, 100, 30);
        assert!(rendered.contains("Test mutation"), "{rendered}");
        assert!(rendered.contains("Name"), "{rendered}");
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &model),
            Some(Action::MutationForm(FormAction::NextFocus))
        );
        model.update(Action::MutationForm(FormAction::NextFocus));
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &model),
            Some(Action::MutationForm(FormAction::NextFocus))
        );
        model.update(Action::MutationForm(FormAction::NextFocus));
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &model),
            Some(Action::MutationForm(FormAction::Cancel))
        );
        model.update(Action::MutationForm(FormAction::NextFocus));
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &model),
            Some(Action::MutationForm(FormAction::Review))
        );
        for _ in 0..3 {
            model.update(Action::MutationForm(FormAction::PreviousFocus));
        }

        let cases = [
            (
                KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
                FormAction::NextFocus,
            ),
            (
                KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
                FormAction::PreviousFocus,
            ),
            (
                KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
                FormAction::PreviousItem,
            ),
            (
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                FormAction::NextItem,
            ),
            (
                KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
                FormAction::Scroll { delta: -5, from: 0 },
            ),
            (
                KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
                FormAction::Scroll { delta: 5, from: 0 },
            ),
            (
                KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
                FormAction::CursorLeft,
            ),
            (
                KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
                FormAction::CursorRight,
            ),
            (
                KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
                FormAction::CursorHome,
            ),
            (
                KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
                FormAction::CursorEnd,
            ),
            (
                KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
                FormAction::Backspace,
            ),
            (
                KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE),
                FormAction::Delete,
            ),
            (
                KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
                FormAction::AddItem,
            ),
            (
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
                FormAction::RemoveItem,
            ),
            (
                KeyEvent::new(KeyCode::Up, KeyModifiers::ALT),
                FormAction::MoveItemUp,
            ),
            (
                KeyEvent::new(KeyCode::Down, KeyModifiers::ALT),
                FormAction::MoveItemDown,
            ),
            (
                KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
                FormAction::Insert("x".to_string()),
            ),
        ];
        for (key, expected) in cases {
            assert_eq!(
                decode_key(key, &model),
                Some(Action::MutationForm(expected))
            );
        }
        let mut release = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert_eq!(decode_key(release, &model), None);
        for mut repeat in [
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Up, KeyModifiers::ALT),
            KeyEvent::new(KeyCode::Down, KeyModifiers::ALT),
        ] {
            repeat.kind = KeyEventKind::Repeat;
            assert_eq!(
                decode_key(repeat, &model),
                None,
                "key repeat cannot activate or structurally edit a form"
            );
        }
        let mut repeated_text = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        repeated_text.kind = KeyEventKind::Repeat;
        assert_eq!(
            decode_key(repeated_text, &model),
            Some(Action::MutationForm(FormAction::Insert("x".to_string())))
        );

        let area = modal_area(Rect::new(0, 0, model.columns, model.rows));
        assert_eq!(
            decode_mouse(
                MouseEvent {
                    kind: MouseEventKind::ScrollDown,
                    column: area.x.saturating_add(1),
                    row: area.y.saturating_add(1),
                    modifiers: KeyModifiers::NONE,
                },
                &model,
            ),
            Some(Action::MutationForm(FormAction::Scroll {
                delta: 3,
                from: 0
            }))
        );

        model.update(Action::MutationForm(FormAction::Insert(
            "complete".to_string(),
        )));
        model.update(Action::MutationForm(FormAction::NextFocus));
        model.update(Action::MutationForm(FormAction::NextFocus));
        model.update(Action::MutationForm(FormAction::NextFocus));
        let preview = model
            .update(Action::MutationForm(FormAction::Review))
            .effect
            .expect("preview request");
        let request = match preview.mutation.as_ref().expect("mutation effect") {
            crate::mutations::MutationEffect::Preview { submission, .. } => submission.request(),
            other => panic!("unexpected preview effect: {other:?}"),
        };
        model.update(Action::EffectCompleted(Box::new(EffectCompletion {
            request: preview,
            outcome: Ok(EffectOutput::Mutation(MutationTransfer::new(
                MutationPayload::PreviewFailed {
                    action: MutationAction::Add,
                    request,
                    message: "typed service refused these fields".to_string(),
                },
            ))),
        })));
        let refused = buffer(&mut model, 100, 30);
        assert!(refused.contains("typed service refused"), "{refused}");

        let layer = model.mutation.as_mut().expect("mutation layer");
        layer.state = MutationState::QuickReview {
            candidate: None,
            form: None,
            rows: (0..3).map(|index| format!("review row {index}")).collect(),
        };
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE), &model),
            Some(Action::MutationScroll(-5))
        );
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), &model),
            Some(Action::MutationScroll(5))
        );
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &model),
            Some(Action::ConfirmMutation)
        );
        let mut repeated_page = KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE);
        repeated_page.kind = KeyEventKind::Repeat;
        assert_eq!(decode_key(repeated_page, &model), None);
        let review = buffer(&mut model, 100, 30);
        assert!(review.contains("Enter Confirm"), "{review}");

        let layer = model.mutation.as_mut().expect("mutation layer");
        layer.state = MutationState::Result {
            form: None,
            rows: vec!["Verdict: NOT PUBLISHED".to_string()],
        };
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &model),
            None
        );
        let result = buffer(&mut model, 100, 30);
        assert!(result.contains("Verdict: NOT PUBLISHED"), "{result}");

        model.mutation.as_mut().expect("mutation layer").committing = true;
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &model),
            None,
            "publishing cannot be dismissed"
        );
        let publishing = buffer(&mut model, 100, 30);
        assert!(
            publishing.contains("publishing; quit waits"),
            "{publishing}"
        );
        assert!(
            publishing.contains("guarded write is running"),
            "{publishing}"
        );
    }

    #[test]
    fn every_shell_route_and_focus_has_a_keyboard_path() {
        let mut model = model();
        model.update(Action::Resize {
            columns: 100,
            rows: 24,
        });
        for (key, route) in ['1', '2', '3', '4', '5', '6'].into_iter().zip(Route::ALL) {
            let action = decode_key(
                KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE),
                &model,
            )
            .expect("route shortcut");
            model.update(action);
            assert_eq!(model.route, route);
        }

        model.update(Action::Focus(Focus::Navigation));
        let action = decode_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &model)
            .expect("next destination");
        model.update(action);
        assert_eq!(model.route, Route::Overview);
        assert_eq!(model.focus, Focus::Navigation);

        let action = decode_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &model)
            .expect("next focus");
        model.update(action);
        assert_eq!(model.focus, Focus::Primary);
        let action = decode_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &model)
            .expect("open selection");
        model.update(action);
        assert_eq!(model.route, Route::Tasks);
        assert_eq!(model.focus, Focus::Primary);
        let action = decode_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &model)
            .expect("return to overview");
        model.update(action);
        assert_eq!(model.route, Route::Overview);
        let action = decode_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &model)
            .expect("focus snapshot identity");
        model.update(action);
        assert_eq!(model.focus, Focus::Detail);
        model.update(Action::CloseLayer);
        assert_eq!(model.focus, Focus::Primary);
        assert!(!model.should_quit());

        for open in [
            decode_key(
                KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT),
                &model,
            )
            .expect("shifted help key"),
            Action::OpenPalette,
        ] {
            model.update(open);
            assert!(model.modal.is_some());
            let action = decode_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &model)
                .expect("cancel modal");
            model.update(action);
            assert!(model.modal.is_none());
            assert!(!model.should_quit());
        }
    }

    #[test]
    fn inbox_and_quick_capture_remain_keyboard_complete_narrow_and_non_colour() {
        use aplan_core::capture::{CaptureSource, CaptureState, RevisitFilter};

        let mut deferred = capture_record(
            "I-01K8Z3A4H00000000000000000",
            "Review pasted \u{754c} detail",
            CaptureState::Open,
            "2026-01-01T00:00:01Z",
        );
        deferred.body = Some("complete \u{754c} body".to_string());
        deferred.revisit_at = Some("2030-01-01T00:00:00Z".to_string());
        deferred.profile = Some("triage".to_string());
        deferred.source = Some(CaptureSource {
            namespace: "pilot".to_string(),
            key: "focused-key".to_string(),
        });
        let mut promoted = capture_record(
            "I-01K8Z3A4H00000000000000001",
            "Promoted intake",
            CaptureState::Promoted,
            "2026-01-01T00:00:02Z",
        );
        promoted.promoted_to = Some("A-1".to_string());
        let mut model = ModelBuilder::default()
            .glyphs(Glyphs::Ascii)
            .inbox(vec![deferred, promoted], "2026-01-01T00:00:10Z")
            .build();
        model.theme = Theme::Monochrome;
        model.update(Action::Route(Route::Inbox));
        model.inbox_state = None;
        model.inbox_revisit = RevisitFilter::All;
        model.selected_capture_id = Some("I-01K8Z3A4H00000000000000000".to_string());
        assert_eq!(
            CommandId::ClearNarrowing.label(&model),
            "clear capture search and filters"
        );

        let narrow = buffer(&mut model, 48, 14);
        assert!(narrow.contains("DEFERRED"), "{narrow}");
        assert!(narrow.is_ascii(), "ASCII mode emitted a non-ASCII cell");

        model.update(Action::OpenDetail);
        let detail = buffer(&mut model, 120, 24);
        for fact in [
            "State: OPEN (deferred from the actionable",
            " view)",
            "Captured at: 2026-01-01T00:00:01Z",
            "Origin: Tui",
            "Revisit at: 2030-01-01T00:00:00Z",
            "Profile: triage",
            "Source namespace: pilot",
            "Source key (focused only): focused-key",
        ] {
            assert!(detail.contains(fact), "missing `{fact}`:\n{detail}");
        }

        model.update(Action::Route(Route::Reports));
        model.update(Action::Focus(Focus::Detail));
        let action = decode_key(
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
            &model,
        )
        .expect("global quick-capture chord");
        assert_eq!(action, Action::OpenMutation(MutationAction::CaptureCreate));
        assert!(model.update(action).effect.is_some());
        assert_eq!(model.route, Route::Reports);
        assert_eq!(model.focus, Focus::Detail);
    }

    #[test]
    fn inbox_rendering_distinguishes_unavailable_empty_filtered_and_terminal_records() {
        use aplan_core::capture::{CaptureState, PromotionDefaults, RevisitFilter};

        let mut unavailable = ModelBuilder::default().build();
        unavailable.snapshot.inbox = Err("cannot read retained intake".to_string());
        unavailable.update(Action::Route(Route::Inbox));
        let rendered = buffer(&mut unavailable, 100, 18);
        assert!(
            rendered.contains("Inbox unavailable: cannot read retained intake"),
            "{rendered}"
        );

        let mut empty = ModelBuilder::default()
            .inbox(Vec::new(), "2026-01-01T00:00:10Z")
            .build();
        empty.update(Action::Route(Route::Inbox));
        let rendered = buffer(&mut empty, 100, 18);
        assert!(
            rendered.contains("No actionable open captures"),
            "{rendered}"
        );
        empty.inbox_query = "missing".to_string();
        let rendered = buffer(&mut empty, 100, 18);
        assert!(
            rendered.contains("No captures match the active search and filters"),
            "{rendered}"
        );

        let actionable = capture_record(
            "I-actionable",
            "Actionable intake",
            CaptureState::Open,
            "2026-01-01T00:00:01Z",
        );
        let mut discarded = capture_record(
            "I-discarded",
            "Discarded intake",
            CaptureState::Discarded,
            "2026-01-01T00:00:02Z",
        );
        discarded.labels = vec!["triage".to_string(), "ux".to_string()];
        discarded.disposed_at = Some("2026-01-02T00:00:00Z".to_string());
        discarded.reason = Some("duplicate".to_string());
        discarded.promotion_defaults = Some(PromotionDefaults {
            phase: Some(2),
            cut: Some("M-next".to_string()),
            effort_days: Some(1.5),
            owner: Some("kim".to_string()),
            paths: vec!["src/**".to_string()],
            tags: vec!["area:tui".to_string()],
        });
        let mut promoted = capture_record(
            "I-promoted",
            "Promoted intake",
            CaptureState::Promoted,
            "2026-01-01T00:00:03Z",
        );
        promoted.promoted_to = Some("A-1".to_string());
        let mut model = ModelBuilder::default()
            .inbox(
                vec![actionable, discarded, promoted],
                "2026-01-01T00:00:10Z",
            )
            .build();
        model.update(Action::Route(Route::Inbox));
        model.inbox_state = None;
        model.inbox_revisit = RevisitFilter::All;
        let rendered = buffer(&mut model, 120, 24);
        for expected in ["OPEN", "PROMOTED", "DISCARDED"] {
            assert!(
                rendered.contains(expected),
                "missing {expected}:\n{rendered}"
            );
        }

        model.update(Action::OpenFilters);
        let rendered = buffer(&mut model, 100, 30);
        for expected in [
            "State: OPEN",
            "State: PROMOTED",
            "State: DISCARDED",
            "Review: actionable",
            "Kind: task",
            "Label: triage",
        ] {
            assert!(
                rendered.contains(expected),
                "missing {expected}:\n{rendered}"
            );
        }
        model.update(Action::CloseLayer);

        model.selected_capture_id = Some("I-discarded".to_string());
        model.update(Action::OpenDetail);
        let rendered = buffer(&mut model, 120, 32);
        for expected in [
            "Labels: triage, ux",
            "Source: none",
            "Promotion defaults: phase Some(2); cut M-",
            "next; effort Some(1.5); owner kim; paths",
            "Disposed at: 2026-01-02T00:00:00Z",
            "Reason: duplicate",
            "No body was captured.",
        ] {
            assert!(
                rendered.contains(expected),
                "missing {expected}:\n{rendered}"
            );
        }

        model.update(Action::CloseLayer);
        model.selected_capture_id = Some("I-promoted".to_string());
        model.update(Action::OpenDetail);
        let rendered = buffer(&mut model, 120, 28);
        assert!(rendered.contains("Promoted task: A-1"), "{rendered}");
        assert!(rendered.contains("Enter opens the loade"), "{rendered}");
    }

    /// Proof for `tui.modal_navigation_is_consistent`: every advertised
    /// next/previous key targets the modal selection, not the obscured main
    /// list, and wraps over the modal's own row count.
    #[test]
    fn every_modal_movement_key_moves_the_modal_selection() {
        for key in [KeyCode::Down, KeyCode::Char('j'), KeyCode::Tab] {
            let mut model = model();
            model.update(Action::OpenVerifyRunSheet);
            assert_eq!(
                decode_key(KeyEvent::new(key, KeyModifiers::NONE), &model),
                Some(Action::SelectModal(1)),
                "next key {key:?} did not target the modal"
            );
        }

        for key in [KeyCode::Up, KeyCode::Char('k'), KeyCode::BackTab] {
            let mut model = model();
            model.update(Action::OpenVerifyRunSheet);
            assert_eq!(
                decode_key(KeyEvent::new(key, KeyModifiers::NONE), &model),
                Some(Action::SelectModal(1)),
                "previous key {key:?} did not wrap in the modal"
            );
        }
    }

    #[test]
    fn every_route_focus_theme_and_glyph_mode_keeps_non_colour_meaning() {
        let mut model = model();
        model.update(Action::Resize {
            columns: 100,
            rows: 24,
        });
        for route in Route::ALL {
            model.update(Action::Route(route));
            let view = buffer(&mut model, 100, 24);
            assert!(view.contains(route.label()));
            assert!(view.contains("ERROR"));
        }
        for theme in Theme::ALL {
            model.theme = theme;
            let view = buffer(&mut model, 100, 24);
            assert!(view.contains("ERROR") && view.contains("1 error"));
        }
        for glyphs in Glyphs::ALL {
            model.glyphs = glyphs;
            let view = buffer(&mut model, 100, 24);
            assert!(view.contains("ERROR"));
            if glyphs == Glyphs::Ascii {
                assert!(view.is_ascii(), "ASCII mode emitted a non-ASCII cell");
            }
        }
        model.update(Action::Route(Route::Reports));
        for (focus, marker) in [
            (Focus::Navigation, "> Destinations <"),
            (
                Focus::Primary,
                "> Reports - which aggregate needs a decision <",
            ),
            (Focus::Detail, "> Roadmap report <"),
        ] {
            model.update(Action::Focus(focus));
            assert_eq!(model.focus, focus);
            assert!(buffer(&mut model, 100, 24).contains(marker));
        }
    }

    /// Proof for `tui.visual_language_is_semantic_and_deterministic`: every
    /// responsive class and built-in theme retains focus, exact values and
    /// redundant semantic roles; compact meters add scan shape without
    /// replacing the numbers they summarize.
    #[test]
    fn visual_language_review_matrix_preserves_hierarchy_and_exact_values() {
        assert_eq!(ratio_meter(3, 7, '#'), "[###.....]");
        assert_eq!(ratio_meter(0, 0, '#'), "[........]");
        assert_eq!(finding_meter(1, 2), "[!??.....]");
        assert_eq!(finding_meter(8, 1), "[!!!!!!!?]");
        assert_eq!(finding_meter(1, 8), "[!???????]");

        for theme in Theme::ALL {
            let styles = palette(theme);
            assert!(styles.accent.add_modifier.contains(Modifier::BOLD));
            assert!(styles.selected.add_modifier.contains(Modifier::BOLD));
            assert_ne!(styles.selected, styles.normal, "{theme:?} hid selection");
            assert_ne!(
                styles.warning, styles.good,
                "{theme:?} merged warning and ready"
            );
            assert_ne!(
                styles.error, styles.good,
                "{theme:?} merged error and ready"
            );
            assert_eq!(
                graph_role_style(GraphRole::Selected, styles),
                styles.selected
            );
            assert_eq!(graph_role_style(GraphRole::Blocker, styles), styles.warning);
            assert_eq!(graph_role_style(GraphRole::Dependent, styles), styles.good);
            assert_eq!(graph_role_style(GraphRole::Critical, styles), styles.accent);
            assert_eq!(
                graph_role_style(GraphRole::Unrelated, styles),
                styles.normal
            );

            for (width, height, marker) in [
                (140, 30, "Snapshot identity"),
                (100, 24, "Overview"),
                (60, 16, "Overview"),
                (48, 14, "CUTS"),
                (47, 13, "TERMINAL TOO SMALL"),
            ] {
                for glyphs in Glyphs::ALL {
                    let mut model = ModelBuilder::default().glyphs(glyphs).build();
                    model.theme = theme;
                    let view = buffer(&mut model, width, height);
                    assert!(view.contains(marker), "{theme:?}/{glyphs:?}:\n{view}");
                    assert_eq!(view.lines().count(), usize::from(height));
                    if glyphs == Glyphs::Ascii {
                        assert!(view.is_ascii(), "ASCII mode emitted a wide token");
                    }
                }
            }
        }

        let mut model = ModelBuilder::default().glyphs(Glyphs::Ascii).build();
        let overview = &mut model.snapshot.overview;
        overview.cut_work = 7;
        overview.done_cut_work = 3;
        overview.errors = 1;
        overview.warnings = 2;
        overview.denied = true;
        overview.ready = vec![aplan_app::OverviewTask {
            id: "A-1".to_string(),
            title: "first".to_string(),
        }];
        overview.critical_path = Some((2.5, overview.ready.clone()));
        let view = buffer(&mut model, 140, 30);
        for exact in [
            "DONE 3/7 | [###.....]",
            "READY 1 | [++++....]",
            "FINDINGS E:1 W:2 | DENIED | [!??.....]",
            "CRITICAL 2.5d | TASKS 1 | [****....]",
        ] {
            assert!(view.contains(exact), "missing `{exact}`:\n{view}");
        }

        for token in ["^", ">", "|", "/", "-", "\\", "\u{2191}", "\u{2026}"] {
            assert_eq!(text::cells(token), 1, "ambiguous-width token `{token}`");
        }

        model.theme = Theme::Monochrome;
        model.color_capability = crate::model::ColorCapability::None;
        assert!(
            CommandId::CycleTheme
                .unavailable(&model)
                .is_some_and(|reason| reason.contains("NO_COLOR"))
        );

        let styles = palette(Theme::LowColor);
        let mut row = model.task_browser.rows[0].clone();
        row.task.status = aplan_app::Status::Todo;
        row.task.ready_rank = Some(1);
        assert_eq!(task_semantic_style(&row, styles), styles.good);
        row.task.status = aplan_app::Status::Blocked;
        assert_eq!(task_semantic_style(&row, styles), styles.warning);
        row.task.status = aplan_app::Status::InProgress;
        assert_eq!(task_semantic_style(&row, styles), styles.accent);
        row.task.status = aplan_app::Status::Done;
        assert_eq!(task_semantic_style(&row, styles), styles.muted);
    }

    // Report discovery, contextual parameters, selection and drills remain distinct.
    fn assert_report_hub_navigation_and_scope_are_operable() {
        let mut model = reports_model();
        model.update(Action::OpenSearch);
        model.update(Action::SearchInput("stale".to_string()));
        let report_search = buffer(&mut model, 140, 30);
        assert!(
            report_search.contains("Search report names"),
            "{report_search}"
        );
        assert!(report_search.contains("1/7 report names shown"));
        model.update(Action::SearchInput(" absent".to_string()));
        let no_report_match_modal = buffer(&mut model, 140, 30);
        assert!(no_report_match_modal.contains("0/7 report names shown"));
        model.update(Action::CloseLayer);
        let no_report_match = buffer(&mut model, 140, 30);
        assert!(
            no_report_match.contains("No report name matches `stale absent`"),
            "{no_report_match}"
        );
        assert!(CommandId::AsOfDate.unavailable(&model).is_none());
        assert_eq!(
            decode_key(
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
                &model,
            ),
            Some(Action::OpenReportDate)
        );
        model.update(Action::ClearNarrowing);
        assert_eq!(model.report_kind, aplan_app::ReportKind::Stale);
        assert_eq!(wheel_action(&model, true), Some(Action::SelectIndex(6)));
        model.report_kind = aplan_app::ReportKind::Roadmap;
        assert!(CommandId::AsOfDate.unavailable(&model).is_some());
        assert_eq!(wheel_action(&model, true), Some(Action::SelectIndex(1)));
        model.report_kind = aplan_app::ReportKind::Deprecations;
        assert_eq!(
            wheel_action(&model, true),
            None,
            "wheel movement stops at the report list boundary"
        );
        assert_eq!(wheel_action(&model, false), Some(Action::SelectIndex(5)));
        open_report(&mut model, 0);
        let roadmap = buffer(&mut model, 140, 30);
        assert!(
            roadmap.contains("0 cut(s); 2.0d total; throughput not declared"),
            "{roadmap}"
        );
        assert!(roadmap.contains("whole plan (filters do not change this report)"));
        assert!(roadmap.contains("Cross-cutting work"));
        assert!(roadmap.contains("[Enter: tasks]"), "{roadmap}");
        model.update(Action::OpenSearch);
        model.update(Action::SearchInput("absent".to_string()));
        let no_row_match = buffer(&mut model, 140, 30);
        assert!(
            no_row_match.contains("Search report rows"),
            "{no_row_match}"
        );
        assert!(no_row_match.contains("0/1 rows shown"), "{no_row_match}");
        model.update(Action::CloseLayer);
        let no_row_match = buffer(&mut model, 140, 30);
        assert!(
            no_row_match.contains("No report row matches the search"),
            "{no_row_match}"
        );
        model.update(Action::SearchClear);

        open_report(&mut model, 1);
        let summary = buffer(&mut model, 140, 30);
        assert!(summary.contains("0/2 done; 2 ready; 2.0d"));
        assert!(summary.contains("selected tasks: all"));
    }

    // Configured argv stays literal and a provider failure remains unavailable.
    fn assert_report_hub_renders_literal_version_sources_and_provider_failures() {
        let mut model = reports_model();
        open_report(&mut model, 2);
        if let Some(aplan_app::ReportBody::Version(report)) = model.report_projection.body.as_mut()
        {
            report.current_source = aplan_app::VersionCurrentSource::Configured(vec![
                "tool".to_string(),
                "two words".to_string(),
                String::new(),
                "a\"b".to_string(),
                "line\nbreak".to_string(),
                "\u{1b}[31m".to_string(),
            ]);
        }
        let source = report_value_summary(&model);
        assert!(source.contains("\"two words\", \"\", \"a\\\"b\""));
        assert!(source.contains("line\\nbreak"));
        assert!(source.contains("\\u{1b}[31m"));
        assert!(!source.contains('\n'));
        assert!(!source.contains('\u{1b}'));
        let version = buffer(&mut model, 140, 30);
        assert!(version.contains("current source argv:"), "{version}");
        assert!(version.contains("[configuration] Current version source missing"));

        open_report(&mut model, 3);
        let unavailable = buffer(&mut model, 140, 30);
        assert!(unavailable.contains("unavailable: synthetic snapshot has no repository"));
        assert!(unavailable.contains("advisory - never a verdict"));
        assert!(unavailable.contains("No report rows are available."));
    }

    // Typed empty, measured, advisory and invalid states retain report-specific data.
    fn assert_report_hub_renders_typed_report_states() {
        let mut model = reports_model();
        open_report(&mut model, 3);
        model.report_projection.state = aplan_app::ReportState::Ready;
        model.report_projection.body =
            Some(aplan_app::ReportBody::Paths(aplan_output::PathsReport {
                sampled: 7,
                asked_for: 100,
                tasks: Vec::new(),
                without_paths: 2,
                unmeasurable: 1,
            }));
        let paths = buffer(&mut model, 140, 30);
        assert!(paths.contains("sampled 7/100 commits; 0 measured row(s)"));
        assert!(paths.contains("2 without"));
        assert!(paths.contains("paths; 1 unmeasurable"));

        open_report(&mut model, 4);
        let vacuity = buffer(&mut model, 140, 30);
        assert!(vacuity.contains("rule reach row(s)"));
        assert!(vacuity.contains("zero subjects is evidence, not"));
        assert!(vacuity.contains(" a pass"));
        assert!(vacuity.contains("[no subjects] APLAN012"));

        model.report_projection.state = aplan_app::ReportState::Ready;
        model.report_projection.body =
            Some(aplan_app::ReportBody::Stale(aplan_output::StaleReport {
                as_of: Some("2026-10-17".to_string()),
                window_days: 30,
                tasks: Vec::new(),
                fresh: 1,
                untracked: 2,
                coordination: aplan_core::ClaimScope::unavailable("test"),
                claim_observed_at_epoch_seconds: 0,
            }));
        model.report_kind = aplan_app::ReportKind::Stale;
        let stale = buffer(&mut model, 140, 30);
        assert!(stale.contains("older than 30d; as of"));
        assert!(stale.contains("1 fresh, 2 untracked"));

        open_report(&mut model, 6);
        let deprecations = buffer(&mut model, 140, 30);
        assert!(deprecations.contains("0 runway(s); 0 unannounced removal(s)"));
        assert!(deprecations.contains("empty result"));

        model.report_projection = model.snapshot.reports.project(
            aplan_app::ReportKind::Stale,
            &aplan_app::ReportRequest {
                stale_as_of: Some("not-a-date".to_string()),
                ..aplan_app::ReportRequest::default()
            },
        );
        model.report_kind = aplan_app::ReportKind::Stale;
        let invalid = buffer(&mut model, 140, 30);
        assert!(invalid.contains("invalid parameters: `not-a-date`"));
    }

    /// Proof for `tui.report_hub_preserves_cli_report_semantics` and
    /// `tui.report_discovery_is_literal_and_parameter_safe`: the terminal
    /// preserves discovery, scope, literal values and typed result states.
    #[test]
    fn report_hub_renders_scope_provenance_and_typed_states() {
        assert_report_hub_navigation_and_scope_are_operable();
        assert_report_hub_renders_literal_version_sources_and_provider_failures();
        assert_report_hub_renders_typed_report_states();
    }

    /// Proof for `tui.shell_is_responsive_and_contextual`: reviewable fixtures
    /// cover every cell sequence for the responsive classes, all three focus
    /// states and the below-minimum refusal. Semantic assertions are kept next
    /// to the pictures so approving a plausible-looking diff cannot silently
    /// change selection, focus or selected-row styling.
    #[test]
    fn fixed_size_buffers_pin_every_layout_focus_and_minimum_refusal() {
        let cases = [
            (
                140,
                30,
                LayoutClass::Wide,
                Focus::Navigation,
                "> Destinations <",
                include_str!("snapshots/shell-wide-ascii.txt"),
            ),
            (
                100,
                24,
                LayoutClass::Normal,
                Focus::Primary,
                "> Overview - where attention goes next <",
                include_str!("snapshots/shell-normal-ascii.txt"),
            ),
            (
                60,
                16,
                LayoutClass::Narrow,
                Focus::Detail,
                "Overview > Snapshot identity",
                include_str!("snapshots/shell-narrow-ascii.txt"),
            ),
            (
                48,
                14,
                LayoutClass::Narrow,
                Focus::Primary,
                "CUTS",
                include_str!("snapshots/shell-minimum-ascii.txt"),
            ),
            (
                47,
                13,
                LayoutClass::TooSmall,
                Focus::Primary,
                "TERMINAL TOO SMALL",
                include_str!("snapshots/shell-too-small-ascii.txt"),
            ),
        ];
        for (width, height, class, focus, marker, expected) in cases {
            let mut model = ModelBuilder::default()
                .focus(focus)
                .glyphs(Glyphs::Ascii)
                .build();
            let view = buffer(&mut model, width, height);
            assert_eq!(classify(Rect::new(0, 0, width, height)), class);
            assert!(view.contains(marker), "{class:?} snapshot:\n{view}");
            assert_eq!(view.lines().count(), usize::from(height));
            assert_eq!(model.focus, focus);
            assert_eq!(model.route, Route::Overview);
            assert_eq!(model.selected, 0);
            assert_eq!(model.glyphs, Glyphs::Ascii);
            assert_eq!(view, fixture_snapshot(expected));
        }

        let mut model = ModelBuilder::default()
            .focus(Focus::Primary)
            .glyphs(Glyphs::Ascii)
            .build();
        model.update(Action::Route(Route::Tasks));
        model.update(Action::Resize {
            columns: 100,
            rows: 24,
        });
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|frame| render(frame, &model)).expect("draw");
        let layout = ShellLayout::new(Rect::new(0, 0, 100, 24), &model);
        let content = layout.primary.inner(Margin::new(1, 1));
        let rendered = terminal.backend().buffer();
        let title_offset = u16::try_from(text::cells("> A-1  ")).expect("fixture label width");
        let first_title = rendered[(
            content.x.saturating_add(title_offset),
            content.y.saturating_add(2),
        )]
            .clone();
        assert_eq!(
            first_title.symbol(),
            "f",
            "the semantic assertion must still point at the selected task label"
        );
        let selected = palette(model.theme).selected;
        assert_eq!(first_title.fg, selected.fg.expect("selected foreground"));
        assert_eq!(first_title.bg, selected.bg.expect("selected background"));
        assert!(
            first_title.modifier.contains(Modifier::BOLD),
            "the selected task label lost its non-colour emphasis"
        );
    }

    /// Proof for `tui.shell_adapts_by_route_and_available_rows`: the support
    /// matrix is a two-dimensional allocation contract. Short layouts yield
    /// bordered chrome before route content, while pane composition continues
    /// to follow the width/height class and the refusal begins only below the
    /// exact supported floor.
    #[test]
    #[allow(clippy::too_many_lines)] // One matrix keeps every route and breakpoint in one audit.
    fn adaptive_shell_allocates_chrome_by_width_height_and_route() {
        let baseline = model();
        let cases = [
            (140, 30, LayoutClass::Wide, ChromeDensity::Full, true, 21),
            (120, 24, LayoutClass::Wide, ChromeDensity::Full, true, 15),
            (119, 24, LayoutClass::Normal, ChromeDensity::Full, true, 12),
            (80, 24, LayoutClass::Normal, ChromeDensity::Full, true, 12),
            (100, 22, LayoutClass::Normal, ChromeDensity::Full, true, 10),
            (
                100,
                21,
                LayoutClass::Normal,
                ChromeDensity::Compact,
                true,
                15,
            ),
            (
                80,
                18,
                LayoutClass::Normal,
                ChromeDensity::Compact,
                true,
                12,
            ),
            (
                79,
                18,
                LayoutClass::Narrow,
                ChromeDensity::Compact,
                false,
                12,
            ),
            (120, 23, LayoutClass::Normal, ChromeDensity::Full, true, 11),
            (60, 24, LayoutClass::Narrow, ChromeDensity::Full, false, 12),
            (
                60,
                16,
                LayoutClass::Narrow,
                ChromeDensity::Compact,
                false,
                10,
            ),
            (
                48,
                14,
                LayoutClass::Narrow,
                ChromeDensity::Compact,
                false,
                8,
            ),
            (
                120,
                17,
                LayoutClass::Narrow,
                ChromeDensity::Compact,
                false,
                11,
            ),
        ];
        for (columns, rows, class, density, split, content_rows) in cases {
            let layout = ShellLayout::new(Rect::new(0, 0, columns, rows), &baseline);
            assert_eq!(layout.class, class, "{columns}x{rows}");
            assert_eq!(layout.density, density, "{columns}x{rows}");
            assert_eq!(layout.detail.is_some(), split, "{columns}x{rows}");
            assert_eq!(
                layout.primary.inner(Margin::new(1, 1)).height,
                content_rows,
                "{columns}x{rows} did not preserve its measured work area"
            );
        }
        for (columns, rows) in [(47, 14), (48, 13), (47, 13)] {
            assert_eq!(
                classify(Rect::new(0, 0, columns, rows)),
                LayoutClass::TooSmall
            );
        }
        assert_eq!(
            classify(Rect::new(0, 0, MIN_COLUMNS, MIN_ROWS)),
            LayoutClass::Narrow
        );

        let mut minimum_model = model();
        minimum_model.update(Action::Resize {
            columns: 48,
            rows: 14,
        });
        let minimum = ShellLayout::new(Rect::new(0, 0, 48, 14), &minimum_model);
        let segments = footer_segments(minimum.footer, &minimum_model, minimum.density);
        for required in [CommandId::Back, CommandId::Palette, CommandId::Help] {
            assert!(
                segments.iter().any(|(id, _)| *id == required),
                "minimum footer lost {required:?}: {segments:?}"
            );
        }

        let mut model = model();
        model.update(Action::Resize {
            columns: 48,
            rows: 14,
        });
        let overview = buffer(&mut model, 48, 14);
        for value in ["CUTS", "EFFORT", "READY", "REVISIT", "FINDINGS", "CRITICAL"] {
            assert!(
                overview.contains(value),
                "minimum Overview lost {value}:\n{overview}"
            );
        }
        model.focus = Focus::Detail;
        let overview_detail = buffer(&mut model, 48, 14);
        assert_eq!(
            overview_detail.matches("Esc metrics").count(),
            2,
            "{overview_detail}"
        );
        model.focus = Focus::Primary;

        model.update(Action::Route(Route::Tasks));
        let tasks = buffer(&mut model, 48, 14);
        assert!(tasks.contains("all tasks; 2/2 shown"), "{tasks}");
        assert!(tasks.contains("A-1") && tasks.contains("A-2"), "{tasks}");
        model.update(Action::OpenDetail);
        let task_detail = buffer(&mut model, 48, 14);
        assert!(
            task_detail.contains("Esc tasks | Tasks > A-1 / Summary"),
            "{task_detail}"
        );
        assert_eq!(task_detail.matches("Esc tasks").count(), 2, "{task_detail}");

        model.update(Action::CloseLayer);
        model.update(Action::Route(Route::Diagnostics));
        let diagnostics = buffer(&mut model, 48, 14);
        assert!(diagnostics.contains("check DENIED E1 W2"), "{diagnostics}");
        assert!(diagnostics.contains("ERROR-0"), "{diagnostics}");
        model.update(Action::OpenDetail);
        let diagnostic_detail = buffer(&mut model, 48, 14);
        assert!(
            diagnostic_detail.contains("Esc findings | Diagnostics > Finding"),
            "{diagnostic_detail}"
        );
        assert!(
            diagnostic_detail.contains("check DENIED E1 W2"),
            "{diagnostic_detail}"
        );
        assert_eq!(
            diagnostic_detail.matches("Esc findings").count(),
            2,
            "{diagnostic_detail}"
        );

        model.update(Action::CloseLayer);
        model.update(Action::Route(Route::Graph));
        let graph = buffer(&mut model, 48, 14);
        assert!(graph.contains("2/2N 0/0E"), "{graph}");
        assert!(graph.contains("COMPLETE NO EDGES"), "{graph}");
        model.update(Action::OpenDetail);
        let phase = buffer(&mut model, 48, 14);
        assert!(phase.contains("Overview > Phase 1"), "{phase}");
        model.update(Action::OpenDetail);
        let task = buffer(&mut model, 48, 14);
        assert!(task.contains("Overview > Phase 1 > A-1"), "{task}");
        model.update(Action::OpenDetail);
        let graph_detail = buffer(&mut model, 48, 14);
        assert!(
            graph_detail.contains("Esc graph | Graph > A-1 / Summary"),
            "{graph_detail}"
        );
        assert_eq!(
            graph_detail.matches("Esc graph").count(),
            2,
            "{graph_detail}"
        );

        let mut reports = reports_model();
        reports.update(Action::Resize {
            columns: 48,
            rows: 14,
        });
        let report_names = buffer(&mut reports, 48, 14);
        for report in [
            "Roadmap",
            "Summary",
            "Version",
            "Paths",
            "Vacuity",
            "Stale",
            "Deprecations",
        ] {
            assert!(
                report_names.contains(report),
                "minimum Reports lost {report}:\n{report_names}"
            );
        }
        reports.update(Action::OpenDetail);
        let report_detail = buffer(&mut reports, 48, 14);
        assert!(
            report_detail.contains("Esc reports | Reports > Roadmap report"),
            "{report_detail}"
        );
        assert_eq!(
            report_detail.matches("Esc reports").count(),
            2,
            "{report_detail}"
        );

        let mut unavailable = baseline;
        unavailable.update(Action::Resize {
            columns: 48,
            rows: 14,
        });
        unavailable.update(Action::Route(Route::Tasks));
        unavailable.selected_id = Some("A-removed".to_string());
        unavailable.focus = Focus::Detail;
        let unavailable_detail = buffer(&mut unavailable, 48, 14);
        assert!(
            unavailable_detail.contains("Esc tasks | Tasks > Task inspector"),
            "{unavailable_detail}"
        );
        assert!(
            unavailable_detail.contains("was removed"),
            "{unavailable_detail}"
        );
    }

    /// Proof for `tui.shell_adapts_by_route_and_available_rows`: resize is
    /// presentation-only for task identity, inspector coordinates, a manual
    /// graph viewport/window and an in-progress form. The effective viewports
    /// clamp to each frame without rewriting those semantic/editable values.
    #[test]
    fn responsive_resize_preserves_semantic_and_editable_state() {
        let mut model = model();
        model.update(Action::Route(Route::Tasks));
        model.update(Action::SelectIndex(1));
        model.update(Action::OpenDetail);
        model.inspector_section = InspectorSection::Paths;
        model.inspector_selected = 0;
        model.inspector_scroll = Some(3);
        model.graph_viewport_mode = GraphViewportMode::Manual;
        model.graph_scroll_row = 7;
        model.graph_scroll_column = 11;
        let graph_request = model.graph_window_request.clone();
        let graph_state = model.graph_window.state.clone();

        let prepare = model
            .update(Action::OpenMutation(MutationAction::Add))
            .effect
            .expect("prepare mutation");
        let form = MutationForm::new(
            "Responsive form",
            model.snapshot.overview.input.sha256.clone(),
            false,
            vec![FieldSpec::new("title", "Title", "required").initial_value("draft")],
        )
        .expect("form");
        model.update(Action::EffectCompleted(Box::new(EffectCompletion {
            request: prepare,
            outcome: Ok(EffectOutput::Mutation(MutationTransfer::new(
                MutationPayload::Prepared(PreparedMutation::Form(Box::new(form))),
            ))),
        })));
        model.update(Action::MutationForm(FormAction::Insert(
            " value".to_string(),
        )));

        for (columns, rows) in [(140, 30), (100, 21), (80, 24), (60, 16), (48, 14)] {
            model.update(Action::Resize { columns, rows });
            let view = buffer(&mut model, columns, rows);
            assert!(
                view.contains("Responsive form"),
                "{columns}x{rows}:\n{view}"
            );
            assert_eq!(model.selected_id.as_deref(), Some("A-2"));
            assert_eq!(model.focus, Focus::Detail);
            assert_eq!(model.inspector_section, InspectorSection::Paths);
            assert_eq!(model.inspector_selected, 0);
            assert_eq!(model.inspector_scroll, Some(3));
            assert_eq!(model.graph_viewport_mode, GraphViewportMode::Manual);
            assert_eq!(model.graph_scroll_row, 7);
            assert_eq!(model.graph_scroll_column, 11);
            assert_eq!(model.graph_window_request, graph_request);
            assert_eq!(model.graph_window.state, graph_state);
            let form = match &model.mutation.as_ref().expect("mutation").state {
                MutationState::Form(form) => form,
                state => panic!("unexpected mutation state: {state:?}"),
            };
            assert_eq!(
                form.fields()[0].values().collect::<Vec<_>>(),
                ["draft value"]
            );
        }

        let viewport = graph_viewport(
            ShellLayout::new(Rect::new(0, 0, 48, 14), &model).primary,
            true,
            &model,
        );
        assert!(viewport.row_start <= model.graph_scroll_row);
        assert!(viewport.column_start <= model.graph_scroll_column);

        let layer = model.mutation.as_mut().expect("mutation");
        layer.state = MutationState::QuickReview {
            candidate: None,
            form: None,
            rows: vec!["PREVIEW: exact responsive change".to_string()],
        };
        let preview = buffer(&mut model, 48, 14);
        assert!(
            preview.contains("PREVIEW: exact responsive change"),
            "{preview}"
        );
        assert!(preview.contains("[Cancel] [Confirm]"), "{preview}");

        model.mutation.as_mut().expect("mutation").state = MutationState::Result {
            form: None,
            rows: vec!["ERROR: typed publication refused".to_string()],
        };
        let result = buffer(&mut model, 48, 14);
        assert!(
            result.contains("ERROR: typed publication refused"),
            "{result}"
        );
        assert!(result.contains("[Close]"), "{result}");
    }

    /// Proof for `tui.overview_is_one_typed_projection`: terminal text uses
    /// the typed Overview values verbatim and keeps every headline at the
    /// normal breakpoint without relying on color.
    #[test]
    fn overview_renders_every_typed_number_and_honest_edge_state() {
        let mut model = ModelBuilder::default().glyphs(Glyphs::Ascii).build();
        model.theme = Theme::Monochrome;
        let overview = &mut model.snapshot.overview;
        overview.cuts = vec![aplan_app::OverviewCut {
            label: "v9.9".to_string(),
            milestone_id: "M-9".to_string(),
            status: aplan_app::Status::Todo,
            borrows: false,
            total: 7,
            done: 3,
            ready: 2,
            blocked: 2,
            effort_days: 12.5,
            done_effort_days: 4.0,
            task_ids: vec!["A-1".to_string(), "A-2".to_string()],
        }];
        overview.cut_work = 7;
        overview.done_cut_work = 3;
        overview.live_effort_days = 12.5;
        overview.done_effort_days = 4.0;
        overview.missing_effort_task_ids = vec!["A-2".to_string()];
        overview.effort_state = aplan_app::OverviewEffortState::Partial;
        overview.ready = vec![
            aplan_app::OverviewTask {
                id: "A-1".to_string(),
                title: "first".to_string(),
            },
            aplan_app::OverviewTask {
                id: "A-2".to_string(),
                title: "second".to_string(),
            },
        ];
        overview.due = vec![aplan_app::OverviewDueTask {
            id: "A-2".to_string(),
            title: "second".to_string(),
            revisit_at: "1970-01-01T00:00:00Z".to_string(),
            overdue_seconds: 42,
        }];
        overview.waiting_revisits = vec![
            aplan_app::OverviewWaitingTask {
                id: "A-3".to_string(),
                title: "third".to_string(),
                revisit_at: "1970-01-01T00:01:00Z".to_string(),
                seconds_until: 18,
            },
            aplan_app::OverviewWaitingTask {
                id: "A-4".to_string(),
                title: "fourth".to_string(),
                revisit_at: "1970-01-01T00:02:00Z".to_string(),
                seconds_until: 78,
            },
        ];
        overview.as_of_epoch_seconds = 42;
        overview.errors = 1;
        overview.warnings = 2;
        overview.denied = true;
        overview.critical_path = Some((8.5, overview.ready.clone()));
        overview.load_state = aplan_app::OverviewLoadState::PartiallyUnloadable;
        overview.unloadable = vec![("BROKEN".to_string(), "tasks/BROKEN.md".into())];
        overview.effort_state = aplan_app::OverviewEffortState::Partial;

        let normal = buffer(&mut model, 80, 18);
        for expected in [
            "CUTS 1 | DONE 3/7",
            "EFFORT 4.0/12.5d | 1 missing estimate(s)",
            "READY 2",
            "REVISIT DUE 1 | LATER 2 | AS-OF 42",
            "FINDINGS E:1 W:2 | DENIED",
            "CRITICAL 8.5d | TASKS 2",
        ] {
            assert!(normal.contains(expected), "missing `{expected}`:\n{normal}");
        }
        let wide = buffer(&mut model, 140, 30);
        assert!(wide.contains("plan: repo/very-long-root"));
        assert!(wide.contains("sha:plan-identit"));
        assert!(wide.contains("config: none selected sha:config-ident"));
        assert!(wide.contains("repository: repo/very-long-root"));
        assert!(wide.contains("repository state: not observed: synthetic snapshot"));
        assert!(wide.contains("build: 0.test src:test-source"));
        assert!(wide.contains("facts:facts binary:binary"));
        assert!(wide.contains("snapshot: current; memory dirty: no"));
        assert!(wide.contains("partially unloadable (1 file(s))"));

        model.snapshot.overview.load_state = aplan_app::OverviewLoadState::Empty;
        model.snapshot.overview.work_state = aplan_app::OverviewWorkState::AllDone;
        model.snapshot.overview.effort_state = aplan_app::OverviewEffortState::None;
        let edge = buffer(&mut model, 140, 30);
        assert!(edge.contains("load: empty plan"));
        assert!(edge.contains("work: all work done; effort: none estimated"));
    }

    /// Proof for `tui.overview_is_one_typed_projection`: the findings
    /// aggregate opens its canonical ordered members, and both the list and
    /// selected subject remain inspectable at the supported minimum size.
    #[test]
    fn overview_findings_drill_is_exact_bounded_and_subject_aware() {
        let mut model = ModelBuilder::default().glyphs(Glyphs::Ascii).build();
        let findings = (0..12)
            .map(|index| aplan_app::OverviewFinding {
                identity: format!("finding-{index}"),
                scope: aplan_app::DiagnosticScope::Check,
                code: format!("W-{index}"),
                severity: aplan_app::Severity::Warning,
                subject: "whole plan".to_string(),
                task: None,
                file: Some(format!("aplan/tasks/broken-{index}.md").into()),
                line: Some(index + 1),
                column: Some(2),
                message: format!(
                    "finding {index} has a deliberately long explanation that must wrap without disappearing"
                ),
                agent_instruction: Some(format!("repair finding {index}")),
                detail: vec![format!("detail {index}")],
                contribution: aplan_app::VerdictContribution::Blocking,
                source_line: Some(format!("source line {index}")),
            })
            .collect::<Vec<_>>();
        model.snapshot.overview.findings = findings.clone();
        model.snapshot.diagnostics.check.findings = findings;
        model.snapshot.diagnostics.check.warnings = 12;
        model.snapshot.diagnostics.check.blocking = 12;
        model.snapshot.diagnostics.check.denied = true;
        model.snapshot.overview.errors = 0;
        model.snapshot.overview.warnings = 12;
        model.snapshot.overview.denied = true;
        model.update(Action::Route(Route::Diagnostics));
        model.update(Action::SelectIndex(11));

        let list = buffer(&mut model, 48, 14);
        assert!(list.contains("W-11"), "{list}");
        assert!(
            list.contains('^'),
            "overflow above must remain explicit:\n{list}"
        );
        assert!(CommandId::Open.unavailable(&model).is_none());

        model.update(Action::OpenDetail);
        let detail = buffer(&mut model, 48, 14);
        assert!(detail.contains("warning W-11"), "{detail}");
        assert!(
            detail.contains('>'),
            "overflow below must remain explicit:\n{detail}"
        );
        model.update(Action::SetInspectorScroll(1));
        let subject = buffer(&mut model, 48, 14);
        assert!(subject.contains("subject: whole plan"), "{subject}");
        model.update(Action::SetInspectorScroll(3));
        let scrolled = buffer(&mut model, 48, 14);
        assert!(scrolled.contains("broken-11.md:12"), "{scrolled}");
        model.update(Action::SetInspectorScroll(5));
        let message = buffer(&mut model, 48, 14);
        assert!(message.contains("message: finding 11"), "{message}");
    }

    /// Proof for `tui.diagnostic_workbench_retains_complete_runs`: the
    /// renderer keeps the complete scope verdict while a filtered row exposes
    /// every actionable entity and instruction. Unavailable document input is
    /// not painted as a clean result.
    #[test]
    fn diagnostic_renderer_keeps_whole_verdict_and_shows_actionable_detail() {
        let mut model = ModelBuilder::default().build();
        let finding = &mut model.snapshot.diagnostics.check.findings[1];
        finding.identity = "effort".to_string();
        finding.code = "APLAN009".to_string();
        finding.subject = "task A-1".to_string();
        finding.task = Some("A-1".to_string());
        finding.file = Some("aplan/tasks/A-1.md".into());
        finding.line = Some(6);
        finding.column = Some(1);
        finding.message = "A-1 has no effort estimate".to_string();
        finding.agent_instruction = Some("Add `effort_days` to A-1.".to_string());
        finding.detail = vec!["the repository requires effort".to_string()];
        finding.contribution = aplan_app::VerdictContribution::Advisory;
        finding.source_line = Some("status: todo".to_string());
        model.snapshot.diagnostics.run_sheet.relaxed = true;
        model.update(Action::Route(Route::Diagnostics));
        model.diagnostic_filters =
            vec![crate::model::DiagnosticFilter::Rule("APLAN009".to_string())];
        model.update(Action::SelectIndex(0));

        let list = buffer(&mut model, 120, 24);
        for expected in [
            "check | FILTERED | ONE-RUN RELAXED DENIED E1 W2",
            "APLAN009",
        ] {
            assert!(list.contains(expected), "missing {expected:?}:\n{list}");
        }

        model.update(Action::OpenDetail);
        let detail = buffer(&mut model, 120, 30);
        for expected in [
            "warning APLAN009",
            "scope: check",
            "subject: task A-1",
            "location: aplan/tasks/A-1.md:6:1",
            "verdict: advisory; does not block",
            "message: A-1 has no effort estimate",
            "agent instruction: Add `effort_days` to A",
            "detail: the repository requires effort",
            "source: status: todo",
            "active tasks car",
        ] {
            assert!(detail.contains(expected), "missing {expected:?}:\n{detail}");
        }

        model.update(Action::Focus(Focus::Primary));
        model.update(Action::CycleDiagnosticScope);
        let documents = buffer(&mut model, 120, 24);
        assert!(documents.contains("check-docs UNAVAILABLE E0 W0"));
        assert!(documents.contains("Not a clean verdict:"), "{documents}");
        assert!(
            documents.contains("no synthetic document configura"),
            "{documents}"
        );

        model.update(Action::CycleDiagnosticScope);
        model
            .snapshot
            .diagnostics
            .run_sheet
            .effective
            .require_effort = false;
        model
            .snapshot
            .diagnostics
            .run_sheet
            .repository
            .require_effort = true;
        model.update(Action::OpenRunSheet);
        let next_run = buffer(&mut model, 120, 24);
        assert!(
            next_run.contains("effort: required (inherited)"),
            "the draft must describe repository policy, not the preceding relaxed result:\n{next_run}"
        );
    }

    /// Proof for `tui.diagnostic_workbench_retains_complete_runs` and
    /// `tui.narrowed_diagnostic_state_is_truthful`: a complete, denied run
    /// narrowed to zero rows cannot use the clean-run sentence, an all-matching
    /// query is still disclosed, and trust-context labels precede metadata that
    /// a minimum-width terminal may truncate.
    #[test]
    fn narrowed_diagnostics_never_render_as_clean() {
        let mut model = ModelBuilder::default().build();
        model.snapshot.diagnostics.run_sheet.relaxed = true;
        model.snapshot.diagnostics.run_sheet.effective.traces_root =
            Some(std::path::PathBuf::from(
                "a/very/long/trace/root/that/cannot/fit/beside/the/rest/of/the/run/metadata",
            ));
        model.update(Action::Route(Route::Diagnostics));

        model.diagnostic_query = "nothing can match this query".to_string();
        model.diagnostic_query_cursor = model.diagnostic_query.len();
        let empty = buffer(&mut model, 120, 24);
        assert!(
            empty.contains("No findings match the active search or filters."),
            "{empty}"
        );
        assert!(
            empty.contains("Whole run remains DENIED: E1 W2."),
            "{empty}"
        );
        assert!(!empty.contains("Clean complete run"), "{empty}");

        model.diagnostic_query = "synthetic".to_string();
        model.diagnostic_query_cursor = model.diagnostic_query.len();
        assert_eq!(
            model.diagnostic_findings().len(),
            model.diagnostic_run().findings.len(),
            "the active query must match every fixture finding"
        );
        let minimum = buffer(&mut model, 48, 14);
        assert!(minimum.contains("FILTERED"), "{minimum}");
        assert!(minimum.contains("ONE-RUN RELAXED"), "{minimum}");

        model.update(Action::OpenDetail);
        let detail = buffer(&mut model, 48, 14);
        assert!(detail.contains("ONE-RUN RELAXED"), "{detail}");
    }

    #[test]
    fn every_diagnostic_modal_renders_its_exact_scope_and_policy_state() {
        let mut model = ModelBuilder::default().build();
        let finding = &mut model.snapshot.diagnostics.check.findings[0];
        finding.code = "APLAN009".to_string();
        finding.subject = "task A-1".to_string();
        finding.task = Some("A-1".to_string());
        finding.file = Some("aplan/tasks/A-1.md".into());
        model.update(Action::Route(Route::Diagnostics));

        model.update(Action::OpenSearch);
        model.update(Action::SearchInput("A-1".to_string()));
        let search = buffer(&mut model, 100, 24);
        assert!(search.contains("Search findings"), "{search}");
        assert!(search.contains("findings shown; whole-run verdict is unchanged"));
        model.update(Action::CloseLayer);

        model.update(Action::OpenFilters);
        model.update(Action::ToggleFilter(0));
        let filters = buffer(&mut model, 100, 24);
        assert!(filters.contains("Diagnostic filters"), "{filters}");
        assert!(filters.contains("[x] severity:"), "{filters}");
        assert!(filters.contains("rule: APLAN009"), "{filters}");
        assert!(filters.contains("task: A-1"), "{filters}");
        assert!(filters.contains("file: aplan/tasks/A-1.md"), "{filters}");
        model.update(Action::CloseLayer);
        model.update(Action::ClearNarrowing);

        model.update(Action::OpenRuleSearch);
        model.update(Action::SearchInput("APLAN009".to_string()));
        let rule_search = buffer(&mut model, 100, 24);
        assert!(rule_search.contains("Explain rule - exact catalogue name"));
        assert!(rule_search.contains("Exact code: APLAN009"));
        assert!(rule_search.contains("unknown names are refused"));
        model.update(Action::SubmitRuleSearch);
        let explanation = buffer(&mut model, 100, 24);
        assert!(explanation.contains("Rule explanation"));
        assert!(explanation.contains("warning APLAN009"));
        assert!(explanation.contains("same typed catalogue"));
        model.update(Action::CloseLayer);

        model.modal = Some(Modal::RuleExplanation);
        model.rule_explanation = None;
        let absent_explanation = buffer(&mut model, 100, 24);
        assert!(absent_explanation.contains("No rule explanation is loaded"));
        model.update(Action::CloseLayer);

        model
            .snapshot
            .diagnostics
            .run_sheet
            .repository
            .require_effort = false;
        model.snapshot.diagnostics.run_sheet.repository.traces_root = Some("traces".into());
        model
            .snapshot
            .diagnostics
            .run_sheet
            .repository
            .deny_warnings = true;
        model.update(Action::OpenRunSheet);
        model.update(Action::ToggleRunSheet(0));
        model.update(Action::ToggleRunSheet(1));
        model.update(Action::ToggleRunSheet(2));
        model.update(Action::ToggleRunSheet(2));
        let run_sheet = buffer(&mut model, 100, 24);
        assert!(run_sheet.contains("effort: optional (same as inherited)"));
        assert!(run_sheet.contains("traces: disabled [relaxed override]"));
        assert!(run_sheet.contains("warnings: allow"));
        assert!(run_sheet.contains("ordinary refresh returns to repository policy"));

        model.pending_check_overrides.no_traces = true;
        model.snapshot.diagnostics.run_sheet.repository.traces_root = None;
        model
            .snapshot
            .diagnostics
            .run_sheet
            .repository
            .deny_warnings = false;
        model.pending_check_overrides.warning_policy = aplan_app::WarningPolicyOverride::Inherit;
        let inherited = buffer(&mut model, 100, 24);
        assert!(inherited.contains("traces: disabled (same as inherited)"));
        assert!(inherited.contains("warnings: inherit (allow)"));
    }

    /// Proof for `tui.shell_is_responsive_and_contextual`: input fitting,
    /// cursor placement, painted labels and mouse targets use cell geometry.
    #[test]
    fn grapheme_cursor_and_hit_targets_share_cell_geometry() {
        let mut model = model();
        model.update(Action::Resize {
            columns: 100,
            rows: 24,
        });
        model.update(Action::OpenPalette);
        model.update(Action::PaletteInput(
            "e\u{301}\u{754c}\u{1b}[2J".to_string(),
        ));
        let view = buffer(&mut model, 100, 24);
        assert!(view.contains("e\u{301}"));
        assert!(view.contains('\u{754c}'));
        assert!(view.contains("\u{fffd}[2J"));
        assert!(!view.contains('\u{1b}'));

        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|frame| render(frame, &model)).expect("draw");
        let overlay = modal_area(Rect::new(0, 0, 100, 24));
        let expected_x = overlay
            .x
            .saturating_add(1)
            .saturating_add(u16::try_from(text::cells("Action: ")).expect("prefix width"))
            .saturating_add(u16::try_from(text::cells(&model.palette_query)).expect("query width"));
        assert_eq!(
            terminal.get_cursor_position().expect("cursor"),
            Position::new(expected_x, overlay.y.saturating_add(1))
        );

        let wide_label = label_target(4, 2, "e\u{301}\u{754c}\u{1b}", Route::Tasks);
        assert_eq!(wide_label.area.width, 4);
        assert!(wide_label.area.contains(Position::new(7, 2)));
        assert!(!wide_label.area.contains(Position::new(8, 2)));

        model.update(Action::CloseLayer);
        let layout = ShellLayout::new(Rect::new(0, 0, 100, 24), &model);
        for target in &layout.route_targets {
            let position = Position::new(target.area.x, target.area.y);
            assert_eq!(
                hit_test(&layout, &model, position),
                Some(Action::Route(target.value))
            );
        }
        for target in &layout.footer_targets {
            let position = Position::new(target.area.x, target.area.y);
            assert_eq!(
                hit_test(&layout, &model, position),
                target.value.action(&model)
            );
        }

        model.update(Action::Resize {
            columns: 60,
            rows: 16,
        });
        model.update(Action::Focus(Focus::Detail));
        let narrow = ShellLayout::new(Rect::new(0, 0, 60, 16), &model);
        assert_eq!(
            hit_test(
                &narrow,
                &model,
                Position::new(
                    narrow.primary.x.saturating_add(1),
                    narrow.primary.y.saturating_add(1),
                ),
            ),
            Some(Action::Focus(Focus::Detail)),
            "the narrow detail layer is not mistaken for task rows"
        );
    }

    #[test]
    fn disabled_actions_explain_themselves_and_modals_cancel_without_invoking() {
        let mut model = model();
        model.update(Action::Resize {
            columns: 100,
            rows: 24,
        });
        model.update(Action::OpenHelp);
        let help = buffer(&mut model, 100, 24);
        assert!(
            help.contains("disabled: available in Tasks, Inbox, Diagnostics, Graph a"),
            "{help}"
        );
        assert!(help.contains("Esc cancels"));
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &model),
            Some(Action::CloseLayer)
        );
        model.update(Action::CloseLayer);
        assert!(model.modal.is_none());
        assert!(!model.should_quit());

        assert_eq!(
            decode_key(
                KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
                &model
            ),
            Some(Action::Notify(
                "available in Tasks, Inbox, Diagnostics, Graph and Reports".to_string()
            ))
        );

        model.update(Action::Route(Route::Tasks));
        assert_eq!(
            decode_key(
                KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
                &model
            ),
            Some(Action::OpenSearch)
        );

        model.update(Action::Route(Route::Diagnostics));
        model.update(Action::CycleDiagnosticScope);
        assert_eq!(
            decode_key(
                KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
                &model
            ),
            Some(Action::Notify(
                "available for the structural check scope; press v to switch".to_string()
            ))
        );

        model.update(Action::OpenPalette);
        let theme = palette_states(&model)
            .iter()
            .position(|state| state.target == PaletteTarget::Command(CommandId::CycleTheme))
            .expect("theme command");
        model.update(Action::SelectModal(theme));
        let invoke = decode_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &model)
            .expect("palette invocation");
        model.update(invoke);
        assert_eq!(model.theme, Theme::Light);
        assert!(model.modal.is_none(), "invocation returns to the shell");
    }

    /// Proof for `tui.graph_explorer_uses_semantic_scene` and
    /// `tui.layered_graph_is_a_semantic_ratatui_widget`: the fixed backend
    /// paints cards and routed edges in monochrome ASCII, reports exact extents,
    /// and maps node and edge cells through the painter's transform.
    #[allow(clippy::too_many_lines)] // One end-to-end viewport, keyboard and pointer contract.
    #[test]
    fn graph_viewport_is_operable_and_identity_safe_in_ascii_and_small_terminals() {
        let mut model = graph_model();
        let rendered = buffer(&mut model, 80, 18);
        assert!(rendered.contains("r1:"), "{rendered}");
        assert!(rendered.contains(" c"), "{rendered}");
        assert!(rendered.contains("phase 1"), "{rendered}");
        assert!(
            rendered.contains("#>"),
            "selected card missing:\n{rendered}"
        );
        assert!(rendered.contains("TODO"), "{rendered}");
        assert!(
            !rendered.contains(['\u{25cf}', '\u{25cb}', '\u{2588}']),
            "{rendered}"
        );

        let layout = ShellLayout::new(Rect::new(0, 0, 80, 18), &model);
        let viewport = graph_viewport(layout.primary, false, &model);
        let scene = model.graph_cell_scene().expect("scene");
        let graph_area = Rect::new(
            viewport.content.x,
            viewport.content.y.saturating_add(viewport.intro_rows),
            viewport.content.width,
            viewport.content.height.saturating_sub(viewport.intro_rows),
        );
        let screen_position = |point: aplan_app::CellPoint| {
            let x = point.x - scene.bounds.x - i32::try_from(viewport.column_start).unwrap();
            let y = point.y - scene.bounds.y - i32::try_from(viewport.row_start).unwrap();
            (x >= 0 && y >= 0).then(|| {
                Position::new(
                    graph_area.x.saturating_add(u16::try_from(x).unwrap()),
                    graph_area.y.saturating_add(u16::try_from(y).unwrap()),
                )
            })
        };
        let node = scene
            .nodes
            .iter()
            .find(|node| node.identity.id() == "B")
            .expect("B");
        let node_position = screen_position(node.bounds.center()).expect("visible B");
        assert!(graph_area.contains(node_position));
        assert_eq!(
            decode_mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: node_position.x,
                    row: node_position.y,
                    modifiers: KeyModifiers::NONE,
                },
                &model,
            ),
            Some(Action::SelectGraphIdentity(
                aplan_app::GraphExplorerIdentity::Task("B".to_string())
            ))
        );

        let edge_point = scene
            .edges
            .iter()
            .flat_map(|edge| edge.points.iter().copied())
            .filter_map(screen_position)
            .find_map(|screen| {
                let hit = graph_widget::hit_test(
                    scene,
                    graph_area,
                    LayeredViewport {
                        row: viewport.row_start,
                        column: viewport.column_start,
                    },
                    screen,
                    graph_widget::GraphEdgePresentation::All,
                );
                match hit {
                    Some(GraphHit::Edge(identity)) => Some((identity.to, screen)),
                    _ => None,
                }
            })
            .expect("a visible edge-only cell");
        assert_eq!(
            decode_mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: edge_point.1.x,
                    row: edge_point.1.y,
                    modifiers: KeyModifiers::NONE,
                },
                &model,
            ),
            Some(Action::SelectGraphIdentity(edge_point.0))
        );
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), &model),
            Some(Action::SetGraphViewport { row: 5, column: 0 })
        );
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), &model),
            Some(Action::MoveGraph(SpatialDirection::Right))
        );

        model.update(Action::SelectTask("D".to_string()));
        let small = buffer(&mut model, 48, 14);
        assert!(small.contains("|r") && small.contains(" c"), "{small}");
        assert!(
            small.contains("#>"),
            "selection was not kept visible:\n{small}"
        );
    }

    /// Proof for `tui.layered_graph_is_a_semantic_ratatui_widget`: the widget
    /// paints every admitted edge by default, returns semantic targets from
    /// the same cell transform, and does not mutate or reconstruct its scene.
    #[allow(clippy::too_many_lines)] // One complete widget, clipping and presentation audit.
    #[test]
    fn layered_graph_widget_paints_complete_semantic_scene() {
        use ratatui::buffer::Buffer;
        use ratatui::widgets::StatefulWidget as _;

        let model = graph_model();
        let scene = model.graph_cell_scene().expect("layered scene").clone();
        let area = Rect::new(0, 0, 80, 24);
        let mut output = Buffer::empty(area);
        let mut state = LayeredGraphState::default();
        let styles = palette(model.theme);
        LayeredGraph {
            scene: &scene,
            projection: &model.graph_explorer,
            viewport: LayeredViewport { row: 0, column: 0 },
            density: crate::graph_widget::ResolvedGraphDensity::Comfortable,
            ascii: true,
            edge_presentation: graph_widget::GraphEdgePresentation::All,
            styles: GraphStyles {
                normal: styles.normal,
                muted: styles.muted,
                accent: styles.accent,
                selected: styles.selected,
                warning: styles.warning,
            },
        }
        .render(area, &mut output, &mut state);

        assert_eq!(state.painted_nodes, scene.nodes.len());
        assert_eq!(state.painted_edges, scene.edges.len());
        assert!(
            state
                .hits
                .iter()
                .any(|hit| matches!(hit.target, GraphHit::Phase(1)))
        );
        assert!(state.hits.iter().any(|hit| {
            matches!(&hit.target, GraphHit::Node(identity) if identity.id() == "A")
        }));
        assert!(
            state
                .hits
                .iter()
                .any(|hit| matches!(hit.target, GraphHit::Edge(_)))
        );
        assert!(
            state.visited_cells < 20_000,
            "bounded scene paint escaped its budget"
        );
        assert_eq!(model.graph_cell_scene().expect("unchanged scene"), &scene);

        let focus = scene.focus.as_ref().expect("focus");
        let selected = &output[(
            u16::try_from(focus.bounds.x - scene.bounds.x + 2).expect("selected x"),
            u16::try_from(focus.bounds.y - scene.bounds.y).expect("selected y"),
        )];
        assert_eq!(selected.symbol(), "=");
        assert!(selected.modifier.contains(Modifier::BOLD));
        let first_edge = scene.edges.first().expect("routed edge");
        let source = first_edge.source_port.expect("source port").point;
        let target = first_edge.target_port.expect("target port").point;
        let source = &output[(
            u16::try_from(source.x - scene.bounds.x).expect("source x"),
            u16::try_from(source.y - scene.bounds.y).expect("source y"),
        )];
        let target = &output[(
            u16::try_from(target.x - scene.bounds.x).expect("target x"),
            u16::try_from(target.y - scene.bounds.y).expect("target y"),
        )];
        assert_eq!(source.symbol(), "o");
        assert!(matches!(target.symbol(), ">" | "<" | "^" | "v"));
        assert!(matches!(
            graph_widget::hit_test(
                &scene,
                area,
                LayeredViewport { row: 0, column: 0 },
                Position::new(
                    u16::try_from(first_edge.source_port.expect("source port").point.x)
                        .expect("source x"),
                    u16::try_from(first_edge.source_port.expect("source port").point.y)
                        .expect("source y"),
                ),
                graph_widget::GraphEdgePresentation::All,
            ),
            Some(GraphHit::Edge(_))
        ));

        let clipped_area = Rect::new(0, 0, 12, 6);
        let mut clipped = Buffer::empty(clipped_area);
        let mut clipped_state = LayeredGraphState::default();
        LayeredGraph {
            scene: &scene,
            projection: &model.graph_explorer,
            viewport: LayeredViewport { row: 0, column: 0 },
            density: crate::graph_widget::ResolvedGraphDensity::Comfortable,
            ascii: true,
            edge_presentation: graph_widget::GraphEdgePresentation::All,
            styles: GraphStyles {
                normal: styles.normal,
                muted: styles.muted,
                accent: styles.accent,
                selected: styles.selected,
                warning: styles.warning,
            },
        }
        .render(clipped_area, &mut clipped, &mut clipped_state);
        assert!(
            clipped_state
                .hits
                .iter()
                .any(|hit| hit.target == GraphHit::VerticalScrollbar)
        );
        assert!(
            clipped_state
                .hits
                .iter()
                .any(|hit| hit.target == GraphHit::HorizontalScrollbar)
        );
        assert!(
            clipped_state.visited_cells < 1_000,
            "clipped paint iterated off-screen scene cells"
        );

        let mut relations = Buffer::empty(area);
        let mut relation_state = LayeredGraphState::default();
        LayeredGraph {
            scene: &scene,
            projection: &model.graph_explorer,
            viewport: LayeredViewport { row: 0, column: 0 },
            density: crate::graph_widget::ResolvedGraphDensity::Comfortable,
            ascii: true,
            edge_presentation: graph_widget::GraphEdgePresentation::RelationsOnly,
            styles: GraphStyles {
                normal: styles.normal,
                muted: styles.muted,
                accent: styles.accent,
                selected: styles.selected,
                warning: styles.warning,
            },
        }
        .render(area, &mut relations, &mut relation_state);
        let focus = scene.focus.as_ref().expect("focus");
        let direct = scene
            .edges
            .iter()
            .filter(|edge| {
                edge.identity.from == focus.identity || edge.identity.to == focus.identity
            })
            .count();
        assert_eq!(relation_state.painted_edges, direct);
    }

    /// Proof for `tui.selected_graph_card_is_one_continuous_surface`: the
    /// selected style covers the complete card interior rather than ending at
    /// the last character on each semantic row.
    #[test]
    #[allow(clippy::too_many_lines)] // One matrix keeps task/aggregate, theme and glyph coverage together.
    fn selected_graph_card_paints_one_continuous_surface() {
        use ratatui::buffer::Buffer;
        use ratatui::widgets::StatefulWidget as _;

        fn assert_selected_surface(
            model: &Model,
            subject: &str,
            ascii: bool,
            check_isolation: bool,
        ) {
            let scene = model.graph_cell_scene().expect("layered scene");
            let focus = scene.focus.as_ref().expect("selected scene node");
            assert_eq!(focus.identity.id(), subject);
            assert!(
                focus.bounds.width > 3 && focus.bounds.height > 3,
                "the fixture must have a real card interior"
            );
            let area = Rect::new(0, 0, scene.bounds.width, scene.bounds.height);
            let mut output = Buffer::empty(area);
            let mut state = LayeredGraphState::default();
            let styles = palette(model.theme);
            LayeredGraph {
                scene,
                projection: &model.graph_explorer,
                viewport: LayeredViewport { row: 0, column: 0 },
                density: crate::graph_widget::ResolvedGraphDensity::Comfortable,
                ascii,
                edge_presentation: graph_widget::GraphEdgePresentation::All,
                styles: GraphStyles {
                    normal: styles.normal,
                    muted: styles.muted,
                    accent: styles.accent,
                    selected: styles.selected,
                    warning: styles.warning,
                },
            }
            .render(area, &mut output, &mut state);

            let expected = styles.selected.add_modifier(Modifier::BOLD);
            let mut saw_unused_suffix = false;
            for scene_y in focus.bounds.y.saturating_add(1)
                ..focus
                    .bounds
                    .y
                    .saturating_add(i32::from(focus.bounds.height.saturating_sub(1)))
            {
                for scene_x in focus.bounds.x.saturating_add(1)
                    ..focus
                        .bounds
                        .x
                        .saturating_add(i32::from(focus.bounds.width.saturating_sub(1)))
                {
                    let x = u16::try_from(scene_x - scene.bounds.x).expect("visible selected x");
                    let y = u16::try_from(scene_y - scene.bounds.y).expect("visible selected y");
                    let cell = &output[(x, y)];
                    if cell.symbol() == " " {
                        saw_unused_suffix = true;
                    }
                    assert_eq!(
                        cell.fg,
                        expected.fg.unwrap_or_default(),
                        "{subject} at {x},{y}"
                    );
                    assert_eq!(
                        cell.bg,
                        expected.bg.unwrap_or_default(),
                        "{subject} at {x},{y}"
                    );
                    assert!(
                        cell.modifier.contains(expected.add_modifier),
                        "{subject} at {x},{y} lost selected modifiers: {:?}",
                        cell.modifier
                    );
                }
            }
            assert!(
                saw_unused_suffix,
                "fixture has no unused selected-row suffix"
            );

            if check_isolation {
                let neighbour = scene
                    .nodes
                    .iter()
                    .find(|node| node.identity != focus.identity && node.bounds.width > 2)
                    .expect("isolation fixture must include a non-selected card");
                let neighbour_x =
                    u16::try_from(neighbour.bounds.x.saturating_add(1) - scene.bounds.x)
                        .expect("visible neighbour x");
                let neighbour_y =
                    u16::try_from(neighbour.bounds.y.saturating_add(1) - scene.bounds.y)
                        .expect("visible neighbour y");
                let neighbour_cell = &output[(neighbour_x, neighbour_y)];
                if let Some(selected_background) = expected.bg {
                    assert_ne!(
                        neighbour_cell.bg,
                        selected_background,
                        "{subject} leaked its selected background into {}",
                        neighbour.identity.id(),
                    );
                }
                if expected.add_modifier.contains(Modifier::REVERSED) {
                    assert!(
                        !neighbour_cell.modifier.contains(Modifier::REVERSED),
                        "{subject} leaked reverse-video selection into {}",
                        neighbour.identity.id(),
                    );
                }
            }
        }

        let mut task = graph_model();
        for theme in Theme::ALL {
            task.theme = theme;
            for ascii in [true, false] {
                assert_selected_surface(&task, "A", ascii, true);
            }
        }

        let mut milestone = graph_task("M-v1", "release", &["A", "B"]);
        milestone
            .tags
            .push(aplan_core::task::MILESTONE_TAG.to_string());
        let tasks = vec![
            graph_task("A", "root", &[]),
            graph_task("B", "dependent", &["A"]),
            milestone,
        ];
        let catalogue = tasks
            .iter()
            .map(|task| {
                let mut row = task_row(&task.id, &task.title);
                row.tags.clone_from(&task.tags);
                if matches!(task.id.as_str(), "A" | "B") {
                    row.milestone = Some("M-v1".to_string());
                }
                row
            })
            .collect();
        let mut aggregate = ModelBuilder::default()
            .graph_with_catalogue(tasks, catalogue)
            .glyphs(Glyphs::Ascii)
            .build();
        aggregate.update(Action::Route(Route::Graph));
        aggregate.update(Action::OpenGraphLocation(
            aplan_app::GraphExplorerLocation::Phase(1),
        ));
        for theme in Theme::ALL {
            aggregate.theme = theme;
            for ascii in [true, false] {
                assert_selected_surface(&aggregate, "M-v1", ascii, false);
            }
        }
    }

    #[test]
    fn layered_graph_fixed_buffer_is_reviewable_cell_for_cell() {
        let mut model = graph_model();
        let view = buffer(&mut model, 80, 18);
        let expected = include_str!("snapshots/graph-layered-normal-ascii.txt");
        assert_eq!(view, fixture_snapshot(expected));
    }

    #[test]
    fn layered_graph_crossing_cell_has_non_colour_semantics() {
        use ratatui::buffer::Buffer;
        use ratatui::widgets::StatefulWidget as _;

        let model = graph_model();
        let mut scene = model.graph_cell_scene().expect("layered scene").clone();
        let edges = scene.edges.iter().take(2).collect::<Vec<_>>();
        assert_eq!(edges.len(), 2);
        let segment = edges[0]
            .points
            .windows(2)
            .find(|pair| pair[0].x.abs_diff(pair[1].x) + pair[0].y.abs_diff(pair[1].y) > 2)
            .expect("long route segment");
        let at = aplan_app::CellPoint {
            x: i32::midpoint(segment[0].x, segment[1].x),
            y: i32::midpoint(segment[0].y, segment[1].y),
        };
        scene.crossings = vec![aplan_app::CellCrossing {
            at,
            first: edges[0].identity.clone(),
            second: edges[1].identity.clone(),
        }];
        scene.junctions.clear();
        let area = Rect::new(0, 0, 80, 24);
        let mut output = Buffer::empty(area);
        let styles = palette(model.theme);
        LayeredGraph {
            scene: &scene,
            projection: &model.graph_explorer,
            viewport: LayeredViewport { row: 0, column: 0 },
            density: crate::graph_widget::ResolvedGraphDensity::Comfortable,
            ascii: true,
            edge_presentation: graph_widget::GraphEdgePresentation::All,
            styles: GraphStyles {
                normal: styles.normal,
                muted: styles.muted,
                accent: styles.accent,
                selected: styles.selected,
                warning: styles.warning,
            },
        }
        .render(area, &mut output, &mut LayeredGraphState::default());
        let cell = &output[(
            u16::try_from(at.x - scene.bounds.x).expect("crossing x"),
            u16::try_from(at.y - scene.bounds.y).expect("crossing y"),
        )];
        assert_eq!(cell.symbol(), "+");
        assert!(
            cell.modifier
                .contains(Modifier::BOLD | Modifier::UNDERLINED)
        );
    }

    #[test]
    fn layered_graph_adapts_across_sizes_themes_glyphs_and_reduced_motion() {
        for (width, height) in [(48, 14), (80, 18), (120, 30), (160, 40)] {
            for theme in Theme::ALL {
                for glyphs in Glyphs::ALL {
                    let mut model = graph_model();
                    model.theme = theme;
                    model.glyphs = glyphs;
                    model.motion = crate::model::Motion::Reduced;
                    let view = buffer(&mut model, width, height);
                    assert_eq!(view.lines().count(), usize::from(height));
                    assert!(
                        view.contains("Overview | COMPLETE") && view.contains("4/4N 4/4E"),
                        "{theme:?} {glyphs:?}:\n{view}"
                    );
                    assert!(view.contains(" | U"), "{theme:?} {glyphs:?}:\n{view}");
                    if width >= 80 {
                        assert!(view.contains("phase 1"), "{theme:?} {glyphs:?}:\n{view}");
                    }
                    assert!(!view.contains("Graph layout refused"), "{view}");
                    assert_eq!(
                        model
                            .graph_cell_scene()
                            .and_then(|scene| scene.focus.as_ref())
                            .map(|focus| focus.identity.id()),
                        Some("A")
                    );
                    if glyphs == Glyphs::Ascii {
                        assert!(
                            !view.contains(['\u{2588}', '\u{2550}', '\u{25b6}']),
                            "{view}"
                        );
                    }
                }
            }
        }
        let mut comfortable = graph_model();
        let _ = buffer(&mut comfortable, 80, 18);
        comfortable.update(Action::CycleGraphDensity);
        comfortable.update(Action::CycleGraphDensity);
        let view = buffer(&mut comfortable, 160, 40);
        assert!(view.contains("left \u{754c}"), "{view}");
        assert!(view.contains("right e\u{301}"), "{view}");
    }

    /// Proof for `tui.layered_graph_is_a_semantic_ratatui_widget`: selection,
    /// density and Relations Only are presentation state. Relations Only and
    /// selection preserve exact geometry; reversing either aid restores the
    /// deterministic scene without changing semantic identities.
    #[test]
    fn layered_graph_presentation_is_reversible_without_relayout() {
        let mut model = graph_model();
        let all = buffer(&mut model, 80, 18);
        assert!(all.contains("v auto"), "{all}");
        let original = model.graph_cell_scene().expect("scene").clone();

        model.update(Action::ToggleGraphRelationsOnly);
        let relations = buffer(&mut model, 80, 18);
        assert_ne!(all, relations, "the presentation aid suppressed no route");
        assert_eq!(model.graph_cell_scene().expect("same geometry"), &original);
        assert!(relations.contains("o all edges"), "{relations}");

        model.update(Action::ToggleGraphRelationsOnly);
        let restored = buffer(&mut model, 80, 18);
        assert_eq!(restored, all);
        assert_eq!(model.graph_cell_scene().expect("same geometry"), &original);

        model.update(Action::CycleGraphDensity);
        let pinned_compact = buffer(&mut model, 80, 18);
        assert!(pinned_compact.contains("v compact"), "{pinned_compact}");
        assert_eq!(
            model.graph_cell_scene().expect("same compact geometry"),
            &original
        );
        model.update(Action::CycleGraphDensity);
        let pinned_comfortable = buffer(&mut model, 80, 18);
        assert!(
            pinned_comfortable.contains("v comfortable"),
            "{pinned_comfortable}"
        );
        assert_eq!(
            model.graph_density,
            crate::graph_widget::GraphDensity::Comfortable
        );
        assert_eq!(
            model
                .graph_cell_scene()
                .expect("comfortable scene")
                .nodes
                .iter()
                .map(|node| &node.identity)
                .collect::<Vec<_>>(),
            original
                .nodes
                .iter()
                .map(|node| &node.identity)
                .collect::<Vec<_>>()
        );
        model.update(Action::CycleGraphDensity);
        assert_eq!(model.graph_density, crate::graph_widget::GraphDensity::Auto);
        let returned = model.graph_cell_scene().expect("returned compact scene");
        assert_eq!(returned.bounds, original.bounds);
        assert_eq!(returned.nodes, original.nodes);
        assert_eq!(returned.edges, original.edges);
        let auto = buffer(&mut model, 80, 18);
        assert!(auto.contains("v auto"), "{auto}");

        model.update(Action::SelectTask("D".to_string()));
        let selected = model.graph_cell_scene().expect("selected scene");
        assert_eq!(selected.geometry_key, original.geometry_key);
        assert_eq!(selected.bounds, original.bounds);
        assert_eq!(selected.nodes, original.nodes);
        assert_eq!(selected.edges, original.edges);
        assert!(matches!(
            selected.movement,
            aplan_app::CellSceneMovement::ReusedStatusOnly
        ));
        assert_eq!(
            selected.focus.as_ref().map(|focus| focus.identity.id()),
            Some("D")
        );
    }

    #[test]
    fn layered_graph_fallback_is_scrollable_selectable_and_honest() {
        let mut model = fallback_graph_model();
        assert!(model.graph_cell_scene().is_none());
        let fallback = model.graph_scene_fallback().expect("typed fallback");
        assert_eq!(fallback.nodes.len(), 71);
        assert_eq!(fallback.edges.len(), 70);
        let exact_width = model
            .graph_explorer
            .graph
            .nodes
            .iter()
            .map(graph_widget::fallback_node_width)
            .chain(std::iter::once(text::cells(
                &graph_widget::fallback_message(fallback),
            )))
            .max()
            .expect("fallback content");
        assert_eq!(model.graph_content_extent(), (72, exact_width));

        let first = buffer(&mut model, 80, 18);
        assert!(first.contains("FALLBACK"), "{first}");
        assert!(first.contains("Spatial layout is unavailable"), "{first}");
        assert!(!first.contains("Graph layout refused"), "{first}");

        let pan_right = decode_key(
            KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT),
            &model,
        )
        .expect("fallback pan right");
        assert!(matches!(pan_right, Action::SetGraphViewport { column, .. } if column > 0));
        model.update(pan_right);
        let shifted = buffer(&mut model, 80, 18);
        assert_ne!(shifted, first, "horizontal pan did not slice fallback rows");
        assert!(shifted.contains(" c5:"), "{shifted}");

        let action = decode_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), &model)
            .expect("fallback page down");
        assert!(matches!(action, Action::SetGraphViewport { row, .. } if row > 0));
        model.update(action);
        let later = buffer(&mut model, 80, 18);
        assert_ne!(later, first, "page down did not move the fallback viewport");
        assert!(later.contains("r6:14/72"), "{later}");
        assert!(!later.contains("Spatial layout is unavailable"), "{later}");
        assert!(later.contains("T-00"), "{later}");
    }

    /// Proof for `graph.aggregate_levels_are_bounded_and_continuable`: an
    /// oversized plan overview paints a real bounded scene with semantic
    /// continuation controls instead of first failing the cell-scene budget.
    #[test]
    fn oversized_aggregate_graph_opens_a_layered_window() {
        let mut model = aggregate_window_graph_model();
        let scene = model.graph_cell_scene().expect("bounded aggregate scene");
        assert!(scene.nodes.len() <= 10);
        assert_eq!(model.graph_explorer.accounting.admitted_task_ids.len(), 71);
        assert!(!model.graph_explorer.boundaries.is_empty());
        assert!(model.graph_scene_fallback().is_none());

        let rendered = buffer(&mut model, 80, 18);
        assert!(rendered.contains("WINDOWED"), "{rendered}");
        assert!(rendered.contains("n open"), "{rendered}");
        assert!(!rendered.contains("FALLBACK"), "{rendered}");
    }

    #[test]
    /// Proof for `tui.graph_fallback_paint_is_viewport_bounded`.
    fn fallback_paint_visits_only_the_visible_node_range() {
        let model = fallback_graph_model();
        let styles = palette(model.theme);
        let lines = graph_fallback_lines(
            &model,
            GraphViewport {
                content: Rect::new(0, 0, 80, 5),
                intro_rows: 0,
                row_start: 55,
                row_end: 60,
                column_start: 0,
                column_end: 80,
            },
            80,
            styles,
        );
        let visible = lines
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.into_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert_eq!(visible.len(), 5);
        assert!(visible[0].contains("T-054"), "{visible:?}");
        assert!(visible[4].contains("T-058"), "{visible:?}");
        assert!(visible.iter().all(|line| !line.contains("T-000")));
        assert!(
            visible
                .iter()
                .all(|line| !line.contains("Spatial layout is unavailable"))
        );
    }

    #[test]
    /// Proof for `graph.layered_scene_is_complete_or_fallback`.
    fn retained_geometry_is_recovery_input_not_current_paint_or_hits() {
        let mut model = graph_model();
        let _ = buffer(&mut model, 80, 18);
        let mut retained = model.graph_cell_scene().expect("initial scene").clone();
        retained.nodes[0].identity = aplan_app::GraphExplorerIdentity::Task("STALE".to_string());
        let reason = aplan_app::CellSceneDegradation::LimitExceeded {
            resource: "edges".to_string(),
            got: 97,
            limit: 96,
        };
        model.graph_cell_scene =
            aplan_app::CellSceneResult::Fallback(Box::new(aplan_app::CellSceneFallback {
                generation: retained.generation.saturating_add(1),
                input: model.graph_explorer.input.clone(),
                location: model.graph_explorer.location.clone(),
                reasons: vec![reason.clone()],
                nodes: model
                    .graph_explorer
                    .graph
                    .nodes
                    .iter()
                    .map(|node| aplan_app::CellNodeOmission {
                        identity: node.identity.clone(),
                        reason: reason.clone(),
                    })
                    .collect(),
                edges: Vec::new(),
                retained: Some(Box::new(retained)),
            }));

        assert!(model.graph_cell_scene().is_none());
        assert!(model.graph_retained_cell_scene().is_some());
        let rendered = current_buffer(&model, 80, 18);
        assert!(rendered.contains("FALLBACK"), "{rendered}");
        assert!(rendered.contains("edges need"), "{rendered}");
        assert!(
            graph_widget::fallback_message(model.graph_scene_fallback().expect("fallback"))
                .contains("edges need 97, above the layout limit 96")
        );
        assert!(rendered.contains(" A [TODO] root"), "{rendered}");
        assert!(!rendered.contains("STALE"), "{rendered}");
    }

    #[test]
    fn layered_visual_contract_detects_missing_semantic_layers() {
        let mut model = graph_model();
        let baseline = buffer(&mut model, 120, 30);
        let original = model.graph_cell_scene().expect("complete scene").clone();

        let mut variants = Vec::new();
        let mut without_arrow = original.clone();
        without_arrow.edges[0].target_port = None;
        without_arrow.edges[0].arrowhead = None;
        variants.push(("target arrow", without_arrow));

        let mut without_edge = original.clone();
        without_edge.edges.clear();
        variants.push(("dependency routes", without_edge));

        let mut without_node = original.clone();
        without_node.nodes.remove(0);
        variants.push(("node card", without_node));

        let mut without_selection = original.clone();
        without_selection.focus = None;
        variants.push(("selection highlight", without_selection));

        let mut without_grouping = original.clone();
        without_grouping.bands.clear();
        variants.push(("phase grouping", without_grouping));

        for (missing, scene) in variants {
            model.graph_cell_scene = aplan_app::CellSceneResult::Ready(Box::new(scene));
            let mutated = buffer(&mut model, 120, 30);
            assert_ne!(
                mutated, baseline,
                "the fixed-buffer contract did not detect missing {missing}"
            );
        }

        model.graph_cell_scene = aplan_app::CellSceneResult::Ready(Box::new(original));
        model.graph_explorer.graph.nodes[0].title.clear();
        let without_label = buffer(&mut model, 120, 30);
        assert_ne!(
            without_label, baseline,
            "the fixed-buffer contract did not detect a missing node label"
        );

        let mut oversized = oversized_graph_model();
        let with_continuation = buffer(&mut oversized, 80, 18);
        assert!(!oversized.graph_explorer.boundaries.is_empty());
        oversized.graph_explorer.boundaries.clear();
        let without_continuation = buffer(&mut oversized, 80, 18);
        assert_ne!(
            without_continuation, with_continuation,
            "the fixed-buffer contract did not detect missing continuation controls"
        );
    }

    /// Proof for `tui.graph_explorer_windows_oversized_inputs`: accounted
    /// window state and semantic continuation targets remain usable without
    /// colour at normal, narrow and minimum supported sizes.
    #[test]
    fn oversized_graph_renders_and_activates_semantic_window_controls() {
        let mut model = oversized_graph_model();
        let rendered = buffer(&mut model, 80, 18);
        assert!(rendered.contains("WINDOWED"), "{rendered}");
        assert!(
            (2..=model.graph_explorer_request.budget.nodes)
                .contains(&model.graph_explorer.accounting.visible_task_ids.len())
        );
        assert_eq!(model.graph_explorer.accounting.admitted_task_ids.len(), 61);
        assert_eq!(
            model
                .graph_explorer
                .accounting
                .window_omitted_task_ids
                .len(),
            61 - model.graph_explorer.accounting.visible_task_ids.len()
        );
        assert!(rendered.contains("n open"), "{rendered}");
        assert!(!rendered.contains("Graph layout refused"), "{rendered}");

        let layout = ShellLayout::new(Rect::new(0, 0, 80, 18), &model);
        let viewport = graph_viewport(layout.primary, false, &model);
        let target = graph_control_targets(viewport, &model)
            .into_iter()
            .find(|target| target.value == Action::OpenGraphContinuation)
            .expect("painted open-continuation target");
        assert_eq!(
            decode_mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: target.area.x,
                    row: target.area.y,
                    modifiers: KeyModifiers::NONE,
                },
                &model,
            ),
            Some(Action::OpenGraphContinuation)
        );
        model.update(Action::OpenGraphContinuation);
        let traversed = buffer(&mut model, 80, 18);
        assert!(traversed.contains("Esc prior"), "{traversed}");
        assert!(model.has_graph_window_history());

        let request = model.graph_explorer_request.clone();
        for (columns, rows) in [(48, 14), (120, 30), (60, 16), (80, 18)] {
            model.update(Action::Resize { columns, rows });
        }
        assert_eq!(model.graph_explorer_request, request);
        let narrow = buffer(&mut model, 48, 14);
        assert!(narrow.contains("WINDOWED"), "{narrow}");
        assert!(narrow.contains("#>"), "{narrow}");
    }

    #[test]
    fn graph_node_search_is_disclosed_without_changing_scope() {
        let mut model = graph_model();
        model.update(Action::OpenSearch);
        model.update(Action::SearchInput("root".to_string()));
        model.update(Action::CloseLayer);
        let searched = buffer(&mut model, 80, 18);
        assert!(searched.contains("find=root"), "{searched}");
        assert_eq!(model.graph_request.scope, aplan_app::GraphScope::Whole);
    }

    /// Proof for `tui.graph_context_discloses_scope_and_grouping_omissions`.
    #[test]
    fn graph_context_discloses_scope_edges_and_grouping_omissions() {
        let mut milestone = graph_task("M-v1", "release", &["A", "B", "C", "D"]);
        milestone
            .tags
            .push(aplan_core::task::MILESTONE_TAG.to_string());
        let mut root = graph_task("A", "root", &[]);
        root.tags.push("cli".to_string());
        let tasks = vec![
            root,
            graph_task("B", "left", &["A"]),
            graph_task("C", "right", &["A"]),
            graph_task("D", "merge", &["B", "C"]),
            milestone,
        ];
        let mut model = ModelBuilder::default()
            .graph(tasks)
            .glyphs(Glyphs::Unicode)
            .build();
        model.theme = Theme::Monochrome;
        model.update(Action::Route(Route::Graph));
        let grouping = model.graph_explorer.accounting.grouping.clone();
        assert!(grouping.any(), "the fixture must exercise omitted grouping");
        let omission = format!("omit {}M/{}G", grouping.edges, grouping.tasks);

        for (columns, rows) in [(48, 14), (120, 30)] {
            let rendered = buffer(&mut model, columns, rows);
            assert!(rendered.contains(&omission), "{columns}x{rows}: {rendered}");
        }
        model.update(Action::ToggleGlyphs);
        assert!(buffer(&mut model, 48, 14).contains(&omission));

        assert_eq!(graph_scope_control_label(&model), "whole");
        model.update(Action::SelectTask("A".to_string()));
        model.task_selector.tags.push("cli".to_string());
        model.update(Action::CycleGraphScope);
        assert_eq!(graph_scope_control_label(&model), "up:A/dep+1F");
        let upstream = buffer(&mut model, 120, 30);
        assert!(upstream.contains("g up:A/dep+1F"), "{upstream}");
        model.update(Action::ToggleGraphMembership);
        assert_eq!(graph_scope_control_label(&model), "up:A/all+1F");
        model.update(Action::CycleGraphScope);
        model.update(Action::CycleGraphScope);
        assert_eq!(graph_scope_control_label(&model), "unreachable/member+1F");
        let unreachable = buffer(&mut model, 120, 30);
        assert!(
            unreachable.contains("g unreachable/member+1F"),
            "{unreachable}"
        );
        for _ in 0..4 {
            model.update(Action::CycleGraphScope);
        }
        assert_eq!(graph_scope_control_label(&model), "near:A:all:live/all+1F");
        model.update(Action::CycleGraphScope);
        assert_eq!(graph_scope_control_label(&model), "critical/dep+contain+1F");

        model.update(Action::ClearNarrowing);
        model.update(Action::ToggleGraphMembership);
        assert!(!model.graph_explorer.accounting.grouping.any());
        assert!(buffer(&mut model, 48, 14).contains("omit 0M/0G"));
    }

    /// Proof for `tui.graph_explorer_uses_semantic_scene`: changing from
    /// selection-following to manual panning uses the visible origin, not an
    /// obsolete stored offset that would jump away from the task.
    #[test]
    fn manual_graph_pan_begins_at_the_followed_viewport() {
        let mut tasks = vec![graph_task("root", "root", &[])];
        tasks.extend(
            (0..30).map(|index| graph_task(&format!("leaf-{index:02}"), "leaf", &["root"])),
        );
        let mut model = ModelBuilder::default().graph(tasks).build();
        model.update(Action::Route(Route::Graph));
        model.update(Action::Resize {
            columns: 48,
            rows: 14,
        });
        model.update(Action::SelectTask("leaf-29".to_string()));
        let layout = ShellLayout::new(Rect::new(0, 0, 48, 14), &model);
        let centred = graph_viewport(layout.primary, true, &model);
        assert!(
            centred.row_start > 5,
            "fixture must require vertical centering"
        );
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), &model),
            Some(Action::SetGraphViewport {
                row: centred.row_start + 5,
                column: centred.column_start,
            }),
            "the first manual pan must continue from the centred viewport"
        );
    }

    /// Reproducible viewport-render baseline for `A1-435`. Elapsed time is
    /// recorded for release budgeting rather than treated as a stable shared-
    /// runner verdict; the broad limits catch only catastrophic regressions.
    #[test]
    #[ignore = "records the 10,000-node TUI viewport render baseline"]
    fn oversized_graph_viewport_render_latency_baseline() {
        use std::hint::black_box;
        use std::time::{Duration, Instant};

        const NODES: usize = 10_000;
        const LANES: usize = 8;
        const RENDERS: usize = 250;

        let tasks = (0..NODES)
            .map(|index| {
                let blocker = (index >= LANES).then(|| format!("T-{:05}", index - LANES));
                graph_task(
                    &format!("T-{index:05}"),
                    &format!("representative task {index}"),
                    &blocker.as_deref().into_iter().collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        let started = Instant::now();
        let mut model = ModelBuilder::default().graph(tasks).build();
        model.update(Action::Route(Route::Graph));
        let construction = started.elapsed();
        model.update(Action::Resize {
            columns: 80,
            rows: 18,
        });
        let backend = TestBackend::new(80, 18);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let started = Instant::now();
        for _ in 0..RENDERS {
            terminal
                .draw(|frame| render(frame, black_box(&model)))
                .expect("draw oversized viewport");
        }
        let rendering = started.elapsed();

        assert_eq!(
            model.graph_window.source_nodes, NODES,
            "the baseline silently measured a smaller graph"
        );
        assert!(model.graph_window.view.nodes.len() <= 64);
        assert!(model.graph_window.required_lanes <= 20);
        assert!(
            construction < Duration::from_secs(30) && rendering < Duration::from_secs(10),
            "catastrophic viewport regression: construction {construction:?}, {RENDERS} renders {rendering:?}"
        );
        eprintln!(
            "TUI graph viewport baseline: {NODES} nodes, {} edges; construction {construction:?}; {RENDERS} 80x18 renders {rendering:?}",
            model.graph_window.source_edges
        );
    }

    /// Proof for `tui.task_browser_preserves_selector_semantics`: task
    /// discovery is incremental, owns text keys while editing, exposes its
    /// narrowing, and paints the exact matched field cells.
    #[test]
    fn task_browser_search_filters_sort_and_match_highlighting_are_operable() {
        let mut model = model();
        model.update(Action::Resize {
            columns: 100,
            rows: 24,
        });
        model.update(Action::Route(Route::Tasks));
        assert_eq!(
            decode_key(
                KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
                &model
            ),
            Some(Action::OpenSearch)
        );
        model.update(Action::OpenSearch);
        assert_eq!(
            decode_key(
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
                &model
            ),
            Some(Action::SearchInput("q".to_string())),
            "the search field owns printable global shortcuts"
        );
        for character in "first".chars() {
            let action = decode_key(
                KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE),
                &model,
            )
            .expect("search input");
            model.update(action);
        }
        assert_eq!(model.task_browser.rows.len(), 1);
        assert_eq!(
            decode_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &model),
            Some(Action::CloseLayer)
        );
        model.update(Action::CloseLayer);
        let view = buffer(&mut model, 100, 24);
        assert!(view.contains("search `first`; 1/2 shown; sort: ready rank"));

        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|frame| render(frame, &model)).expect("draw");
        assert!(
            terminal.backend().buffer().content().iter().any(|cell| {
                cell.symbol() == "f" && cell.modifier.contains(Modifier::UNDERLINED)
            }),
            "the matched title span lacks non-colour emphasis"
        );

        model.update(Action::ClearNarrowing);
        assert_eq!(
            decode_key(
                KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
                &model
            ),
            Some(Action::CycleTaskSort)
        );
        model.update(Action::CycleTaskSort);
        assert_eq!(model.task_sort, aplan_app::TaskSort::Id);
        assert_eq!(
            decode_key(
                KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE),
                &model
            ),
            Some(Action::OpenFilters)
        );
        model.update(Action::OpenFilters);
        let filter_view = buffer(&mut model, 100, 24);
        assert!(filter_view.contains("status: todo"));
        assert!(filter_view.contains("Enter/space toggles"));
    }

    /// Proof for `tui.task_inspector_uses_show_projection`: every
    /// canonical `Show` section is reachable, wrapped and terminal-inert.
    /// Also proves `tui.distinguishes_owner_and_ephemeral_claim`: the list and
    /// summary render active claim data separately from durable owner.
    #[test]
    fn complete_task_inspector_renders_every_section_and_relationship_state() {
        let mut model = model();
        model.update(Action::Route(Route::Tasks));
        let claim = aplan_core::ClaimRecord {
            holder: "worker".to_string(),
            identity_source: "argument".to_string(),
            claimed_at_epoch_seconds: 40,
            origin: aplan_core::ClaimOrigin {
                host: Some("host-a".to_string()),
                worktree: Some("/repo/worktree".to_string()),
            },
        };
        model.snapshot.task_rows[0].claim = Some(claim.clone());
        model.update(Action::ClearNarrowing);
        let list = buffer(&mut model, 100, 30);
        // At this responsive width the list deliberately elides the tail of
        // the metadata. Assert the semantic prefix survives rather than
        // requiring the hidden closing cells; the inspector below proves the
        // complete holder (`A1-531`).
        assert!(list.contains("[claim:worke"), "{list}");
        assert!(!list.contains("[owner:worker"), "{list}");

        let show = &mut model.snapshot.inspectors[0];
        show.owner = Some("owner\u{1b}[2J".to_string());
        show.claim = Some(claim);
        show.claim_age_seconds = Some(17);
        show.effort_days = Some(3.5);
        show.tags = vec!["ui".to_string(), "urgent".to_string()];
        show.cut = Some("current".to_string());
        show.milestone = Some("M-current".to_string());
        show.body = Some("## Why\n\nA long body with \u{1b}[2J inert control text.".to_string());
        show.paths = vec!["src/a-very-long-path-that-wraps-across-the-pane/**".to_string()];
        show.traces = vec!["DOC.md#decision".to_string()];
        show.trace_fingerprints
            .insert("DOC.md#decision".to_string(), "trace-stamp".to_string());
        show.trace_fingerprints
            .insert("OLD.md#gone".to_string(), "orphan-stamp".to_string());
        show.evidence = vec![aplan_app::Cited {
            kind: "file".to_string(),
            reference: "proof.md#result".to_string(),
            captured: None,
            observation: aplan_app::EvidenceObservation::not_observed("test fixture"),
        }];
        show.evidence_fingerprints.insert(
            "file:proof.md#result".to_string(),
            "evidence-stamp".to_string(),
        );
        show.waits_on = vec![
            aplan_app::Related {
                id: "MISSING".to_string(),
                status: "unknown".to_string(),
                availability: aplan_app::RelatedAvailability::Missing,
                source: None,
            },
            aplan_app::Related {
                id: "BROKEN".to_string(),
                status: "unknown".to_string(),
                availability: aplan_app::RelatedAvailability::Unloadable,
                source: Some("aplan/tasks/BROKEN.md".to_string()),
            },
        ];
        show.findings = vec![aplan_app::Finding {
            code: "APLAN018".to_string(),
            severity: "warning".to_string(),
            message: "trace DOC.md#decision is stale".to_string(),
            fix: Some("review and restamp it".to_string()),
            repair: None,
        }];
        model.update(Action::Resize {
            columns: 100,
            rows: 30,
        });
        model.update(Action::OpenDetail);

        let summary = buffer(&mut model, 100, 30);
        assert!(summary.contains("[Summary] Work Body Relations"));
        assert!(summary.contains("RESPONSIBILITY"));
        assert!(summary.contains("Owner"));
        assert!(summary.contains("owner\u{fffd}[2J"));
        assert!(summary.contains("Claim"));
        assert!(summary.contains("held by worker for 17s"));
        assert!(summary.contains("Claim source"));
        assert!(summary.contains("argument; host host-a"));
        assert!(summary.contains("/repo/worktree"), "{summary}");
        assert!(!summary.contains('\u{1b}'));

        let expected = [
            (InspectorSection::Work, "Why traversal"),
            (InspectorSection::Body, "A long body with \u{fffd}[2J"),
            (
                InspectorSection::Relationships,
                "MISSING (missing reference)",
            ),
            (InspectorSection::Paths, "src/a-very-long-path"),
            (InspectorSection::Traces, "DOC.md#decision"),
            (InspectorSection::Evidence, "proof.md#result"),
            (InspectorSection::Findings, "APLAN018"),
        ];
        for (section, text) in expected {
            model.inspector_section = section;
            model.inspector_selected = 0;
            model.inspector_scroll = None;
            let view = buffer(&mut model, 100, 30);
            assert!(view.contains(text), "{} section:\n{view}", section.label());
            assert!(!view.contains('\u{1b}'));
        }
        model.inspector_section = InspectorSection::Relationships;
        let relationships = buffer(&mut model, 100, 30);
        assert!(relationships.contains("BROKEN"));
        assert!(relationships.contains("unloadable task at"));
        assert!(
            model
                .inspector_items()
                .iter()
                .any(|item| item.semantic_text().contains("aplan/tasks/BROKEN.md"))
        );
        model.inspector_section = InspectorSection::Traces;
        let traces = buffer(&mut model, 100, 30);
        assert!(traces.contains("orphaned fingerprint"), "{traces}");
        assert!(traces.contains("trace is no longer declared"), "{traces}");
        assert!(
            model
                .inspector_items()
                .iter()
                .any(|item| item.semantic_text().contains("orphan-stamp"))
        );

        model.update(Action::Focus(Focus::Primary));
        model.snapshot.task_rows[0].body = "buried search phrase".to_string();
        model.update(Action::Route(Route::Tasks));
        model.update(Action::OpenSearch);
        model.update(Action::SearchInput("buried".to_string()));
        model.update(Action::CloseLayer);
        assert!(buffer(&mut model, 100, 30).contains("[body]"));
    }

    /// Proof for `tui.inspectors_use_semantic_information_hierarchy`: typed
    /// groups, badges, callouts and exact values retain non-colour meaning in
    /// every appearance, stack at the minimum width and keep link identity
    /// stable across reflow.
    #[test]
    fn semantic_inspector_hierarchy_survives_appearance_width_and_reflow() {
        let mut model = model();
        model.update(Action::Route(Route::Tasks));
        let show = &mut model.snapshot.inspectors[0];
        show.status = "blocked".to_string();
        show.ready = false;
        show.workable = true;
        show.owner = Some("owner\u{1b}[2J\u{754c}".to_string());
        show.findings = vec![aplan_app::Finding {
            code: "APLAN999".to_string(),
            severity: "warning".to_string(),
            message: "review the exact value".to_string(),
            fix: None,
            repair: None,
        }];
        model.update(Action::OpenDetail);

        let status = model
            .inspector_items()
            .iter()
            .position(|item| {
                matches!(
                    &item.presentation,
                    InspectorPresentation::Badge { label, .. } if label == "Status"
                )
            })
            .expect("status badge");
        let owner = model
            .inspector_items()
            .iter()
            .position(|item| {
                matches!(
                    &item.presentation,
                    InspectorPresentation::Labelled { label, .. } if label == "Owner"
                )
            })
            .expect("owner field");
        let work = model
            .inspector_items()
            .iter()
            .position(|item| {
                item.link == Some(crate::model::InspectorLink::Section(InspectorSection::Work))
            })
            .expect("work link");

        for theme in [
            Theme::Dark,
            Theme::Light,
            Theme::LowColor,
            Theme::Monochrome,
        ] {
            for glyphs in [Glyphs::Unicode, Glyphs::Ascii] {
                model.theme = theme;
                model.glyphs = glyphs;
                model.inspector_section = InspectorSection::Summary;
                model.inspector_selected = status;
                model.inspector_scroll = None;
                let compact = buffer(&mut model, 48, 14);
                assert!(
                    compact.contains("-- STATE --"),
                    "{theme:?} {glyphs:?}:\n{compact}"
                );
                assert!(
                    compact.contains("Status"),
                    "{theme:?} {glyphs:?}:\n{compact}"
                );
                assert!(
                    compact.contains("[blocked]"),
                    "{theme:?} {glyphs:?}:\n{compact}"
                );

                model.inspector_selected = owner;
                model.inspector_scroll = None;
                let normal = buffer(&mut model, 100, 30);
                assert!(normal.contains("-- RESPONSIBILITY --"), "{normal}");
                assert!(normal.contains("owner\u{fffd}[2J\u{754c}"), "{normal}");
                assert!(!normal.contains('\u{1b}'));
            }
        }

        model.inspector_selected = work;
        model.inspector_scroll = Some(3);
        for (columns, rows) in [(140, 36), (80, 24), (48, 14), (100, 30)] {
            model.update(Action::Resize { columns, rows });
            assert_eq!(model.inspector_selected, work);
            assert_eq!(model.inspector_scroll, Some(3));
            assert_eq!(
                model.inspector_items()[work].link,
                Some(crate::model::InspectorLink::Section(InspectorSection::Work))
            );
        }
        model.update(Action::OpenDetail);
        assert_eq!(model.inspector_section, InspectorSection::Work);

        model.inspector_section = InspectorSection::Findings;
        model.inspector_selected = 0;
        model.inspector_scroll = None;
        let finding_group = buffer(&mut model, 100, 30);
        assert!(
            finding_group.contains("-- PLAN HEALTH --"),
            "{finding_group}"
        );
        model.inspector_scroll = None;
        let finding = buffer(&mut model, 48, 14);
        assert!(finding.contains("[!] warning APLAN999"), "{finding}");
        assert!(finding.contains("review the exact value"), "{finding}");
    }

    /// Proof for `tui.inspectors_use_semantic_information_hierarchy`: a
    /// terminal task keeps its exact state and explicit readiness distinction
    /// without relying on its muted colour.
    #[test]
    fn semantic_inspector_terminal_state_retains_textual_meaning() {
        let mut model = model();
        model.update(Action::Route(Route::Tasks));
        model.snapshot.inspectors[0].status = "done".to_string();
        model.snapshot.inspectors[0].workable = false;
        model.snapshot.inspectors[0].ready = false;
        model.update(Action::OpenDetail);
        model.inspector_selected = 1;

        let terminal = buffer(&mut model, 100, 30);
        assert!(terminal.contains("[done]"), "{terminal}");
        assert!(
            terminal.contains("not applicable to termina")
                && terminal.contains("l work or milestones]"),
            "{terminal}"
        );
    }

    /// Proof for `tui.work_discovery_is_one_typed_flow` and
    /// `tui.work_discovery_state_is_semantically_exact`: the rendered Work
    /// section exposes rank inputs and attributes an inactive preference to
    /// its unavailable target, while contextual keys change traversal and open
    /// the distinct due-clock editor.
    #[test]
    fn work_discovery_commands_are_contextual_and_rendered() {
        let other = graph_task("OTHER", "other ready task", &[]);
        let mut root = graph_task("ROOT", "ranked root", &[]);
        root.prefer_after = vec!["OTHER".to_string(), "DONE".to_string()];
        let mut done = graph_task("DONE", "completed preference", &[]);
        done.status = aplan_app::Status::Done;
        let child = graph_task("CHILD", "waiting child", &["ROOT"]);
        let mut model = ModelBuilder::default()
            .graph(vec![other, root, done, child])
            .build();
        model.update(Action::Route(Route::Tasks));
        let root = model
            .task_browser
            .rows
            .iter()
            .position(|row| row.task.id == "ROOT")
            .expect("root row");
        model.update(Action::SelectIndex(root));
        model.update(Action::OpenDetail);
        model.inspector_section = InspectorSection::Work;
        assert!(model.inspector_items().iter().any(|item| {
            item.semantic_text()
                .contains("Prefers after (declared; preferred task is not currently offered): DONE")
        }));
        let view = buffer(&mut model, 100, 45);
        assert!(view.contains("Ready rank"), "{view}");
        assert!(view.contains("#2 - releases 1"), "{view}");
        assert!(
            view.contains("Prefers after (active in this ready"),
            "{view}"
        );
        assert!(view.contains("OTHER (todo) - other ready task"), "{view}");
        assert!(view.contains("DONE (done)"), "{view}");
        assert!(view.contains("Why traversal"));
        assert!(view.contains("cut-bounded"));

        let why = decode_key(
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE),
            &model,
        )
        .expect("why traversal key");
        assert_eq!(why, Action::ToggleWhyTraversal);
        model.update(why);
        let complete = buffer(&mut model, 100, 45);
        assert!(complete.contains("Why traversal"));
        assert!(complete.contains("complete"));

        let date = decode_key(
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
            &model,
        )
        .expect("work date key");
        assert_eq!(date, Action::OpenWorkDate);
        model.update(date);
        let modal_view = buffer(&mut model, 100, 30);
        assert!(modal_view.contains("Work discovery as-of instant"));
        assert!(modal_view.contains("RFC 3339 or signed epoch seconds"));
    }

    /// Proof for `tui.task_inspector_uses_show_projection`: page movement is
    /// clamped to the exact responsive pane and an explicit top offset remains
    /// distinct from automatic selected-item following.
    #[test]
    fn inspector_page_scrolling_clamps_and_can_leave_selection_following() {
        let mut model = model();
        model.update(Action::Route(Route::Tasks));
        model.update(Action::Resize {
            columns: 48,
            rows: 14,
        });
        model.update(Action::OpenDetail);

        model.inspector_selected = model.inspector_items().len().saturating_sub(1);
        let (automatic, _) = inspector_scroll_bounds(&model).expect("inspector bounds");
        assert!(
            automatic > 0,
            "the selected item should start below the viewport"
        );
        for _ in 0..automatic.div_ceil(5) {
            let action = decode_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE), &model)
                .expect("page up");
            model.update(action);
        }
        assert_eq!(model.inspector_scroll, Some(0));
        assert_eq!(
            inspector_scroll_bounds(&model).map(|bounds| bounds.0),
            Some(0)
        );

        model.inspector_section = InspectorSection::Body;
        model.snapshot.inspectors[0].body = Some(
            (0..100)
                .map(|index| format!("body line {index}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        model.inspector_scroll = None;
        for _ in 0..100 {
            let action = decode_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), &model)
                .expect("page down");
            model.update(action);
        }
        let (_, maximum) = inspector_scroll_bounds(&model).expect("body bounds");
        assert_eq!(model.inspector_scroll, Some(maximum));

        let action = decode_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE), &model)
            .expect("page up from bottom");
        model.update(action);
        assert_eq!(
            model.inspector_scroll,
            Some(maximum.saturating_sub(5)),
            "overscrolling the bottom must not make PageUp appear stuck"
        );
    }

    /// Proof for `tui.task_browser_state_is_refresh_and_repeat_safe`: editing
    /// repeats are meaningful, but a repeat never accepts or dismisses a modal.
    #[test]
    fn task_search_accepts_enhanced_keyboard_repeat_events() {
        let mut model = model();
        model.update(Action::Resize {
            columns: 100,
            rows: 24,
        });
        model.update(Action::Route(Route::Tasks));
        model.update(Action::OpenSearch);

        let repeated = |code| KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Repeat,
            state: KeyEventState::NONE,
        };
        assert_eq!(
            decode_key(repeated(KeyCode::Char('a')), &model),
            Some(Action::SearchInput("a".to_string()))
        );
        assert_eq!(
            decode_key(repeated(KeyCode::Backspace), &model),
            Some(Action::SearchBackspace)
        );
        assert_eq!(
            decode_key(repeated(KeyCode::Delete), &model),
            Some(Action::SearchDelete)
        );
        assert_eq!(
            decode_key(repeated(KeyCode::Left), &model),
            Some(Action::SearchLeft)
        );
        assert_eq!(
            decode_key(repeated(KeyCode::Right), &model),
            Some(Action::SearchRight)
        );
        assert_eq!(
            decode_key(repeated(KeyCode::Home), &model),
            Some(Action::SearchHome)
        );
        assert_eq!(
            decode_key(repeated(KeyCode::End), &model),
            Some(Action::SearchEnd)
        );
        assert_eq!(
            decode_key(
                KeyEvent {
                    modifiers: KeyModifiers::CONTROL,
                    ..repeated(KeyCode::Char('u'))
                },
                &model
            ),
            Some(Action::SearchClear)
        );
        assert_eq!(decode_key(repeated(KeyCode::Enter), &model), None);
        assert_eq!(decode_key(repeated(KeyCode::Esc), &model), None);
    }

    /// Proof for `tui.task_browser_state_is_refresh_and_repeat_safe`: hidden
    /// identity cannot become a stale visual or pointer viewport coordinate.
    #[test]
    fn a_hidden_selection_anchors_rendering_and_hit_testing_at_the_visible_start() {
        let mut model = model();
        model.snapshot.task_rows = (1..=20)
            .map(|index| {
                let mut row = task_row(format!("A-{index}"), format!("task {index}"));
                if index <= 10 {
                    row.owner = Some("lee".to_string());
                }
                row
            })
            .collect();
        model.update(Action::ClearNarrowing);
        model.update(Action::Resize {
            columns: 48,
            rows: 14,
        });
        model.update(Action::Route(Route::Tasks));
        select_task(&mut model, "A-19");
        model.update(Action::OpenFilters);
        let owner = model
            .filter_choices()
            .iter()
            .position(|filter| filter.label() == "owner: lee")
            .expect("owner filter");
        model.update(Action::ToggleFilter(owner));
        model.update(Action::CloseLayer);

        assert!(model.selected_task().is_none());
        let view = buffer(&mut model, 48, 14);
        assert!(
            view.contains("A-1"),
            "visible results started at a stale row:\n{view}"
        );

        let layout = ShellLayout::new(Rect::new(0, 0, 48, 14), &model);
        let viewport = task_viewport(
            layout.primary,
            layout.class == LayoutClass::Narrow,
            model.selected,
            model.task_browser.rows.len(),
        );
        assert_eq!(viewport.start, 0);
        let first_row = Position::new(
            viewport.content.x,
            viewport.content.y.saturating_add(viewport.intro_rows),
        );
        assert_eq!(
            hit_test(&layout, &model, first_row),
            Some(Action::SelectIndex(0))
        );
    }

    /// Proof for `tui.selection_is_visible_and_pointer_safe`: the selected
    /// task, painted row and pointer target are one viewport calculation.
    #[test]
    fn selection_viewport_and_pointer_geometry_stay_in_lockstep() {
        let mut model = model();
        model.update(Action::Route(Route::Tasks));
        model.snapshot.task_rows = (1..=8)
            .map(|index| task_row(format!("A-{index}"), format!("task {index}")))
            .collect();
        model.update(Action::ClearNarrowing);
        model.update(Action::Resize {
            columns: 48,
            rows: 14,
        });
        model.update(Action::SelectIndex(7));
        let view = buffer(&mut model, 48, 14);
        assert!(view.contains("A-8"), "selected row was clipped:\n{view}");
        assert!(
            !view.contains("A-1"),
            "viewport did not follow selection:\n{view}"
        );

        let layout = ShellLayout::new(Rect::new(0, 0, 48, 14), &model);
        let content = layout.primary.inner(ratatui::layout::Margin::new(1, 1));
        let viewport = task_viewport(
            layout.primary,
            layout.class == LayoutClass::Narrow,
            model.selected,
            model.task_browser.rows.len(),
        );
        let selected_position = Position::new(
            content.x,
            content
                .y
                .saturating_add(viewport.intro_rows)
                .saturating_add(u16::try_from(7usize.saturating_sub(viewport.start)).unwrap()),
        );
        assert_eq!(
            hit_test(&layout, &model, selected_position),
            Some(Action::SelectIndex(7))
        );
        assert_eq!(
            hit_test(
                &layout,
                &model,
                Position::new(layout.primary.x, layout.primary.y)
            ),
            None,
            "a border cell is not a task row"
        );

        model.update(Action::Focus(Focus::Navigation));
        let click = hit_test(&layout, &model, selected_position).expect("task click");
        model.update(click);
        assert_eq!(model.focus, Focus::Primary, "a task click focuses its list");

        model.update(Action::Focus(Focus::Navigation));
        let scroll = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: content.x,
            row: selected_position.y,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(
            decode_mouse(scroll, &model),
            None,
            "wheel input at the last row must not wrap to unrelated work"
        );
        assert_eq!(model.selected, 7);

        model.update(Action::SelectIndex(6));
        let action = decode_mouse(scroll, &model).expect("wheel advances within the list");
        assert_eq!(action, Action::SelectIndex(7));
        model.update(action);
        assert_eq!(
            model.focus,
            Focus::Primary,
            "wheel selection focuses its list"
        );

        model.update(Action::Focus(Focus::Detail));
        assert_eq!(
            decode_mouse(scroll, &model),
            None,
            "scrolling visible narrow detail must not move a hidden task list"
        );

        let normal = ShellLayout::new(Rect::new(0, 0, 100, 24), &model);
        let detail = normal.detail.expect("normal detail");
        assert_eq!(
            hit_test(&normal, &model, Position::new(detail.x, detail.y)),
            None,
            "a detail border is not interactive content"
        );

        model.update(Action::Focus(Focus::Primary));
        model.snapshot.task_rows[0] = task_row(format!("0{}", "ID".repeat(80)), "title".repeat(80));
        model.update(Action::ClearNarrowing);
        model.update(Action::SelectIndex(0));
        let view = buffer(&mut model, 60, 16);
        let layout = ShellLayout::new(Rect::new(0, 0, 60, 16), &model);
        let first_row = usize::from(layout.primary.y.saturating_add(2));
        let lines = view.lines().collect::<Vec<_>>();
        assert!(lines[first_row].contains('>'));
        assert!(
            lines[first_row + 1].contains("A-2"),
            "a long id wrapped:\n{view}"
        );
    }

    /// Proof for `tui.selection_is_visible_and_pointer_safe`: hidden layers
    /// cannot trap the refusal's quit key and editable/rendered text is bounded.
    #[test]
    fn minimum_refusal_and_palette_inputs_remain_bounded_and_escapable() {
        let mut model = model();
        model.update(Action::Resize {
            columns: 48,
            rows: 14,
        });
        model.update(Action::OpenPalette);
        let q = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(
            decode_key(q, &model),
            Some(Action::PaletteInput("q".to_string()))
        );

        model.update(Action::Resize {
            columns: 47,
            rows: 13,
        });
        assert_eq!(decode_key(q, &model), Some(Action::Quit));
        model.update(Action::CloseLayer);
        model.update(Action::OpenHelp);
        assert_eq!(decode_key(q, &model), Some(Action::Quit));

        model.update(Action::Resize {
            columns: 48,
            rows: 14,
        });
        model.update(Action::CloseLayer);
        model.update(Action::OpenPalette);
        model.update(Action::PaletteInput("beginning-".repeat(12)));
        let view = buffer(&mut model, 48, 14);
        assert!(
            view.contains("beginning-"),
            "query tail disappeared:\n{view}"
        );
        let backend = TestBackend::new(48, 14);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|frame| render(frame, &model)).expect("draw");
        let cursor = terminal.get_cursor_position().expect("cursor");
        let overlay = modal_area(Rect::new(0, 0, 48, 14));
        assert!(cursor.x < overlay.right().saturating_sub(1));

        model.update(Action::CloseLayer);
        model.snapshot.overview.input.plan_root = "repository-root/".repeat(100).into();
        model.update(Action::Notify("status-message/".repeat(100)));
        model.glyphs = Glyphs::Ascii;
        let view = buffer(&mut model, 48, 14);
        let layout = ShellLayout::new(Rect::new(0, 0, 48, 14), &model);
        let lines = view.lines().collect::<Vec<_>>();
        let status = lines[usize::from(layout.footer.y.saturating_add(1))];
        assert!(
            status.contains("status-message/") && status.ends_with('>'),
            "oversized status needs an explicit marker:\n{view}"
        );
        assert!(
            lines[usize::from(layout.header.y)].contains("repo")
                && lines[usize::from(layout.header.y)].contains('>'),
            "oversized root needs an explicit marker:\n{view}"
        );
    }

    fn assert_hostile_inspector_is_bounded(
        model: &mut Model,
        width: u16,
        height: u16,
        glyphs: Glyphs,
    ) -> ShellLayout {
        model.glyphs = glyphs;
        model.update(Action::Route(Route::Tasks));
        model.snapshot.overview.input.plan_root = "repo".into();
        let hostile = format!(
            "A\u{1b}e\u{301}\u{754c}{}-unbroken-tail",
            "\u{200b}".repeat(4_096)
        );
        model.snapshot.task_rows[0] = task_row("A-hostile".to_string(), hostile.clone());
        model.snapshot.inspectors[0] = task_inspector(&model.snapshot.task_rows[0]);
        model.update(Action::ClearNarrowing);
        select_task(model, "A-hostile");
        model.update(Action::Focus(Focus::Detail));

        let view = buffer(model, width, height);
        let layout = ShellLayout::new(Rect::new(0, 0, width, height), model);
        assert_ne!(layout.class, LayoutClass::TooSmall);
        let detail = if layout.class == LayoutClass::Narrow {
            layout.primary
        } else {
            layout.detail.expect("split layouts have a detail pane")
        };
        let rows = view.lines().collect::<Vec<_>>();
        let marker = if glyphs == Glyphs::Ascii {
            ">"
        } else {
            "\u{2026}"
        };
        let detail_rows = &rows[usize::from(detail.y.saturating_add(1))
            ..usize::from(detail.bottom().saturating_sub(1))];
        assert!(
            detail_rows.iter().any(|row| row.contains(marker)),
            "wrapped inspector overflow has no continuation marker:\n{view}"
        );

        // Jump to the exact end of the semantic inspector. The first frame
        // above proves undisclosed overflow; this one proves the far end of a
        // hostile identity remains reachable without reconstructing renderer
        // line counts from display prose.
        model.inspector_selected = 0;
        let content_width = usize::from(detail.width.saturating_sub(2));
        let (_, selected_range) = inspector_lines(
            model,
            content_width,
            glyphs == Glyphs::Ascii,
            palette(model.theme),
        );
        let (_, title_end) = selected_range.expect("the identity row is selected");
        let capacity = usize::from(detail.height.saturating_sub(2));
        model.inspector_scroll = Some(title_end.saturating_sub(capacity));
        let view = buffer(model, width, height);
        let rows = view.lines().collect::<Vec<_>>();
        let detail_rows = &rows[usize::from(detail.y.saturating_add(1))
            ..usize::from(detail.bottom().saturating_sub(1))];
        assert!(
            detail_rows.iter().any(|row| row.contains("unbroken-tail")),
            "wrapped hostile value disappeared:\n{view}"
        );
        let upward = if glyphs == Glyphs::Ascii {
            "^"
        } else {
            "\u{2191}"
        };
        assert!(
            detail_rows.iter().any(|row| row.contains(upward)),
            "scrolled inspector hides its preceding content:\n{view}"
        );
        for row in detail_rows {
            assert!(!row.contains('\u{1b}'), "control escaped sanitization");
            assert!(row.len() < 1_024, "zero-width content escaped byte bound");
        }
        let fitted = text::fit(
            &hostile,
            usize::from(detail.width.saturating_sub(2)),
            glyphs == Glyphs::Ascii,
        );
        assert!(text::cells(&fitted) <= usize::from(detail.width.saturating_sub(2)));
        assert!(fitted.len() < 1_024);
        layout
    }

    fn assert_bounded_layout(width: u16, height: u16, glyphs: Glyphs) {
        let mut model = model();
        let layout = assert_hostile_inspector_is_bounded(&mut model, width, height, glyphs);
        let marker = if glyphs == Glyphs::Ascii {
            ">"
        } else {
            "\u{2026}"
        };

        let chrome_content = |area: Rect| {
            if layout.density == ChromeDensity::Compact {
                area
            } else {
                area.inner(Margin::new(1, 1))
            }
        };
        for (area, targets) in [
            (
                chrome_content(layout.navigation),
                layout
                    .route_targets
                    .iter()
                    .map(|target| target.area)
                    .collect::<Vec<_>>(),
            ),
            (
                chrome_content(layout.footer),
                layout
                    .footer_targets
                    .iter()
                    .map(|target| target.area)
                    .collect::<Vec<_>>(),
            ),
        ] {
            for target in targets {
                assert!(area.contains(Position::new(target.x, target.y)));
                assert!(target.right() <= area.right());
                assert!(target.bottom() <= area.bottom());
            }
        }

        model.update(Action::Focus(Focus::Primary));
        model.update(Action::Route(Route::Graph));
        let graph = buffer(&mut model, width, height);
        let graph_layout = ShellLayout::new(Rect::new(0, 0, width, height), &model);
        if graph_layout.class != LayoutClass::Narrow {
            assert!(graph.contains("2/2N 0/0E"), "{graph}");
            assert!(graph.contains("g whole"), "{graph}");
        }

        model.update(Action::OpenHelp);
        let search = action_states(&model)
            .iter()
            .position(|state| state.target == PaletteTarget::Command(CommandId::Search))
            .expect("search command");
        model.update(Action::SelectModal(search));
        let help = buffer(&mut model, width, height);
        let overlay = modal_area(Rect::new(0, 0, width, height));
        let full = "/       find a node without changing graph scope - available";
        if text::cells(full) > usize::from(overlay.width.saturating_sub(2)) {
            assert!(
                help.lines()
                    .any(|row| row.contains("find a node") && row.contains(marker)),
                "modal row clipped without a marker:\n{help}"
            );
        } else {
            assert!(
                help.contains(full),
                "modal lost its disabled reason:\n{help}"
            );
        }
    }

    /// Proof for `tui.shell_content_is_explicitly_bounded`: every shell-owned
    /// surface fits hostile content at the exact responsive class boundaries,
    /// and every pointer target remains inside the cells that painted it.
    #[test]
    fn every_responsive_surface_fits_content_at_layout_boundaries() {
        for (width, height) in [(48, 14), (79, 17), (80, 18), (119, 23), (120, 24)] {
            for glyphs in Glyphs::ALL {
                assert_bounded_layout(width, height, glyphs);
            }
        }

        let mut model = model();
        model.update(Action::Resize {
            columns: 48,
            rows: 14,
        });
        model.update(Action::Route(Route::Tasks));
        model.update(Action::SelectIndex(0));
        let layout = ShellLayout::new(Rect::new(0, 0, 48, 14), &model);
        let point = layout.primary.inner(Margin::new(1, 1));
        let up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: point.x,
            row: point.y.saturating_add(1),
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(decode_mouse(up, &model), None);
        assert_eq!(model.selected, 0);

        model.snapshot.task_rows[0] = task_row(String::new(), String::new());
        model.update(Action::ClearNarrowing);
        model.update(Action::Focus(Focus::Detail));
        let empty = buffer(&mut model, 48, 14);
        assert!(empty.contains("> A-1 / Summary <"));
        assert_eq!(empty.lines().count(), 14);
        assert_eq!(model.selected, 0);
        assert_eq!(model.focus, Focus::Detail);
    }
}
