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

#[cfg(test)]
mod tests {
    use super::compile;

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
}
