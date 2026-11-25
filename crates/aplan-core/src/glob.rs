//! Compiling a `paths` entry, in one place (`A1-066`).
//!
//! `paths` is the field that binds evidence to a task, and two layers read it:
//! the hermetic rules ask whether an entry is usable at all (`APLAN032`), and
//! the ladder matches it against what the repository tracks (`APLAN012`,
//! `APLAN019`, `APLAN024`, `APLAN025`, `APLAN031`). Both must agree about what
//! a glob *is*, or `check` accepts a plan whose globs `verify` cannot compile,
//! which is the disagreement `A1-066` exists to end.
//!
//! So the builder and its options live here, in the crate both layers depend on,
//! rather than being written twice with the same three method calls.

/// Compile one `paths` entry.
///
/// `literal_separator(true)`: a single `*` matches within a path segment and
/// does not cross `/`, so `paths: ['src/*']` owns `src/a.rs` but not
/// `src/sub/deep.rs`. Crossing directories needs an explicit `**`, which is the
/// gitignore and ripgrep convention authors already expect.
///
/// **Spelled however the author spelled it** (`A1-254`). A glob is a locator,
/// and `./src/**` names what `src/**` names. Git reports tracked paths in one
/// form, so an entry written the other way matched nothing and the rules built
/// on it said so in terms that read as false: `APLAN012` reported that a commit
/// "touches none of this task's `paths`" about a commit that touches them. The
/// third field to get this reading, after `traces`, which always had it, and
/// `file` evidence, which got it with this task.
///
/// The normalisation is lexical and only removes `.` and `..` components, so a
/// pattern with neither is byte-identical afterwards and `**` is untouched. A
/// The one spelling of a `paths` pattern, so two writings of one path are one
/// entry everywhere that reads them.
///
/// `A1-254` made `./src/**` and `src/**` the same path to a citation and to a
/// glob; this is where that reading lives, so a rule comparing two entries and
/// the compiler matching files against them cannot disagree about whether they
/// are the same (`A1-267`).
#[must_use]
pub fn spelling(pattern: &str) -> String {
    let normalised = crate::anchor::normalise(std::path::Path::new(pattern));
    let normalised = normalised.to_string_lossy();
    // A pattern that normalises to nothing keeps its original text, so a
    // degenerate entry is still judged as what the author wrote.
    if normalised.is_empty() {
        pattern.to_string()
    } else {
        normalised.into_owned()
    }
}

/// Whether `pattern` is a literal file or directory path with no glob
/// metacharacters (`*`, `?`, `[`, `]`, `{`, `}`).
///
/// Used by `APLAN063` (`A2-101`, `ADR-0071`) to distinguish literal paths from
/// glob patterns so prospective or still-matching globs are not treated as
/// single-file history lookups.
#[must_use]
pub fn is_literal_path(pattern: &str) -> bool {
    !pattern.is_empty()
        && !pattern
            .chars()
            .any(|c| matches!(c, '*' | '?' | '[' | ']' | '{' | '}'))
}

/// pattern that normalises to nothing keeps its original text, so a degenerate
/// entry fails exactly as it did rather than becoming a new kind of error.
///
/// # Errors
///
/// The compiler's own error, whose words name the position and the character.
pub fn compile(pattern: &str) -> Result<globset::Glob, globset::Error> {
    globset::GlobBuilder::new(&spelling(pattern))
        .literal_separator(true)
        .build()
}

/// Outcome classification for evaluating one `paths` entry against tracked
/// repository files (`A2-092`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathMatchStatus {
    /// Entry compiled and matched at least one tracked file.
    Matched,
    /// Entry compiled cleanly, but matched zero currently tracked files.
    Empty,
    /// Entry failed to compile under [`compile`] (`APLAN032`).
    Invalid,
}

/// Exact tracked-file match result for one `paths` entry (`A2-092`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PathMatchEntry {
    /// Original pattern text exactly as supplied by the caller or task file.
    pub pattern: String,
    /// Lexically normalised spelling passed to `globset` via [`spelling`].
    pub normalised: String,
    /// Explicit machine-readable status (`matched`, `empty`, or `invalid`).
    pub status: PathMatchStatus,
    /// Compiler error message when `status == invalid`, otherwise `None`.
    pub error: Option<String>,
    /// `true` when `status == empty` and `normalised` is a literal path naming
    /// a directory that contains currently tracked files (missing `/**`).
    pub bare_tracked_directory: bool,
    /// Exact total count of tracked files matched before applying `limit`.
    pub total_matched: usize,
    /// Number of paths emitted in `matches`.
    pub returned: usize,
    /// `true` when `total_matched > returned` due to a bounded `limit`.
    pub truncated: bool,
    /// Lexicographically sorted matched tracked file paths (bounded by `limit`).
    pub matches: Vec<String>,
}

