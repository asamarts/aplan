//! The palette: every style this tool emits, by the role it plays (`A1-287`).
//!
//! Before this there was no palette. Eleven call sites built their own styles
//! inline and seven of them were the same `Style::new().bold()`, so "what does
//! bold mean here" had eleven answers and changing one changed nothing else.
//! A role reads as intent at the call site -- `style::ERROR` rather than a
//! colour somebody picked -- and swapping the palette is a one-file edit.
//!
//! ## The rules this palette is under
//!
//! **Four-bit ANSI, never 256-colour and never truecolor.** [`anstyle::AnsiColor`]
//! resolves against the reader's own terminal theme, which is the difference
//! between a palette that works on a light background and one that was chosen on
//! a dark one. A hex value would look exactly as intended on the machine it was
//! picked on and nowhere else.
//!
//! **Colour is never the only carrier.** `check` prints `error[APLAN005]` in red
//! and the word `error` is what a reader with a red-green deficiency, a log file,
//! or a screenshot still has. The property is asserted rather than intended:
//! `--color=always` with every escape stripped is byte-identical to
//! `--color=never`, over every human rendering the tool has.
//!
//! **Few roles.** Six would be a palette and sixteen would be a theme, and a
//! theme is what makes output look busy rather than legible. There are eight
//! here, and the three status styles below are the ones that earned their place
//! by appearing on every list surface the tool prints.

use anstyle::{AnsiColor, Color, Style};

/// A finding that fails a build, and the tool's own refusals.
pub const ERROR: Style = Style::new()
    .fg_color(Some(Color::Ansi(AnsiColor::Red)))
    .bold();

/// A finding that does not fail a build by default.
pub const WARNING: Style = Style::new()
    .fg_color(Some(Color::Ansi(AnsiColor::Yellow)))
    .bold();

/// A verdict that passed: the `ok:` a reader is looking for.
pub const OK: Style = Style::new()
    .fg_color(Some(Color::Ansi(AnsiColor::Green)))
    .bold();

/// A task id or a rule code: the token a reader copies into the next command.
///
/// Cyan, and not bold: it appears several times per line on a list surface, and
/// a bold token repeated twelve times is a paragraph of emphasis.
pub const ID: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Cyan)));

/// A column header, a section label, or the heading of a cut: structure rather
/// than content.
///
/// **What counts as a heading was decided in one go** (`A1-296`), because it had
/// been decided a line at a time and the three kinds disagreed. A column header
/// was bold; a section label -- `show`'s `paths:`, `tree`'s `waits on:` -- was
/// nothing at all; and a cut's heading was the *version alone*, four bold
/// characters in a line of a hundred, with the title it names unstyled. All
/// three are this role now.
///
/// **The contrast comes from both directions.** A cut's line is a heading and a
/// rollup in one: `v0.5  Roadmap self-hosting` is the heading, `5/5 done, 0
/// ready, crit 9.0d ...` is metadata, and the metadata takes [`DIM`]. Making the
/// heading louder and the numbers quieter moves them further apart than either
/// alone.
///
/// **Still no hue, and that was the decision rather than the default.** Bold
/// plus a colour would be more prominent and would keep the accessibility
/// property, since the weight survives -- so
/// `the_structural_roles_do_not_rest_on_hue` could have been relaxed to "carries
/// weight as well as any hue". It was not, for two reasons. A roadmap already
/// carries three hues per row (an id, a status, a verdict), and a fourth on
/// every cut line is the "theme rather than palette" this module opens by
/// warning about. And weight is the signal that survives a log file, a
/// screenshot and a reader who cannot see the colour, which is what a *heading*
/// most needs to do.
///
/// **A rule line and a two-line heading were both declined for the same
/// measurable reason.** `alint` draws `--- label ------------` and it is the most
/// prominent option there is, but it is a *layout* change: six `trycmd` cases
/// and a 372-line generated `ROADMAP.status.md` move for a gain that only a
/// terminal ever sees. Styling changes nothing a pipe reads. A rule line would
/// also need a width, and `fold::out_width()` answers `usize::MAX` when there is
/// no terminal, which is right for "do not fold" and catastrophic for "repeat
/// this character that many times".
pub const HEADING: Style = Style::new().bold();

