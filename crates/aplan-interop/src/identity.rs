//! Who is acting, resolved from the environment rather than from the plan
//! (`A1-359`).
//!
//! A claim has to be attributed, and the name is a *label* rather than a
//! security boundary: the exclusion that makes `claim` safe comes from
//! `aplan_core::save::replace` being a compare-and-swap, not from who the
//! caller says they are. So this resolves a name helpfully and refuses only
//! when it has nothing at all.
//!
//! **Never from `.aplan.yml`.** Several actors share one repository -- that is
//! the whole case `claim` exists for -- so a repository-level owner would name
//! the wrong one of them by construction. Git's own `user.name` already solves
//! the multi-actor case the right way round, with a per-repository value
//! overriding a global one, so that is the derived source this uses.
//!
//! Measured on one machine the three derived sources disagree: `git config
//! user.name` says `t`, the GitHub account is `asamarts`, and the OS user is
//! `kaminsod`. That disagreement is why the explicit forms come first and why
//! the refusal names every source rather than just failing.

use std::path::Path;

/// The name each source offers, in precedence order, before any of them is
/// chosen.
///
/// Collected separately from being resolved so the precedence is testable
/// without an environment: [`resolve`] is a pure function of this, and
/// [`from_environment`] is the part that reads the world.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Sources {
    /// `--as <name>`, or the MCP tool's `as` argument.
    pub flag: Option<String>,
    /// `APLAN_OWNER`.
    pub env: Option<String>,
    /// `me` from the per-user file (`A1-364`).
    ///
    /// Between the environment and git deliberately: a variable is something
    /// the caller set for *this run* and should win over a file, and git's name
    /// is the guess this file exists to replace. Measured on one machine,
    /// `git config user.name` said `t`, left by a test setup, where the plan's
    /// own convention is `kaminsod` on 226 tasks.
    pub user_file: Option<String>,
    /// `git config user.name`, which already prefers a per-repository value.
    pub git: Option<String>,
    /// The operating system's idea of the user.
    pub os: Option<String>,
}

/// Why no name could be used.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IdentityError {
    /// Nothing offered a name.
    #[error(
        "cannot tell who you are, so this claim would be unattributable. Name yourself with \
         `--as <name>`, or set `APLAN_OWNER`, or write `me: <name>` in \
         `~/.config/aplan/config.yml` to say it once for every repository on this machine, or \
         set `git config user.name` (the operating system's user is the last fallback). Not \
         read from `.aplan.yml`: several actors share one repository, so a name recorded there \
         would be the wrong one"
    )]
    Unknown,
    /// A name was offered that cannot be written or read back safely.
    #[error(
        "`{shown}` cannot be used as a name: {why}. A name is written into the local claim \
         record and printed back to whoever reads the plan"
    )]
    Unusable {
        /// The name, with anything non-printing made visible.
        shown: String,
        /// What is wrong with it.
        why: &'static str,
    },
}

/// Which deterministic source supplied a holder label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentitySource {
    Argument,
    Environment,
    UserConfig,
    Git,
    OperatingSystem,
}

impl IdentitySource {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Argument => "argument",
            Self::Environment => "environment",
            Self::UserConfig => "user_config",
            Self::Git => "git",
            Self::OperatingSystem => "operating_system",
        }
    }
}

/// A resolved identity label with the source that won precedence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedIdentity {
    pub name: String,
    pub source: IdentitySource,
}

/// The name to attribute this action to.
///
/// Pure, and the precedence is the whole of it: `--as`, then `APLAN_OWNER`,
/// then `git config user.name`, then the operating system. Explicit before
/// derived, because the derived sources are the ones that disagree.
///
/// # Errors
///
/// [`IdentityError::Unknown`] when no source offered a usable name, naming all
/// four so the reader can supply one. [`IdentityError::Unusable`] when a source
/// offered something that cannot be a name -- which is only ever reported for a
/// name the *caller* supplied, because a derived source that answers
/// unusably is treated as not having answered.
pub fn resolve(sources: &Sources) -> Result<String, IdentityError> {
    resolve_detailed(sources).map(|resolved| resolved.name)
}

