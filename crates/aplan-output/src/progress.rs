//! Automation-safe progress presentation policy.
//!
//! Application events deliberately live outside this crate. This module owns
//! only the decision to hide, stream or display them in-frame, plus the clock
//! policy a renderer follows. Keeping those halves separate prevents a plan
//! operation from learning whether its caller has a terminal.

use std::time::Duration;
use std::{io, io::Write as _};

use unicode_segmentation::UnicodeSegmentation as _;

/// The values accepted by the global `--progress` option.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ProgressChoice {
    #[default]
    Auto,
    Always,
    Never,
}

impl ProgressChoice {
    pub const NAMES: [&'static str; 3] = ["auto", "always", "never"];

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "always" => Some(Self::Always),
            "never" => Some(Self::Never),
            _ => None,
        }
    }
}

/// Output ownership of the command being run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressSurface {
    /// A person-facing CLI rendering.
    Human,
    /// A structured or generated stdout protocol.
    Machine,
    /// An alternate-screen application which owns progress in-frame.
    Interactive,
}

/// Which standard streams are attached to terminals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressStreams {
    BothTerminals,
    StdoutTerminal,
    StderrTerminal,
    Redirected,
}

impl ProgressStreams {
    #[must_use]
    pub const fn from_terminals(stdout: bool, stderr: bool) -> Self {
        match (stdout, stderr) {
            (true, true) => Self::BothTerminals,
            (true, false) => Self::StdoutTerminal,
            (false, true) => Self::StderrTerminal,
            (false, false) => Self::Redirected,
        }
    }

    const fn both_terminals(self) -> bool {
        matches!(self, Self::BothTerminals)
    }

    const fn stderr_terminal(self) -> bool {
        matches!(self, Self::BothTerminals | Self::StderrTerminal)
    }
}

/// Facts detected once by the host before work begins. `static_only` combines
/// every capability or preference which removes animation; its cause does not
/// change the resulting output contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressEnvironment {
    pub streams: ProgressStreams,
    pub ci: bool,
    pub no_color: bool,
    pub static_only: bool,
}

/// Where progress belongs for this invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressPresentation {
    Hidden,
    Stderr { animated: bool, color: bool },
    InFrame { animated: bool, color: bool },
}

/// Resolve policy without consulting global process state.
#[must_use]
pub const fn resolve_progress(
    choice: ProgressChoice,
    surface: ProgressSurface,
    environment: ProgressEnvironment,
) -> ProgressPresentation {
    let animated = environment.streams.stderr_terminal() && !environment.static_only;
    let color = environment.streams.stderr_terminal() && !environment.no_color;
    if matches!(surface, ProgressSurface::Interactive) {
        return ProgressPresentation::InFrame { animated, color };
    }
    match choice {
        ProgressChoice::Always => ProgressPresentation::Stderr { animated, color },
        ProgressChoice::Auto
            if matches!(surface, ProgressSurface::Human)
                && environment.streams.both_terminals()
                && !environment.ci =>
        {
            ProgressPresentation::Stderr { animated, color }
        }
        ProgressChoice::Never | ProgressChoice::Auto => ProgressPresentation::Hidden,
    }
}

/// Interpret the conventional `CI` environment signal.
///
/// A missing, empty, `0`, `false`, `no` or `off` value is false; every other
/// non-empty value is true. The spelling is ASCII-only so an arbitrary host
/// byte string cannot affect terminal output or cause a Unicode failure.
#[must_use]
pub fn ci_signal(value: Option<&std::ffi::OsStr>) -> bool {
    let Some(value) = value else {
        return false;
    };
    let Some(value) = value.to_str() else {
        // An existing value automation supplied must fail closed even when it
        // is not representable as Unicode on this host.
        return !value.is_empty();
    };
    let value = value.trim();
    !value.is_empty()
        && !["0", "false", "no", "off"]
            .iter()
            .any(|false_value| value.eq_ignore_ascii_case(false_value))
}

/// Renderer timing. Values are product policy rather than library defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressTiming {
    pub grace: Duration,
    pub minimum_update_interval: Duration,
}

/// Fit a transient progress row to an exact terminal width.
///
/// Progress cannot use the ordinary prose folder: a wrapped transient row
/// leaves stale terminal cells when the following frame is shorter. This
/// helper truncates only at grapheme boundaries, measures display columns
/// rather than bytes, and keeps the omission visible in every theme.
#[must_use]
pub fn fit_line(text: &str, width: usize) -> String {
    if width == usize::MAX || crate::fold::display_width(text) <= width {
        return text.to_string();
    }
    if width <= 3 {
        return ".".repeat(width);
    }
    let available = width - 3;
    let mut fitted = String::new();
    let mut used = 0_usize;
    for grapheme in text.graphemes(true) {
        let columns = crate::fold::display_width(grapheme);
        if used.saturating_add(columns) > available {
            break;
        }
        fitted.push_str(grapheme);
        used = used.saturating_add(columns);
    }
    fitted.push_str("...");
    fitted
}

