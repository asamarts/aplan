//! Does the locked dependency graph honour the MSRV this workspace declares?
//! (`A1-327`)
//!
//! `rust-version` is a consumer contract: a source adopter reads it and expects
//! the reviewed revision to build with that compiler. Nothing checked it, and
//! it was false. `P0000019`'s static-artifact build pinned this revision, used
//! the declared `1.85`, and cargo refused the locked graph because `darling
//! 0.23.0` requires `1.88` (pre-public adoption evidence item 15).
//!
//! **The declaration was also load-bearing for a decision it could not
//! support.** `Cargo.toml` pinned `rmcp` at 2.x with a comment saying 3.x
//! "requires rustc 1.88, and this workspace commits to `rust-version =
//! "1.85"`". The 2.x it stayed on pulls `darling 0.23`, which requires 1.88
//! anyway, so the constraint bought nothing and cost an SDK major. An untested
//! contract does not merely go stale; it gets reasoned from.
//!
//! **This is the half that needs no toolchain.** Every package declares the
//! compiler it needs, and cargo has already resolved which packages are in the
//! graph, so the floor is a maximum over `cargo metadata` and the comparison is
//! arithmetic. What it does *not* check is that this workspace's own source
//! compiles at that floor, which genuinely needs the compiler: see
//! `ci/scripts/spec.sh` for where that is stated, and `A1-330` for wiring it.

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

/// A `rust-version`, comparable. `1.88` and `1.88.0` are the same floor, and
/// both appear in the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(u64, u64, u64);

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// Parse a `rust-version`. A missing component is zero, which is what makes
/// `1.88` and `1.88.0` compare equal rather than one of them sorting first.
///
/// `None` rather than a default on anything unparseable: a floor this cannot
/// read is a floor it must not silently treat as zero, which would hide exactly
/// the dependency the gate exists to catch.
#[must_use]
pub fn parse(v: &str) -> Option<Version> {
    let mut it = v.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next().unwrap_or("0").parse().ok()?;
    let patch = it.next().unwrap_or("0").parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some(Version(major, minor, patch))
}

/// One dependency and the compiler it asks for.
#[derive(Debug, Clone)]
pub struct Floor {
    pub name: String,
    pub version: String,
    pub rust_version: Version,
}