/// Resolve a holder label and retain which source supplied it.
///
/// The provenance is recorded with an ephemeral claim so a reader can tell an
/// explicit actor label from a Git or operating-system fallback. It is still
/// attribution only, never authentication.
pub fn resolve_detailed(sources: &Sources) -> Result<ResolvedIdentity, IdentityError> {
    // **A supplied blank is refused, not skipped** (`A1-359`, audit). The
    // caller almost always wrote `--as "$SOMEVAR"` against an unset variable,
    // and falling through would attribute their claim to whatever this machine
    // says, silently. That is the same stance `refuse_blank_tags` already takes
    // on `--tag ""`, and the same asymmetry as below: what a caller supplies is
    // their mistake to hear about, what a source derives is not.
    if let Some(raw) = sources.flag.as_deref() {
        let Some(name) = usable(Some(raw)) else {
            return Err(IdentityError::Unusable {
                shown: aplan_output::sanitize::visible(raw).into_owned(),
                why: "it is blank, so it names nobody",
            });
        };
        return check(name).map(|name| ResolvedIdentity {
            name,
            source: IdentitySource::Argument,
        });
    }
    if let Some(name) = usable(sources.env.as_deref()) {
        return check(name).map(|name| ResolvedIdentity {
            name,
            source: IdentitySource::Environment,
        });
    }
    // The per-user file is explicit too -- somebody wrote it -- so an unusable
    // value in it is refused rather than skipped, for the reason `--as` is.
    if let Some(raw) = sources.user_file.as_deref() {
        let Some(name) = usable(Some(raw)) else {
            return Err(IdentityError::Unusable {
                shown: aplan_output::sanitize::visible(raw).into_owned(),
                why: "it is blank in the per-user file, so it names nobody",
            });
        };
        return check(name).map(|name| ResolvedIdentity {
            name,
            source: IdentitySource::UserConfig,
        });
    }
    // Derived sources are *skipped* when unusable rather than refused. A
    // `user.name` with a stray control character is not this caller's mistake
    // and not something they can fix from here, so falling through to the next
    // source is more useful than refusing; `--as` is always available to say
    // something else.
    for (source, derived) in [
        (IdentitySource::Git, sources.git.as_deref()),
        (IdentitySource::OperatingSystem, sources.os.as_deref()),
    ] {
        if let Some(name) = usable(derived)
            && let Ok(name) = check(name)
        {
            return Ok(ResolvedIdentity { name, source });
        }
    }
    Err(IdentityError::Unknown)
}

/// Actionable warning for labels commonly left by test or machine setup.
#[must_use]
pub fn placeholder_warning(identity: &ResolvedIdentity) -> Option<String> {
    let folded = identity.name.trim().to_ascii_lowercase();
    let placeholder = matches!(
        folded.as_str(),
        "t" | "test" | "user" | "username" | "your name" | "unknown" | "root" | "runner"
    );
    placeholder.then(|| {
        format!(
            "identity label `{}` from {} looks like a machine or test placeholder. Use `--as \
             <name>`, set `APLAN_OWNER`, or write `me: <name>` in \
             `~/.config/aplan/config.yml` before other actors rely on it",
            aplan_output::sanitize::visible(&identity.name),
            identity.source.as_str()
        )
    })
}

/// The name a source offers, trimmed, or `None` when it offers nothing.
///
/// Blank is absent: an exported `APLAN_OWNER=` and an unset one mean the same
/// thing to whoever set it, and `git config user.name` on a fresh machine
/// prints an empty line rather than failing.
fn usable(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim)
        .filter(|name| !aplan_core::task::is_vacuous_ref(name))
}

/// A name that can be written into a local claim record and printed back.
fn check(name: &str) -> Result<String, IdentityError> {
    let unusable = |why| {
        Err(IdentityError::Unusable {
            shown: aplan_output::sanitize::visible(name).into_owned(),
            why,
        })
    };
    if name.chars().any(char::is_control) {
        // The plan already carries one of these, written by `import` from a
        // terminal capture (`A1-290`). Arriving by import is a thing to
        // sanitize on the way out; arriving by `--as` is a thing to refuse on
        // the way in.
        return unusable("it holds a control character");
    }
    if name.chars().count() > MAX_NAME {
        return unusable("it is longer than a name ever is");
    }
    Ok(name.to_string())
}