/// Write one already-fitted progress row to stderr.
///
/// Cursor controls are presentation, not styling. `anstream` deliberately
/// strips ANSI when color is disabled and would strip the erase sequence with
/// it, concatenating animated frames under `NO_COLOR`. This narrow owner emits
/// only its fixed carriage-return/erase prefix; `row` still comes from the
/// privacy-safe renderer vocabulary rather than plan or provider text.
pub fn write_row(row: &str, transient: bool) -> io::Result<()> {
    let mut stderr = io::stderr().lock();
    if transient {
        write!(stderr, "\r\u{1b}[2K{row}")?;
        stderr.flush()
    } else {
        writeln!(stderr, "{row}")
    }
}

/// Remove the transient progress row without changing cursor visibility.
pub fn clear_row() -> io::Result<()> {
    let mut stderr = io::stderr().lock();
    write!(stderr, "\r\u{1b}[2K")?;
    stderr.flush()
}

impl Default for ProgressTiming {
    fn default() -> Self {
        Self {
            // Below the shortest reliably perceived wait and above the warm
            // read budget. Fast operations therefore finish without a flash.
            grace: Duration::from_millis(120),
            // Ten frames per second is fluid for a textual activity marker and
            // bounded enough not to dominate a terminal or captured log.
            minimum_update_interval: Duration::from_millis(100),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressDraw {
    WaitUntil(Duration),
    Reveal,
    Update,
    FinishVisible,
    FinishHidden,
}

/// Deterministic grace and rate-limit state driven by a renderer's clock.
#[derive(Debug, Clone, Copy)]
pub struct ProgressGate {
    timing: ProgressTiming,
    started_at: Duration,
    last_draw: Option<Duration>,
    terminal: bool,
}

impl ProgressGate {
    #[must_use]
    pub const fn new(started_at: Duration, timing: ProgressTiming) -> Self {
        Self {
            timing,
            started_at,
            last_draw: None,
            terminal: false,
        }
    }

    /// React to an event at a fake- or real-clock instant.
    pub fn changed(&mut self, now: Duration, terminal: bool) -> ProgressDraw {
        if terminal {
            self.terminal = true;
            return if self.last_draw.is_some() {
                ProgressDraw::FinishVisible
            } else {
                ProgressDraw::FinishHidden
            };
        }
        self.due(now)
    }

    /// React to the timer deadline even when the operation itself is blocked.
    pub fn due(&mut self, now: Duration) -> ProgressDraw {
        if self.terminal {
            return if self.last_draw.is_some() {
                ProgressDraw::FinishVisible
            } else {
                ProgressDraw::FinishHidden
            };
        }
        let reveal_at = self.started_at.saturating_add(self.timing.grace);
        let Some(last_draw) = self.last_draw else {
            if now < reveal_at {
                return ProgressDraw::WaitUntil(reveal_at);
            }
            self.last_draw = Some(now);
            return ProgressDraw::Reveal;
        };
        let update_at = last_draw.saturating_add(self.timing.minimum_update_interval);
        if now < update_at {
            ProgressDraw::WaitUntil(update_at)
        } else {
            self.last_draw = Some(now);
            ProgressDraw::Update
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment() -> ProgressEnvironment {
        ProgressEnvironment {
            streams: ProgressStreams::BothTerminals,
            ci: false,
            no_color: false,
            static_only: false,
        }
    }

    #[test]
    fn every_documented_choice_parses_and_no_undocumented_alias_does() {
        assert_eq!(ProgressChoice::parse("auto"), Some(ProgressChoice::Auto));
        assert_eq!(
            ProgressChoice::parse("always"),
            Some(ProgressChoice::Always)
        );
        assert_eq!(ProgressChoice::parse("never"), Some(ProgressChoice::Never));
        for value in ["", "yes", "on", "AUTO", "sometimes"] {
            assert_eq!(ProgressChoice::parse(value), None);
        }
    }

    #[test]
    fn auto_requires_two_terminals_human_output_and_no_ci_signal() {
        for (change, expected) in [
            ("none", true),
            ("stdout", false),
            ("stderr", false),
            ("ci", false),
        ] {
            let mut env = environment();
            match change {
                "stdout" => env.streams = ProgressStreams::StderrTerminal,
                "stderr" => env.streams = ProgressStreams::StdoutTerminal,
                "ci" => env.ci = true,
                _ => {}
            }
            assert_eq!(
                !matches!(
                    resolve_progress(ProgressChoice::Auto, ProgressSurface::Human, env),
                    ProgressPresentation::Hidden
                ),
                expected,
                "wrong auto decision after changing {change}"
            );
        }
        assert_eq!(
            resolve_progress(
                ProgressChoice::Auto,
                ProgressSurface::Machine,
                environment()
            ),
            ProgressPresentation::Hidden
        );
    }

    #[test]
    fn explicit_choices_are_stderr_only_and_interactive_state_stays_in_frame() {
        let mut redirected = environment();
        redirected.streams = ProgressStreams::Redirected;
        assert_eq!(
            resolve_progress(ProgressChoice::Always, ProgressSurface::Machine, redirected),
            ProgressPresentation::Stderr {
                animated: false,
                color: false,
            }
        );
        assert_eq!(
            resolve_progress(ProgressChoice::Never, ProgressSurface::Human, environment()),
            ProgressPresentation::Hidden
        );
        assert!(matches!(
            resolve_progress(
                ProgressChoice::Never,
                ProgressSurface::Interactive,
                environment()
            ),
            ProgressPresentation::InFrame { .. }
        ));
    }

    #[test]
    fn no_color_and_reduced_capabilities_keep_a_plain_ascii_fallback() {
        let mut env = environment();
        env.no_color = true;
        env.static_only = true;
        assert_eq!(
            resolve_progress(ProgressChoice::Always, ProgressSurface::Human, env),
            ProgressPresentation::Stderr {
                animated: false,
                color: false,
            }
        );
        env.no_color = false;
        assert!(matches!(
            resolve_progress(ProgressChoice::Always, ProgressSurface::Human, env),
            ProgressPresentation::Stderr {
                animated: false,
                ..
            }
        ));
    }

    #[test]
    fn conventional_ci_values_are_explicit_and_non_unicode_is_safe() {
        use std::ffi::OsStr;
        for value in [
            None,
            Some(""),
            Some("0"),
            Some("false"),
            Some("NO"),
            Some("off"),
        ] {
            assert!(!ci_signal(value.map(OsStr::new)));
        }
        for value in ["1", "true", "yes", "github", "Falsehood"] {
            assert!(ci_signal(Some(OsStr::new(value))));
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt as _;
            assert!(ci_signal(Some(OsStr::from_bytes(&[0xff]))));
        }
    }

    #[test]
    fn every_choice_surface_and_stream_combination_has_one_exact_disposition() {
        for choice in [
            ProgressChoice::Auto,
            ProgressChoice::Always,
            ProgressChoice::Never,
        ] {
            for surface in [
                ProgressSurface::Human,
                ProgressSurface::Machine,
                ProgressSurface::Interactive,
            ] {
                for streams in [
                    ProgressStreams::BothTerminals,
                    ProgressStreams::StdoutTerminal,
                    ProgressStreams::StderrTerminal,
                    ProgressStreams::Redirected,
                ] {
                    for ci in [false, true] {
                        for no_color in [false, true] {
                            for static_only in [false, true] {
                                let environment = ProgressEnvironment {
                                    streams,
                                    ci,
                                    no_color,
                                    static_only,
                                };
                                let animated = streams.stderr_terminal() && !static_only;
                                let color = streams.stderr_terminal() && !no_color;
                                let expected = if surface == ProgressSurface::Interactive {
                                    ProgressPresentation::InFrame { animated, color }
                                } else if choice == ProgressChoice::Always
                                    || (choice == ProgressChoice::Auto
                                        && surface == ProgressSurface::Human
                                        && streams == ProgressStreams::BothTerminals
                                        && !ci)
                                {
                                    ProgressPresentation::Stderr { animated, color }
                                } else {
                                    ProgressPresentation::Hidden
                                };
                                assert_eq!(
                                    resolve_progress(choice, surface, environment),
                                    expected,
                                    "wrong disposition for {choice:?}, {surface:?}, \
                                     {environment:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn fake_clock_hides_short_work_and_reveals_blocked_work_at_the_deadline() {
        let timing = ProgressTiming::default();
        let mut short = ProgressGate::new(Duration::ZERO, timing);
        assert_eq!(
            short.changed(Duration::from_millis(10), false),
            ProgressDraw::WaitUntil(Duration::from_millis(120))
        );
        assert_eq!(
            short.changed(Duration::from_millis(119), true),
            ProgressDraw::FinishHidden
        );

        let mut blocked = ProgressGate::new(Duration::ZERO, timing);
        assert_eq!(
            blocked.due(Duration::from_millis(120)),
            ProgressDraw::Reveal
        );
        assert_eq!(
            blocked.changed(Duration::from_millis(121), false),
            ProgressDraw::WaitUntil(Duration::from_millis(220))
        );
        assert_eq!(
            blocked.due(Duration::from_millis(220)),
            ProgressDraw::Update
        );
        assert_eq!(
            blocked.changed(Duration::from_millis(221), true),
            ProgressDraw::FinishVisible
        );
    }

    #[test]
    fn transient_rows_are_width_safe_at_grapheme_boundaries() {
        assert_eq!(
            fit_line("progress: reading sources", 18),
            "progress: readi..."
        );
        assert!(crate::fold::display_width(&fit_line("alpha \u{754c} omega", 10)) <= 10);
        assert_eq!(fit_line("abcdef", 3), "...");
        assert_eq!(fit_line("abcdef", 2), "..");
        assert_eq!(fit_line("short", usize::MAX), "short");
    }
}
