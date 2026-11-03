use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

/// Make repository and terminal input inert before it reaches layout. This is
/// the same two-part boundary as the CLI human renderer: controls become
/// visible replacement markers, then layout whitespace becomes an ordinary
/// cell rather than a row or cursor movement.
pub(crate) fn safe(text: &str) -> String {
    aplan_output::fold::fold(&aplan_output::sanitize::visible(text), usize::MAX, 0)
}

pub(crate) fn cells(text: &str) -> usize {
    text.width()
}

fn render_limits(max_cells: usize) -> (usize, usize) {
    (
        max_cells.saturating_mul(4).saturating_add(16),
        max_cells.saturating_mul(16).saturating_add(64),
    )
}

fn prefix_within(text: &str, max_cells: usize) -> (String, bool) {
    let (max_graphemes, max_bytes) = render_limits(max_cells);
    let mut fitted = String::new();
    let mut used_cells = 0usize;
    for (used_graphemes, grapheme) in text.graphemes(true).enumerate() {
        let width = cells(grapheme);
        if used_graphemes == max_graphemes
            || fitted.len().saturating_add(grapheme.len()) > max_bytes
            || used_cells.saturating_add(width) > max_cells
        {
            return (fitted, true);
        }
        fitted.push_str(grapheme);
        used_cells += width;
    }
    (fitted, false)
}

fn suffix_within(text: &str, max_cells: usize) -> (String, bool) {
    let (max_graphemes, max_bytes) = render_limits(max_cells);
    let mut reversed = Vec::new();
    let mut used_cells = 0usize;
    let mut used_bytes = 0usize;
    for grapheme in text.graphemes(true).rev() {
        let width = cells(grapheme);
        if reversed.len() == max_graphemes
            || used_bytes.saturating_add(grapheme.len()) > max_bytes
            || used_cells.saturating_add(width) > max_cells
        {
            reversed.reverse();
            return (reversed.concat(), true);
        }
        reversed.push(grapheme);
        used_bytes += grapheme.len();
        used_cells += width;
    }
    reversed.reverse();
    (reversed.concat(), false)
}

/// Fit inert text to a cell budget without splitting an extended grapheme.
/// The continuation marker is itself part of the budget.
pub(crate) fn fit(text: &str, max_cells: usize, ascii: bool) -> String {
    let text = safe(text);
    if max_cells == 0 {
        return String::new();
    }

    let (fitted, truncated) = prefix_within(&text, max_cells);
    if !truncated {
        return fitted;
    }

    let marker = if ascii { ">" } else { "\u{2026}" };
    let content_budget = max_cells.saturating_sub(cells(marker));
    let (mut fitted, _) = prefix_within(&text, content_budget);
    fitted.push_str(marker);
    fitted
}

/// Fit inert text to a cell budget while preserving its editable tail. The
/// continuation marker occupies the first cell when an earlier prefix is not
/// shown.
pub(crate) fn fit_tail(text: &str, max_cells: usize, ascii: bool) -> String {
    let text = safe(text);
    if max_cells == 0 {
        return String::new();
    }

    let (fitted, truncated) = suffix_within(&text, max_cells);
    if !truncated {
        return fitted;
    }

    let marker = if ascii { ">" } else { "\u{2026}" };
    let content_budget = max_cells.saturating_sub(cells(marker));
    let (fitted, _) = suffix_within(&text, content_budget);
    format!("{marker}{fitted}")
}

/// A cell-coordinate slice that never splits an extended grapheme.
///
/// When a boundary crosses a wide grapheme, spaces occupy the intersecting
/// cells. That preserves scene coordinates without drawing half a character.
pub(crate) fn slice_cells(text: &str, start: usize, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let text = safe(text);
    let end = start.saturating_add(width);
    let mut column = 0usize;
    let mut out = String::new();
    for grapheme in text.graphemes(true) {
        let grapheme_width = cells(grapheme);
        let next = column.saturating_add(grapheme_width);
        if grapheme_width == 0 {
            if column >= start && column < end {
                out.push_str(grapheme);
            }
        } else if next > start && column < end {
            if column >= start && next <= end {
                out.push_str(grapheme);
            } else {
                let overlap_start = column.max(start);
                let overlap_end = next.min(end);
                out.push_str(&" ".repeat(overlap_end.saturating_sub(overlap_start)));
            }
        }
        column = next;
        if column >= end {
            break;
        }
    }
    out
}

