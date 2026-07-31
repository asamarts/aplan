//! `.aplan.yml`: the repository's policy, in one schema-derived type.
//!
//! A repo declares its policy once, in `.aplan.yml`, rather than threading flags
//! through CI. The `Config` type is the single source of that schema, exactly as
//! `aplan_core::task::Task` is for a task: `schemas/v1/config.json` is *derived*
//! from it (`cargo xtask gen-schema`) and gated by `--check`.
//!
//! Two rules give the file teeth:
//!
//! - it is `deny_unknown_fields`, so a key the tool does not understand is a
//!   reported error, never silently ignored. A config the tool ignores is drift
//!   on day one, the exact failure `aplan` exists to catch.
//! - the `test` runner is the trust boundary (HLD §5.5): it is the only place a
//!   command may be named, never a task file. [`TestRunner`] carries that
//!   guarantee (a whole-element placeholder, validated at load, spawned with no
//!   shell), so it lives here, in trusted top-level config, and nowhere else.

use std::path::{Path, PathBuf};

use aplan_core::EvidenceKind;
use schemars::JsonSchema;
use serde::Deserialize;

/// The whole-element token replaced by the test name in a `run` template.
pub const PLACEHOLDER: &str = "{test}";

/// The config file, at the repository root.
pub const CONFIG_FILE: &str = ".aplan.yml";

/// A failure loading or validating `.aplan.yml`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid test runner in `.aplan.yml`: {0}")]
    BadRunner(String),
    #[error("could not read `.aplan.yml`: {0}")]
    Io(String),
    #[error("could not parse `.aplan.yml`: {0}")]
    Parse(String),
}

/// The parsed `.aplan.yml`. Every field is optional: an absent key means "use
/// the built-in default", and a CLI flag overrides whatever is here (the CLI
/// owns precedence). Unknown keys are rejected, not ignored.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Require an `effort_days` estimate on active tasks (`APLAN009`). Default:
    /// `true`. The `--no-require-effort` flag overrides this to `false`.
    #[serde(default)]
    pub require_effort: Option<bool>,
    /// Resolve `traces` anchors (`APLAN008`) against this root. Default: the
    /// current directory. The `--traces-root` flag overrides it.
    #[serde(default)]
    pub traces_root: Option<String>,
    /// Skip `traces` checking entirely. Default: `false`. The `--no-traces` flag
    /// overrides it.
    #[serde(default)]
    pub no_traces: Option<bool>,
    /// The test runner for `aplan verify --execute`.
    #[serde(default)]
    pub test: Option<TestRunner>,
    /// Evidence-strength policy (`APLAN017`).
    #[serde(default)]
    pub evidence: Option<EvidenceConfig>,
    /// Turn on stale-`done` drift detection (`APLAN018`): `aplan close` stamps a
    /// content fingerprint of each traced section, and `check` warns when a closed
    /// task's traced section changes afterward. Off unless set.
    #[serde(default)]
    pub detect_drift: Option<bool>,
    /// Require every non-milestone task to be a member of exactly one cut
    /// (`APLAN022`): in exactly one milestone's `blocked_by`, unless the task is
    /// tagged `cross-cutting`. This makes a milestone's `done` prove the cut is
    /// *exhaustively* complete, not just that its listed members are. Off unless
    /// set; only meaningful once the plan uses milestone meta-tasks (ADR-0008).
    #[serde(default)]
    pub require_cut_membership: Option<bool>,
    /// Directory holding the plan's task files. Default: `aplan`. A relative value
    /// is resolved against this config file's directory, so `root: plan` means
    /// `<repo>/plan` wherever the command runs. The `--root` flag overrides it.
    #[serde(default)]
    pub root: Option<String>,
}

/// How strong a `done` task's evidence must be (`APLAN017`), and which tasks are
/// exempt because nothing stronger exists.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceConfig {
    /// Evidence kinds strong enough to close a task. A `done` task must carry at
    /// least one of these; otherwise `APLAN017` fires. Absent leaves the rule off.
    #[serde(default)]
    pub require_min_strength: Option<Vec<EvidenceKind>>,
    /// Tags that exempt a task from the requirement (a phone call, a regulator
    /// letter, where a dated `note` is genuinely the best evidence there is).
    #[serde(default)]
    pub exempt_tags: Option<Vec<String>>,
}

