//! What a failed command's output is allowed to say in a finding (`A1-365`).
//!
//! The pilot's complaint, in full: `verify --execute` rejected two cited tests
//! that failed only inside a nested-Podman runner, and the whole actionable
//! output was `test ... failed under --execute` with the instruction "make the
//! cited test pass". Both streams had been captured and then thrown away, so
//! the only way to find the cause was to enter the runner and invoke each
//! selector by hand -- which turned out to expose five independent boundary
//! faults, none of which that instruction could distinguish.
//!
//! **The contract, decided rather than drifted into.** A few lines are present
//! *by default*, bounded and redacted, because the requirement is to diagnose a
//! failure from the CI record and an opt-in reproduces the pilot's experience:
//! they met the opaque failure first and would only then have discovered a key
//! existed. `test.list` has retained one line of stderr for launch failures
//! since it was written, so this is that precedent applied to the other half
//! rather than a new policy.
//!
//! **The cost, recorded rather than hidden.** A CI log can now contain a little
//! output the adopter did not explicitly ask for. Three things keep that honest:
//! the bound is small, every line goes through the same sanitizer a generated
//! document uses, and obvious credential shapes are masked. `test.output_lines:
//! 0` suppresses it entirely for a repository whose tests print things it would
//! rather not publish.
//!
//! **What redaction does not promise.** It is a best-effort filter over shapes
//! that are recognisably credentials, not a secret scanner. It deliberately does
//! *not* mask bare hex or base64 runs, because commit shas, digests and test
//! hashes are exactly what an adopter needs to read. A repository whose tests
//! print secrets in an unrecognisable shape should set `test.output_lines: 0`,
//! and the upgrade guidance says so in those words.
//!
//! **What its audit added.** Three shapes leaked when the rules were first
//! written, and all three occur in ordinary output: a credential *inside* a
//! larger token, which is what `https://user:ghp_...@host` is and what a failing
//! `git clone` prints; `Authorization:Bearer <token>` with no space, where the
//! keyed word holds the scheme and the credential is the next one; and a session
//! cookie, which is a bearer credential that is not called one. The distinctive
//! markers are searched anywhere in a token now rather than anchored to its
//! start, which is the fix for the first and the reason `sk-` is handled
//! separately: three characters appear inside ordinary words, and `task-force`
//! contains them.

/// How much of a failed command's output a finding may carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bound {
    /// Lines across both streams. `0` suppresses the excerpt entirely.
    pub lines: usize,
    /// How wide one line may be before it is cut, in characters.
    pub line_chars: usize,
}

/// Lines kept by default.
///
/// Eight, because a failure that explains itself usually does so in a handful
/// of lines -- an assertion with its location, or a runner's error with the
/// command it tried -- and because eight repeated across the citations of one
/// test is a paragraph rather than a page.
pub const DEFAULT_LINES: usize = 8;

/// How wide a kept line may be.
///
/// Wider than a terminal on purpose: a path inside a container is long and
/// cutting it at eighty would remove the part that identifies it. The renderers
/// fold what they are given.
pub const DEFAULT_LINE_CHARS: usize = 200;

impl Default for Bound {
    fn default() -> Self {
        Self {
            lines: DEFAULT_LINES,
            line_chars: DEFAULT_LINE_CHARS,
        }
    }
}

/// What `marker` a cut line ends with, so truncation is never silent.
const CUT: &str = " [cut]";

/// The text a masked credential is replaced by.
const MASKED: &str = "[redacted]";

/// A bounded, sanitized, redacted view of what a failed command printed.
///
/// `None` when there is nothing to show, or when the bound suppresses it.
/// Otherwise lines prefixed by the stream they came from, with every omission
/// stated: a reader must never have to wonder whether they are looking at all
/// of it.
///
/// **The tail, not the head.** A runner that fails says why at the end: the
/// assertion, the summary, the non-zero reason. Taking the first lines would
/// return the banner.
#[must_use]
pub fn of(stdout: &[u8], stderr: &[u8], bound: Bound) -> Option<String> {
    if bound.lines == 0 {
        return None;
    }
    let err = kept(stderr, bound);
    let out = kept(stdout, bound);
    if err.is_empty() && out.is_empty() {
        return None;
    }

    // **stderr is served first**, because that is where a failing command
    // usually explains itself, and stdout takes what is left. A runner that
    // fails on stdout alone therefore gets the whole budget, which is the half
    // of this that the pilot's own runner needed.
    let take_err = err.len().min(bound.lines);
    let take_out = out.len().min(bound.lines - take_err);

    let mut shown = Vec::new();
    push_stream(&mut shown, "stderr", &err, take_err);
    push_stream(&mut shown, "stdout", &out, take_out);
    Some(shown.join("\n"))
}