/// Ancillary text -- counts, parentheticals, the note under an answer.
pub const DIM: Style = Style::new().dimmed();

/// `status: done`.
///
/// Green like [`OK`] and deliberately not bold. The verdict line is one per run
/// and wants the weight; a status is one per row and a hundred bold greens is a
/// wall.
pub const DONE: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Green)));

/// `status: blocked`, and `in_progress`.
///
/// Yellow like [`WARNING`], because it is the same signal at a different
/// severity: something a reader may need to act on. Not bold, for the reason
/// [`DONE`] is not.
pub const ATTENTION: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Yellow)));

/// The style for a task's status, by the word the plan spells it with.
///
/// One function rather than a role per status: `todo` is the ordinary case and
/// styling it would mean styling every row, which is the same as styling none.
/// `dropped` is [`DIM`] because a dropped task is kept for the record and read
/// past.
#[must_use]
pub fn status(status: &str) -> Style {
    match status {
        "done" => DONE,
        "blocked" | "in_progress" => ATTENTION,
        "dropped" => DIM,
        _ => Style::new(),
    }
}

/// The style for a diagnostic severity, by the word it renders as.
#[must_use]
pub fn severity(severity: &str) -> Style {
    match severity {
        "error" => ERROR,
        "warning" => WARNING,
        _ => DIM,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every status the model has gets an answer, and `todo` gets the empty one.
    ///
    /// Written against the `Status` vocabulary rather than a list here: a status
    /// added to the model without a decision made here would otherwise fall
    /// through to "no style" silently, which is a decision made by omission.
    #[test]
    fn every_status_in_the_model_is_answered_here() {
        // The five the model declares, in `aplan-core`'s `Status`. Named rather
        // than imported: `aplan-output` is a leaf and does not depend on the
        // core, and `the_status_vocabulary_is_the_models` in `aplan` is what
        // holds this list to that one.
        for (word, want) in [
            ("todo", Style::new()),
            ("in_progress", ATTENTION),
            ("blocked", ATTENTION),
            ("done", DONE),
            ("dropped", DIM),
        ] {
            assert_eq!(status(word), want, "`{word}` is styled unexpectedly");
        }
    }

    /// Colour is emphasis: no role may be *only* a colour change that a reader
    /// who cannot see it would need. Structural roles carry weight instead.
    #[test]
    fn the_structural_roles_do_not_rest_on_hue() {
        for (name, style) in [("HEADING", HEADING), ("DIM", DIM)] {
            assert!(
                style.get_fg_color().is_none(),
                "`{name}` carries a hue, so a reader without colour loses it"
            );
        }
    }

    /// The severity roles are distinguishable without colour too: each is bold,
    /// and each renders beside its own word.
    #[test]
    fn every_severity_role_is_bold_as_well_as_coloured() {
        for (name, style) in [("ERROR", ERROR), ("WARNING", WARNING), ("OK", OK)] {
            assert!(
                style.get_fg_color().is_some()
                    && style.get_effects().contains(anstyle::Effects::BOLD),
                "`{name}` is not carrying both signals"
            );
        }
    }

    /// Four-bit ANSI only, so the palette resolves against the reader's theme.
    #[test]
    fn every_colour_is_a_four_bit_ansi_colour() {
        for (name, style) in [
            ("ERROR", ERROR),
            ("WARNING", WARNING),
            ("OK", OK),
            ("ID", ID),
            ("DONE", DONE),
            ("ATTENTION", ATTENTION),
        ] {
            match style.get_fg_color() {
                Some(anstyle::Color::Ansi(_)) => {}
                other => panic!("`{name}` is not a four-bit ANSI colour: {other:?}"),
            }
        }
    }

    #[test]
    fn a_severity_the_renderer_does_not_know_is_dimmed_not_loud() {
        assert_eq!(severity("note"), DIM);
        assert_eq!(severity("error"), ERROR);
        assert_eq!(severity("warning"), WARNING);
    }
}