/// Wrap inert text at extended-grapheme boundaries without dropping ordinary
/// content. Explicit source lines remain lines; long values are split by
/// terminal cells and by the same grapheme/byte safety budgets as [`fit`]. A
/// single pathological grapheme that exceeds those budgets is visibly
/// truncated because it cannot be split safely.
pub(crate) fn wrap(text: &str, max_cells: usize) -> Vec<String> {
    if max_cells == 0 {
        return vec![String::new()];
    }
    let mut out = Vec::new();
    for source_line in text.split('\n') {
        let safe = safe(source_line);
        let (max_graphemes, max_bytes) = render_limits(max_cells);
        let mut line = String::new();
        let mut used_cells = 0usize;
        let mut used_graphemes = 0usize;
        let mut emitted = false;
        for grapheme in safe.graphemes(true) {
            let width = cells(grapheme);
            if width > max_cells || grapheme.len() > max_bytes {
                if !line.is_empty() {
                    out.push(std::mem::take(&mut line));
                    used_cells = 0;
                    used_graphemes = 0;
                }
                out.push(fit(grapheme, max_cells, true));
                emitted = true;
                continue;
            }
            if !line.is_empty()
                && (used_cells.saturating_add(width) > max_cells
                    || used_graphemes == max_graphemes
                    || line.len().saturating_add(grapheme.len()) > max_bytes)
            {
                out.push(std::mem::take(&mut line));
                used_cells = 0;
                used_graphemes = 0;
                emitted = true;
            }
            line.push_str(grapheme);
            used_cells += width;
            used_graphemes += 1;
        }
        if !line.is_empty() || !emitted {
            out.push(line);
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Append inert user input while preserving complete graphemes and a bounded
/// display width. Returns whether the visible value changed.
pub(crate) fn append_bounded(
    value: &mut String,
    input: &str,
    max_cells: usize,
    max_graphemes: usize,
    max_bytes: usize,
) -> bool {
    let input = safe(input);
    let mut used = cells(value);
    let existing_graphemes = value.graphemes(true).count();
    let before = value.len();
    for (offset, grapheme) in input.graphemes(true).enumerate() {
        let width = cells(grapheme);
        if existing_graphemes.saturating_add(offset) >= max_graphemes
            || value.len().saturating_add(grapheme.len()) > max_bytes
            || used.saturating_add(width) > max_cells
        {
            break;
        }
        value.push_str(grapheme);
        used += width;
    }
    value.len() != before
}

pub(crate) fn pop_grapheme(value: &mut String) -> bool {
    let Some((index, _)) = value.grapheme_indices(true).next_back() else {
        return false;
    };
    value.truncate(index);
    true
}

fn boundary_at_or_before(value: &str, mut cursor: usize) -> usize {
    cursor = cursor.min(value.len());
    while !value.is_char_boundary(cursor) {
        cursor -= 1;
    }
    cursor
}

/// Insert inert input at an editable UTF-8 cursor while preserving the same
/// cell, grapheme and byte bounds as append-only input.
pub(crate) fn insert_bounded(
    value: &mut String,
    cursor: &mut usize,
    input: &str,
    max_cells: usize,
    max_graphemes: usize,
    max_bytes: usize,
) -> bool {
    *cursor = boundary_at_or_before(value, *cursor);
    let input = safe(input);
    let mut changed = false;
    for grapheme in input.graphemes(true) {
        if value.graphemes(true).count() >= max_graphemes
            || value.len().saturating_add(grapheme.len()) > max_bytes
            || cells(value).saturating_add(cells(grapheme)) > max_cells
        {
            break;
        }
        value.insert_str(*cursor, grapheme);
        *cursor += grapheme.len();
        changed = true;
    }
    changed
}

pub(crate) fn delete_previous_grapheme(value: &mut String, cursor: &mut usize) -> bool {
    *cursor = boundary_at_or_before(value, *cursor);
    let Some((start, _)) = value[..*cursor].grapheme_indices(true).next_back() else {
        return false;
    };
    value.drain(start..*cursor);
    *cursor = start;
    true
}

pub(crate) fn delete_next_grapheme(value: &mut String, cursor: &mut usize) -> bool {
    *cursor = boundary_at_or_before(value, *cursor);
    let Some(grapheme) = value[*cursor..].graphemes(true).next() else {
        return false;
    };
    value.drain(*cursor..*cursor + grapheme.len());
    true
}

pub(crate) fn move_cursor(value: &str, cursor: &mut usize, forward: bool) -> bool {
    *cursor = boundary_at_or_before(value, *cursor);
    let next = if forward {
        value[*cursor..]
            .graphemes(true)
            .next()
            .map_or(*cursor, |grapheme| *cursor + grapheme.len())
    } else {
        value[..*cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(*cursor, |(start, _)| start)
    };
    let changed = next != *cursor;
    *cursor = next;
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_fitting_keeps_graphemes_and_controls_inert() {
        let combining = "e\u{301}";
        let wide = "\u{754c}";
        let hostile = format!("{combining}{wide}\u{1b}[2J-unbroken-value");

        assert_eq!(cells(combining), 1);
        assert_eq!(cells(wide), 2);
        assert_eq!(
            fit(&hostile, 5, true),
            format!("{combining}{wide}\u{fffd}>")
        );
        assert_eq!(cells(&fit(&hostile, 5, true)), 5);
        assert_eq!(
            fit(&hostile, 4, false),
            format!("{combining}{wide}\u{2026}")
        );
        assert!(!fit(&hostile, 4, false).contains('\u{1b}'));
        assert_eq!(fit("anything", 0, false), "");
    }

    #[test]
    fn bounded_editing_never_splits_a_display_cluster() {
        let mut value = String::new();
        assert!(append_bounded(&mut value, "a\u{301}\u{754c}z", 3, 8, 32));
        assert_eq!(value, "a\u{301}\u{754c}");
        assert_eq!(cells(&value), 3);
        assert!(pop_grapheme(&mut value));
        assert_eq!(value, "a\u{301}");
        assert!(pop_grapheme(&mut value));
        assert!(!pop_grapheme(&mut value));
    }

    #[test]
    fn cell_slices_preserve_coordinates_across_wide_and_combining_text() {
        let text = "a\u{301}\u{754c}bc";
        assert_eq!(slice_cells(text, 0, 3), "a\u{301}\u{754c}");
        assert_eq!(slice_cells(text, 1, 2), "\u{754c}");
        assert_eq!(slice_cells(text, 2, 2), " b");
        assert_eq!(cells(&slice_cells(text, 2, 2)), 2);
        assert_eq!(slice_cells(text, 4, 4), "c");
        assert_eq!(slice_cells(text, 0, 0), "");
    }

    #[test]
    fn cursor_editing_inserts_moves_and_deletes_by_grapheme() {
        let mut value = "a\u{301}\u{754c}".to_string();
        let mut cursor = value.len();
        assert!(move_cursor(&value, &mut cursor, false));
        assert_eq!(cursor, "a\u{301}".len());
        assert!(insert_bounded(&mut value, &mut cursor, "Z\n", 8, 8, 32));
        assert_eq!(value, "a\u{301}Z \u{754c}");
        assert!(delete_previous_grapheme(&mut value, &mut cursor));
        assert_eq!(value, "a\u{301}Z\u{754c}");
        assert!(delete_next_grapheme(&mut value, &mut cursor));
        assert_eq!(value, "a\u{301}Z");
        assert!(move_cursor(&value, &mut cursor, false));
        assert!(delete_previous_grapheme(&mut value, &mut cursor));
        assert_eq!(value, "Z");
        assert_eq!(cursor, 0);
    }

    #[test]
    fn fitting_and_editing_bound_zero_width_content_as_well_as_cells() {
        let invisible = "\u{200b}".repeat(4_096);
        let fitted = fit(&invisible, 12, true);
        assert!(
            fitted.len() <= 256,
            "fit retained an unbounded invisible tail"
        );
        assert!(fitted.ends_with('>'), "truncation must remain visible");

        let mut value = String::new();
        assert!(append_bounded(&mut value, &invisible, 64, 32, 128));
        assert!(value.graphemes(true).count() <= 32);
        assert!(value.len() <= 128);

        assert_eq!(
            fit_tail("beginning-and-the-visible-end", 12, true),
            ">visible-end"
        );
        assert!(cells(&fit_tail("beginning-and-the-visible-end", 12, true)) <= 12);
    }

    #[test]
    fn wrapping_preserves_lines_graphemes_and_inert_text() {
        let lines = wrap("ab\u{754c}c\nnext\u{1b}[2J", 4);
        assert_eq!(lines, ["ab\u{754c}", "c", "next", "\u{fffd}[2J"]);
        assert!(lines.iter().all(|line| cells(line) <= 4));
    }

    #[test]
    fn wrapping_bounds_zero_width_content_and_pathological_graphemes() {
        let invisible = "\u{200b}".repeat(4_096);
        let lines = wrap(&invisible, 12);
        assert!(lines.len() > 1, "zero-width content must still be chunked");
        assert_eq!(lines.concat(), invisible);
        assert!(lines.iter().all(|line| line.len() <= 256));

        let pathological = format!("a{}", "\u{301}".repeat(4_096));
        let lines = wrap(&pathological, 12);
        assert_eq!(lines, [">"]);
    }
}