/// Whether `pattern` is a literal path naming a directory that contains at
/// least one tracked file (`ADR-0071`, `A2-092`).
#[must_use]
pub fn is_bare_tracked_directory(pattern: &str, tracked: &[String]) -> bool {
    let norm = spelling(pattern);
    let trimmed = norm.trim_end_matches('/');
    if trimmed.is_empty() || !is_literal_path(trimmed) {
        return false;
    }
    let dir = std::path::Path::new(trimmed);
    tracked.iter().any(|path| {
        let p = std::path::Path::new(path);
        p != dir && p.starts_with(dir)
    })
}

/// Evaluate one `paths` pattern against lexicographically sorted tracked paths
/// using the exact [`compile`] matcher that `check` and `verify` share
/// (`A2-092`).
///
/// `tracked` is expected to use forward-slash repository-relative paths in
/// deterministic order; callers passing unsorted paths get the same match set
/// but should sort first for deterministic bounded output.
#[must_use]
pub fn match_tracked(pattern: &str, tracked: &[String], limit: Option<usize>) -> PathMatchEntry {
    let normalised = spelling(pattern);
    let compiled = match compile(pattern) {
        Ok(glob) => glob.compile_matcher(),
        Err(err) => {
            return PathMatchEntry {
                pattern: pattern.to_string(),
                normalised,
                status: PathMatchStatus::Invalid,
                error: Some(err.to_string()),
                bare_tracked_directory: false,
                total_matched: 0,
                returned: 0,
                truncated: false,
                matches: Vec::new(),
            };
        }
    };

    let mut total_matched = 0usize;
    let mut matches = Vec::new();
    for path in tracked {
        if compiled.is_match(path) {
            total_matched += 1;
            if limit.is_none_or(|cap| matches.len() < cap) {
                matches.push(path.clone());
            }
        }
    }

    let returned = matches.len();
    let truncated = total_matched > returned;
    let (status, bare_tracked_directory) = if total_matched > 0 {
        (PathMatchStatus::Matched, false)
    } else {
        (
            PathMatchStatus::Empty,
            is_bare_tracked_directory(pattern, tracked),
        )
    };

    PathMatchEntry {
        pattern: pattern.to_string(),
        normalised,
        status,
        error: None,
        bare_tracked_directory,
        total_matched,
        returned,
        truncated,
        matches,
    }
}

/// Evaluate a sequence of `paths` entries against sorted `tracked` paths,
/// returning each per-entry result plus the exact deduplicated count of tracked
/// files matched by at least one valid entry (`A2-092`).
#[must_use]
pub fn match_tracked_batch(
    patterns: &[String],
    tracked: &[String],
    limit: Option<usize>,
) -> (Vec<PathMatchEntry>, usize) {
    let mut entries = Vec::with_capacity(patterns.len());
    let mut any_matched = vec![false; tracked.len()];
    for pattern in patterns {
        let entry = match_tracked(pattern, tracked, limit);
        if entry.status == PathMatchStatus::Matched
            && let Ok(glob) = compile(pattern)
        {
            let matcher = glob.compile_matcher();
            for (idx, path) in tracked.iter().enumerate() {
                if !any_matched[idx] && matcher.is_match(path) {
                    any_matched[idx] = true;
                }
            }
        }
        entries.push(entry);
    }
    let unique = any_matched.into_iter().filter(|hit| *hit).count();
    (entries, unique)
}

#[cfg(test)]
mod tests {
    use super::{PathMatchStatus, compile, match_tracked, match_tracked_batch};

    #[test]
    fn a_single_star_stays_inside_one_segment() {
        let g = compile("src/*").expect("valid").compile_matcher();
        assert!(g.is_match("src/a.rs"));
        assert!(!g.is_match("src/sub/deep.rs"), "`*` crossed a separator");
        assert!(
            compile("src/**")
                .expect("valid")
                .compile_matcher()
                .is_match("src/sub/deep.rs")
        );
    }

    /// The shapes `APLAN032` reports, and the words it borrows to report them.
    #[test]
    fn an_unclosed_class_is_refused_and_says_why() {
        let err = compile("src/[").expect_err("not a glob");
        let text = err.to_string();
        assert!(
            text.contains('['),
            "the message must name what it choked on: {text}"
        );
        assert!(compile("src/[a-z].rs").is_ok(), "a closed class is fine");
    }

