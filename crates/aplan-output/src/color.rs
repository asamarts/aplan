//! When to emit ANSI colour, and the one place that decides it (`A1-286`).
//!
//! **Almost all of this already worked, and measuring that is what this module
//! is shaped by.** `anstream` resolves an `Auto` stream against the environment
//! before it decides: `NO_COLOR` disables, `CLICOLOR_FORCE` forces, `CLICOLOR=0`
//! disables, and a terminal that says it supports colour enables. All four were
//! honoured by the shipped binary with no code here at all, which is why
//! `A1-206` could say colour "vanishes in a pipe with no flag to set" and be
//! describing something real.
//!
//! What was missing is the *explicit* half. There was no way to ask for colour
//! **into** a pipe, which is what `aplan roadmap | less -R` and any screen
//! capture need, and no way to refuse it at a terminal without setting an
//! environment variable for one command. Both are cases auto-detection cannot
//! serve, because the thing it detects is right and the reader wants something
//! else.
//!
//! So this is deliberately thin: [`ColorChoice`]
//! parses one flag and writes the process-global choice `anstream` already
//! consults. It adds no detection of its own, and the env contract above keeps
//! working because nothing here reimplements it.
//!
//! ## Why a global rather than a value passed around
//!
//! `anstream::stdout()` calls `AutoStream::auto`, which reads
//! `ColorChoice::global()` on every call. Setting it once therefore reaches
//! every renderer in this crate without any of them being handed anything, and
//! it reaches `clap`'s own error and help colouring too, since `clap` builds an
//! `AutoStream` with `Auto` and gets the same resolution. A choice threaded as a
//! parameter would have covered the renderers and left `clap` colouring a usage
//! error red under `--color=never`.

/// When to emit ANSI colour: the value of `--color`.
///
/// `Auto` is the default and is the only value most readers will ever get.
/// `Always` and `Never` are explicit overrides and turn the detection off in the
/// direction the reader asked for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ColorChoice {
    /// Follow the stream and the environment, which is what `anstream` does.
    #[default]
    Auto,
    /// Emit colour even into a pipe.
    Always,
    /// Emit none, even at a terminal.
    Never,
}

impl ColorChoice {
    /// The values `--color` accepts, in the order the help lists them.
    pub const NAMES: [&'static str; 3] = ["auto", "always", "never"];

    /// Parse one of [`Self::NAMES`], or `None`.
    ///
    /// Deliberately *not* a `FromStr` that accepts `yes`/`on`/`true`: `clap`
    /// rejects anything outside `NAMES` before this is reached, so a second,
    /// wider vocabulary here would be a set of spellings the help does not
    /// document and nothing else can reach. The pre-scan in the CLI is the one
    /// caller that meets an unvalidated string, and leaving it at `Auto` is
    /// right, because `clap` is about to refuse the run anyway.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Self::Auto),
            "always" => Some(Self::Always),
            "never" => Some(Self::Never),
            _ => None,
        }
    }

    /// Write this choice as the process-wide one every later `anstream` stream
    /// reads.
    ///
    /// Called once, before anything is printed. `Auto` is written too rather
    /// than skipped: it is already the default, so writing it is a no-op, and a
    /// branch that skipped it would make `--color=auto` mean "whatever was set
    /// earlier" if anything ever sets it earlier.
    pub fn apply(self) {
        anstream::ColorChoice::from(self).write_global();
    }
}

impl From<ColorChoice> for anstream::ColorChoice {
    fn from(c: ColorChoice) -> Self {
        match c {
            ColorChoice::Auto => Self::Auto,
            ColorChoice::Always => Self::Always,
            ColorChoice::Never => Self::Never,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_documented_name_parses_to_a_distinct_choice() {
        let parsed: Vec<ColorChoice> = ColorChoice::NAMES
            .iter()
            .map(|n| ColorChoice::parse(n).expect("a documented name parses"))
            .collect();
        assert_eq!(
            parsed,
            vec![ColorChoice::Auto, ColorChoice::Always, ColorChoice::Never],
            "the names and the variants disagree, so the help documents one thing \
             and the flag does another"
        );
    }

    /// The wider vocabulary is refused on purpose, and this is what says so.
    #[test]
    fn a_spelling_the_help_does_not_document_is_not_accepted() {
        for other in ["yes", "on", "true", "1", "", "AUTO", "Always", "sparkles"] {
            assert_eq!(
                ColorChoice::parse(other),
                None,
                "`{other}` parsed, so the flag accepts a spelling `--help` never names"
            );
        }
    }

    #[test]
    fn the_default_is_auto() {
        assert_eq!(ColorChoice::default(), ColorChoice::Auto);
    }

    /// The mapping is the whole of `apply`, so it is what a test can hold.
    #[test]
    fn each_choice_maps_to_its_anstream_twin() {
        for (ours, theirs) in [
            (ColorChoice::Auto, anstream::ColorChoice::Auto),
            (ColorChoice::Always, anstream::ColorChoice::Always),
            (ColorChoice::Never, anstream::ColorChoice::Never),
        ] {
            assert_eq!(anstream::ColorChoice::from(ours), theirs);
        }
    }
}