/// How to list and run tests, for `aplan verify`. Both are argv vectors, spawned
/// without a shell (HLD §5.5).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestRunner {
    /// argv that prints the available test names to stdout, one per line. A
    /// trailing `: test` / `: benchmark` (cargo's `--list` format) is stripped.
    pub list: Vec<String>,
    /// argv that runs one test. Exactly one element must be [`PLACEHOLDER`]; it
    /// is replaced, whole, by the test name.
    pub run: Vec<String>,
}

impl TestRunner {
    /// Reject a runner that could not receive a test name safely.
    pub fn validate(&self) -> Result<(), Error> {
        if self.list.is_empty() {
            return Err(Error::BadRunner("`test.list` is an empty command".into()));
        }
        if self.run.is_empty() {
            return Err(Error::BadRunner("`test.run` is an empty command".into()));
        }
        let holes = self
            .run
            .iter()
            .filter(|e| e.as_str() == PLACEHOLDER)
            .count();
        if holes != 1 {
            return Err(Error::BadRunner(format!(
                "`test.run` must contain `{PLACEHOLDER}` exactly once, as a whole argument \
                 (found {holes})"
            )));
        }
        // The placeholder must never be the program (argv[0]): `run: ['{test}']`
        // would make the untrusted test name the command that is executed. The
        // command must be a fixed, trusted program, this is the core of the trust
        // boundary (HLD 5.5), and `validate` is where it is enforced.
        if self.run.first().map(String::as_str) == Some(PLACEHOLDER) {
            return Err(Error::BadRunner(format!(
                "`{PLACEHOLDER}` may not be the command (the first element) of `test.run`: the \
                 program to run must be a fixed command, never the test name"
            )));
        }
        // The name must never be concatenated into a larger argument: that is
        // the only way a shell-using runner could be injected by a task file.
        if self
            .run
            .iter()
            .any(|e| e.as_str() != PLACEHOLDER && e.contains(PLACEHOLDER))
        {
            return Err(Error::BadRunner(format!(
                "`test.run` may use `{PLACEHOLDER}` only as a whole argument, never spliced \
                 inside another one"
            )));
        }
        Ok(())
    }

    /// The argv to run `test_name`. The name replaces only the standalone
    /// placeholder element, so it is never spliced into a shell string.
    pub fn run_argv(&self, test_name: &str) -> Vec<String> {
        self.run
            .iter()
            .map(|e| {
                if e == PLACEHOLDER {
                    test_name.to_string()
                } else {
                    e.clone()
                }
            })
            .collect()
    }
}

/// Load `.aplan.yml`, discovered by walking up from `start` to the filesystem
/// root (the nearest one wins), as git and most linters find their config. So
/// `check` and `verify`, given any path inside the repo, converge on the same
/// top-level file. Absent is not an error: it yields the default (empty) config,
/// so a repo with no `.aplan.yml` works unchanged.
pub fn load_config(start: &Path) -> Result<Config, Error> {
    Ok(load_config_located(start)?.0)
}

/// Like [`load_config`], but also returns the directory of the `.aplan.yml` that
/// was loaded (`None` when none was found). A *relative* path in the config, such
/// as `traces_root`, must be resolved against that directory, not the process's
/// current directory, or the verdict would depend on where the command was run
/// from, exactly the location dependence walking-up discovery exists to remove.
pub fn load_config_located(start: &Path) -> Result<(Config, Option<PathBuf>), Error> {
    let Some(path) = find_config(start) else {
        return Ok((Config::default(), None));
    };
    let text = std::fs::read_to_string(&path)
        .map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
    let config: Config = serde_yaml_ng::from_str(&text)
        .map_err(|e| Error::Parse(format!("{}: {e}", path.display())))?;
    if let Some(runner) = &config.test {
        runner.validate()?;
    }
    let dir = path.parent().map(Path::to_path_buf);
    Ok((config, dir))
}

