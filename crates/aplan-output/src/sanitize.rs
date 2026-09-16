//! Text a plan supplied, made safe to print (`A1-289`).
//!
//! A plan is data and this tool prints it to a terminal. YAML refuses a raw
//! control character in a scalar and then hands one over for the price of a
//! double-quoted escape, so a title can carry an escape sequence, `aplan check`
//! is clean -- nothing about the plan is wrong -- and the sequence reaches
//! whoever reads the plan.
//!
//! Escapes are not only paint. `CSI 2 J` erases the display, `OSC 0` sets the
//! window title, `CSI ? 1049 h` switches to the alternate screen. Reviewing a
//! plan before trusting it is a normal thing to do -- `--config` naming a policy
//! outside the repository exists for that case (`A1-155`) -- and `aplan show` on
//! somebody else's task is how it is done.
//!
//! ## Why the reader lost either way before this
//!
//! With colour on, the sequence was executed. With colour off, `anstream`
//! stripped it, which deleted it *and its parameters* from the text: a title
//! reading `carrying <ESC>[41m a marker` printed as `carrying  a marker`.
//! Neither rendering showed what the title actually said. Replacing the control
//! character with [`MARKER`] and leaving the rest as ordinary text does: the
//! sequence becomes a replacement character followed by `[41m`, which is inert,
//! one column wide, and legible as exactly what it is.
//!
//! ## Why this is not a rule
//!
//! `A1-290` asked whether a control character in a plan field should also be a
//! *finding*, since replacing it silently is the half-measure this repository
//! dislikes elsewhere. It is not, and the reason is a measurement rather than a
//! preference. **Not one field carries one**, in any plan this tool has been
//! pointed at. The sample behind that, measured on 2026-09-08 by loading each
//! plan and reading the values the parser yields rather than grepping the files:
//! fourteen plans, 2,336 tasks. The sample is a *floor* and will drift; the
//! zero is the finding, and a new plan does not move it unless somebody puts a
//! control character in a field. A rule code is a
//! permanent consumer contract, frozen at v1.0, and `aplan vacuity` exists
//! precisely to report a rule that had nothing to check; a new code here would
//! be reported by the tool's own vacuity command on every plan that exists.
//!
//! `check`'s silence is also correct on its own terms. Nothing about such a plan
//! is *invalid*: the character is legal YAML in a double-quoted scalar and legal
//! in a title, and a rule that fires on the unusual rather than the wrong
//! teaches an adopter to skim findings.
//!
//! What the question was right about is that somebody should be told. The one
//! path by which this reaches a plan without being typed on purpose is `aplan
//! import` from a document that picked up a terminal capture, and that path now
//! carries a `provenance` notice naming the count -- which reaches the person
//! who ran the import, at the moment they can still fix it, and costs no code.
//!
//! ## Why not at the loader, and why not at the writer
//!
//! Not the loader: the machine formats must keep the bytes the plan holds --
//! `json` already escapes a control character, and a consumer parsing it must
//! not have its input rewritten. Not the writer either: by the time a line
//! reaches the stream it carries the palette's own escapes, and nothing there
//! can tell those from the plan's. So it happens where a *plan-supplied* string
//! enters a human rendering, and
//! `no_human_surface_lets_a_plan_write_to_the_terminal` is what proves none was
//! missed.

use std::borrow::Cow;

/// What a control character renders as: it was here, it is not being run.
pub const MARKER: char = '\u{fffd}';

/// `text` with every control character replaced by [`MARKER`].
///
/// Whitespace is left alone. Newline, carriage return and tab are control
/// characters too, and they already have an owner: `fold::flatten` and
/// `one_line` collapse them to spaces, which is a decision about layout that
/// predates this one and that a second rewrite here would fight with.
///
/// Borrowed when there is nothing to do, which is almost always: the check is
/// one pass over the characters, and the allocation only happens for a string
/// that actually carries one.
#[must_use]
pub fn visible(text: &str) -> Cow<'_, str> {
    if !text.chars().any(is_dangerous) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(
        text.chars()
            .map(|c| if is_dangerous(c) { MARKER } else { c })
            .collect(),
    )
}

