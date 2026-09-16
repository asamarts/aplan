//! Folding a line so it continues in the column it began in (`A1-206`).
//!
//! `A1-205` gave `clap` a width and the help stopped folding to column zero.
//! This is the same defect on the surface that matters more: help is read while
//! learning the tool, a finding is read every time CI fails. Measured against
//! the generated catalogue, **42 of 61 diagnostic messages and 44 of 61 `fix:`
//! lines exceed eighty columns**, the longest fix line running to 404. A finding
//! is an indented block, and that indent is exactly what a terminal's own fold
//! discards.
//!
//! **Why this is not `chars().count()`.** The wrapper that already existed here
//! counted characters, which is wrong twice over: an ANSI escape is bytes the
//! terminal never shows, so a styled line was measured as far wider than it
//! looks, and a wide character occupies two columns, so a CJK title was measured
//! as far narrower. Both mis-measure exactly the lines that are hardest to read.
//! `display_width` skips the escapes and asks `unicode-width` about the rest.
//!
//! **What it deliberately does not do.** A word longer than the width is emitted
//! whole and overflows, rather than being broken mid-token. A URL or a path is
//! usually the thing a reader most wants to copy, and a fold through the middle
//! of one is worse than a long line.

use unicode_width::UnicodeWidthChar;

/// One row of a table: the fixed columns, then the column that folds under
/// itself (`A1-291`).
///
/// **The width is not a parameter, and that is the whole point.** `A1-206` gave
/// the tool two ways to ask -- [`out_width`] for stdout, [`err_width`] for
/// stderr -- and four commands built text for stdout and folded it against
/// stderr. It is invisible while both streams are one terminal and wide open
/// the moment they are not: with stderr redirected, `err_width` answers "do not
/// fold" and the line runs off the screen. A row goes to stdout, so this asks
/// stdout, and no call site can get it wrong by passing the other one.
///
/// **`prefix` is the columns that do not fold**, already padded and styled;
/// `trailing` is the one that does. The continuation lands under `trailing`'s
/// own start rather than at column zero, which is what makes a wrapped cell read
/// as part of its row instead of as a new one.
///
/// **A word longer than the column is still emitted whole**, and that is
/// deliberate rather than a remaining bug. `A1-206` chose it: a path or a URL is
/// the thing a reader most wants to copy, and a break through the middle of one
/// is worse than a long line. The consequence is visible on a narrow terminal,
/// where it fires for ordinary words too -- at sixty columns `aplan tasks`
/// leaves nine columns for the title and `deliberately` is twelve, so that row
/// runs three past the edge and the terminal breaks it. A reader meeting a
/// ragged line has met either this or a fold that did not happen, and the
/// difference is that this one is a single word with no break to take.
/// `nothing_a_person_reads_is_wider_than_their_terminal` encodes exactly that
/// distinction, so the two cannot be confused for each other again.
#[must_use]
pub fn row(prefix: &str, trailing: &str) -> String {
    fold_after(prefix, trailing, out_width())
}

/// A sentence printed under a table, folded to stdout's width.
///
/// The same reason as [`row`]: a note is prose and folds at column zero, and the
/// only thing a call site could get wrong is which stream's width it used.
#[must_use]
pub fn note(text: &str) -> String {
    fold(text, out_width(), 0)
}

/// The width to fold to for standard output: [`MAX_WIDTH`] when a person is
/// reading, and no limit when it is redirected.
///
/// **This is the same decision colour already makes here.** `anstream` styles
/// for a terminal and emits plain bytes into a pipe, with no flag to set, and
/// `A1-206` filed itself saying width is that decision with no implementation.
/// So: the defect is a *terminal* folding a long line at its screen edge. A pipe
/// has no screen edge, and folding into one only breaks `grep` and pins where a
/// fold happens to land into anything that reads the output.
///
/// Applying it to a pipe was tried first, and the test suite said no: twenty-odd
/// assertions on ordinary phrases broke because a fold had landed inside them.
/// Every one of those is a reader who would have been broken the same way.
///
/// [`MAX_WIDTH`]: crate::MAX_WIDTH
#[must_use]
pub fn out_width() -> usize {
    capped(terminal_size::terminal_size_of(std::io::stdout()).map(|(w, _)| w.0 as usize))
}

