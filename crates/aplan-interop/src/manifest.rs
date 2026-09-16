//! A list of tasks to add as one change (`A1-241`).
//!
//! Two pilots arrived with a written plan and no way to get it in. `p00016`
//! turned a 1,400-line research document into 153 tasks, 11 cuts and 3 releases
//! in one pass, and `p00002` into 64 tasks with cuts, effort, two tag families,
//! paths, traces and blockers. Both wrote a script, because `aplan add` is one
//! task per invocation.
//!
//! `p00002`'s script shelled out 64 times and every task landed correctly.
//! `p00016`'s emitted frontmatter directly, which is the outcome worth
//! preventing: the plan was then authored by a program that knows the file
//! format, which is the coupling `import` exists to avoid for *other* tools'
//! formats, appearing for the case where the source is the author's own document
//! (pre-public adoption evidence item 3, pre-public adoption evidence item 2).
//!
//! **This is not an importer.** ADR-0012's honesty rules are about a format the
//! task model cannot hold, and there is no such loss here: an entry is the set
//! of arguments `aplan add` already takes, so the fields are the model's own.
//! What it removes is the invocation count and the second implementation of the
//! frontmatter contract, not a translation.

use serde::Deserialize;

/// One task to add: what `aplan add` takes, and the body it cannot.
///
/// `deny_unknown_fields` for the reason `.aplan.yml` has it: a misspelled
/// `efort` that is silently ignored writes 153 tasks with no estimates and says
/// nothing, and the author finds out from `APLAN009` much later.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// Title, as a person would read it. The one required field.
    pub title: String,
    /// Task id. Minted from the plan's convention when absent, exactly as
    /// `aplan add` mints it.
    #[serde(default)]
    pub id: Option<String>,
    /// `phase`. Defaults to the cut's, then to the highest in use.
    #[serde(default)]
    pub phase: Option<u32>,
    /// Milestone to join, by id or by the label the cut reports itself under.
    #[serde(default)]
    pub cut: Option<String>,
    /// Estimate in days.
    #[serde(default)]
    pub effort: Option<f64>,
    /// Who owns it.
    #[serde(default)]
    pub owner: Option<String>,
    /// Tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Globs this task is responsible for.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Document sections this task answers to.
    #[serde(default)]
    pub traces: Vec<String>,
    /// Tasks this one waits on. May name a task the plan already has, or one any
    /// entry in the same file declares an `id` for -- including a later one, so
    /// a milestone can be written above the work it lists.
    #[serde(default)]
    pub blocked_by: Vec<String>,
    /// The task's body: everything after the frontmatter, as Markdown.
    ///
    /// The one field here that is not an `aplan add` flag, and it is here
    /// because the scaffold it replaces is what makes a wholesale-authored plan
    /// uncloseable. `p0000014` authored 134 tasks through this seam, every one
    /// carrying `- [ ] state what done means for this task`: unticked,
    /// `APLAN038` reports each closure; ticked, `APLAN039` does. There is no
    /// third option and no way to supply the real criteria without editing 134
    /// files through the format this seam exists to hide
    /// (pre-public adoption evidence item 2, `A1-260`).
    ///
    /// Absent keeps the scaffold, which is what an author who has not written
    /// the body yet wants. Present replaces it whole -- this is not a template
    /// to fill in, and merging one would be inventing content.
    #[serde(default)]
    pub body: Option<String>,
}

/// What went wrong reading a manifest.
#[derive(Debug)]
pub enum ManifestError {
    /// The file could not be read.
    Io {
        /// The file.
        path: std::path::PathBuf,
        /// What the filesystem said.
        source: std::io::Error,
    },
    /// The file is not a list of entries.
    Parse {
        /// The file.
        path: std::path::PathBuf,
        /// What the parser said.
        said: String,
    },
    /// The file parsed and named no tasks.
    Empty {
        /// The file.
        path: std::path::PathBuf,
    },
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "reading {}: {source}", path.display()),
            Self::Parse { path, said } => write!(
                f,
                "{} is not a list of tasks: {said}. Each entry is a mapping with a `title` \
                 and any of the fields `aplan add` takes as flags",
                path.display()
            ),
            Self::Empty { path } => write!(
                f,
                "{} names no tasks, so there is nothing to add",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ManifestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Parse { .. } | Self::Empty { .. } => None,
        }
    }
}