/// One stream's kept lines, with whatever was left out said out loud.
fn push_stream(into: &mut Vec<String>, name: &str, lines: &[String], take: usize) {
    if lines.is_empty() {
        return;
    }
    if take == 0 {
        // Said rather than dropped: a reader who sees only stderr has to know
        // stdout had something too, or they will go looking for it twice.
        into.push(format!("  {name}: {} line(s) not shown", lines.len()));
        return;
    }
    let omitted = lines.len() - take;
    if omitted > 0 {
        into.push(format!("  {name}: {omitted} earlier line(s) not shown"));
    } else {
        into.push(format!("  {name}:"));
    }
    for line in &lines[lines.len() - take..] {
        into.push(format!("    {line}"));
    }
}

/// A stream's non-blank lines, sanitized, redacted and cut to width.
///
/// Blank lines are dropped before the budget is spent: a runner that pads its
/// output with them would otherwise fill the excerpt with nothing.
fn kept(raw: &[u8], bound: Bound) -> Vec<String> {
    String::from_utf8_lossy(raw)
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            cut(
                &redact(&aplan_output::sanitize::visible(l)),
                bound.line_chars,
            )
        })
        .collect()
}

/// `line` at most `width` characters, saying so when it was cut.
fn cut(line: &str, width: usize) -> String {
    if line.chars().count() <= width {
        return line.to_string();
    }
    let kept: String = line.chars().take(width).collect();
    format!("{kept}{CUT}")
}

/// Keys whose *value* is masked wherever one appears with `:` or `=` after it.
const SECRET_KEYS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "api_key",
    "apikey",
    "api-key",
    "access_key",
    "auth_token",
    "authorization",
    "private_key",
    "token",
    // A session identifier is a bearer credential in everything but name, and
    // `Set-Cookie: session=...` is an ordinary line for a failing HTTP test
    // (`A1-365`, audit).
    "session",
    "cookie",
];

/// Words that introduce a credential rather than being one.
///
/// `Authorization: Bearer <token>` puts the key, the scheme and the secret in
/// three separate words, so masking "the word after the key" masks the scheme
/// and publishes the secret. These are kept and the word after them is masked.
const AUTH_SCHEMES: &[&str] = &["bearer", "basic", "digest", "token"];

/// Credential shapes distinctive enough to mask wherever they appear *inside* a
/// token (`A1-365`, audit).
///
/// Searched rather than matched at the start, because the shape that actually
/// reaches a CI log is a credential embedded in something larger:
/// `https://user:ghp_...@github.com/x/y` is what a failing `git clone` prints,
/// and a start-only match left the whole line in the clear.
const SECRET_MARKERS: &[&str] = &[
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "AKIA",
    "ASIA",
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "xoxr-",
    "-----BEGIN",
];

/// Shapes too short to search for safely, so matched only at a token's start.
///
/// `sk-` is three characters and appears inside ordinary words -- `task-force`
/// contains it -- so searching for it anywhere would mask prose. At the start
/// of a token it is `OpenAI`'s key prefix and little else.
const SECRET_PREFIXES: &[&str] = &["sk-"];