/// The same, for standard error, where refusals and notes are written.
#[must_use]
pub fn err_width() -> usize {
    capped(terminal_size::terminal_size_of(std::io::stderr()).map(|(w, _)| w.0 as usize))
}

/// The terminal's own width, capped at [`MAX_WIDTH`], or unbounded when there is
/// no terminal.
///
/// Capped rather than fixed, which is what `A1-205` gave `clap` and what this
/// has to match or the two surfaces disagree about how wide the screen is: a
/// narrow terminal gets its own width, a very wide one stops at the cap because
/// a line of prose two hundred columns long is harder to scan, not easier.
///
/// [`MAX_WIDTH`]: crate::MAX_WIDTH
fn capped(columns: Option<usize>) -> usize {
    match columns {
        Some(cols) => cols.min(crate::MAX_WIDTH),
        // `fold` returns its input unchanged when it already fits, so an
        // unbounded width is exactly "do not fold" without a second code path.
        None => usize::MAX,
    }
}

/// The columns `text` occupies on a terminal.
///
/// ANSI escapes are skipped (they are instructions, not glyphs) and each
/// remaining character is asked how wide it is, so a two-column glyph counts as
/// two. A control character counts as nothing.
#[must_use]
pub fn display_width(text: &str) -> usize {
    let mut width = 0;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // A CSI sequence is `ESC [` then parameters then a final byte in
            // `@`..`~`. The `[` must be consumed before looking for that final
            // byte, because `[` is itself inside the range -- the first version
            // stopped there and counted the parameters as glyphs, measuring
            // `ESC [ 1 m bold ESC [ 0 m` as eight columns rather than four.
            // Anything else after ESC is a short sequence: dropping one
            // character resynchronises, and this is measurement, not parsing.
            if chars.next() == Some('[') {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            continue;
        }
        width += c.width().unwrap_or(0);
    }
    width
}