/// The nearest `.aplan.yml` at or above `start`, or `None` if there is none up to
/// the filesystem root.
fn find_config(start: &Path) -> Option<PathBuf> {
    // Absolute, so `ancestors()` reaches the real root rather than stopping at a
    // relative path with no parent (`.` has none).
    let start = std::fs::canonicalize(start).ok()?;
    start
        .ancestors()
        .map(|dir| dir.join(CONFIG_FILE))
        // Stop at the first `.aplan.yml` that *exists as anything*, not just a
        // regular file: a `.aplan.yml` that is a directory (or a broken symlink)
        // must become a clear read error, not be skipped so the tool silently
        // falls back to defaults, which is the config drift this tool preaches
        // against. `load_config` then reports the unreadable file (exit 2).
        .find(|candidate| candidate.symlink_metadata().is_ok())
}

/// The JSON Schema derived from [`Config`]. `cargo xtask gen-schema` writes this
/// to `schemas/v1/config.json`, and `--check` fails on drift.
pub fn derived_schema() -> serde_json::Value {
    let schema = schemars::schema_for!(Config);
    serde_json::to_value(schema).expect("schema is serialisable")
}

/// The starter `.aplan.yml` that `aplan init` writes: every setting shown at its
/// default, but commented out, so the file is a *valid no-op* until the reader
/// opts into a policy. That matches the tool's ethos, every rule beyond the core
/// is off until a repo turns it on, so `init` hands you a documented menu, not an
/// imposed policy. The runner and evidence examples are the exact shapes the
/// `aplan-config` tests prove parse, so uncommenting them yields a valid file.
pub const SCAFFOLD: &str = r"# .aplan.yml, this repository's `aplan` policy.
#
# Every setting is optional and this file may be empty: an absent key uses the
# built-in default, and a command-line flag overrides whatever is set here. Editor
# autocomplete and validation come from schemas/v1/config.json. Unknown keys are a
# reported error, never silently ignored.
#
# Uncomment and edit the settings you want; the values shown are the defaults.

# The directory holding the plan's task files (default: aplan), resolved against
# this file's directory. The --root flag overrides it.
# root: aplan

# Warn when an active task has no `effort_days` estimate (APLAN009).
# require_effort: true

# Resolve `traces:` anchors (APLAN008) against this directory, relative to the
# repo root. To turn the check off entirely instead, set `no_traces: true`.
# traces_root: .
# no_traces: false

# Minimum evidence strength (APLAN017). A `done` task's evidence must include at
# least one kind listed here, so a bare `note` no longer closes it the way a
# `commit` or a `test` does. Tasks carrying an exempt tag are spared, for the work
# where a dated note genuinely is the best evidence there is (a phone call).
# evidence:
#   require_min_strength: [commit, test, file]
#   exempt_tags: [external]

# Stale-`done` drift detection (APLAN018). When on, `aplan close` stamps a content
# fingerprint of each traced section, and `check` warns (advisory) when a closed
# task's traced section has changed since. Clear a warning by re-reviewing and
# `aplan close --restamp <id>`.
# detect_drift: false

# The test runner for `aplan verify --execute` (rung L3). `list` prints the
# available test names, one per line; `run` runs one, with `{test}` (a whole
# argument, never spliced into another, spawned with no shell) replaced by the
# name. This is the only place a command may be named (HLD 5.5).
# test:
#   list: [cargo, test, --, --list]
#   run: [cargo, test, --exact, '{test}']
";

/// Why `aplan init` could not scaffold a config.
#[derive(Debug, thiserror::Error)]
pub enum InitError {
    #[error(".aplan.yml already exists at {0}; refusing to overwrite")]
    Exists(PathBuf),
    #[error("could not write {0}: {1}")]
    Io(PathBuf, String),
}