/// Obvious credential shapes masked (`A1-365`).
///
/// Best effort, and said so where an adopter reads it. Two rules, both chosen
/// to be explainable rather than clever:
///
/// 1. a word from [`SECRET_KEYS`] followed by `:` or `=` masks the value up to
///    the next whitespace, which covers `TOKEN=abc`, `password: hunter2` and
///    `Authorization: Bearer xyz`;
/// 2. a token starting with one of [`SECRET_PREFIXES`] is masked wherever it
///    appears, which covers the shapes that are credentials on sight.
///
/// It does **not** mask bare hex or base64 runs. That would catch commit shas,
/// image digests and test hashes, which are the things an adopter is reading
/// this output to find.
fn redact(line: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut mask_next = false;
    for word in line.split_inclusive(char::is_whitespace) {
        let (token, trailing) = split_trailing_space(word);
        if mask_next && !token.is_empty() {
            // **An auth scheme is not the credential** (`A1-365`, found by this
            // module's own test). `Authorization: Bearer <token>` masked the
            // word `Bearer` and left the token in the clear, which is the one
            // shape where masking the next word is exactly wrong. The scheme is
            // kept and the word after it is masked instead.
            if AUTH_SCHEMES.contains(&token.to_ascii_lowercase().as_str()) {
                out.push(word.to_string());
                continue;
            }
            out.push(format!("{MASKED}{trailing}"));
            mask_next = false;
            continue;
        }
        let lowered = token.to_ascii_lowercase();
        // `key=value` and `key: value`: the value may be in this word or the
        // next one, so both are handled.
        if let Some((key, rest)) = lowered
            .split_once('=')
            .or_else(|| lowered.split_once(':'))
            .filter(|(k, _)| {
                SECRET_KEYS
                    .iter()
                    .any(|s| k.trim_end_matches('"').ends_with(s))
            })
        {
            let shown = &token[..=key.len()];
            let value = rest.trim().trim_matches('"');
            if rest.trim().is_empty() {
                // The value is the next word.
                out.push(format!("{shown}{trailing}"));
                mask_next = true;
            } else if AUTH_SCHEMES.contains(&value) {
                // `Authorization:Bearer <token>`, with no space after the
                // colon: what sits in this word is the scheme, and masking it
                // published the credential in the next one -- the same defect
                // the spaced form had, by another route (`A1-365`, audit).
                out.push(format!("{token}{trailing}"));
                mask_next = true;
            } else {
                out.push(format!("{shown}{MASKED}{trailing}"));
            }
            continue;
        }
        if AUTH_SCHEMES.contains(&lowered.as_str()) {
            out.push(format!("{token}{trailing}"));
            mask_next = true;
            continue;
        }
        let marked = SECRET_MARKERS
            .iter()
            .any(|m| token.contains(m) || lowered.contains(&m.to_ascii_lowercase()));
        let prefixed = SECRET_PREFIXES
            .iter()
            .any(|p| lowered.starts_with(&p.to_ascii_lowercase()));
        if marked || prefixed {
            // The whole token, not the matched part: a URL carrying a token is
            // not safe to publish with the token snipped out of the middle, and
            // the host is rarely the thing the reader needed.
            out.push(format!("{MASKED}{trailing}"));
            continue;
        }
        out.push(word.to_string());
    }
    out.concat()
}