/// Whitespace that is not a plain space, flattened to one.
///
/// This is the same defect the module exists to fix, arriving through the
/// content instead of through the width. A title carrying a newline -- YAML
/// gives one away for the price of a `|-` block -- puts its continuation at
/// column zero no matter how wide the terminal is, and the row stops reading as
/// a row. A tab is that plus a measurement error: where the next tab stop falls
/// is the terminal's business, so [`display_width`] can only count it as
/// nothing and every line carrying one is measured narrower than it renders.
///
/// Applied *before* the already-fits fast path, deliberately. Folding collapses
/// whitespace on its own, so leaving this to the fold would repair a long title
/// and leave a short one broken -- the same value rendering two ways depending
/// on its length. `render_tasks` has flattened its titles all along; this is
/// what stops the answer from depending on which renderer you asked.
fn flatten(text: &str) -> std::borrow::Cow<'_, str> {
    if text.chars().any(|c| c.is_whitespace() && c != ' ') {
        std::borrow::Cow::Owned(
            text.chars()
                .map(|c| if c.is_whitespace() { ' ' } else { c })
                .collect(),
        )
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

/// Fold `text` to `width` columns, continuing each line at `indent`.
///
/// The first line is emitted as given; every line after it is prefixed with
/// `indent` spaces, so a folded sentence continues in the column it began in
/// rather than under whatever started the line.
///
/// Returns `text` unchanged when it already fits and carries no whitespace
/// other than spaces, so a caller can use this unconditionally without a short
/// line acquiring a trailing difference. A newline or a tab is always flattened
/// to a space, at any width.
#[must_use]
pub fn fold(text: &str, width: usize, indent: usize) -> String {
    carry_styles(&fold_unstyled(text, width, indent))
}

/// Close an open style at every line break and re-open it after the indent.
///
/// **A fold through the middle of a styled run leaves the style open.** Measured
/// on this function's own input: a bold run folded at twenty columns produced
/// `\e[1malpha beta gamma` / `delta epsilon zeta` / `eta theta\e[0m tail` -- the
/// first line opening a style it never closes, the middle inheriting it, and the
/// last closing one it never opened. A terminal carries SGR across a newline, so
/// this *looked* right for the one styled thing that existed; it stops looking
/// right the moment a background colour, a pager that re-renders a line, or a
/// reader who copies one line out of the middle is involved. And the indent of a
/// continuation was inside the style, so an indented continuation of a coloured
/// row was a coloured indent (`A1-287`).
///
/// The rule is the one a terminal cannot infer: a line is responsible for its own
/// styling. What is open at a break is closed there and re-opened on the next
/// line, *after* its indent.
///
/// A no-op on text carrying no escape, which is almost all of it, so nothing
/// piped moves.
fn carry_styles(text: &str) -> String {
    if !text.contains('\u{1b}') {
        return text.to_string();
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = String::with_capacity(text.len() + 16);
    // The sequences opened and not yet reset, in the order they were written:
    // re-opening means replaying them, since `\e[1m` and `\e[31m` are two
    // sequences and either can be the one still standing.
    let mut active = String::new();
    for (n, line) in lines.iter().enumerate() {
        if n > 0 {
            out.push('\n');
        }
        if active.is_empty() {
            out.push_str(line);
        } else {
            // After the indent, so a continuation's leading space is not painted.
            let body = line.trim_start_matches(' ');
            let lead = &line[..line.len() - body.len()];
            out.push_str(lead);
            out.push_str(&active);
            out.push_str(body);
        }
        for seq in sgr_sequences(line) {
            match sgr_params(&seq) {
                // Not a style at all. Never carried, and never treated as
                // something to reset: a title carrying `\e[2J` was being
                // replayed on every continuation, so one erase-display in the
                // data became one *per folded line* (`A1-287`, audit). It is
                // passed through exactly as it arrived, which is what the
                // unfolded output does with it.
                None => {}
                Some(params) => {
                    // A leading zero parameter cancels everything before it, so
                    // what is still open afterwards is this sequence alone.
                    // `\e[0m` is the whole of it and opens nothing; `\e[0;31m`
                    // resets and then sets red.
                    if resets(params) {
                        active.clear();
                        if !is_only_reset(params) {
                            active.push_str(&seq);
                        }
                    } else {
                        active.push_str(&seq);
                    }
                }
            }
        }
        // Only at a break: text that ends inside a style ends the way its caller
        // wrote it, and closing that would change bytes nobody asked about.
        if !active.is_empty() && n + 1 < lines.len() {
            out.push_str("\u{1b}[0m");
        }
    }
    out
}

/// The parameters of `seq` if it is an SGR sequence, or `None` if it is any other
/// CSI.
///
/// The distinction is the whole of the fix above: SGR (`m`) is styling and is
/// safe to close and re-open around a line break, and everything else in the CSI
/// range -- cursor movement, erase, scroll -- is an *action*, and repeating an
/// action once per folded line is not what the unfolded text did.
fn sgr_params(seq: &str) -> Option<&str> {
    seq.strip_prefix("\u{1b}[")?.strip_suffix('m')
}

/// Whether an SGR's first parameter is a zero, which cancels what came before.
fn resets(params: &str) -> bool {
    let first = params.split(';').next().unwrap_or("");
    first.is_empty() || first.chars().all(|c| c == '0')
}

/// Whether an SGR is *only* a reset, so nothing is left open by it.
fn is_only_reset(params: &str) -> bool {
    params
        .split(';')
        .all(|p| p.is_empty() || p.chars().all(|c| c == '0'))
}

/// Every `ESC [ ... <final byte>` sequence in `line`, in order.
///
/// The final byte is searched for *after* the `[`: `[` is itself in the `@`..`~`
/// range a sequence terminates on, so scanning from the escape returns `ESC [`
/// and leaves the rest of the sequence in the text as though it were content.
fn sgr_sequences(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(i) = rest.find('\u{1b}') {
        rest = &rest[i..];
        let Some(body) = rest.strip_prefix("\u{1b}[") else {
            rest = &rest['\u{1b}'.len_utf8()..];
            continue;
        };
        let Some((end, _)) = body.char_indices().find(|(_, c)| ('@'..='~').contains(c)) else {
            break;
        };
        let len = "\u{1b}[".len() + end + 1;
        out.push(rest[..len].to_string());
        rest = &rest[len..];
    }
    out
}

fn fold_unstyled(text: &str, width: usize, indent: usize) -> String {
    let text = flatten(text);
    let text: &str = &text;
    if display_width(text) <= width {
        return text.to_string();
    }
    let pad = " ".repeat(indent);
    let mut out = String::with_capacity(text.len() + text.len() / width.max(1));
    let mut column = 0;
    let mut first_on_line = true;

    for word in text.split_whitespace() {
        let w = display_width(word);
        if first_on_line {
            out.push_str(word);
            column += w;
            first_on_line = false;
        } else if column + 1 + w > width {
            out.push('\n');
            out.push_str(&pad);
            out.push_str(word);
            column = indent + w;
        } else {
            out.push(' ');
            out.push_str(word);
            column += 1 + w;
        }
    }
    out
}

/// Fold `trailing` after a `prefix` that must survive verbatim, continuing in
/// the column the trailing text began in.
///
/// A table row is the case: `  A1-143      done          <title>` aligns by
/// runs of spaces, and folding the whole line would collapse them, so the
/// prefix is emitted untouched and only what follows is folded. Continuations
/// line up under the trailing text rather than under the id, which is what makes
/// a row still read as a row after it folds.
#[must_use]
pub fn fold_after(prefix: &str, trailing: &str, width: usize) -> String {
    let trailing = flatten(trailing);
    let trailing: &str = &trailing;
    let column = display_width(prefix);
    let room = width.saturating_sub(column);
    if room == 0 || display_width(trailing) <= room {
        return format!("{prefix}{trailing}");
    }
    let folded = fold_unstyled(trailing, room, 0);
    let pad = " ".repeat(column);
    let mut out = String::with_capacity(prefix.len() + folded.len() + pad.len());
    out.push_str(prefix);
    for (n, line) in folded.lines().enumerate() {
        if n > 0 {
            out.push('\n');
            out.push_str(&pad);
        }
        out.push_str(line);
    }
    // After the pad is added, not before: the prefix may open a style of its own,
    // and the continuation's indent belongs outside whatever is carried.
    carry_styles(&out)
}

/// Fold a block that already carries line breaks, keeping them.
///
/// [`fold`] folds one logical line and flattens every whitespace character in
/// it, which is what a *value* needs: a title with a newline in it is a broken
/// row, not a paragraph. A composed block is the opposite case -- the breaks are
/// the caller's structure, and a refusal that lists findings under a headline
/// stops meaning anything if the list is run together with the headline.
///
/// That is not hypothetical. `A1-206` folded the whole of `aplan: {e:#}` at
/// once, so on a terminal `dep add` printed
/// `... cycle: A-01 -> M-s1 -> ... -> A-01 fix: Break the cycle ...` as one
/// paragraph, reading as though `fix:` were part of the cycle. In a pipe the
/// width is unbounded and nothing folded, so the committed corpus went on
/// showing the correct shape.
///
/// Each line after the first keeps its own leading indent and continues under
/// it, which is what makes a nested list still read as nested once it folds.
#[must_use]
pub fn fold_block(text: &str, width: usize, indent: usize) -> String {
    text.split('\n')
        .enumerate()
        .map(|(n, line)| {
            if n == 0 {
                return fold(line, width, indent);
            }
            let body = line.trim_start_matches(' ');
            let lead = &line[..line.len() - body.len()];
            fold_after(lead, body, width)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_that_fits_is_returned_unchanged() {
        assert_eq!(fold("short enough", 40, 2), "short enough");
    }

    #[test]
    fn a_continuation_starts_in_the_column_it_was_given() {
        let folded = fold("aaa bbb ccc ddd", 7, 4);
        assert_eq!(folded, "aaa bbb\n    ccc\n    ddd");
    }

    #[test]
    fn no_line_exceeds_the_width() {
        let text = "the quick brown fox jumps over the lazy dog and keeps on going for a while";
        for width in [12, 20, 40] {
            for line in fold(text, width, 3).lines() {
                assert!(
                    display_width(line) <= width,
                    "{width}: {} columns: {line}",
                    display_width(line)
                );
            }
        }
    }

    /// **A style that spans a fold is closed at the break and re-opened after
    /// it**, which is the defect `A1-287` measured on this function (`A1-286`
    /// shipped the first style long enough to meet it).
    #[test]
    fn a_style_spanning_a_break_is_closed_and_reopened() {
        let styled = "\u{1b}[1malpha beta gamma delta epsilon zeta eta theta\u{1b}[0m tail";
        let out = fold(styled, 20, 0);
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines.len() > 2, "did not fold: {out:?}");

        for (n, line) in lines.iter().enumerate() {
            let opens = sgr_sequences(line)
                .iter()
                .filter(|s| *s != "\u{1b}[0m")
                .count();
            let closes = line.matches("\u{1b}[0m").count();
            assert_eq!(
                opens, closes,
                "line {n} is not balanced, so a reader of one line gets a style \
                 nothing closes: {line:?}"
            );
        }
        // And the text is unchanged once the escapes are removed, or the fix
        // would have moved a word rather than a style.
        let bare: String = fold("alpha beta gamma delta epsilon zeta eta theta tail", 20, 0);
        let stripped: String = {
            let mut t = out.clone();
            for seq in sgr_sequences(&out) {
                t = t.replace(&seq, "");
            }
            t
        };
        assert_eq!(stripped, bare, "styling changed where the text folded");
    }

    /// **A CSI that is not styling is never replayed across a break.**
    ///
    /// Found by auditing `A1-287`: a task title carrying `\e[2J` (erase display)
    /// was treated as an open style, so folding it over four lines emitted the
    /// erase four times. One escape in the data became one action per folded
    /// line, which the unfolded text never did.
    #[test]
    fn a_cursor_sequence_is_passed_through_and_not_carried() {
        let hostile = "\u{1b}[2Jclear and home \u{1b}[H then a tail long enough to fold twice over";
        let out = fold(hostile, 20, 4);
        assert!(out.lines().count() > 2, "did not fold: {out:?}");
        assert_eq!(
            out.matches("\u{1b}[2J").count(),
            1,
            "an erase-display was repeated once per folded line: {out:?}"
        );
        assert_eq!(
            out.matches("\u{1b}[H").count(),
            1,
            "a cursor-home was repeated once per folded line: {out:?}"
        );
        // And nothing was closed on its behalf, since nothing was open.
        assert_eq!(
            out.matches("\u{1b}[0m").count(),
            0,
            "a reset was emitted for a sequence that opened no style: {out:?}"
        );
    }

    /// A reset combined with a colour resets *and* opens: what carries after it
    /// is that sequence alone, not everything that came before.
    #[test]
    fn a_combined_reset_replaces_what_was_open_rather_than_adding_to_it() {
        let styled =
            "\u{1b}[1mbold then \u{1b}[0;31mred for a tail long enough to fold at least twice";
        let out = fold(styled, 22, 0);
        let last = out.lines().last().expect("a line");
        assert!(
            last.contains("\u{1b}[0;31m") && !last.contains("\u{1b}[1m"),
            "the continuation carried a style the combined reset had cancelled: {last:?}"
        );
    }

    /// A pure reset spelled any of the ways a terminal accepts it closes the
    /// style, so nothing is carried past it.
    #[test]
    fn every_spelling_of_a_reset_closes_what_is_open() {
        for reset in ["\u{1b}[0m", "\u{1b}[m", "\u{1b}[00m", "\u{1b}[0;0m"] {
            let text = format!(
                "{}{}bold{reset} and a tail long enough to fold twice over",
                "\u{1b}[1m", ""
            );
            let out = fold(&text, 20, 0);
            let last = out.lines().last().expect("a line");
            assert!(
                !last.contains("\u{1b}[1m"),
                "`{reset}` did not close the style, so it was carried on: {last:?}"
            );
        }
    }

    /// The indent of a continuation is outside the style it carries.
    ///
    /// A coloured row folded under an indent used to paint the indent too, which
    /// is the difference between a wrapped cell and a coloured margin.
    #[test]
    fn a_continuation_indents_before_it_reopens() {
        let styled = "\u{1b}[1malpha beta gamma delta epsilon zeta\u{1b}[0m";
        let out = fold(styled, 20, 4);
        let second = out.lines().nth(1).expect("a second line");
        assert!(
            second.starts_with("    \u{1b}["),
            "the continuation opened its style before its indent: {second:?}"
        );
    }

    /// Text carrying no escape is returned unchanged, which is what keeps the
    /// committed corpus byte-identical.
    #[test]
    fn unstyled_text_is_untouched_by_the_style_carrier() {
        for text in [
            "one two three",
            "a\nb\nc",
            "",
            "a very long sentence that will certainly need to be folded somewhere",
        ] {
            assert_eq!(carry_styles(text), text);
        }
    }

    /// Carrying is idempotent: a second pass over balanced lines adds nothing.
    ///
    /// `fold_block` composes `fold` and `fold_after`, both of which carry, so a
    /// line can meet this twice.
    #[test]
    fn carrying_styles_twice_is_carrying_them_once() {
        let styled = "\u{1b}[1malpha beta gamma delta epsilon zeta eta theta\u{1b}[0m tail";
        let once = fold(styled, 20, 2);
        assert_eq!(carry_styles(&once), once);
    }

    /// Two open sequences are both replayed: `\e[1m` and `\e[31m` are separate,
    /// and re-opening only the last would drop the bold.
    #[test]
    fn every_open_sequence_is_replayed_not_just_the_last() {
        let styled = "\u{1b}[1m\u{1b}[31malpha beta gamma delta epsilon\u{1b}[0m";
        let out = fold(styled, 18, 0);
        let second = out.lines().nth(1).expect("a second line");
        assert!(
            second.contains("\u{1b}[1m") && second.contains("\u{1b}[31m"),
            "the continuation lost half the style: {second:?}"
        );
    }

    /// The reason this is not `chars().count()`, half one.
    #[test]
    fn ansi_escapes_take_no_columns() {
        let styled = "\u{1b}[1mbold\u{1b}[0m";
        assert_eq!(display_width(styled), 4, "escapes were counted as glyphs");
        assert_eq!(styled.chars().count(), 12, "the naive count, for contrast");
    }

    /// A styled line must fold on what it looks like, not on its byte count, or
    /// the lines carrying emphasis fold earliest.
    #[test]
    fn a_styled_line_folds_where_it_looks_like_it_should() {
        let plain = "one two three four five";
        let styled = "\u{1b}[1mone\u{1b}[0m two three four five";
        assert_eq!(
            fold(plain, 13, 2).lines().count(),
            fold(styled, 13, 2).lines().count(),
            "styling changed where the text folded"
        );
    }

    /// The reason this is not `chars().count()`, half two.
    #[test]
    fn a_wide_character_takes_two_columns() {
        assert_eq!(display_width("日本語"), 6);
        assert_eq!("日本語".chars().count(), 3, "the naive count, for contrast");
    }

    #[test]
    fn a_non_ascii_title_folds_on_what_it_occupies() {
        // Three wide words: six columns each, so two fit in fourteen.
        let text = "日本語 日本語 日本語";
        let folded = fold(text, 14, 2);
        assert_eq!(folded.lines().count(), 2, "{folded}");
        for line in folded.lines() {
            assert!(display_width(line) <= 14, "{line}");
        }
    }

    /// A path or a URL is the thing a reader most wants to copy, so it is left
    /// whole and allowed to overflow rather than folded through the middle.
    #[test]
    fn a_word_longer_than_the_width_is_not_broken() {
        let long = "https://example.com/a/very/long/path/that/exceeds/the/width";
        let folded = fold(&format!("see {long} for more"), 20, 2);
        assert!(
            folded.contains(long),
            "the unbreakable word was broken: {folded}"
        );
    }

    #[test]
    fn an_indent_of_zero_continues_at_the_margin() {
        assert_eq!(fold("aaa bbb ccc", 7, 0), "aaa bbb\nccc");
    }

    /// Whitespace is normalised to single spaces, which is what makes a folded
    /// paragraph read evenly; a caller wanting literal spacing should not fold.
    #[test]
    fn runs_of_whitespace_collapse() {
        assert_eq!(fold("aaa   bbb ccc", 7, 0), "aaa bbb\nccc");
    }

    /// The defect arriving through the content rather than the width: a title
    /// with a newline in it puts its continuation at column zero on any
    /// terminal, however wide.
    #[test]
    fn a_newline_inside_the_text_never_survives() {
        assert_eq!(
            fold("first line\nsecond line", 80, 2),
            "first line second line"
        );
        assert_eq!(
            fold_after("  T-001  ", "first line\nsecond line", 80),
            "  T-001  first line second line"
        );
    }

    /// A tab is unmeasurable -- where the next stop falls is the terminal's
    /// business -- so it is flattened rather than counted as nothing.
    #[test]
    fn a_tab_never_survives_and_is_not_measured_as_nothing() {
        assert_eq!(fold("alpha\tbeta", 80, 0), "alpha beta");
        assert_eq!(fold_after("  x  ", "a\tb", 80), "  x  a b");
        // Why it has to be removed rather than measured: `display_width` is a
        // count of columns, and how far a tab advances depends on where the
        // terminal's stops are, so it can only answer zero. A line keeping one
        // would be measured narrower than it renders.
        assert_eq!(display_width("alpha\tbeta"), 9, "the unmeasurable case");
        assert_eq!(display_width(&fold("alpha\tbeta", 80, 0)), 10);
    }

    /// The reason flattening happens before the fast path. Without it the same
    /// title renders repaired when it is long and broken when it is short.
    #[test]
    fn flattening_does_not_depend_on_whether_the_line_folds() {
        let short = "one\ntwo";
        let long = "one\ntwo ".repeat(30);
        for text in [short, long.as_str()] {
            for line in fold(text, 40, 2).lines() {
                assert!(!line.is_empty(), "a flattened line went missing: {text:?}");
            }
            assert!(
                !fold(text, 40, 2).lines().any(str::is_empty),
                "a raw newline survived: {text:?}"
            );
        }
        assert_eq!(
            fold(short, 40, 2),
            "one two",
            "the short line was left broken"
        );
    }

    /// Carriage returns, vertical tabs and form feeds all move the cursor, and
    /// all measure as nothing. None of them belongs in a folded line.
    #[test]
    fn every_control_whitespace_is_flattened() {
        for c in ['\n', '\r', '\t', '\u{b}', '\u{c}', '\u{85}'] {
            let text = format!("a{c}b");
            let out = fold(&text, 80, 0);
            assert_eq!(out, "a b", "{c:?} survived folding as {out:?}");
        }
    }

    /// Spaces are left alone when the line fits, or a table's alignment would
    /// move under a caller that only asked for a fold.
    #[test]
    fn a_run_of_spaces_survives_a_line_that_fits() {
        assert_eq!(fold("a    b", 80, 0), "a    b");
    }

    #[test]
    fn a_row_keeps_its_prefix_and_folds_only_the_trailing_column() {
        let out = fold_after(
            "  A1-143      done      ",
            "one two three four five six",
            40,
        );
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].starts_with("  A1-143      done      "), "{out}");
        for line in &lines[1..] {
            assert!(
                line.starts_with(&" ".repeat(24)),
                "a continuation did not line up under the title: {line:?}"
            );
        }
        for line in &lines {
            assert!(display_width(line) <= 40, "{line}");
        }
    }

    #[test]
    fn a_row_that_fits_is_one_line_and_unchanged() {
        assert_eq!(fold_after("  id  ", "short", 40), "  id  short");
    }

    /// A prefix wider than the width leaves no room, and emitting the row long
    /// beats emitting it one character per line.
    #[test]
    fn a_prefix_wider_than_the_width_does_not_fold_forever() {
        let wide = " ".repeat(50);
        let out = fold_after(&wide, "some title here", 20);
        assert_eq!(out.lines().count(), 1, "{out:?}");
    }

    #[test]
    fn an_unbounded_width_leaves_text_alone() {
        let long = "a ".repeat(200) + "end";
        assert_eq!(fold(&long, usize::MAX, 2), long, "text was altered");
        assert_eq!(
            fold_after("  x  ", &long, usize::MAX),
            format!("  x  {long}")
        );
    }

    /// The regression this function exists for: a refusal whose findings are
    /// listed under a headline must not come out as one paragraph.
    #[test]
    fn a_block_keeps_the_breaks_its_caller_put_in() {
        let block = concat!(
            "aplan: refusing: it would add 1 finding(s):\n",
            "  APLAN004 A-01: dependency cycle: A-01 -> M-s1 -> A-01\n",
            "      fix: Break the cycle by removing one edge; a plan with a cycle has no order.",
        );
        let out = fold_block(block, 80, 7);

        assert!(
            out.lines().count() > 3,
            "the block was run together:\n{out}"
        );
        assert!(
            out.lines().any(|l| l.starts_with("  APLAN004")),
            "the finding lost its own indent:\n{out}"
        );
        assert!(
            out.lines().any(|l| l.starts_with("      fix:")),
            "the remedy lost its own indent:\n{out}"
        );
        for line in out.lines() {
            assert!(
                display_width(line) <= 80,
                "{} columns: {line}",
                display_width(line)
            );
        }
        // The defect, stated: `fold` alone merges the remedy into the cycle.
        assert!(
            fold(block, 80, 7).contains("-> A-01 fix:"),
            "this test no longer distinguishes fold_block from fold"
        );
    }

    /// An unbounded width must leave a block exactly as it was, or the piped
    /// corpus moves the moment an error grows a second line.
    #[test]
    fn an_unbounded_block_is_returned_byte_for_byte() {
        let block = "headline:\n  first item\n      a nested remedy line";
        assert_eq!(fold_block(block, usize::MAX, 7), block);
    }

    #[test]
    fn a_redirected_stream_is_unbounded_and_a_terminal_is_its_own_width() {
        assert_eq!(capped(None), usize::MAX, "a pipe must not fold");
        assert_eq!(capped(Some(60)), 60, "a narrow terminal gets its own width");
        assert_eq!(
            capped(Some(400)),
            crate::MAX_WIDTH,
            "a very wide terminal stops at the cap"
        );
    }
}