/// A character that steers a terminal, or the reading of what is on it.
///
/// Two families, and the second was found by auditing the first.
///
/// **Controls.** The C0 range and `DEL`, plus C1 (`U+0080`..=`U+009F`), which
/// some terminals read as single-byte equivalents of the two-character escapes:
/// `U+009B` is CSI on its own, so a filter that only looked for the escape
/// character would let it past. (Rust's `is_control` is the `Cc` category, which
/// already covers both; the range is named for the reader rather than for the
/// compiler.)
///
/// **Bidirectional formatting.** `U+202E RIGHT-TO-LEFT OVERRIDE` and its family
/// are category `Cf`, not `Cc`, so nothing above sees them -- and they do not
/// steer the terminal, they steer the *reading*. A title of
/// `safe<RLO> evitcirtser<PDF> tail` displays as `safe restrictive tail`: the
/// text a reviewer reads is not the text the plan holds. That is the Trojan
/// Source shape, and a defence with it left out is a defence with a published
/// bypass.
///
/// Only the bidi controls, not `Cf` as a class: `U+200D ZERO WIDTH JOINER` is
/// `Cf` and is what holds an emoji sequence together, and `U+00AD SOFT HYPHEN`
/// is `Cf` and is ordinary typography. Replacing those would corrupt titles that
/// are merely written by people.
fn is_dangerous(c: char) -> bool {
    if c.is_whitespace() {
        return false;
    }
    c.is_control()
        || ('\u{80}'..='\u{9f}').contains(&c)
        || matches!(c,
            '\u{61c}'                  // ARABIC LETTER MARK
            | '\u{200e}' | '\u{200f}' // LRM, RLM
            | '\u{202a}'..='\u{202e}' // LRE, RLE, PDF, LRO, RLO
            | '\u{2066}'..='\u{2069}' // LRI, RLI, FSI, PDI
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ESC: char = '\u{1b}';

    #[test]
    fn an_escape_is_replaced_and_the_rest_is_left_as_text() {
        assert_eq!(
            visible(&format!("clear {ESC}[2J now")),
            format!("clear {MARKER}[2J now")
        );
    }

    #[test]
    fn a_string_with_nothing_to_do_is_borrowed() {
        assert!(matches!(visible("an ordinary title"), Cow::Borrowed(_)));
        assert!(matches!(visible(""), Cow::Borrowed(_)));
        // Whitespace is not this function's business, so a tab does not make it
        // allocate either.
        assert!(matches!(visible("a\ttitle"), Cow::Borrowed(_)));
    }

    /// Layout owns whitespace, and two owners would fight.
    #[test]
    fn whitespace_is_left_for_the_folding_to_collapse() {
        assert_eq!(visible("a\tb\nc\rd"), "a\tb\nc\rd");
    }

    /// A single-byte CSI is the one a naive filter misses.
    #[test]
    fn a_c1_control_is_caught_as_well_as_an_escape() {
        assert_eq!(visible("\u{9b}2J"), format!("{MARKER}2J"));
        assert_eq!(visible("\u{7f}"), MARKER.to_string(), "DEL");
        assert_eq!(visible("\u{0}"), MARKER.to_string(), "NUL");
    }

    /// Two titles differing only in an escape must not render alike, which is
    /// what a silent drop would have made them.
    ///
    /// The *visible* half of `output.plan_text_cannot_steer_a_terminal`: a
    /// reader deciding whether to trust a plan needs to be told something was
    /// there, not merely protected from it.
    #[test]
    fn two_titles_differing_only_in_an_escape_still_differ() {
        assert_ne!(visible(&format!("a {ESC}[31m b")), visible("a  b"));
    }

    /// **The marker is visible**, which is the half of this that a silent drop
    /// would also have satisfied.
    ///
    /// Found by mutation: setting [`MARKER`] to a space broke no test.
    /// `two_titles_differing_only_in_an_escape_still_differ` passes with a space
    /// too, because the `[31m` after the escape is still there to tell the two
    /// apart -- so it proves the sequence is not dropped whole, and says nothing
    /// about what stands in for the character itself. A reader deciding whether
    /// to trust a plan is owed a mark they can see.
    #[test]
    fn the_marker_is_something_a_reader_can_see() {
        assert!(
            !MARKER.is_whitespace() && !MARKER.is_control(),
            "the marker is invisible, so a replaced character reads as one that \
             was never there: {MARKER:?}"
        );
        assert_eq!(
            unicode_width::UnicodeWidthChar::width(MARKER),
            Some(1),
            "the marker is not one column wide, so replacing a character moves \
             every column after it"
        );
        // And it is not a character a title could plausibly contain on purpose,
        // which is what stops a replacement reading as content.
        assert!(!MARKER.is_alphanumeric() && !MARKER.is_ascii());
    }

    /// **A bidi override is replaced too**, which the first version of this
    /// missed: it is `Cf`, not `Cc`, so `is_control` says no.
    ///
    /// It does not steer the terminal, it steers the reading -- the text a
    /// reviewer sees is not the text the plan holds -- and a defence against
    /// escape sequences that leaves the published bypass in is not one.
    #[test]
    fn a_bidi_override_is_replaced_as_well_as_a_control() {
        for c in [
            '\u{202e}', '\u{202d}', '\u{202a}', '\u{2066}', '\u{2069}', '\u{200f}', '\u{61c}',
        ] {
            assert_eq!(
                visible(&format!("a{c}b")),
                format!("a{MARKER}b"),
                "U+{:04X} reached the reader",
                c as u32
            );
        }
    }

    /// And the formatting characters that are *not* an attack are left alone:
    /// replacing them would corrupt titles people wrote on purpose.
    #[test]
    fn a_joiner_and_a_soft_hyphen_are_not_touched() {
        // A family emoji is held together by zero-width joiners.
        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
        assert_eq!(visible(family), family);
        assert_eq!(visible("co\u{ad}operate"), "co\u{ad}operate", "soft hyphen");
    }

    /// Ordinary text is untouched, so the width the folder measures does not
    /// move for any plan that carries none of this.
    #[test]
    fn non_ascii_text_is_not_a_control_character() {
        for text in ["naive", "\u{4e00}\u{4e8c}", "emoji \u{1f600}", "\u{a0}nbsp"] {
            assert_eq!(visible(text), text, "{text:?} was rewritten");
        }
    }
}