/// Read a manifest: a YAML list of [`Entry`].
///
/// # Errors
///
/// [`ManifestError`] when the file cannot be read, is not a list of entries, or
/// holds none.
pub fn read(path: &std::path::Path) -> Result<Vec<Entry>, ManifestError> {
    let text = std::fs::read_to_string(path).map_err(|source| ManifestError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let entries: Vec<Entry> = serde_yaml_ng::from_str(&text).map_err(|e| ManifestError::Parse {
        path: path.to_path_buf(),
        said: e.to_string(),
    })?;
    if entries.is_empty() {
        return Err(ManifestError::Empty {
            path: path.to_path_buf(),
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::{Entry, ManifestError, read};

    fn at(dir: &std::path::Path, text: &str) -> std::path::PathBuf {
        let p = dir.join("m.yml");
        std::fs::write(&p, text).expect("write");
        p
    }

    /// An entry carries the fields `add` takes, and no others.
    #[test]
    fn an_entry_reads_the_fields_add_takes() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let p = at(
            dir.path(),
            "- title: A first task\n  cut: M-s0\n  effort: 2.5\n  tags: [kind-build]\n  \
             paths: ['src/**']\n  blocked_by: [A-1]\n- title: A second\n",
        );
        let got = read(&p).expect("reads");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].title, "A first task");
        assert_eq!(got[0].cut.as_deref(), Some("M-s0"));
        assert_eq!(got[0].effort, Some(2.5));
        assert_eq!(got[0].blocked_by, vec!["A-1".to_string()]);
        assert_eq!(
            got[1],
            Entry {
                title: "A second".into(),
                id: None,
                phase: None,
                cut: None,
                effort: None,
                owner: None,
                tags: Vec::new(),
                paths: Vec::new(),
                traces: Vec::new(),
                blocked_by: Vec::new(),
                body: None,
            },
            "everything but the title is optional"
        );
    }

    /// The body reaches the entry as written, including the newlines a block
    /// scalar preserves: it becomes the file's Markdown byte for byte, so a
    /// parser that trimmed or rewrapped it would change what an author wrote
    /// (`A1-260`).
    #[test]
    fn an_entry_carries_the_body_verbatim() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        // Concatenated rather than continued: a `\` at the end of a Rust
        // string line eats the leading whitespace of the next, which in YAML is
        // the indentation that makes a block scalar (`A1-274` was found the same
        // way, and this test was written wrong once for the same reason).
        let p = at(
            dir.path(),
            concat!(
                "- title: With a body\n",
                "  body: |\n",
                "    ## Why\n",
                "\n",
                "    Two paragraphs, and\n",
                "\n",
                "    - [ ] one criterion\n",
                "- title: Without one\n",
            ),
        );
        let got = read(&p).expect("reads");
        assert_eq!(
            got[0].body.as_deref(),
            Some("## Why\n\nTwo paragraphs, and\n\n- [ ] one criterion\n")
        );
        assert_eq!(got[1].body, None, "absent is absent, not empty");
    }

    /// **A misspelled field is an error, not a silent omission.**
    ///
    /// The same stance `.aplan.yml` takes, for the same reason at a hundred and
    /// fifty times the scale: `efort` accepted quietly writes a plan with no
    /// estimates and says nothing until `APLAN009` does, long afterwards.
    #[test]
    fn a_misspelled_field_is_refused_by_name() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let p = at(dir.path(), "- title: A task\n  efort: 2\n");
        let err = read(&p).expect_err("unknown field");
        let said = err.to_string();
        assert!(
            said.contains("efort"),
            "the name is what the author fixes: {said}"
        );
        assert!(
            said.contains("aplan add"),
            "and the message says where the field list comes from: {said}"
        );
    }

    /// A file that parses and holds nothing is refused, rather than reported as
    /// a successful run that added no tasks.
    #[test]
    fn an_empty_manifest_is_refused_rather_than_reported_as_done() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let p = at(dir.path(), "[]\n");
        assert!(matches!(
            read(&p).expect_err("empty"),
            ManifestError::Empty { .. }
        ));
    }

    /// A mapping where a list belongs says so, naming the shape it wanted.
    #[test]
    fn a_document_that_is_not_a_list_says_what_one_looks_like() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let p = at(dir.path(), "tasks:\n  - title: A task\n");
        let said = read(&p).expect_err("not a list").to_string();
        assert!(said.contains("is not a list of tasks"), "{said}");
    }

    /// A file that is not there names itself, and carries the cause for a
    /// caller that wants it.
    #[test]
    fn a_missing_manifest_names_the_file_and_keeps_the_cause() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        let missing = dir.path().join("nope.yml");
        let err = read(&missing).expect_err("not there");
        assert!(matches!(err, ManifestError::Io { .. }), "{err:?}");
        let said = err.to_string();
        assert!(said.contains("nope.yml"), "{said}");
        assert!(
            std::error::Error::source(&err).is_some(),
            "the filesystem's own error is kept: {said}"
        );
    }

    /// The two errors that are their own explanation carry no source, so a
    /// caller printing the chain does not print the sentence twice.
    #[test]
    fn a_parse_or_empty_error_is_its_own_explanation() {
        let dir = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("tempdir");
        for text in ["[]\n", "- title: A task\n  efort: 2\n"] {
            let p = at(dir.path(), text);
            let err = read(&p).expect_err("refused");
            assert!(
                std::error::Error::source(&err).is_none(),
                "{}",
                err.to_string()
            );
            assert!(!err.to_string().is_empty());
        }
    }
}