    /// **A glob is spelled however the author spelled it** (`A1-254`).
    ///
    /// Git reports one form, so an entry written the other way matched nothing
    /// and every rule built on `paths` then said something that read as false.
    #[test]
    fn a_leading_dot_or_a_backed_out_directory_names_the_same_files() {
        for pattern in ["src/**", "./src/**", "src/../src/**"] {
            let m = compile(pattern)
                .unwrap_or_else(|e| panic!("`{pattern}` must compile: {e}"))
                .compile_matcher();
            assert!(
                m.is_match("src/a.rs"),
                "`{pattern}` must name what git calls `src/a.rs`"
            );
        }
    }

    /// **A pattern that normalises to nothing keeps its own text.**
    ///
    /// `.` and `./` normalise to an empty path, and handing the compiler an
    /// empty pattern would make a degenerate entry fail in a new way. It
    /// compiles exactly as it did and matches exactly what it did, which is
    /// nothing in any repository that has files.
    #[test]
    fn a_pattern_that_normalises_away_is_left_as_written() {
        let dot = compile(".")
            .expect("`.` compiled before and must still")
            .compile_matcher();
        assert!(
            dot.is_match("."),
            "`.` names the path `.`, which is what it named before this function \
             normalised anything"
        );
        assert!(
            !dot.is_match(""),
            "and not the empty path, which is what handing the compiler an empty pattern \
             would have made it mean"
        );
        assert!(
            !dot.is_match("src/a.rs"),
            "either way it does not name the tree"
        );
    }

    #[test]
    fn match_tracked_covers_bare_dirs_wildcards_dots_separators_invalid_and_bounds() {
        let mut tracked: Vec<String> = [
            ".env.example",
            ".github/workflows/ci.yml",
            ".hidden/nested/file.txt",
            "docs/guide.md",
            "src/a.rs",
            "src/ab.rs",
            "src/mod.rs",
            "src/sub/deep.rs",
            "src/sub/mod.rs",
            "src/a/b/mod.rs",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        tracked.sort();

        // Bare directory is a valid empty match flagged as a bare tracked directory.
        let bare = match_tracked("src", &tracked, Some(50));
        assert_eq!(bare.status, PathMatchStatus::Empty);
        assert!(bare.bare_tracked_directory);
        assert_eq!(bare.total_matched, 0);
        assert!(bare.matches.is_empty());

        // `/**` matches nested files; `*` stays within one segment; `?` matches one non-separator char.
        let deep = match_tracked("src/**", &tracked, Some(50));
        assert_eq!(deep.status, PathMatchStatus::Matched);
        assert_eq!(deep.total_matched, 6);
        assert!(!deep.truncated);

        let one_seg = match_tracked("src/*", &tracked, Some(50));
        assert_eq!(one_seg.matches, vec!["src/a.rs", "src/ab.rs", "src/mod.rs"]);

        let single_char = match_tracked("src/?.rs", &tracked, Some(50));
        assert_eq!(single_char.matches, vec!["src/a.rs"]);

        // `/**/` matches zero or more intermediate directories.
        let any_depth = match_tracked("src/**/mod.rs", &tracked, Some(50));
        assert_eq!(
            any_depth.matches,
            vec!["src/a/b/mod.rs", "src/mod.rs", "src/sub/mod.rs"]
        );

        // Dot-prefixed paths and `./` lexical normalisation work identically.
        let gh = match_tracked(".github/workflows/*", &tracked, Some(50));
        assert_eq!(gh.matches, vec![".github/workflows/ci.yml"]);
        let hidden = match_tracked(".hidden/**", &tracked, Some(50));
        assert_eq!(hidden.matches, vec![".hidden/nested/file.txt"]);
        let dot_rel = match_tracked("./src/?.rs", &tracked, Some(50));
        assert_eq!(dot_rel.normalised, "src/?.rs");
        assert_eq!(dot_rel.matches, vec!["src/a.rs"]);

        // Invalid glob is distinctly classified with its compiler error.
        let invalid = match_tracked("src/[", &tracked, Some(50));
        assert_eq!(invalid.status, PathMatchStatus::Invalid);
        assert!(invalid.error.as_deref().is_some_and(|e| e.contains('[')));
        assert!(!invalid.bare_tracked_directory);

        // Bounded limit truncates deterministically and counts exact totals.
        let bounded = match_tracked("src/**", &tracked, Some(2));
        assert_eq!(bounded.total_matched, 6);
        assert_eq!(bounded.returned, 2);
        assert!(bounded.truncated);
        assert_eq!(bounded.matches, vec!["src/a.rs", "src/a/b/mod.rs"]);

        // Batch deduplicates unique matched files across overlapping patterns.
        let (entries, unique) = match_tracked_batch(
            &[
                "src/*".to_string(),
                "src/a.rs".to_string(),
                "nowhere/**".to_string(),
                "src/[".to_string(),
            ],
            &tracked,
            Some(50),
        );
        assert_eq!(entries.len(), 4);
        assert_eq!(unique, 3);
    }
}