/// How long a name may be.
///
/// Not a storage limit -- the field is a `String` -- but a line in `ready`'s
/// output is a column, and the honest reason is that anything longer is a
/// mistake rather than a name.
const MAX_NAME: usize = 64;

/// What each source says, here and now.
///
/// The impure half, kept to one function so every caller resolves the same way
/// and the precedence lives in [`resolve`] where it can be tested.
#[must_use]
pub fn from_environment(flag: Option<&str>, root: &Path) -> Sources {
    Sources {
        flag: flag.map(str::to_string),
        env: std::env::var("APLAN_OWNER").ok(),
        // A file that will not parse is reported rather than skipped: somebody
        // wrote it and believes it is in effect. An absent one is `None`, which
        // is the ordinary case on a machine that has not opted in.
        user_file: match aplan_config::user::load() {
            Ok(user) => user.me,
            // Not fatal here, and not silent either: `resolve` has three more
            // sources and the caller is trying to record a claim, not to fix
            // their configuration. Said on stderr so the reason is visible when
            // the name that follows is not the one they expected.
            Err(e) => {
                eprintln!("note: {e}");
                None
            }
        },
        git: git_user_name(root),
        // The environment's answer rather than a lookup through the password
        // database, which would be a dependency for the least load-bearing of
        // the four: by the time this is reached, three better sources have
        // already declined. `USERNAME` is where Windows puts it.
        os: ["USER", "LOGNAME", "USERNAME"]
            .iter()
            .find_map(|key| std::env::var(key).ok()),
    }
}