/// Write [`SCAFFOLD`] to `dir/.aplan.yml`, returning the path written, or refuse
/// if anything is already at that path. The refusal is the whole safety property:
/// `init` never clobbers a policy a repo has already committed to. It writes only
/// on the success path, so a refused `init` changes nothing on disk.
pub fn init(dir: &Path) -> Result<PathBuf, InitError> {
    use std::io::Write as _;
    let path = dir.join(CONFIG_FILE);
    // `create_new` (O_EXCL) is atomic and does not follow a final symlink, so it
    // closes the check-then-write race a `symlink_metadata` + `write` would leave
    // open: nothing at that path is ever overwritten, whatever it is (a file, a
    // directory, or a symlink a concurrent writer just planted).
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut f) => {
            f.write_all(SCAFFOLD.as_bytes())
                .map_err(|e| InitError::Io(path.clone(), e.to_string()))?;
            Ok(path)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(InitError::Exists(path)),
        Err(e) => Err(InitError::Io(path, e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runner(run: &[&str]) -> TestRunner {
        TestRunner {
            list: vec!["true".into()],
            run: run.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    #[test]
    fn a_hostile_test_name_stays_one_unmodified_argv_element() {
        let argv = runner(&["cargo", "test", "--exact", PLACEHOLDER]).run_argv("; rm -rf / #");
        assert_eq!(argv, vec!["cargo", "test", "--exact", "; rm -rf / #"]);
        let argv = runner(&["prog", PLACEHOLDER]).run_argv("--help");
        assert_eq!(argv, vec!["prog", "--help"]);
    }

    #[test]
    fn a_substring_placeholder_runner_is_rejected() {
        let bad = runner(&["sh", "-c", &format!("run {PLACEHOLDER}")]);
        assert!(bad.validate().is_err());
    }

    #[test]
    fn run_needs_exactly_one_placeholder() {
        assert!(runner(&["true"]).validate().is_err());
        assert!(runner(&[PLACEHOLDER, PLACEHOLDER]).validate().is_err());
        assert!(runner(&["true", PLACEHOLDER]).validate().is_ok());
    }

    /// The placeholder must not be the program: `run: ['{test}']` would execute
    /// the untrusted test name as a command (the trust-boundary breach).
    #[test]
    fn run_rejects_the_placeholder_as_the_program() {
        assert!(
            runner(&[PLACEHOLDER]).validate().is_err(),
            "a bare `{{test}}` run makes the test name the program"
        );
        assert!(
            runner(&[PLACEHOLDER, "--flag"]).validate().is_err(),
            "the placeholder as argv[0] must be rejected regardless of later args"
        );
        // The safe shape (fixed program, name as an argument) still validates.
        assert!(
            runner(&["cargo", "test", "--exact", PLACEHOLDER])
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn a_full_config_round_trips() {
        let yml = "\
require_effort: false
traces_root: docs
no_traces: false
test:
  list: [cargo, test, --, --list]
  run: [cargo, test, --exact, '{test}']
";
        let cfg: Config = serde_yaml_ng::from_str(yml).unwrap();
        assert_eq!(cfg.require_effort, Some(false));
        assert_eq!(cfg.traces_root.as_deref(), Some("docs"));
        let runner = cfg.test.expect("runner present");
        runner.validate().unwrap();
        assert_eq!(runner.run_argv("x"), vec!["cargo", "test", "--exact", "x"]);
    }

    #[test]
    fn an_empty_config_is_all_defaults() {
        let cfg: Config = serde_yaml_ng::from_str("{}").unwrap();
        assert_eq!(cfg.require_effort, None);
        assert!(cfg.traces_root.is_none());
        assert!(cfg.test.is_none());
        assert_eq!(cfg.detect_drift, None);
    }

    #[test]
    fn detect_drift_parses() {
        let cfg: Config = serde_yaml_ng::from_str("detect_drift: true\n").unwrap();
        assert_eq!(cfg.detect_drift, Some(true));
    }

    #[test]
    fn root_key_parses() {
        let cfg: Config = serde_yaml_ng::from_str("root: myplan\n").unwrap();
        assert_eq!(cfg.root.as_deref(), Some("myplan"));
    }

    #[test]
    fn the_evidence_strength_block_parses_typed_kinds() {
        let yml =
            "evidence:\n  require_min_strength: [commit, test, file]\n  exempt_tags: [phone]\n";
        let cfg: Config = serde_yaml_ng::from_str(yml).unwrap();
        let ev = cfg.evidence.expect("evidence block");
        assert_eq!(
            ev.require_min_strength.unwrap(),
            vec![EvidenceKind::Commit, EvidenceKind::Test, EvidenceKind::File]
        );
        assert_eq!(ev.exempt_tags.unwrap(), vec!["phone".to_string()]);
        // An unknown evidence kind is a typed parse error, not silently accepted.
        assert!(
            serde_yaml_ng::from_str::<Config>("evidence:\n  require_min_strength: [bogus]\n")
                .is_err()
        );
    }

    #[test]
    fn an_unknown_key_is_rejected_not_ignored() {
        // The ethos: a config key the tool does not understand is drift.
        let err = serde_yaml_ng::from_str::<Config>("polciy: strict\n").unwrap_err();
        assert!(err.to_string().contains("polciy") || err.to_string().contains("unknown"));
    }

    #[test]
    fn the_derived_schema_forbids_unknown_properties() {
        let schema = derived_schema();
        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
    }

    fn write(dir: &Path, contents: &str) {
        std::fs::write(dir.join(CONFIG_FILE), contents).unwrap();
    }

    #[test]
    fn a_repo_with_no_config_loads_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = load_config(dir.path()).unwrap();
        assert!(cfg.require_effort.is_none() && cfg.test.is_none());
    }

    #[test]
    fn an_empty_or_comment_only_file_is_the_default_not_an_error() {
        for body in ["", "   \n\n", "# just a comment\n"] {
            let dir = tempfile::tempdir().unwrap();
            write(dir.path(), body);
            let cfg = load_config(dir.path()).unwrap_or_else(|e| panic!("{body:?}: {e}"));
            assert!(cfg.require_effort.is_none(), "{body:?} should be default");
        }
    }

    /// A `.aplan.yml` that is a directory must be a reported error, not silently
    /// skipped so the tool falls back to defaults (config drift, day one).
    #[test]
    fn a_directory_named_aplan_yml_is_an_error_not_a_silent_default() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(CONFIG_FILE)).unwrap();
        assert!(
            matches!(load_config(dir.path()), Err(Error::Io(_))),
            "a directory `.aplan.yml` must error, not default"
        );
    }

    #[test]
    fn a_config_is_read_from_the_start_directory() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "require_effort: false\n");
        assert_eq!(load_config(dir.path()).unwrap().require_effort, Some(false));
    }

    #[test]
    fn a_config_is_discovered_by_walking_up_from_a_subdirectory() {
        // The file is at the repo root; loading from a nested path still finds it.
        let repo = tempfile::tempdir().unwrap();
        write(repo.path(), "require_effort: false\n");
        let nested = repo.path().join("plan").join("tasks");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(load_config(&nested).unwrap().require_effort, Some(false));
    }

    #[test]
    fn malformed_yaml_and_unknown_keys_and_bad_runners_are_reported() {
        let dir = tempfile::tempdir().unwrap();

        write(dir.path(), "require_effort: [not a bool\n");
        assert!(matches!(load_config(dir.path()), Err(Error::Parse(_))));

        write(dir.path(), "polciy: strict\n");
        assert!(matches!(load_config(dir.path()), Err(Error::Parse(_))));

        write(dir.path(), "test:\n  list: [t]\n  run: [t]\n"); // no placeholder
        assert!(matches!(load_config(dir.path()), Err(Error::BadRunner(_))));
    }

    #[test]
    fn init_writes_a_scaffold_that_loads_as_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = init(dir.path()).unwrap();
        assert_eq!(path, dir.path().join(CONFIG_FILE));
        // The file it wrote is valid and a no-op: `load_config` reads it back as the
        // built-in default, so `init` imposes nothing until the user opts in.
        let cfg = load_config(dir.path()).unwrap();
        assert!(cfg.require_effort.is_none() && cfg.test.is_none() && cfg.evidence.is_none());
    }

    #[test]
    fn init_refuses_to_overwrite_an_existing_config() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "require_effort: false\n");
        assert!(matches!(init(dir.path()), Err(InitError::Exists(_))));
        // The existing policy is untouched: the refusal wrote nothing.
        assert_eq!(load_config(dir.path()).unwrap().require_effort, Some(false));
    }

    #[test]
    fn the_scaffold_examples_are_the_shapes_the_tests_prove_valid() {
        // The commented runner and evidence examples are byte-identical to the
        // shapes `a_full_config_round_trips` and `the_evidence_strength_block_...`
        // prove parse and validate, so uncommenting them yields a valid file. If
        // an example is edited, this fails until the proving test is updated too.
        assert!(SCAFFOLD.contains("run: [cargo, test, --exact, '{test}']"));
        assert!(SCAFFOLD.contains("list: [cargo, test, --, --list]"));
        assert!(SCAFFOLD.contains("require_min_strength: [commit, test, file]"));
    }
}
