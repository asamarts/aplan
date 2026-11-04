//! The per-user file, which is about the *actor* and never about the plan
//! (`A1-364`).
//!
//! `.aplan.yml` is the repository's policy: shared, committed, and binding on
//! everybody who works the checkout. This file is the other half, and the line
//! between them is the whole design. A repository must be able to require
//! things of its plan; a person must be able to say who they are once, for
//! every repository on the machine. Those are different questions and they
//! live in different files.
//!
//! **Why the two cannot be one type.** `Config` has 76 fields, among them
//! `deny_warnings`, `advisory`, `require_min_strength` and `detect_drift`. A
//! user-level file deserialised into that type would let anybody turn a
//! repository's gate down on their own machine and still pass it -- silently,
//! and exactly where nobody reviews. So precedence here is not a rule somebody
//! has to remember: the type simply has no field a rule reads, and a census in
//! this module keeps it that way as the file grows.
//!
//! **Why not a dotenv.** `APLAN_OWNER` already answers "set it for this shell",
//! so a `.env` would be a second way to say one thing. It is also in the wrong
//! place twice: committed, it is shared between actors, which is what a
//! repository key was rejected for; ignored, it is per-machine configuration in
//! a directory that is not per-machine. Answered here rather than left as a
//! separate half-remembered idea (`A1-364`).
//!
//! **Why a file at all, when an environment variable exists.** The actor this
//! serves is the one `A1-359` was built for: an agent reaching aplan through
//! its MCP server, started by a desktop client that sources no shell profile.
//! It has no `APLAN_OWNER` to inherit, and the next source down is
//! `git config user.name`, which on the machine this was measured on says `t` --
//! a placeholder left by a test setup. A guess that is wrong and silent is the
//! thing this file removes.

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::Deserialize;

/// The file's name inside its directory.
pub const USER_CONFIG_FILE: &str = "config.yml";

/// The directory under the user's configuration root.
pub const USER_CONFIG_DIR: &str = "aplan";

/// The parsed per-user file. Every field is optional, and every field is about
/// the actor or their terminal.
///
/// Unknown keys are rejected rather than ignored, for the reason `.aplan.yml`
/// rejects them: a key that is quietly dropped is a setting somebody believes
/// is in effect.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UserConfig {
    /// Who you are, for `claim`, `release` and the holder `ready` reports.
    ///
    /// Read between `APLAN_OWNER` and `git config user.name`: an environment
    /// variable is something the caller set for this run and wins over a file,
    /// and git's name is a guess this exists to replace.
    pub me: Option<String>,
}

/// Why a per-user key is not policy.
///
/// **The gate for growing this file.** The decision that shaped it was
/// "identity, designed to grow", and the risk a growing per-user file carries
/// is that one of its keys turns out to be something a repository should have
/// decided. So each field says which side of the line it is on, and the census
/// below refuses a field that says nothing -- the same shape every other
/// classification in this workspace has.
///
/// The test a new row has to pass: *could a repository reasonably want to
/// require this of everybody who works its plan?* If yes, it belongs in
/// `.aplan.yml` and not here.
/// Compiled only for the test that reads it. It is documentation a reviewer
/// adding a field sees in the source either way, and carrying it into release
/// binaries would be a string table nothing prints.
#[cfg(test)]
const ABOUT_THE_ACTOR: &[(&str, &str)] = &[(
    "me",
    "a name for this person or agent. A repository cannot require it of \
     everybody, because several actors share one checkout -- which is the \
     reason this file exists at all",
)];

