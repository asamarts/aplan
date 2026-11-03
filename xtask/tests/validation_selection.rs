//! The production development classifier runs before Rust build selection (`A1-651`).
//!
//! These tests execute the production Python entry point. A Rust model beside
//! it would make a parity test pass while the pre-build classifier rotted.

mod tests {

    use std::ffi::{OsStr, OsString};
    use std::fmt::Write as _;
    use std::os::unix::ffi::OsStringExt as _;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    use sha2::{Digest as _, Sha256};

    use crate::fixture_executable::{install_executable, install_executable_copy};

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .to_path_buf()
    }

    fn stderr(output: &Output) -> String {
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    fn git(root: &Path, args: &[&OsStr]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .expect("run fixture Git");
        assert!(
            output.status.success(),
            "git {}: {}",
            args.iter()
                .map(|argument| argument.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" "),
            stderr(&output)
        );
        String::from_utf8(output.stdout)
            .expect("fixture Git output is UTF-8")
            .trim()
            .to_owned()
    }

    fn selector(root: &Path, args: &[&str]) -> Output {
        Command::new("python3")
            .arg(root.join("ci/scripts/validation-select.py"))
            .args(["--root", root.to_str().expect("UTF-8 fixture root")])
            .args(args)
            .current_dir(root)
            .output()
            .expect("run production validation selector")
    }

    fn performance(root: &Path, args: &[&str]) -> Output {
        Command::new("python3")
            .arg(root.join("ci/scripts/validation-select-performance.py"))
            .args(["--root", root.to_str().expect("UTF-8 fixture root")])
            .args(args)
            .current_dir(root)
            .output()
            .expect("run selector performance harness")
    }

    fn digest(source: &[u8]) -> String {
        Sha256::digest(source)
            .iter()
            .fold(String::with_capacity(64), |mut output, byte| {
                write!(output, "{byte:02x}").expect("write policy digest");
                output
            })
    }

    fn fixture_inventory() -> serde_json::Value {
        serde_json::json!({
            "schema": 1,
            "profiles": {
                "development": {
                    "assurance": "selected",
                    "requires": null,
                    "same_source": false,
                },
                "full": {
                    "assurance": "full",
                    "requires": null,
                    "same_source": false,
                },
                "release": {
                    "assurance": "release",
                    "requires": "full",
                    "same_source": true,
                },
            },
            "selection": {
                "schema": 1,
                "rust": {
                    "gates": ["test"],
                    "cross_cutting_packages": ["audit"],
                    "global_basenames": ["Cargo.toml", "build.rs"],
                },
                "global_gates": ["test"],
                "paths": [
                    {
                        "name": "build-and-ci",
                        "disposition": "global",
                        "gates": ["test"],
                        "exact": ["Cargo.toml", "Cargo.lock"],
                        "prefixes": ["ci/"],
                    },
                    {
                        "name": "prose",
                        "disposition": "selected",
                        "gates": [],
                        "exact": ["README.md"],
                        "prefixes": [],
                    },
                ],
            },
            "gates": [
                {
                    "name": "fmt",
                    "command": "ci/scripts/fmt.sh",
                    "input_class": "repository",
                    "isolation": ["none"],
                    "development": "always",
                    "full": "required",
                    "release": "inherited",
                },
                {
                    "name": "test",
                    "command": "ci/scripts/test.sh",
                    "input_class": "rust-workspace",
                    "isolation": ["shared-cargo-target"],
                    "development": "selectable",
                    "full": "required",
                    "release": "inherited",
                },
                {
                    "name": "coverage",
                    "command": "ci/scripts/coverage.sh",
                    "input_class": "rust-workspace",
                    "isolation": ["isolated-coverage-target"],
                    "development": "deferred",
                    "full": "required",
                    "release": "inherited",
                },
            ],
        })
    }

    fn policy(root: &Path, source: &[u8]) -> serde_json::Value {
        let inputs = [
            "Cargo.lock",
            "Cargo.toml",
            "pkgs/audit/Cargo.toml",
            "pkgs/cli/Cargo.toml",
            "pkgs/core/Cargo.toml",
        ]
        .map(|path| {
            serde_json::json!({
                "path": path,
                "sha256": digest(&std::fs::read(root.join(path)).expect("read graph input")),
            })
        });
        serde_json::json!({
            "schema": "aplan/validation-selection-policy/v1",
            "source": {
                "path": "ci/validation.yml",
                "sha256": digest(source),
            },
            "inventory": fixture_inventory(),
            "workspace": {
                "inputs": inputs,
                "packages": [
                    {"name": "audit", "root": "pkgs/audit/"},
                    {"name": "cli", "root": "pkgs/cli/"},
                    {"name": "core", "root": "pkgs/core/"},
                ],
                "reverse": {
                    "core": ["cli"],
                },
            },
        })
    }

    struct Fixture {
        directory: tempfile::TempDir,
        source: Vec<u8>,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = tempfile::Builder::new()
                .prefix("validation-selector-")
                .tempdir_in(aplan_core::fixture::fixture_root())
                .expect("create selector fixture");
            let root = directory.path();
            for package in ["core", "cli", "audit"] {
                std::fs::create_dir_all(root.join(format!("pkgs/{package}/src")))
                    .expect("create package source");
                let dependency = if package == "cli" {
                    "core = { path = \"../core\" }\n"
                } else {
                    ""
                };
                std::fs::write(
                root.join(format!("pkgs/{package}/Cargo.toml")),
                format!(
                    "[package]\nname = \"{package}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\n{dependency}"
                ),
            )
            .expect("write package manifest");
                std::fs::write(
                    root.join(format!("pkgs/{package}/src/lib.rs")),
                    format!("pub fn {package}() {{}}\n"),
                )
                .expect("write package source");
            }
            std::fs::write(
                root.join("Cargo.toml"),
                "[workspace]\nmembers = [\"pkgs/*\"]\nresolver = \"3\"\n",
            )
            .expect("write workspace manifest");
            std::fs::write(root.join("pkgs/core/build.rs"), "fn main() {}\n")
                .expect("write build script");
            std::fs::write(root.join("pkgs/audit/src/keep.rs"), "pub fn keep() {}\n")
                .expect("write retained audit source");
            std::fs::write(root.join("README.md"), "fixture\n").expect("write fixture prose");
            std::fs::create_dir_all(root.join("ci/scripts")).expect("create CI scripts");
            install_executable_copy(
                &workspace_root().join("ci/scripts/validation-select.py"),
                &root.join("ci/scripts/validation-select.py"),
            );
            install_executable_copy(
                &workspace_root().join("ci/scripts/validation-select-performance.py"),
                &root.join("ci/scripts/validation-select-performance.py"),
            );
            for gate in ["fmt", "test", "coverage"] {
                install_executable(
                    &root.join(format!("ci/scripts/{gate}.sh")),
                    "#!/usr/bin/env bash\nexit 0\n",
                );
            }
            let lock = Command::new("cargo")
                .arg("generate-lockfile")
                .current_dir(root)
                .output()
                .expect("generate fixture lockfile");
            assert!(lock.status.success(), "{}", stderr(&lock));
            let source = b"fixture validation inventory\n".to_vec();
            std::fs::write(root.join("ci/validation.yml"), &source)
                .expect("write inventory source");
            std::fs::write(
                root.join("ci/validation-selection.json"),
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&policy(root, &source))
                        .expect("render selector policy")
                ),
            )
            .expect("write selector policy");
            git(root, &[OsStr::new("init"), OsStr::new("-q")]);
            git(
                root,
                &[
                    OsStr::new("config"),
                    OsStr::new("user.email"),
                    OsStr::new("selector@example.invalid"),
                ],
            );
            git(
                root,
                &[
                    OsStr::new("config"),
                    OsStr::new("user.name"),
                    OsStr::new("Selector Fixture"),
                ],
            );
            git(root, &[OsStr::new("add"), OsStr::new(".")]);
            git(
                root,
                &[
                    OsStr::new("commit"),
                    OsStr::new("-q"),
                    OsStr::new("-m"),
                    OsStr::new("fixture"),
                ],
            );
            std::fs::write(root.join("README.md"), "fixture at measured head\n")
                .expect("write measured fixture revision");
            git(root, &[OsStr::new("add"), OsStr::new("README.md")]);
            git(
                root,
                &[
                    OsStr::new("commit"),
                    OsStr::new("-q"),
                    OsStr::new("-m"),
                    OsStr::new("measured head"),
                ],
            );
            Self { directory, source }
        }

        fn root(&self) -> &Path {
            self.directory.path()
        }

        fn restore_policy(&self) {
            std::fs::write(
                self.root().join("ci/validation-selection.json"),
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&policy(self.root(), &self.source))
                        .expect("render selector policy")
                ),
            )
            .expect("restore selector policy");
            std::fs::write(self.root().join("ci/validation.yml"), &self.source)
                .expect("restore inventory source");
        }

        fn mutate_policy(&self, mutate: impl FnOnce(&mut serde_json::Value)) {
            let path = self.root().join("ci/validation-selection.json");
            let mut document: serde_json::Value = serde_json::from_slice(
                &std::fs::read(&path).expect("read selector policy for mutation"),
            )
            .expect("parse selector policy for mutation");
            mutate(&mut document);
            let mut rendered =
                serde_json::to_string_pretty(&document).expect("render mutated policy");
            rendered.push('\n');
            std::fs::write(path, rendered).expect("write mutated selector policy");
        }
    }

    #[test]
    fn production_selector_starts_no_rust_compiler_and_reports_each_phase() {
        let root = workspace_root();
        let fixture = tempfile::Builder::new()
            .prefix("validation-selector-processes-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("create process fixture");
        let bin = fixture.path().join("bin");
        std::fs::create_dir(&bin).expect("create process bin");
        let log = fixture.path().join("processes.log");
        std::fs::write(&log, "").expect("create process ledger");
        install_executable(
            &bin.join("cargo"),
            "#!/usr/bin/env bash\nprintf 'cargo\\n' >> \"$APLAN_SELECTOR_PROCESS_LOG\"\nexit 97\n",
        );
        install_executable(
            &bin.join("rustc"),
            "#!/usr/bin/env bash\nprintf 'rustc\\n' >> \"$APLAN_SELECTOR_PROCESS_LOG\"\nexit 97\n",
        );
        let path = std::env::join_paths(std::iter::once(bin.clone()).chain(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        )))
        .expect("construct selector PATH");
        let output = Command::new("python3")
            .arg(root.join("ci/scripts/validation-select.py"))
            .args(["--root", root.to_str().expect("UTF-8 workspace root")])
            .args(["--census", "--format", "json", "--timings"])
            .env("PATH", path)
            .env("RUSTC", bin.join("rustc"))
            .env("APLAN_SELECTOR_PROCESS_LOG", &log)
            .current_dir(&root)
            .output()
            .expect("run instrumented production selector");
        assert!(output.status.success(), "{}", stderr(&output));
        let processes = std::fs::read_to_string(&log).expect("read process ledger");
        assert_eq!(
            processes, "",
            "classification may start neither Cargo nor the Rust compiler"
        );
        let report: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("parse selector report");
        assert_eq!(report["mode"], "census");
        let errors = stderr(&output);
        let timing = errors
            .lines()
            .find_map(|line| line.strip_prefix("validation-select: timings="))
            .expect("one timing diagnostic");
        let timing: serde_json::Value = serde_json::from_str(timing).expect("parse timings");
        assert_eq!(timing["cargo_processes"], 0);
        assert_eq!(timing["rustc_processes"], 0);
        assert_eq!(timing["git_processes"], 1);
        for field in [
            "python_startup_ms",
            "inventory_ms",
            "metadata_ms",
            "output_ms",
            "census_ms",
            "git_ms",
            "classification_ms",
            "total_ms",
        ] {
            assert!(timing[field].as_f64().is_some(), "missing timing `{field}`");
        }
        assert!(
            !errors.contains(root.to_str().expect("UTF-8 workspace root")),
            "bounded telemetry must not expose the host checkout path"
        );
    }

    #[test]
    fn performance_harness_binds_one_clean_source_and_process_census() {
        let fixture = Fixture::new();
        let root = fixture.root();
        let output = performance(
            root,
            &[
                "--runs",
                "5",
                "--check",
                "--max-cache-free-p95-ms",
                "10000",
                "--max-warm-p95-ms",
                "10000",
            ],
        );
        assert!(output.status.success(), "{}", stderr(&output));
        let report: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("performance report JSON");
        assert_eq!(report["schema"], "aplan/validation-selector-performance/v1");
        assert_eq!(report["source"]["working_tree_clean"], true);
        assert_eq!(
            report["source"]["harness_sha256"].as_str().map(str::len),
            Some(64)
        );
        assert_eq!(
            report["source"]["commit"],
            git(root, &[OsStr::new("rev-parse"), OsStr::new("HEAD")])
        );
        assert_eq!(report["deterministic"]["cargo_processes"], 0);
        assert_eq!(report["deterministic"]["rustc_processes"], 0);
        assert!(
            report["deterministic"]["git_processes"]
                .as_u64()
                .is_some_and(|count| count > 0),
            "{report}"
        );
        let counts: std::collections::BTreeSet<_> = report["samples"]
            .as_array()
            .expect("samples")
            .iter()
            .map(|sample| sample["timings"]["git_processes"].as_u64())
            .collect();
        assert_eq!(counts.len(), 1, "{report}");
        assert_eq!(
            counts.iter().next().copied().flatten(),
            report["deterministic"]["git_processes"].as_u64()
        );
    }

    #[test]
    fn performance_harness_refuses_dirty_sources_and_non_finite_limits() {
        let fixture = Fixture::new();
        let root = fixture.root();
        std::fs::write(root.join("README.md"), "dirty during measurement\n")
            .expect("dirty measured source");
        let dirty = performance(root, &["--runs", "5"]);
        assert_eq!(dirty.status.code(), Some(1));
        assert!(stderr(&dirty).contains("measured checkout is not clean"));
        git(root, &[OsStr::new("restore"), OsStr::new("README.md")]);

        for arguments in [
            ["--runs", "5", "--timeout-seconds", "NaN"],
            ["--runs", "5", "--max-warm-p95-ms", "inf"],
            ["--runs", "5", "--max-cache-free-p95-ms", "0"],
        ] {
            let invalid = performance(root, &arguments);
            assert_eq!(invalid.status.code(), Some(2), "{}", stderr(&invalid));
            assert!(stderr(&invalid).contains("must be finite and positive"));
        }
    }

    #[test]
    fn performance_harness_rejects_malformed_selector_evidence() {
        let fixture = Fixture::new();
        let root = fixture.root();
        install_executable(
            &root.join("ci/scripts/validation-select.py"),
            r#"#!/usr/bin/env python3
import json
import sys

base, head = sys.argv[sys.argv.index("--revisions") + 1:][:2]
report = {
    "schema": 2,
    "mode": "revisions",
    "inputs": {"resolved_base": base, "resolved_head": head},
}
timing = {
    "cargo_processes": 0,
    "census_ms": 0,
    "classification_ms": 0,
    "git_ms": 0,
    "git_processes": True,
    "inventory_ms": 0,
    "metadata_ms": 0,
    "output_ms": 0,
    "python_startup_ms": 0,
    "rustc_processes": 0,
    "total_ms": 0,
}
print(json.dumps(report))
print("validation-select: timings=" + json.dumps(timing), file=sys.stderr)
"#,
        );
        git(
            root,
            &[
                OsStr::new("add"),
                OsStr::new("ci/scripts/validation-select.py"),
            ],
        );
        git(
            root,
            &[
                OsStr::new("commit"),
                OsStr::new("-q"),
                OsStr::new("-m"),
                OsStr::new("malformed selector evidence"),
            ],
        );

        let malformed = performance(root, &["--runs", "5"]);
        assert_eq!(malformed.status.code(), Some(1));
        assert!(
            stderr(&malformed)
                .contains("selector timing `git_processes` is not a non-negative integer"),
            "{}",
            stderr(&malformed)
        );
    }

    #[test]
    fn a_package_change_selects_reverse_dependents_and_declared_audits() {
        let fixture = Fixture::new();
        let root = fixture.root();
        let census = selector(root, &["--census", "--format", "json"]);
        assert!(census.status.success(), "{}", stderr(&census));

        std::fs::write(
            root.join("pkgs/core/src/lib.rs"),
            "pub fn core_changed() {}\n",
        )
        .expect("change core package");
        let changed = selector(root, &["--working-tree", "--format", "json"]);
        assert!(changed.status.success(), "{}", stderr(&changed));
        let changed: serde_json::Value =
            serde_json::from_slice(&changed.stdout).expect("parse package selection");
        assert_eq!(changed["selected_gates"], serde_json::json!(["test"]));
        assert_eq!(
            changed["selected_packages"],
            serde_json::json!(["audit", "cli", "core"])
        );
        assert_eq!(changed["widened"], false);
        git(root, &[OsStr::new("restore"), OsStr::new(".")]);

        std::fs::write(
            root.join("ci/validation.yml"),
            b"changed without regeneration\n",
        )
        .expect("stale source");
        let stale = selector(root, &["--working-tree", "--format", "json"]);
        assert_eq!(stale.status.code(), Some(1));
        assert!(stderr(&stale).contains("generated selector policy is stale"));
        fixture.restore_policy();

        let root_manifest = root.join("Cargo.toml");
        let mut manifest = std::fs::read_to_string(&root_manifest).expect("read root manifest");
        manifest.push_str("\n# changed graph input\n");
        std::fs::write(&root_manifest, manifest).expect("change graph input");
        let stale_graph = selector(root, &["--working-tree"]);
        assert_eq!(stale_graph.status.code(), Some(1));
        assert!(stderr(&stale_graph).contains("workspace graph is stale for `Cargo.toml`"));
    }

    #[test]
    fn production_selector_rejects_missing_and_malformed_policy_inputs() {
        let fixture = Fixture::new();
        let root = fixture.root();

        std::fs::remove_file(root.join("ci/validation-selection.json"))
            .expect("remove selector policy");
        let missing_policy = selector(root, &["--census"]);
        assert_eq!(missing_policy.status.code(), Some(1));
        assert!(stderr(&missing_policy).contains("policy input is unavailable"));
        fixture.restore_policy();

        std::fs::remove_file(root.join("Cargo.lock")).expect("remove bound graph input");
        let missing_input = selector(root, &["--census"]);
        assert_eq!(missing_input.status.code(), Some(1));
        assert!(stderr(&missing_input).contains("`Cargo.lock` is unavailable"));
        git(root, &[OsStr::new("restore"), OsStr::new("Cargo.lock")]);

        fixture.mutate_policy(|document| document["inventory"]["schema"] = true.into());
        let boolean_schema = selector(root, &["--census"]);
        assert_eq!(boolean_schema.status.code(), Some(1));
        assert!(stderr(&boolean_schema).contains("schema must be 1"));
        assert!(!stderr(&boolean_schema).contains("Traceback"));
        fixture.restore_policy();

        fixture.mutate_policy(|document| {
            document["inventory"]["gates"][0]["input_class"] = serde_json::json!([]);
        });
        let malformed_type = selector(root, &["--census"]);
        assert_eq!(malformed_type.status.code(), Some(1));
        assert!(stderr(&malformed_type).contains("invalid input class"));
        assert!(!stderr(&malformed_type).contains("Traceback"));
        fixture.restore_policy();

        std::fs::write(
            root.join("ci/validation-selection.json"),
            "{\"schema\":1,\"schema\":2}\n",
        )
        .expect("write duplicate policy fields");
        let duplicate = selector(root, &["--census"]);
        assert_eq!(duplicate.status.code(), Some(1));
        assert!(stderr(&duplicate).contains("repeats field `schema`"));
        fixture.restore_policy();

        let policy_path = root.join("ci/validation-selection.json");
        let target_path = root.join("ci/selector-policy-target.json");
        std::fs::rename(&policy_path, &target_path).expect("move selector policy");
        std::os::unix::fs::symlink(&target_path, &policy_path).expect("symlink selector policy");
        let linked = selector(root, &["--census"]);
        assert_eq!(linked.status.code(), Some(1));
        assert!(stderr(&linked).contains("must be regular files"));
        std::fs::remove_file(&policy_path).expect("remove selector symlink");
        std::fs::remove_file(&target_path).expect("remove selector target");
        fixture.restore_policy();

        let hostile = OsString::from_vec(b"tracked-\xff".to_vec());
        std::fs::write(root.join(&hostile), "hostile\n").expect("write non-UTF-8 path");
        git(root, &[OsStr::new("add"), hostile.as_os_str()]);
        let non_utf8 = selector(root, &["--census"]);
        assert_eq!(non_utf8.status.code(), Some(1));
        let errors = stderr(&non_utf8);
        assert!(errors.contains("non-UTF-8 path"));
        assert!(errors.contains("hex:"));

        for arguments in [
            vec!["--working-tree", "--census"],
            vec!["--base", "only-one-side"],
            vec!["--format", "json"],
        ] {
            let invalid = selector(root, &arguments);
            assert_eq!(invalid.status.code(), Some(2));
        }
    }

    #[test]
    fn census_rejects_new_unknown_duplicate_and_stale_dispositions() {
        let fixture = Fixture::new();
        let root = fixture.root();
        fixture.mutate_policy(|document| {
            document["inventory"]["selection"]["paths"][1]["prefixes"] =
                serde_json::json!(["pkgs/"]);
        });
        let overlapping = selector(root, &["--census"]);
        assert_eq!(overlapping.status.code(), Some(1));
        assert!(stderr(&overlapping).contains("more than one validation-selection disposition"));
        fixture.restore_policy();

        fixture.mutate_policy(|document| {
            document["inventory"]["selection"]["paths"][1]["exact"] =
                serde_json::json!(["README.md", "missing.md"]);
        });
        let stale_exact = selector(root, &["--census"]);
        assert_eq!(stale_exact.status.code(), Some(1));
        assert!(stderr(&stale_exact).contains("stale exact path `missing.md`"));
        fixture.restore_policy();

        std::fs::write(root.join("unknown-root-input"), "unknown\n").expect("write unknown path");
        git(root, &[OsStr::new("add"), OsStr::new("unknown-root-input")]);
        let unknown = selector(root, &["--census"]);
        assert_eq!(unknown.status.code(), Some(1));
        assert!(stderr(&unknown).contains("no validation-selection disposition"));
    }

    #[test]
    fn working_tree_merges_staged_unstaged_untracked_deleted_and_renamed_paths() {
        let fixture = Fixture::new();
        let root = fixture.root();

        git(
            root,
            &[
                OsStr::new("mv"),
                OsStr::new("pkgs/core/src/lib.rs"),
                OsStr::new("pkgs/core/src/renamed.rs"),
            ],
        );
        git(
            root,
            &[
                OsStr::new("rm"),
                OsStr::new("-q"),
                OsStr::new("pkgs/audit/src/lib.rs"),
            ],
        );
        std::fs::write(root.join("pkgs/core/new.rs"), "pub fn new() {}\n")
            .expect("write untracked package input");
        let working = selector(root, &["--working-tree", "--format", "json"]);
        assert!(working.status.success(), "{}", stderr(&working));
        let working: serde_json::Value =
            serde_json::from_slice(&working.stdout).expect("parse working-tree selection");
        let changes = working["changes"].as_array().expect("working-tree changes");
        for (path, observation) in [
            ("pkgs/core/src/lib.rs", "rename-old"),
            ("pkgs/core/src/renamed.rs", "rename-new"),
            ("pkgs/audit/src/lib.rs", "staged:D"),
            ("pkgs/core/new.rs", "untracked"),
        ] {
            assert!(
                changes.iter().any(|change| {
                    change["path"] == path
                        && change["observations"]
                            .as_array()
                            .expect("change observations")
                            .iter()
                            .any(|value| {
                                value
                                    .as_str()
                                    .is_some_and(|value| value.contains(observation))
                            })
                }),
                "missing {path} / {observation}: {working}"
            );
        }
        assert_eq!(working["widened"], false);
        assert_eq!(
            working["selected_packages"],
            serde_json::json!(["audit", "cli", "core"])
        );
    }

    #[test]
    fn production_selector_preserves_revision_modes_and_widens_uncertainty() {
        let fixture = Fixture::new();
        let root = fixture.root();
        let base = git(root, &[OsStr::new("rev-parse"), OsStr::new("HEAD")]);
        std::fs::write(
            root.join("pkgs/core/src/lib.rs"),
            "pub fn core_committed() {}\n",
        )
        .expect("change core package");
        git(
            root,
            &[OsStr::new("add"), OsStr::new("pkgs/core/src/lib.rs")],
        );
        git(
            root,
            &[
                OsStr::new("commit"),
                OsStr::new("-q"),
                OsStr::new("-m"),
                OsStr::new("change core"),
            ],
        );
        let head = git(root, &[OsStr::new("rev-parse"), OsStr::new("HEAD")]);
        for arguments in [
            vec!["--revisions", &base, &head, "--format", "json"],
            vec!["--base", &base, "--head", &head, "--format", "json"],
            vec!["--pre-push", &base, &head, "--format", "json"],
        ] {
            let output = selector(root, &arguments);
            assert!(output.status.success(), "{}", stderr(&output));
            let report: serde_json::Value =
                serde_json::from_slice(&output.stdout).expect("parse immutable selection");
            assert_eq!(report["widened"], false, "{report}");
            assert_eq!(
                report["selected_packages"],
                serde_json::json!(["audit", "cli", "core"])
            );
        }

        std::fs::write(root.join("README.md"), "dirty metadata source\n")
            .expect("dirty immutable checkout");
        let dirty = selector(root, &["--revisions", &base, &head, "--format", "json"]);
        assert!(dirty.status.success(), "{}", stderr(&dirty));
        let dirty: serde_json::Value =
            serde_json::from_slice(&dirty.stdout).expect("parse dirty selection");
        assert_eq!(dirty["widened"], true);
        assert!(
            dirty["widening_reasons"]
                .as_array()
                .expect("widening reasons")
                .iter()
                .any(|reason| reason
                    .as_str()
                    .is_some_and(|reason| reason.contains("staged, unstaged or untracked")))
        );
        git(root, &[OsStr::new("restore"), OsStr::new("README.md")]);

        let missing = selector(
            root,
            &[
                "--base",
                "absent-shallow-base",
                "--head",
                &head,
                "--format",
                "json",
            ],
        );
        assert!(missing.status.success(), "{}", stderr(&missing));
        let missing: serde_json::Value =
            serde_json::from_slice(&missing.stdout).expect("parse missing-history selection");
        assert_eq!(missing["widened"], true);
        assert_eq!(missing["no_changes"], false);
        assert!(
            missing["widening_reasons"]
                .as_array()
                .expect("widening reasons")
                .iter()
                .any(|reason| reason
                    .as_str()
                    .is_some_and(|reason| reason.contains("history may be shallow")))
        );
    }

    #[test]
    fn pre_push_merges_exact_updates_with_every_working_tree_surface() {
        let fixture = Fixture::new();
        let root = fixture.root();
        let before = git(root, &[OsStr::new("rev-parse"), OsStr::new("HEAD")]);
        std::fs::write(
            root.join("pkgs/core/src/lib.rs"),
            "pub fn core_committed() {}\n",
        )
        .expect("write committed package change");
        git(
            root,
            &[OsStr::new("add"), OsStr::new("pkgs/core/src/lib.rs")],
        );
        git(
            root,
            &[
                OsStr::new("commit"),
                OsStr::new("-q"),
                OsStr::new("-m"),
                OsStr::new("core change"),
            ],
        );
        let after = git(root, &[OsStr::new("rev-parse"), OsStr::new("HEAD")]);
        std::fs::write(root.join("README.md"), "staged prose\n").expect("write staged input");
        git(root, &[OsStr::new("add"), OsStr::new("README.md")]);
        std::fs::write(
            root.join("pkgs/cli/src/lib.rs"),
            "pub fn cli_changed() {}\n",
        )
        .expect("write unstaged input");
        std::fs::write(
            root.join("pkgs/audit/src/untracked.rs"),
            "pub fn untracked() {}\n",
        )
        .expect("write untracked input");

        let output = selector(
            root,
            &[
                "--pre-push",
                &before,
                &after,
                "--pre-push",
                &after,
                &after,
                "--format",
                "json",
            ],
        );
        assert!(output.status.success(), "{}", stderr(&output));
        let report: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("parse pre-push selection");
        assert_eq!(report["mode"], "pre-push");
        assert_eq!(
            report["inputs"]["push_updates"].as_array().unwrap().len(),
            2
        );
        assert_eq!(report["widened"], false, "{report}");
        assert_eq!(
            report["selected_packages"],
            serde_json::json!(["audit", "cli", "core"])
        );
        for (path, observation) in [
            ("pkgs/core/src/lib.rs", "pre-push:1:"),
            ("README.md", "staged:M"),
            ("pkgs/cli/src/lib.rs", "unstaged:M"),
            ("pkgs/audit/src/untracked.rs", "untracked"),
        ] {
            assert!(
                report["changes"].as_array().unwrap().iter().any(|change| {
                    change["path"] == path
                        && change["observations"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|value| {
                                value
                                    .as_str()
                                    .is_some_and(|value| value.starts_with(observation))
                            })
                }),
                "missing {path} / {observation}: {report}"
            );
        }
    }

    #[test]
    fn production_selector_malformed_git_record_widens_without_partial_success() {
        let fixture = Fixture::new();
        let root = fixture.root();
        let shim = tempfile::Builder::new()
            .prefix("validation-selector-git-shim-")
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("create Git shim fixture");
        let bin = shim.path().join("bin");
        std::fs::create_dir(&bin).expect("create Git shim bin");
        install_executable(
            &bin.join("git"),
            "#!/usr/bin/env bash\nif [[ \"$1\" == diff && \"${2:-}\" == --cached ]]; then printf 'R100\\0only-old\\0'; exit 0; fi\nexec \"$APLAN_REAL_GIT\" \"$@\"\n",
        );
        let real_git = git(root, &[OsStr::new("--exec-path")]);
        let real_git = Path::new(&real_git).join("git");
        let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        )))
        .expect("construct shim PATH");
        let output = Command::new("python3")
            .arg(root.join("ci/scripts/validation-select.py"))
            .args(["--root", root.to_str().expect("UTF-8 fixture root")])
            .args(["--working-tree", "--format", "json"])
            .env("PATH", path)
            .env("APLAN_REAL_GIT", real_git)
            .current_dir(root)
            .output()
            .expect("run selector with malformed Git record");
        assert!(output.status.success(), "{}", stderr(&output));
        let report: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("parse widened malformed record");
        assert_eq!(report["widened"], true);
        assert_eq!(report["changes"], serde_json::json!([]));
        assert!(
            report["widening_reasons"]
                .as_array()
                .expect("widening reasons")
                .iter()
                .any(|reason| reason
                    .as_str()
                    .is_some_and(|reason| reason.contains("rename/copy record is missing a path")))
        );
    }

    #[test]
    fn xtask_compatibility_launcher_delegates_to_the_production_selector() {
        let root = workspace_root();
        let direct = selector(&root, &["--census", "--format", "json"]);
        assert!(direct.status.success(), "{}", stderr(&direct));
        let wrapped = Command::new(crate::common::xtask())
            .args(["validation-select", "--root"])
            .arg(&root)
            .args(["--census", "--format", "json"])
            .current_dir(&root)
            .output()
            .expect("run xtask compatibility launcher");
        assert!(wrapped.status.success(), "{}", stderr(&wrapped));
        let direct: serde_json::Value =
            serde_json::from_slice(&direct.stdout).expect("direct JSON");
        let wrapped: serde_json::Value =
            serde_json::from_slice(&wrapped.stdout).expect("wrapped JSON");
        assert_eq!(wrapped, direct);
    }
}