/// A word split from the whitespace `split_inclusive` left on it.
fn split_trailing_space(word: &str) -> (&str, &str) {
    let end = word.trim_end_matches(char::is_whitespace).len();
    (&word[..end], &word[end..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(stdout: &str, stderr: &str) -> String {
        of(stdout.as_bytes(), stderr.as_bytes(), Bound::default()).unwrap_or_default()
    }

    /// **A runner that fails on stdout alone is as diagnosable as one that
    /// fails on stderr alone** (`A1-365`, criterion 1).
    ///
    /// Both halves, because the pilot's own runner failed on one of them and a
    /// design that served only stderr would have left it exactly where it was.
    #[test]
    fn either_stream_alone_carries_the_failure() {
        let only_err = show("", "assertion failed: left == right\n  at engine.rs:42\n");
        assert!(only_err.contains("assertion failed"), "{only_err}");
        assert!(
            only_err.contains("stderr:"),
            "the stream is named: {only_err}"
        );

        let only_out = show("FAIL phase1.browser-inspection: exit 3\n", "");
        assert!(only_out.contains("exit 3"), "{only_out}");
        assert!(
            only_out.contains("stdout:"),
            "the stream is named: {only_out}"
        );

        // Nothing to say is `None`, not an empty block that reads as a failure
        // with no output when the truth is there was none.
        assert!(of(b"", b"", Bound::default()).is_none());
        assert!(
            of(b"   \n\n", b"\n", Bound::default()).is_none(),
            "blank is nothing"
        );
    }

    /// **Both streams are served, and stderr goes first** (`A1-365`).
    #[test]
    fn stderr_is_served_first_and_stdout_takes_what_is_left() {
        let both = show("out-one\nout-two\n", "err-one\nerr-two\n");
        let err_at = both.find("err-two").expect("stderr is shown");
        let out_at = both.find("out-two").expect("stdout is shown");
        assert!(err_at < out_at, "stdout came first:\n{both}");
    }

    /// **Every omission is stated** (`A1-365`, criterion 3).
    ///
    /// Truncation that cannot be seen is worse than truncation: a reader who
    /// cannot tell whether they have all of it will either trust a fragment or
    /// rerun the command, and the second is what this exists to avoid.
    #[test]
    fn nothing_is_dropped_without_saying_so() {
        let mut many = String::new();
        for i in 1..=50 {
            use std::fmt::Write as _;
            let _ = writeln!(many, "line {i}");
        }
        let shown = show("", &many);
        assert!(
            shown.contains("earlier line(s) not shown"),
            "a truncated stream did not say so:\n{shown}"
        );
        // The tail, because that is where a failure explains itself.
        assert!(
            shown.contains("line 50"),
            "the last line is missing:\n{shown}"
        );
        assert!(!shown.contains("line 1\n"), "the head was kept:\n{shown}");
        assert_eq!(
            shown.lines().filter(|l| l.starts_with("    ")).count(),
            DEFAULT_LINES,
            "the bound was not honoured:\n{shown}"
        );

        // A stream squeezed out entirely still says it had something.
        let shown = show("pushed out\n", &many);
        assert!(
            shown.contains("stdout: 1 line(s) not shown"),
            "a squeezed-out stream vanished:\n{shown}"
        );

        // A long line is cut visibly rather than wrapped or dropped.
        let long = "x".repeat(DEFAULT_LINE_CHARS * 2);
        let shown = show("", &format!("{long}\n"));
        assert!(
            shown.contains(CUT),
            "a long line was not marked as cut:\n{shown}"
        );
        for line in shown.lines().filter(|l| l.starts_with("    ")) {
            assert!(
                line.chars().count() <= DEFAULT_LINE_CHARS + CUT.len() + 4,
                "a line is longer than the bound: {} chars",
                line.chars().count()
            );
        }
    }

    /// **Hostile bytes cannot corrupt the result** (`A1-365`, criterion 3).
    ///
    /// Invalid UTF-8, terminal controls and embedded newlines all arrive from a
    /// command this tool did not write. Everything goes through the sanitizer a
    /// generated document uses, so an escape sequence is shown rather than run.
    #[test]
    fn control_bytes_and_invalid_utf8_are_shown_rather_than_run() {
        let shown = of(
            b"",
            b"\x1b[41mred\x1b[0m \xff\xfe invalid \x07bell\r\n",
            Bound::default(),
        )
        .expect("there is output");
        assert!(
            !shown.contains('\u{1b}') && !shown.contains('\u{7}'),
            "a control byte survived into the finding: {shown:?}"
        );
        assert!(
            shown.contains(aplan_output::sanitize::MARKER),
            "nothing was marked as replaced: {shown:?}"
        );
        // A `\r` must not leave a bare carriage return that rewrites the line.
        assert!(!shown.contains('\r'), "{shown:?}");
    }

    /// **Credential shapes are masked** (`A1-365`, criterion 3).
    ///
    /// Best effort and documented as such, so the test pins the shapes the
    /// policy claims rather than implying completeness.
    #[test]
    fn recognisable_credentials_are_masked() {
        for (line, leaked) in [
            (
                "TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123",
                "ghp_abcdefghijklmnopqrstuvwxyz0123",
            ),
            ("password: hunter2", "hunter2"),
            (
                "Authorization: Bearer eyJhbGciOi.payload.sig",
                "eyJhbGciOi.payload.sig",
            ),
            ("api_key=sk-livekeyvalue", "sk-livekeyvalue"),
            ("aws id AKIAIOSFODNN7EXAMPLE here", "AKIAIOSFODNN7EXAMPLE"),
            ("slack xoxb-1234-5678-abcdef", "xoxb-1234-5678-abcdef"),
            ("-----BEGIN RSA PRIVATE KEY-----", "-----BEGIN RSA"),
        ] {
            let shown = show("", &format!("{line}\n"));
            assert!(
                !shown.contains(leaked),
                "`{leaked}` survived redaction in `{line}`:\n{shown}"
            );
            assert!(
                shown.contains(MASKED),
                "nothing was masked in `{line}`:\n{shown}"
            );
        }
    }

    /// **A sha is not a secret** (`A1-365`).
    ///
    /// The other half of the redaction decision, and the reason it is not a
    /// generic entropy filter: commit shas, image digests and test hashes are
    /// exactly what an adopter reads this output to find, and masking them would
    /// make the excerpt useless for the case it was built for.
    #[test]
    fn hashes_and_paths_survive_because_they_are_the_answer() {
        let shown = show(
            "",
            "failed at 6bdc428dfce6934a5e47a6092d55028fe31b8ee7 \
             sha256:ab12cd34ef56 /var/lib/containers/storage/overlay\n",
        );
        for needed in [
            "6bdc428dfce6934a5e47a6092d55028fe31b8ee7",
            "sha256:ab12cd34ef56",
            "/var/lib/containers/storage/overlay",
        ] {
            assert!(shown.contains(needed), "`{needed}` was masked:\n{shown}");
        }
    }

    /// **The bound can suppress the excerpt entirely** (`A1-365`, criterion 2).
    ///
    /// The escape hatch for a repository whose tests print things it would
    /// rather not publish, and the reason the privacy cost of the default is
    /// bearable: it is answerable in one key.
    #[test]
    fn a_zero_bound_shows_nothing_at_all() {
        let bound = Bound {
            lines: 0,
            ..Bound::default()
        };
        assert!(of(b"secrets everywhere", b"and here", bound).is_none());
    }
}

/// Guards against the shape this module was built to replace (`A1-365`,
/// criterion 7).
#[cfg(test)]
mod not_a_boolean_again {
    /// **A failed execution carries evidence, not a flag** (`A1-365`).
    ///
    /// The defect was structural rather than a missing feature: `Outcome`
    /// reduced a failed run to `Failed` with no payload, and `Executed` to
    /// `Ran(bool)`, so both streams were captured and then had nowhere to go.
    /// Any future refactor that collapses them back -- in the name of
    /// simplifying an enum nobody reads -- would restore the pilot's experience
    /// exactly, and would do it silently, because nothing else in the suite
    /// asserts the *shape*.
    ///
    /// So this asserts the shape. `Outcome::Failed` must carry something, and
    /// the source is read rather than the behaviour exercised, because a type
    /// that has lost its payload no longer compiles into the tests that use it
    /// and the failure would arrive as a hundred compile errors rather than as
    /// one sentence explaining why the payload is there.
    #[test]
    fn the_failed_outcome_still_carries_what_the_command_printed() {
        let src = include_str!("resolve.rs");
        assert!(
            src.contains("Failed(Option<String>)"),
            "`Outcome::Failed` no longer carries the output. That is the shape `A1-365` \
             existed to remove: both streams are captured a few lines above it, and a variant \
             with no payload is where they used to go to die."
        );
        assert!(
            !src.contains("Ran(bool)"),
            "`Executed::Ran(bool)` is back. A citation that merely passed or failed cannot \
             tell its reader why, which is the whole finding of `A1-365`."
        );
        // And the excerpt is actually attached, not computed and dropped.
        assert!(
            src.contains("with_detail(detail)"),
            "the excerpt is no longer attached to the finding, so it is computed and thrown \
             away -- which is the original defect with extra steps."
        );
    }
}

#[cfg(test)]
mod redaction_shapes {
    use super::*;

    fn shown(line: &str) -> String {
        of(b"", format!("{line}\n").as_bytes(), Bound::default()).unwrap_or_default()
    }

    /// **The shapes that actually reach a CI log, each masked** (`A1-365`,
    /// audit).
    ///
    /// Three of these leaked when this table was first written, and each was a
    /// different hole in the same rule:
    ///
    /// - `Authorization:Bearer <token>` with no space put the *scheme* in the
    ///   keyed word, so masking "the value after the key" masked `Bearer` and
    ///   published the token. The spaced form had the same defect and was fixed
    ///   a session earlier; this is the other route to it.
    /// - `https://user:ghp_...@host` matched nothing, because the credential
    ///   prefixes were anchored to the start of a token and this one is in the
    ///   middle. That is the shape a failing `git clone` prints, which makes it
    ///   the likeliest of all of them.
    /// - `Set-Cookie: session=...` matched nothing, because a session
    ///   identifier is a bearer credential that is not called one.
    ///
    /// Written as a table of *what must not survive*, so a future rule change
    /// is measured against the leak rather than against the implementation.
    #[test]
    fn no_recognisable_credential_survives_the_shapes_that_occur() {
        for (line, must_not_survive) in [
            (
                "TOKEN = ghp_abcdefghijklmnopqrstuvwxyz0123",
                "ghp_abcdefghijklmnopqrstuvwxyz0123",
            ),
            (
                "Authorization:Bearer eyJhbGciOi.payload.sig",
                "eyJhbGciOi.payload.sig",
            ),
            (
                "git clone https://user:ghp_abcdefghijklmnopqrstuvwxyz0123@github.com/x/y",
                "ghp_abcdefghijklmnopqrstuvwxyz0123",
            ),
            ("password:hunter2", "hunter2"),
            (
                "export AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI0K7MDENGbPxRfiCYEXAMPLEKEY",
                "wJalrXUtnFEMI0K7MDENGbPxRfiCYEXAMPLEKEY",
            ),
            (
                "--token=ghp_abcdefghijklmnopqrstuvwxyz0123",
                "ghp_abcdefghijklmnopqrstuvwxyz0123",
            ),
            (
                "{\"token\":\"ghp_abcdefghijklmnopqrstuvwxyz0123\"}",
                "ghp_abcdefghijklmnopqrstuvwxyz0123",
            ),
            ("Set-Cookie: session=abc123def456", "abc123def456"),
            (
                "slack hook xoxb-1-2-abcdef in the middle",
                "xoxb-1-2-abcdef",
            ),
            ("sk-liveopenaikeyvalue", "sk-liveopenaikeyvalue"),
        ] {
            let out = shown(line);
            assert!(
                !out.contains(must_not_survive),
                "`{must_not_survive}` survived in `{line}`:\n{out}"
            );
            assert!(
                out.contains(MASKED),
                "nothing was masked in `{line}`:\n{out}"
            );
        }
    }

    /// **And the things an adopter is reading the output to find survive**
    /// (`A1-365`, audit).
    ///
    /// The other half of the decision, and the reason the markers are a short
    /// explicit list rather than an entropy filter. `sk-` is the case that
    /// forced the distinction: three characters, and `task-force` contains
    /// them, so searching for it anywhere would mask prose. It is matched at a
    /// token's start only, where it is a key prefix and little else.
    #[test]
    fn what_the_reader_came_for_is_not_masked() {
        for (line, must_survive) in [
            ("task-force completed in 3s", "task-force"),
            (
                "failed at 6bdc428dfce6934a5e47a6092d55028fe31b8ee7",
                "6bdc428dfce6934a5e47a6092d55028fe31b8ee7",
            ),
            ("image sha256:ab12cd34ef56", "sha256:ab12cd34ef56"),
            (
                "/var/lib/containers/storage/overlay/ask-dir",
                "/var/lib/containers/storage/overlay/ask-dir",
            ),
            (
                "assertion failed: left == right",
                "assertion failed: left == right",
            ),
        ] {
            let out = shown(line);
            assert!(
                out.contains(must_survive),
                "`{must_survive}` was masked:\n{out}"
            );
            assert!(
                !out.contains(MASKED),
                "an ordinary line was masked, which makes the excerpt useless for the case it \
                 exists for: `{line}`\n{out}"
            );
        }
    }
}