/// `git config user.name`, or `None` where git cannot answer.
///
/// Run in `root` so a per-repository value wins, which is git's own precedence
/// and the reason this is the derived source rather than a GitHub account: a
/// contributor who sets `user.name` for one checkout has already said who they
/// are there.
fn git_user_name(root: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["config", "user.name"])
        .current_dir(root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let name = String::from_utf8(out.stdout).ok()?;
    usable(Some(&name)).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources(
        flag: Option<&str>,
        env: Option<&str>,
        git: Option<&str>,
        os: Option<&str>,
    ) -> Sources {
        Sources {
            flag: flag.map(str::to_string),
            env: env.map(str::to_string),
            user_file: None,
            git: git.map(str::to_string),
            os: os.map(str::to_string),
        }
    }

    /// The same, with the per-user file's `me` (`A1-364`).
    fn with_user(user: Option<&str>, env: Option<&str>, git: Option<&str>) -> Sources {
        Sources {
            user_file: user.map(str::to_string),
            ..sources(None, env, git, None)
        }
    }

    /// **The four sources, in order, each one winning over the next**
    /// (`A1-359`).
    ///
    /// Written as the whole ladder rather than four separate tests, because
    /// what is being asserted is the *order*: a test per source passes just as
    /// happily when two of them are swapped.
    #[test]
    fn the_explicit_name_wins_and_the_os_is_the_last_resort() {
        let all = sources(Some("flag"), Some("env"), Some("git"), Some("os"));
        assert_eq!(resolve(&all).as_deref(), Ok("flag"));
        assert_eq!(
            resolve(&Sources {
                flag: None,
                ..all.clone()
            })
            .as_deref(),
            Ok("env")
        );
        assert_eq!(
            resolve(&Sources {
                flag: None,
                env: None,
                ..all.clone()
            })
            .as_deref(),
            Ok("git")
        );
        assert_eq!(
            resolve(&sources(None, None, None, Some("os"))).as_deref(),
            Ok("os")
        );
    }

    /// **The per-user file sits between the environment and git** (`A1-364`).
    ///
    /// Both halves of that placement matter. A variable is something the caller
    /// exported for this run, so it wins over a file somebody wrote once. And
    /// git's `user.name` is the guess the file exists to replace: on the
    /// machine this was measured on it said `t`, left by a test setup, where
    /// the plan's own convention is `kaminsod`.
    ///
    /// Proof of `identity.reads_a_per_user_file`: that the file is consulted at
    /// all, and where in the chain.
    #[test]
    fn the_user_file_loses_to_the_environment_and_beats_git() {
        assert_eq!(
            resolve(&with_user(
                Some("from-file"),
                Some("from-env"),
                Some("from-git")
            ))
            .as_deref(),
            Ok("from-env"),
            "a variable set for this run should win over a file"
        );
        assert_eq!(
            resolve(&with_user(Some("from-file"), None, Some("from-git"))).as_deref(),
            Ok("from-file"),
            "the file exists precisely to replace git's guess"
        );
        assert_eq!(
            resolve(&Sources {
                flag: Some("explicit".into()),
                ..with_user(Some("from-file"), Some("from-env"), Some("from-git"))
            })
            .as_deref(),
            Ok("explicit"),
            "`--as` still beats both"
        );
    }

    /// **A blank `me` in the file is refused, like a blank `--as`** (`A1-364`).
    ///
    /// Somebody wrote this file and believes it is in effect, so an empty value
    /// in it is a mistake to report rather than a source to skip past -- the
    /// same line the audit of `A1-359` drew between what a caller supplies and
    /// what a source derives.
    #[test]
    fn a_blank_me_in_the_file_is_a_mistake_rather_than_an_absence() {
        let e = resolve(&with_user(Some("  "), None, Some("from-git")))
            .expect_err("a blank `me` names nobody");
        assert!(matches!(e, IdentityError::Unusable { .. }), "{e:?}");
        assert!(e.to_string().contains("per-user file"), "{e}");
    }

    /// **A refusal names every source**, so the reader can act on it
    /// (`A1-359`).
    ///
    /// The whole point of four sources is that one of them will work, so the
    /// case where none did is the case where the message has to carry the
    /// remedy. It also has to say what is *not* consulted: `.aplan.yml` is the
    /// obvious place to look for an owner and the one place this must never
    /// read, because several actors share one repository.
    #[test]
    fn a_refusal_names_all_four_sources_and_the_one_it_does_not_read() {
        let e = resolve(&Sources::default()).expect_err("nothing offered a name");
        let said = e.to_string();
        for needle in [
            "--as",
            "APLAN_OWNER",
            "~/.config/aplan/config.yml",
            "git config user.name",
            "operating system",
        ] {
            assert!(
                said.contains(needle),
                "the refusal omits `{needle}`: {said}"
            );
        }
        assert!(
            said.contains(".aplan.yml") && said.contains("several actors"),
            "the refusal does not say why the repository's config is not consulted: {said}"
        );
    }

    /// **Blank is absent, at every source** (`A1-359`).
    ///
    /// `git config user.name` on a machine that has never been configured
    /// prints an empty line and exits zero, and an exported `APLAN_OWNER=` is
    /// how a shell spells "unset" by accident. Treating either as a name
    /// records a claim held by nobody.
    ///
    /// Sources only. A blank the *caller* typed is a different question, and
    /// the test above answers it: refused rather than skipped.
    #[test]
    fn a_blank_name_is_not_a_name() {
        assert_eq!(
            resolve(&sources(None, Some("\t"), Some(""), Some("real"))).as_deref(),
            Ok("real"),
            "a blank source was taken as an answer"
        );
        assert!(
            resolve(&sources(None, None, Some("\n"), None)).is_err(),
            "nothing usable was offered, so this must refuse"
        );
        assert_eq!(
            resolve(&sources(None, Some("\u{200b}"), None, Some("real"))).as_deref(),
            Ok("real"),
            "a zero-width source was taken as an answer"
        );
        // And a name that is merely padded is used, trimmed.
        assert_eq!(
            resolve(&sources(Some("  ada  "), None, None, None)).as_deref(),
            Ok("ada")
        );
    }

    /// **A blank the caller typed is refused; a blank from a source is absent**
    /// (`A1-359`, audit).
    ///
    /// `--as "$SOMEVAR"` against an unset variable arrives here as `Some("")`,
    /// and skipping it attributed the claim to whatever the machine said --
    /// silently, and to the wrong person. `refuse_blank_tags` already takes
    /// this stance on `--tag ""` one crate over. An exported `APLAN_OWNER=` is
    /// the other case and still means unset: nobody typed it at this command.
    #[test]
    fn a_blank_the_caller_supplied_is_refused_and_a_blank_source_is_not() {
        let e = resolve(&sources(Some(""), None, None, Some("machine")))
            .expect_err("a supplied blank names nobody");
        assert!(
            matches!(e, IdentityError::Unusable { .. }),
            "refused as {e:?} rather than as an unusable name"
        );
        assert!(e.to_string().contains("blank"), "{e}");
        assert!(
            resolve(&sources(Some("   "), None, None, Some("machine"))).is_err(),
            "whitespace is blank too"
        );
        assert!(
            resolve(&sources(Some("\u{200b}"), None, None, Some("machine"))).is_err(),
            "zero-width text is blank too"
        );
        // The same emptiness, from a source rather than from the caller.
        assert_eq!(
            resolve(&sources(None, Some(""), Some(""), Some("machine"))).as_deref(),
            Ok("machine"),
            "an unset variable is not a mistake to refuse"
        );
    }

    /// **A supplied name that cannot be written is refused; a derived one is
    /// skipped** (`A1-359`).
    ///
    /// The asymmetry is deliberate. A control character in `--as` is this
    /// caller's input and refusing it is the only way they learn; the same
    /// character in `git config user.name` is not their mistake, not fixable
    /// from here, and falling through to the next source is more useful than
    /// failing. The plan already carries an `owner` holding an ANSI escape, so
    /// this is a shape that occurs rather than one invented for a test.
    #[test]
    fn a_name_that_cannot_be_written_is_refused_when_supplied_and_skipped_when_derived() {
        let supplied = resolve(&sources(Some("ada\u{1b}[41m"), None, None, None))
            .expect_err("a control character in a supplied name must be refused");
        assert!(
            matches!(supplied, IdentityError::Unusable { .. }),
            "refused as {supplied:?}"
        );
        // Made visible rather than echoed, so the refusal cannot itself move the
        // reader's cursor.
        assert!(
            !supplied.to_string().contains('\u{1b}'),
            "the refusal echoes the escape it is complaining about"
        );

        assert_eq!(
            resolve(&sources(None, None, Some("t\u{1b}[41m"), Some("kaminsod"))).as_deref(),
            Ok("kaminsod"),
            "an unusable derived name should be skipped, not fatal"
        );
        // And when every derived source is unusable, that is the no-name case.
        assert!(resolve(&sources(None, None, Some("\u{7}"), Some("\u{7}"))).is_err());
    }

    /// **A name longer than a name is refused** (`A1-359`).
    #[test]
    fn an_absurdly_long_name_is_refused() {
        let long = "a".repeat(MAX_NAME + 1);
        assert!(resolve(&sources(Some(&long), None, None, None)).is_err());
        let fits = "a".repeat(MAX_NAME);
        assert_eq!(
            resolve(&sources(Some(&fits), None, None, None)).as_deref(),
            Ok(fits.as_str())
        );
    }

    /// **The collector reads `APLAN_OWNER` and nothing from the plan**
    /// (`A1-359`).
    ///
    /// Asserted on the shape of what it collects rather than by setting an
    /// environment variable, because the test binary is shared: `set_var`
    /// reaches every other test in this process.
    #[test]
    fn the_collector_consults_the_environment_and_not_the_repository() {
        let here = std::path::Path::new(".");
        let got = from_environment(Some("explicit"), here);
        assert_eq!(got.flag.as_deref(), Some("explicit"));
        assert_eq!(got.env, std::env::var("APLAN_OWNER").ok());
        // Whatever this machine says, the flag still decides.
        assert_eq!(resolve(&got).as_deref(), Ok("explicit"));
    }
}