/// Every package a *consumer* builds that declares a floor above `declared`.
///
/// Two exclusions, for two different reasons.
///
/// **Workspace members**, because they all report `declared` itself
/// (`rust-version.workspace = true`), so including them would make the gate
/// compare a value with itself -- the shape `A1-210` is about.
///
/// **Dev-only dependencies**, because `rust-version` is a promise to somebody
/// running `cargo build` on this as a dependency, and that never compiles a
/// dev-dependency. Twenty-five of this graph's packages are dev-only, and the
/// highest floor among them is `1.85` against `1.88` for the rest, so today
/// the two answers agree -- but a test-only crate raising its floor tomorrow
/// would push this gate to demand a higher declaration than any consumer
/// needs, which is this task's own defect pointing the other way. A dev-only
/// floor above the declaration is a real problem for contributors and CI will
/// meet it directly, by failing to build the tests.
#[must_use]
pub fn over(metadata: &serde_json::Value, declared: Version) -> Vec<Floor> {
    let members: std::collections::BTreeSet<&str> = metadata["workspace_members"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let reachable = consumer_reachable(metadata, &members);
    let mut out: Vec<Floor> = metadata["packages"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|p| {
            let id = p["id"].as_str().unwrap_or_default();
            !members.contains(id) && reachable.contains(id)
        })
        .filter_map(|p| {
            let rust_version = parse(p["rust_version"].as_str()?)?;
            (rust_version > declared).then(|| Floor {
                name: p["name"].as_str().unwrap_or_default().to_string(),
                version: p["version"].as_str().unwrap_or_default().to_string(),
                rust_version,
            })
        })
        .collect();
    // Highest first: the one that sets the floor is the one to read.
    out.sort_by(|a, b| {
        b.rust_version
            .cmp(&a.rust_version)
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

/// Every package reachable from a workspace member without crossing a
/// dev-dependency edge: what a consumer of this crate actually compiles.
///
/// Cargo does not resolve dev-dependencies of non-workspace packages at all,
/// so the only dev edges in the graph leave our own crates, and skipping those
/// is the whole of it. Falls back to every package when `resolve` is absent,
/// because a gate that silently checks nothing is worse than one that checks
/// too much.
fn consumer_reachable<'a>(
    metadata: &'a serde_json::Value,
    members: &std::collections::BTreeSet<&'a str>,
) -> std::collections::BTreeSet<&'a str> {
    let Some(nodes) = metadata["resolve"]["nodes"].as_array() else {
        return metadata["packages"]
            .as_array()
            .map(|a| a.iter().filter_map(|p| p["id"].as_str()).collect())
            .unwrap_or_default();
    };
    let by_id: std::collections::BTreeMap<&str, &serde_json::Value> = nodes
        .iter()
        .filter_map(|n| n["id"].as_str().map(|id| (id, n)))
        .collect();
    let mut seen: std::collections::BTreeSet<&str> = members.clone();
    let mut stack: Vec<&str> = members.iter().copied().collect();
    while let Some(cur) = stack.pop() {
        let Some(node) = by_id.get(cur) else { continue };
        for dep in node["deps"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let kinds: std::collections::BTreeSet<Option<&str>> = dep["dep_kinds"]
                .as_array()
                .map(|a| a.iter().map(|k| k["kind"].as_str()).collect())
                .unwrap_or_default();
            // Reached only as a dev-dependency. A package reached both ways is
            // kept: the consumer build still compiles it.
            if !kinds.is_empty() && kinds.iter().all(|k| *k == Some("dev")) {
                continue;
            }
            if let Some(id) = dep["pkg"].as_str()
                && seen.insert(id)
            {
                stack.push(id);
            }
        }
    }
    seen
}

/// The workspace's declared `rust-version`, read from `cargo metadata` rather
/// than from the file, so this and the resolver agree about what was declared.
fn declared_of(metadata: &serde_json::Value) -> Option<Version> {
    let members: std::collections::BTreeSet<&str> = metadata["workspace_members"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    metadata["packages"]
        .as_array()?
        .iter()
        .filter(|p| members.contains(p["id"].as_str().unwrap_or_default()))
        .filter_map(|p| parse(p["rust_version"].as_str()?))
        .max()
}

/// The workspace member ids, as several callers need them.
fn members_of(metadata: &serde_json::Value) -> std::collections::BTreeSet<&str> {
    metadata["workspace_members"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default()
}

/// The highest floor any package a consumer builds asks for, whatever the
/// declaration is. `None` when no dependency declares one at all.
#[must_use]
pub fn highest(
    metadata: &serde_json::Value,
    members: &std::collections::BTreeSet<&str>,
) -> Option<Version> {
    let reachable = consumer_reachable(metadata, members);
    metadata["packages"]
        .as_array()?
        .iter()
        .filter(|p| {
            let id = p["id"].as_str().unwrap_or_default();
            !members.contains(id) && reachable.contains(id)
        })
        .filter_map(|p| parse(p["rust_version"].as_str()?))
        .max()
}

/// `cargo metadata --locked`, so the gate reads the graph that is committed
/// rather than one it resolved for itself.
fn metadata(root: &Path) -> Result<serde_json::Value> {
    let out = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(root)
        .output()
        .context("running `cargo metadata`")?;
    if !out.status.success() {
        bail!(
            "`cargo metadata --locked` failed:\n{}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    serde_json::from_slice(&out.stdout).context("parsing `cargo metadata` output")
}

/// The gate.
pub fn gate(root: &Path) -> Result<()> {
    let metadata = metadata(root)?;
    let declared = declared_of(&metadata)
        .context("this workspace declares no `rust-version`, so it promises nothing")?;
    let over = over(&metadata, declared);
    if over.is_empty() {
        // The graph's own maximum, printed rather than only compared. This gate
        // catches an under-declaration and by construction cannot catch an
        // over-declaration: a `rust-version` raised to 1.99 for no reason
        // passes here while needlessly excluding every adopter below it, and
        // deciding whether the declaration is *higher* than necessary needs the
        // compiler, which is `A1-330`. Showing the headroom is the honest half
        // that costs nothing: a reader who sees a gap knows to ask.
        match highest(&metadata, &members_of(&metadata)) {
            Some(graph) if graph < declared => println!(
                "msrv: the locked graph needs {graph} and this workspace declares {declared}, \
                 which is {declared} promised for our own source rather than for the graph"
            ),
            Some(graph) => println!(
                "msrv: the locked graph needs {graph} and this workspace declares {declared}"
            ),
            None => println!(
                "msrv: no dependency declares a rust-version, so {declared} rests on our \
                 own source alone"
            ),
        }
        return Ok(());
    }
    let worst = over[0].rust_version;
    let named: Vec<String> = over
        .iter()
        .map(|f| format!("{} {} needs {}", f.name, f.version, f.rust_version))
        .collect();
    bail!(
        "the locked graph needs rustc {worst}, and this workspace declares \
         {declared}:\n  {}\nRaise `rust-version` in Cargo.toml to {worst}, or pin these \
         dependencies back to versions that honour {declared}. A declared MSRV nothing \
         checks is a promise to a source adopter that this revision does not keep.",
        named.join("\n  ")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(declared: &str, deps: &[(&str, &str)]) -> serde_json::Value {
        let mut packages = vec![serde_json::json!({
            "id": "ours", "name": "aplan", "version": "0.4.0-dev",
            "rust_version": declared,
        })];
        for (name, rv) in deps {
            packages.push(serde_json::json!({
                "id": format!("dep-{name}"), "name": name, "version": "1.0.0",
                "rust_version": rv,
            }));
        }
        serde_json::json!({ "workspace_members": ["ours"], "packages": packages })
    }

    #[test]
    fn a_dotted_and_an_undotted_floor_are_the_same_floor() {
        // Both spellings are in the registry: `darling` says `1.88.0` and this
        // workspace says `1.88`. Comparing them as strings makes one of them
        // greater, which would be a finding against a graph that is fine.
        assert_eq!(parse("1.88"), parse("1.88.0"));
        assert!(parse("1.88").unwrap() > parse("1.85.0").unwrap());
        assert!(
            parse("1.9").unwrap() < parse("1.10").unwrap(),
            "not string order"
        );
    }

    #[test]
    fn an_unreadable_floor_is_none_rather_than_zero() {
        // A floor this cannot read must not be treated as "needs nothing",
        // which would hide the one dependency the gate exists to find.
        for bad in ["", "1.88.0.1", "nightly", "1.x", "1.88-beta"] {
            assert_eq!(parse(bad), None, "{bad} parsed");
        }
    }

    #[test]
    fn the_dependency_that_sets_the_floor_is_reported_first() {
        let m = meta("1.85", &[("darling", "1.88.0"), ("hashbrown", "1.86")]);
        let over = over(&m, parse("1.85").unwrap());
        assert_eq!(over.len(), 2);
        assert_eq!(over[0].name, "darling", "highest floor is not first");
    }

    #[test]
    fn a_graph_within_the_declared_floor_is_silent() {
        let m = meta("1.88", &[("darling", "1.88.0"), ("serde", "1.31")]);
        assert!(over(&m, parse("1.88").unwrap()).is_empty());
    }

    /// The workspace's own members declare the value being checked, so counting
    /// them would compare it with itself and pass whatever it was set to -- the
    /// shape `A1-210` is about, and the one this gate replaces.
    #[test]
    fn our_own_crates_are_not_evidence_about_our_own_floor() {
        let m = serde_json::json!({
            "workspace_members": ["ours"],
            "packages": [
                { "id": "ours", "name": "aplan", "version": "0.4.0-dev", "rust_version": "1.99" },
            ],
        });
        assert!(
            over(&m, parse("1.88").unwrap()).is_empty(),
            "a workspace member was read as a dependency raising the floor"
        );
    }

    /// A dev-only dependency does not raise the promise (`A1-327`).
    ///
    /// `rust-version` is what somebody running `cargo build` on this as a
    /// dependency needs, and that never compiles a dev-dependency. Failing
    /// here would push the declaration above what any consumer requires --
    /// this task's own defect, pointing the other way.
    #[test]
    fn a_dev_only_dependency_does_not_raise_the_promise() {
        let m = serde_json::json!({
            "workspace_members": ["ours"],
            "packages": [
                { "id": "ours", "name": "aplan", "version": "0.4.0-dev", "rust_version": "1.88" },
                { "id": "dev", "name": "trycmd", "version": "1.0.0", "rust_version": "1.95" },
                { "id": "real", "name": "serde", "version": "1.0.0", "rust_version": "1.88" },
            ],
            "resolve": { "nodes": [
                { "id": "ours", "deps": [
                    { "pkg": "dev", "dep_kinds": [{ "kind": "dev" }] },
                    { "pkg": "real", "dep_kinds": [{ "kind": null }] },
                ]},
                { "id": "dev", "deps": [] },
                { "id": "real", "deps": [] },
            ]},
        });
        assert!(
            over(&m, parse("1.88").unwrap()).is_empty(),
            "a test-only crate was read as raising the consumer contract"
        );
    }

    /// Reached both ways, it is still built by a consumer.
    #[test]
    fn a_dependency_that_is_also_a_dev_dependency_still_counts() {
        let m = serde_json::json!({
            "workspace_members": ["ours"],
            "packages": [
                { "id": "ours", "name": "aplan", "version": "0.4.0-dev", "rust_version": "1.88" },
                { "id": "both", "name": "serde", "version": "1.0.0", "rust_version": "1.95" },
            ],
            "resolve": { "nodes": [
                { "id": "ours", "deps": [
                    { "pkg": "both", "dep_kinds": [{ "kind": "dev" }, { "kind": null }] },
                ]},
                { "id": "both", "deps": [] },
            ]},
        });
        let over = over(&m, parse("1.88").unwrap());
        assert_eq!(over.len(), 1, "a normal dependency was skipped as dev-only");
        assert_eq!(over[0].name, "serde");
    }

    /// A build-dependency is compiled by a consumer, so it counts.
    #[test]
    fn a_build_dependency_counts() {
        let m = serde_json::json!({
            "workspace_members": ["ours"],
            "packages": [
                { "id": "ours", "name": "aplan", "version": "0.4.0-dev", "rust_version": "1.88" },
                { "id": "b", "name": "cc", "version": "1.0.0", "rust_version": "1.95" },
            ],
            "resolve": { "nodes": [
                { "id": "ours", "deps": [{ "pkg": "b", "dep_kinds": [{ "kind": "build" }] }]},
                { "id": "b", "deps": [] },
            ]},
        });
        assert_eq!(over(&m, parse("1.88").unwrap()).len(), 1);
    }

    #[test]
    fn a_dependency_declaring_nothing_is_not_a_finding() {
        // 45 of this graph's packages declare no `rust-version`. Absent is not
        // a claim, and inventing one would make the gate report noise.
        let m = serde_json::json!({
            "workspace_members": ["ours"],
            "packages": [
                { "id": "ours", "name": "aplan", "version": "0.4.0-dev", "rust_version": "1.88" },
                { "id": "dep-x", "name": "x", "version": "1.0.0" },
            ],
        });
        assert!(over(&m, parse("1.88").unwrap()).is_empty());
    }

    /// The graph's own floor is reported whatever the declaration, so an
    /// over-declaration is visible even though it cannot fail here (`A1-327`,
    /// audit).
    ///
    /// This gate catches a declaration that is too *low*. One that is too high
    /// passes -- `rust-version = "1.99"` excludes every adopter below it and no
    /// dependency objects -- and deciding whether our own source needs it takes
    /// the compiler (`A1-330`). Printing the headroom is the half that costs
    /// nothing.
    #[test]
    fn the_graphs_own_floor_is_known_independently_of_the_declaration() {
        let m = meta("1.99", &[("darling", "1.88.0"), ("serde", "1.60")]);
        assert_eq!(
            highest(&m, &members_of(&m)),
            parse("1.88.0"),
            "the graph's floor was read from the declaration rather than the graph"
        );
        assert!(
            over(&m, parse("1.99").unwrap()).is_empty(),
            "an over-declaration must not fail: our own source may need it"
        );
    }

    #[test]
    fn a_graph_declaring_no_floors_has_none() {
        let m = serde_json::json!({
            "workspace_members": ["ours"],
            "packages": [
                { "id": "ours", "name": "aplan", "version": "0.4.0-dev", "rust_version": "1.88" },
                { "id": "x", "name": "x", "version": "1.0.0" },
            ],
        });
        assert_eq!(highest(&m, &members_of(&m)), None);
    }

    #[test]
    fn the_declared_floor_is_the_highest_any_member_asks_for() {
        // Members normally share one `rust-version.workspace = true`. If one
        // ever declared its own and higher, the workspace promises that one:
        // taking the lowest would let a member quietly need more than the
        // number an adopter reads.
        let m = serde_json::json!({
            "workspace_members": ["a", "b"],
            "packages": [
                { "id": "a", "name": "a", "version": "0.1.0", "rust_version": "1.85" },
                { "id": "b", "name": "b", "version": "0.1.0", "rust_version": "1.90" },
            ],
        });
        assert_eq!(declared_of(&m), parse("1.90"));
    }
}