/// A failure reading the per-user file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The file is there and could not be read.
    #[error("could not read `{path}`: {source}")]
    Read {
        /// Which file.
        path: PathBuf,
        /// Why not.
        source: std::io::Error,
    },
    /// The file is there and is not what it claims to be.
    ///
    /// The clause about policy is appended only where it explains something.
    /// Serde reports an unrecognised key as `unknown field`, and *that* is the
    /// reader who has put a `.aplan.yml` key here and needs to know why it is
    /// refused. On a type error -- `me:` given a list -- the same sentence is a
    /// non sequitur, which is the mistake `APLAN053`'s remedy made one audit
    /// earlier (`A1-364`).
    #[error("could not parse `{path}`: {source}{}", wrong_file_hint(source))]
    Parse {
        /// Which file.
        path: PathBuf,
        /// Why not.
        source: yaml_compat::Error,
    },
}

/// The sentence a reader who used the wrong file needs, and nothing otherwise.
fn wrong_file_hint(source: &yaml_compat::Error) -> &'static str {
    if source.to_string().contains("unknown field") {
        ". This file holds per-user settings and never a repository's policy, so the keys \
         `.aplan.yml` takes are not accepted here"
    } else {
        ""
    }
}

/// Where the per-user file lives, whether or not it is there.
///
/// `$XDG_CONFIG_HOME/aplan/config.yml`, falling back to
/// `$HOME/.config/aplan/config.yml`, which is what the XDG base-directory
/// specification says to do and what every other tool a reader has on the
/// machine already does. `None` where neither variable gives an absolute
/// directory, which is a process with no user to speak of rather than a
/// misconfiguration.
///
/// **Both variables are required to be absolute, and that is a trust boundary
/// rather than tidiness** (`A1-364`, audit). `HOME=""` is set-but-empty, which
/// happens in containers, systemd units and some CI runners; joined, it yielded
/// the *relative* path `.config/aplan/config.yml`, so a "per-user" identity was
/// read from the working directory -- which is the repository. A repository
/// could ship that file and name whoever runs aplan there, and a
/// repository-supplied identity is precisely what this design rejected when it
/// refused to read one from `.aplan.yml`. The XDG specification says the same
/// thing for its own reasons: a relative path in these variables is invalid and
/// must be ignored.
#[must_use]
pub fn path() -> Option<PathBuf> {
    path_from(
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

/// [`path`], from given values, so the rule above is testable.
///
/// Split out because the test binary is shared and `set_var` reaches every
/// other test in the process -- the same reason `identity::resolve` is pure and
/// `identity::from_environment` is the thin half that reads the world.
#[must_use]
pub fn path_from(xdg: Option<&std::ffi::OsStr>, home: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let absolute = |p: PathBuf| p.is_absolute().then_some(p);
    let root = xdg.map(PathBuf::from).and_then(absolute).or_else(|| {
        home.map(PathBuf::from)
            .and_then(absolute)
            .map(|h| h.join(".config"))
    })?;
    Some(root.join(USER_CONFIG_DIR).join(USER_CONFIG_FILE))
}

/// The per-user settings, or the defaults where there is no file.
///
/// # Errors
///
/// When the file exists and cannot be read or parsed. **Not** when it is
/// absent: no file is the ordinary case and means "nothing to say", where an
/// unreadable one is a setting somebody believes is in effect.
pub fn load() -> Result<UserConfig, Error> {
    match path() {
        Some(p) => load_from(&p),
        None => Ok(UserConfig::default()),
    }
}

/// [`load`], from a named path, so a test needs no environment.
///
/// # Errors
///
/// As [`load`].
pub fn load_from(path: &Path) -> Result<UserConfig, Error> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(UserConfig::default());
        }
        Err(source) => {
            return Err(Error::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    yaml_compat::from_str(&text).map_err(|source| Error::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// The JSON Schema derived from [`UserConfig`]. `cargo xtask gen-schema` writes
/// this to `schemas/v1/user-config.json`, and `--check` fails on drift.
#[must_use]
pub fn derived_schema() -> serde_json::Value {
    let schema = schemars::schema_for!(UserConfig);
    serde_json::to_value(schema).expect("schema is serialisable")
}

/// Every key this file accepts.
#[must_use]
pub fn accepted_keys() -> Vec<String> {
    let schema = derived_schema();
    let mut out: Vec<String> = schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .map(|props| props.keys().cloned().collect())
        .unwrap_or_default();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Every per-user key says why it is not policy, and names nothing
    /// `.aplan.yml` takes** (`A1-364`).
    ///
    /// The gate for growing this file. The second half is the one that would
    /// catch a real mistake: a key that exists in both files is a setting two
    /// places disagree about, and the per-user copy would be the one nobody
    /// reviews. The first half is the one that makes somebody think, which is
    /// the only thing that catches a key that *should* have been policy.
    ///
    /// Proof of `identity.reads_a_per_user_file`: the half a consumer pinning
    /// that name most needs, which is that the file holds nothing a rule reads
    /// and so cannot weaken what a repository requires.
    #[test]
    fn every_field_is_about_the_actor_and_not_the_plan() {
        let classified: std::collections::BTreeSet<&str> =
            ABOUT_THE_ACTOR.iter().map(|(k, _)| *k).collect();
        let keys = accepted_keys();
        let unclassified: Vec<&String> = keys
            .iter()
            .filter(|k| !classified.contains(k.as_str()))
            .collect();
        assert!(
            unclassified.is_empty(),
            "per-user key(s) with no reason they are not policy: {unclassified:?}. Say why a \
             repository cannot require this of everybody, or move the key to `.aplan.yml`"
        );
        for (key, why) in ABOUT_THE_ACTOR {
            assert!(
                keys.iter().any(|k| k == key),
                "`{key}` is classified and is not a key this file takes"
            );
            assert!(!why.trim().is_empty(), "`{key}` has an empty reason");
        }

        // The structural half: nothing here may name a key `.aplan.yml` takes.
        let repo: std::collections::BTreeSet<String> = crate::accepted_keys().into_iter().collect();
        for key in &keys {
            assert!(
                !repo.contains(key),
                "`{key}` is a key `.aplan.yml` takes as well, so two files would disagree \
                 about it and the per-user one is the copy nobody reviews"
            );
        }
    }

    /// **An absent file is not an error; an unreadable one is** (`A1-364`).
    ///
    /// No file is the ordinary case on every machine that has not opted in, and
    /// a tool that failed there would make a per-user convenience a
    /// prerequisite. A file that is *there* and will not parse is the opposite:
    /// somebody wrote it and believes it is in effect.
    #[test]
    fn an_absent_file_is_the_ordinary_case_and_a_broken_one_is_not() {
        let dir = tempfile::Builder::new().tempdir().expect("tempdir");
        let missing = dir.path().join("nothing.yml");
        assert!(
            load_from(&missing).expect("absent is fine").me.is_none(),
            "an absent file should read as nothing to say"
        );

        let bad = dir.path().join("bad.yml");
        std::fs::write(&bad, "me: [not, a, name\n").expect("write");
        let e = load_from(&bad).expect_err("a file that will not parse is an error");
        assert!(e.to_string().contains("could not parse"), "{e}");
        // A type error gets no lecture about policy keys: nothing here is one.
        assert!(
            !e.to_string().contains("never a repository's policy"),
            "the wrong-file sentence was appended to an error it does not explain: {e}"
        );

        // A key `.aplan.yml` takes is refused here, with the reason.
        let policy = dir.path().join("policy.yml");
        std::fs::write(&policy, "deny_warnings: false\n").expect("write");
        let e = load_from(&policy).expect_err("a repository key is not accepted here");
        assert!(
            e.to_string().contains("never a repository's policy"),
            "the refusal does not say why: {e}"
        );

        let good = dir.path().join("good.yml");
        std::fs::write(&good, "me: ada\n").expect("write");
        assert_eq!(load_from(&good).expect("loads").me.as_deref(), Some("ada"));
    }

    /// **A relative or empty variable yields no path at all** (`A1-364`,
    /// audit).
    ///
    /// The defect this pins, and it is a trust boundary rather than tidiness.
    /// `HOME=""` is set-but-empty, which happens in containers, systemd units
    /// and some CI runners. Joined, it produced the *relative* path
    /// `.config/aplan/config.yml`, so the "per-user" identity was read from the
    /// working directory -- which is the repository. Measured before the fix: a
    /// repository shipping that file named whoever ran `aplan claim` in it.
    ///
    /// A repository-supplied identity is exactly what this design rejected
    /// when it refused to read a name from `.aplan.yml`, so reaching the same
    /// outcome through an empty variable is the same defect by another route.
    /// The XDG specification independently requires it: a relative path in
    /// these variables is invalid and must be ignored.
    ///
    /// Proof of `identity.reads_a_per_user_file`: its boundary. The promise is
    /// about a file outside the repository, so a consumer pinning the name is
    /// entitled to know the repository cannot supply one.
    #[test]
    fn a_relative_or_empty_variable_names_no_file_rather_than_one_in_the_repository() {
        use std::ffi::OsStr;
        for (xdg, home) in [
            (Some(""), Some("")),
            (Some(".config"), Some("")),
            (Some(".config"), None),
            (None, Some("")),
            (Some("relative/dir"), Some("also/relative")),
            (None, None),
        ] {
            let got = path_from(xdg.map(OsStr::new), home.map(OsStr::new));
            assert!(
                got.is_none(),
                "xdg={xdg:?} home={home:?} resolved to {got:?}, which a repository could \
                 supply"
            );
        }

        // And anything it does return is absolute, which is the invariant
        // rather than the enumeration above. Derive native absolute paths:
        // `/home/u` is absolute on Unix but drive-relative on Windows.
        let absolute_root = std::env::current_dir().expect("absolute working directory");
        let xdg_root = absolute_root.join("xdg");
        let home_root = absolute_root.join("home");
        for (xdg, home) in [
            (Some(xdg_root.as_os_str()), Some(home_root.as_os_str())),
            (None, Some(home_root.as_os_str())),
            (Some(OsStr::new("")), Some(home_root.as_os_str())),
            (Some(OsStr::new("relative")), Some(home_root.as_os_str())),
        ] {
            let got = path_from(xdg, home).expect("an absolute source was given");
            assert!(got.is_absolute(), "{got:?} is not absolute");
            assert!(
                got.ends_with(Path::new(USER_CONFIG_DIR).join(USER_CONFIG_FILE)),
                "{got:?}"
            );
        }

        // `XDG_CONFIG_HOME` wins when it is usable, and is skipped when not.
        assert_eq!(
            path_from(Some(xdg_root.as_os_str()), Some(home_root.as_os_str())),
            Some(xdg_root.join(USER_CONFIG_DIR).join(USER_CONFIG_FILE))
        );
        assert_eq!(
            path_from(Some(OsStr::new("relative")), Some(home_root.as_os_str())),
            Some(
                home_root
                    .join(".config")
                    .join(USER_CONFIG_DIR)
                    .join(USER_CONFIG_FILE)
            ),
            "an unusable XDG value should fall through, not refuse"
        );
    }

    /// **The path follows the XDG specification, and `HOME` is the fallback**
    /// (`A1-364`).
    ///
    /// Asserted on the shape rather than by setting variables, because the test
    /// binary is shared and `set_var` reaches every other test in the process.
    #[test]
    fn the_path_is_under_the_users_configuration_root() {
        if let Some(p) = path() {
            assert!(p.is_absolute(), "{p:?} is not absolute");
            assert!(
                p.ends_with(PathBuf::from(USER_CONFIG_DIR).join(USER_CONFIG_FILE)),
                "{p:?} does not end in `aplan/config.yml`"
            );
            let expected_root = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .filter(|r| r.is_absolute());
            if let Some(root) = expected_root {
                assert!(p.starts_with(&root), "{p:?} is not under {root:?}");
            }
        }
    }
}
