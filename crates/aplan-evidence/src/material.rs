//! Exact reusable material identity for a portable test-provider observation.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};

use aplan_config::TestRunner;
use globset::{Glob, GlobSet, GlobSetBuilder};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// The portable material key carried by a test-evidence receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestMaterialIdentity {
    /// Exact declared repository file tree, including executable permissions.
    pub inputs_sha256: String,
    /// Every inherited environment key and value, without exposing either.
    pub environment_sha256: String,
    /// Platform, configured executable bytes and applicable Rustup state.
    pub toolchain_sha256: String,
    /// Composition of the three stronger identities above.
    pub sha256: String,
}

/// The command-bearing material which identifies one direct test execution.
///
/// This is deliberately narrower than [`TestMaterialIdentity`]: a focused
/// close is already bound to the repository snapshot by its mutation preview,
/// while this value binds the configured provider definition, inherited
/// environment, executable bytes and applicable toolchain state. Portable
/// receipts additionally need an exhaustive declared source tree and therefore
/// continue to use [`test_material_identity`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestRuntimeIdentity {
    pub definition_sha256: String,
    pub environment_sha256: String,
    pub toolchain_sha256: String,
    pub sha256: String,
}

/// Identify the exact runtime material of one configured test provider.
///
/// # Errors
///
/// Returns a bounded explanation when a configured executable, script,
/// interpreter or applicable Rustup input cannot be identified stably.
pub fn test_runtime_identity(
    repo_root: &Path,
    runner: &TestRunner,
) -> Result<TestRuntimeIdentity, String> {
    let definition = serde_json::to_vec(runner)
        .map_err(|error| format!("serializing the configured provider: {error}"))?;
    let definition = Sha256::digest(definition);
    let environment = environment_identity();
    let toolchain = toolchain_identity(repo_root, runner)?;
    let mut combined = Sha256::new();
    combined.update(b"aplan-test-runtime-v1\0");
    combined.update(definition);
    combined.update(environment);
    combined.update(toolchain);
    Ok(TestRuntimeIdentity {
        definition_sha256: hex(&definition),
        environment_sha256: hex(&environment),
        toolchain_sha256: hex(&toolchain),
        sha256: hex(&combined.finalize()),
    })
}

/// Compute the portable material key for one configured runner.
///
/// `patterns` is an exhaustive trust claim. Empty matches, symlinks, changing
/// inputs and an executable that cannot be identified are refusals rather than
/// weak keys.
///
/// # Errors
///
/// Returns a bounded explanation of the first incomplete identity input.
pub fn test_material_identity(
    repo_root: &Path,
    patterns: &[String],
    runner: &TestRunner,
) -> Result<TestMaterialIdentity, String> {
    if patterns.is_empty() {
        return Err("evidence.observation_inputs is empty".into());
    }
    let matcher = glob_set(patterns)?;
    let mut files = BTreeMap::new();
    for root in input_roots(patterns)? {
        collect_inputs(repo_root, &repo_root.join(root), &matcher, &mut files)?;
    }
    if files.is_empty() {
        return Err(
            "evidence.observation_inputs matched no regular files; an empty material key cannot prove provider currency"
                .into(),
        );
    }
    let inputs = inputs_identity(repo_root, patterns, &files)?;
    let mut after_files = BTreeMap::new();
    for root in input_roots(patterns)? {
        collect_inputs(repo_root, &repo_root.join(root), &matcher, &mut after_files)?;
    }
    if files.len() != after_files.len()
        || files.iter().any(|(path, before)| {
            after_files
                .get(path)
                .is_none_or(|after| !same_file_observation(before, after))
        })
    {
        return Err("declared material input set changed while it was observed".into());
    }
    let environment = environment_identity();
    let toolchain = toolchain_identity(repo_root, runner)?;
    let mut combined = Sha256::new();
    combined.update(b"aplan-test-material-v1\0");
    combined.update(inputs);
    combined.update(environment);
    combined.update(toolchain);
    Ok(TestMaterialIdentity {
        inputs_sha256: hex(&inputs),
        environment_sha256: hex(&environment),
        toolchain_sha256: hex(&toolchain),
        sha256: hex(&combined.finalize()),
    })
}

/// Whether one repository-relative path is part of the exhaustive provider
/// material declaration.
///
/// Incremental refresh uses the same compiled glob semantics as material
/// hashing; a second matcher would let selection call a changed input
/// unrelated while the receipt identity called it material.
pub fn test_material_includes_path(patterns: &[String], path: &str) -> Result<bool, String> {
    Ok(glob_set(patterns)?.is_match(path))
}

fn glob_set(patterns: &[String]) -> Result<GlobSet, String> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let path = Path::new(pattern);
        if path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(format!(
                "material input pattern `{pattern}` must stay below the repository root"
            ));
        }
        builder.add(
            Glob::new(pattern)
                .map_err(|error| format!("invalid material input pattern `{pattern}`: {error}"))?,
        );
    }
    builder.build().map_err(|error| error.to_string())
}

fn input_roots(patterns: &[String]) -> Result<BTreeSet<PathBuf>, String> {
    let roots = patterns
        .iter()
        .filter_map(|pattern| Path::new(pattern).components().next())
        .filter_map(|component| match component {
            Component::Normal(value) => Some(PathBuf::from(value)),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    if roots.is_empty() {
        return Err("material input patterns have no repository-relative root".into());
    }
    Ok(roots)
}

fn collect_inputs(
    repo_root: &Path,
    path: &Path,
    matcher: &GlobSet,
    files: &mut BTreeMap<PathBuf, fs::Metadata>,
) -> Result<(), String> {
    let relative = path.strip_prefix(repo_root).unwrap_or(path);
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("observing `{}`: {error}", relative.display())),
    };
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "declared material input tree contains symbolic link `{}`",
            relative.display()
        ));
    }
    if metadata.is_file() {
        if matcher.is_match(relative) {
            files.insert(relative.to_path_buf(), metadata);
        }
        return Ok(());
    }
    if !metadata.is_dir() {
        return Ok(());
    }
    let mut entries = fs::read_dir(path)
        .map_err(|error| format!("enumerating `{}`: {error}", relative.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("enumerating `{}`: {error}", relative.display()))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        collect_inputs(repo_root, &entry.path(), matcher, files)?;
    }
    Ok(())
}

fn inputs_identity(
    repo_root: &Path,
    patterns: &[String],
    files: &BTreeMap<PathBuf, fs::Metadata>,
) -> Result<[u8; 32], String> {
    let mut digest = Sha256::new();
    digest.update(b"aplan-test-inputs-v1\0");
    // The declaration is a trust policy, not just a way to find today's
    // files. Bind its order-independent exact set so widening or narrowing a
    // glob cannot reuse a result merely because both policies happen to match
    // the same tree at this revision.
    for pattern in patterns.iter().collect::<BTreeSet<_>>() {
        hash_bytes(&mut digest, pattern.as_bytes());
    }
    for (relative, before) in files {
        let path = repo_root.join(relative);
        hash_path(&mut digest, relative);
        hash_permissions(&mut digest, before);
        let label = format!("material input `{}`", relative.display());
        let bytes = read_stable_file(&path, before, &label)?;
        hash_bytes(&mut digest, &bytes);
    }
    Ok(digest.finalize().into())
}

fn environment_identity() -> [u8; 32] {
    environment_identity_from(std::env::vars_os())
}

fn environment_identity_from(
    environment: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> [u8; 32] {
    let mut environment = environment.into_iter().collect::<Vec<_>>();
    environment.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    digest.update(b"aplan-test-environment-v1\0");
    for (key, value) in environment {
        hash_os(&mut digest, &key);
        hash_os(&mut digest, &value);
    }
    digest.finalize().into()
}

fn toolchain_identity(repo_root: &Path, runner: &TestRunner) -> Result<[u8; 32], String> {
    let mut programs = BTreeSet::new();
    for argv in [
        runner.wrap.as_ref(),
        Some(&runner.list),
        Some(&runner.run),
        runner.skipped.as_ref(),
        runner.batch.as_ref().map(|batch| &batch.run),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(program) = argv.first() {
            programs.insert(program.clone());
        }
    }
    if programs.is_empty() {
        return Err("the configured runner names no executable".into());
    }
    let working_directory = runner
        .dir
        .as_deref()
        .map_or_else(|| repo_root.to_path_buf(), |dir| repo_root.join(dir));
    let mut digest = Sha256::new();
    digest.update(b"aplan-test-toolchain-v1\0");
    hash_bytes(&mut digest, std::env::consts::OS.as_bytes());
    hash_bytes(&mut digest, std::env::consts::ARCH.as_bytes());
    hash_bytes(&mut digest, std::env::consts::FAMILY.as_bytes());
    let mut hashed = BTreeSet::new();
    let mut uses_rustup = false;
    for program in programs {
        hash_bytes(&mut digest, program.as_bytes());
        let path = resolve_program(&program, &working_directory)
            .ok_or_else(|| format!("configured provider executable `{program}` was not found"))?;
        hash_executable(&mut digest, &path, &working_directory, &mut hashed)?;
        uses_rustup |= path
            .canonicalize()
            .ok()
            .and_then(|target| target.file_name().map(OsStr::to_owned))
            .is_some_and(|name| name == "rustup");
    }
    // Interpreted tools commonly put the actual program in argv[1]
    // (`sh runner.sh`, `python /opt/adapter.py`). Hash every existing file
    // argument as well as argv[0], so moving a script outside the repository
    // cannot silently remove it from the portable identity. Non-path selectors
    // and flags simply do not resolve to files and remain bound by the runner
    // definition digest.
    for argv in [
        runner.wrap.as_ref(),
        Some(&runner.list),
        Some(&runner.run),
        runner.skipped.as_ref(),
        runner.batch.as_ref().map(|batch| &batch.run),
    ]
    .into_iter()
    .flatten()
    {
        for argument in argv.iter().skip(1) {
            if let Some(path) = configured_file_argument(argument, &working_directory) {
                hash_bytes(&mut digest, argument.as_bytes());
                hash_executable(&mut digest, &path, &working_directory, &mut hashed)?;
            }
        }
    }
    if uses_rustup {
        hash_rustup_state(&mut digest)?;
    }
    Ok(digest.finalize().into())
}

fn configured_file_argument(argument: &str, working_directory: &Path) -> Option<PathBuf> {
    if argument.contains('{') || argument.contains('}') {
        return None;
    }
    let candidate = argument
        .strip_prefix('-')
        .and_then(|argument| argument.split_once('=').map(|(_, value)| value))
        .unwrap_or(argument);
    if candidate.is_empty() {
        return None;
    }
    let path = Path::new(candidate);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        working_directory.join(path)
    };
    path.is_file().then_some(path)
}

fn hash_executable(
    digest: &mut Sha256,
    path: &Path,
    working_directory: &Path,
    hashed: &mut BTreeSet<PathBuf>,
) -> Result<(), String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("identifying executable `{}`: {error}", path.display()))?;
    if !hashed.insert(canonical.clone()) {
        return Ok(());
    }
    let before = fs::metadata(&canonical)
        .map_err(|error| format!("identifying executable `{}`: {error}", canonical.display()))?;
    if !before.is_file() {
        return Err(format!(
            "provider executable `{}` is not a regular file",
            canonical.display()
        ));
    }
    hash_permissions(digest, &before);
    let label = format!("provider executable `{}`", canonical.display());
    let bytes = read_stable_file(&canonical, &before, &label)?;
    hash_bytes(digest, &bytes);
    if let Some(interpreter) = shebang_program(&bytes) {
        let interpreter_path = resolve_program(&interpreter, working_directory)
            .ok_or_else(|| format!("script interpreter `{interpreter}` was not found"))?;
        hash_bytes(digest, interpreter.as_bytes());
        hash_executable(digest, &interpreter_path, working_directory, hashed)?;
    }
    Ok(())
}

fn shebang_program(bytes: &[u8]) -> Option<String> {
    let line = bytes
        .strip_prefix(b"#!")?
        .split(|byte| *byte == b'\n')
        .next()?;
    let text = std::str::from_utf8(line).ok()?.trim();
    let mut words = text.split_whitespace();
    let first = words.next()?;
    if Path::new(first)
        .file_name()
        .is_some_and(|name| name == "env")
    {
        words
            .find(|word| !word.starts_with('-'))
            .map(str::to_string)
    } else {
        Some(first.to_string())
    }
}

fn resolve_program(program: &str, working_directory: &Path) -> Option<PathBuf> {
    let direct = Path::new(program);
    if direct.components().count() > 1 {
        let path = if direct.is_absolute() {
            direct.to_path_buf()
        } else {
            working_directory.join(direct)
        };
        return executable_path(&path);
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .find_map(|directory| executable_path(&directory.join(program)))
}

fn executable_path(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return Some(path.to_path_buf());
    }
    #[cfg(windows)]
    if path.extension().is_none() {
        let executable = path.with_extension("exe");
        if executable.is_file() {
            return Some(executable);
        }
    }
    None
}

fn hash_rustup_state(digest: &mut Sha256) -> Result<(), String> {
    let root = std::env::var_os("RUSTUP_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".rustup")))
        .ok_or_else(|| "the Rustup proxy has no identifiable RUSTUP_HOME or HOME".to_string())?;
    hash_rustup_state_at(digest, &root)
}

fn hash_rustup_state_at(digest: &mut Sha256, root: &Path) -> Result<(), String> {
    hash_external_file(digest, &root.join("settings.toml"))?;
    let update_hashes = root.join("update-hashes");
    let mut entries = fs::read_dir(&update_hashes)
        .map_err(|error| format!("reading `{}`: {error}", update_hashes.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("reading `{}`: {error}", update_hashes.display()))?;
    if entries.is_empty() {
        return Err(format!(
            "Rustup toolchain identity directory `{}` is empty",
            update_hashes.display()
        ));
    }
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        hash_external_file(digest, &entry.path())?;
    }
    Ok(())
}

fn hash_external_file(digest: &mut Sha256, path: &Path) -> Result<(), String> {
    let before =
        fs::metadata(path).map_err(|error| format!("identifying `{}`: {error}", path.display()))?;
    if !before.is_file() {
        return Err(format!("identity input `{}` is not a file", path.display()));
    }
    hash_permissions(digest, &before);
    let label = format!("identity input `{}`", path.display());
    let bytes = read_stable_file(path, &before, &label)?;
    hash_bytes(digest, &bytes);
    Ok(())
}

fn read_stable_file(path: &Path, before: &fs::Metadata, label: &str) -> Result<Vec<u8>, String> {
    read_stable_file_with(path, before, label, || {}, || {})
}

fn read_stable_file_with(
    path: &Path,
    before: &fs::Metadata,
    label: &str,
    after_read: impl FnOnce(),
    after_metadata: impl FnOnce(),
) -> Result<Vec<u8>, String> {
    let bytes = fs::read(path).map_err(|error| format!("reading {label}: {error}"))?;
    after_read();
    let after = fs::metadata(path).map_err(|error| format!("rechecking {label}: {error}"))?;
    if !same_file_observation(before, &after) {
        return Err(format!("{label} changed while it was read"));
    }
    after_metadata();
    let confirmed = fs::read(path).map_err(|error| format!("re-reading {label}: {error}"))?;
    if bytes != confirmed {
        return Err(format!("{label} changed while it was read"));
    }
    Ok(bytes)
}

fn same_file_observation(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    before.len() == after.len()
        && before.modified().ok() == after.modified().ok()
        && permissions(before) == permissions(after)
}

#[cfg(unix)]
fn permissions(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::MetadataExt as _;
    metadata.mode()
}

#[cfg(not(unix))]
fn permissions(metadata: &fs::Metadata) -> u32 {
    u32::from(metadata.permissions().readonly())
}

fn hash_permissions(digest: &mut Sha256, metadata: &fs::Metadata) {
    digest.update(permissions(metadata).to_le_bytes());
}

fn hash_path(digest: &mut Sha256, path: &Path) {
    hash_os(digest, path.as_os_str());
}

#[cfg(unix)]
fn hash_os(digest: &mut Sha256, value: &OsStr) {
    use std::os::unix::ffi::OsStrExt as _;
    hash_bytes(digest, value.as_bytes());
}

#[cfg(not(unix))]
fn hash_os(digest: &mut Sha256, value: &OsStr) {
    hash_bytes(digest, value.to_string_lossy().as_bytes());
}

fn hash_bytes(digest: &mut Sha256, bytes: &[u8]) {
    digest.update(bytes.len().to_le_bytes());
    digest.update(bytes);
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runner(program: &str) -> TestRunner {
        TestRunner {
            weight: None,
            exclusive: None,
            name: Some("unit".into()),
            dir: None,
            covers: None,
            covers_by_test: None,
            inputs_by_test: None,
            skipped: None,
            output_lines: None,
            max_output: None,
            expect_tests: None,
            max_memory: None,
            max_processes: None,
            wrap: None,
            run_selects: Some(true),
            list: vec![program.into()],
            run: vec![program.into(), "{}".into()],
            batch: None,
            timeout_seconds: None,
        }
    }

    /// Proof for `evidence.provider_executable_resolution_is_portable`: the
    /// execution-material boundary follows the native executable spelling,
    /// including Windows' implicit `.exe` suffix.
    #[test]
    fn material_provider_executable_resolution_follows_native_lookup() {
        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let git = resolve_program("git", root.path()).expect("Git executable on the test runner");
        assert!(git.is_file());
        #[cfg(windows)]
        assert_eq!(git.extension().and_then(OsStr::to_str), Some("exe"));
    }

    #[cfg(unix)]
    #[test]
    fn material_changes_with_declared_source_and_an_external_tool_binary() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let source = root.path().join("source.txt");
        let tool = root.path().join("tool.sh");
        fs::write(&source, "one\n").expect("source");
        fs::write(&tool, "#!/bin/sh\nexit 0\n").expect("tool");
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o700)).expect("mode");
        let mut configured = runner("sh");
        configured.run = vec![
            "sh".into(),
            tool.to_str().expect("utf8").into(),
            "{}".into(),
        ];
        let patterns = vec!["source.txt".to_string()];
        let first = test_material_identity(root.path(), &patterns, &configured).expect("first");

        fs::write(&source, "two\n").expect("source changes");
        let source_changed =
            test_material_identity(root.path(), &patterns, &configured).expect("source changed");
        assert_ne!(first.inputs_sha256, source_changed.inputs_sha256);

        fs::write(&tool, "#!/bin/sh\nexit 1\n").expect("tool changes");
        let tool_changed =
            test_material_identity(root.path(), &patterns, &configured).expect("tool changed");
        assert_ne!(
            source_changed.toolchain_sha256,
            tool_changed.toolchain_sha256
        );
    }

    #[cfg(unix)]
    #[test]
    fn runtime_identity_binds_provider_definition_and_executable_bytes() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let tool = root.path().join("focused-runner.sh");
        fs::write(&tool, "#!/bin/sh\nexit 0\n").expect("tool");
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o700)).expect("mode");
        let mut configured = runner("sh");
        configured.run = vec![
            "sh".into(),
            tool.to_str().expect("utf8").into(),
            "{}".into(),
        ];
        let first = test_runtime_identity(root.path(), &configured).expect("first identity");

        configured.name = Some("renamed".into());
        let definition_changed =
            test_runtime_identity(root.path(), &configured).expect("definition identity");
        assert_ne!(
            first.definition_sha256,
            definition_changed.definition_sha256
        );
        assert_ne!(first.sha256, definition_changed.sha256);

        fs::write(&tool, "#!/bin/sh\nexit 1\n").expect("tool changes");
        let tool_changed = test_runtime_identity(root.path(), &configured).expect("tool identity");
        assert_ne!(
            definition_changed.toolchain_sha256,
            tool_changed.toolchain_sha256
        );
        assert_ne!(definition_changed.sha256, tool_changed.sha256);
    }

    #[cfg(unix)]
    #[test]
    fn material_binds_the_input_policy_and_external_wrapper_script() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        fs::create_dir(root.path().join("src")).expect("source directory");
        fs::write(root.path().join("src/lib.rs"), "fn one() {}\n").expect("source");
        let wrapper = root.path().join("wrapper.sh");
        fs::write(&wrapper, "#!/bin/sh\nexec \"$@\"\n").expect("wrapper");
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).expect("mode");
        let mut configured = runner("true");
        configured.wrap = Some(vec!["sh".into(), wrapper.to_str().expect("utf8").into()]);

        let broad = test_material_identity(root.path(), &["src/**".into()], &configured)
            .expect("broad policy");
        let narrow = test_material_identity(root.path(), &["src/*.rs".into()], &configured)
            .expect("narrow policy matching the same current file");
        assert_ne!(broad.inputs_sha256, narrow.inputs_sha256);

        fs::write(&wrapper, "#!/bin/sh\nexit 99\n").expect("changed wrapper");
        let changed = test_material_identity(root.path(), &["src/**".into()], &configured)
            .expect("changed wrapper identity");
        assert_ne!(broad.toolchain_sha256, changed.toolchain_sha256);
    }

    #[test]
    fn missing_or_empty_material_is_refused() {
        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let error =
            test_material_identity(root.path(), &[], &runner("true")).expect_err("empty claim");
        assert!(error.contains("empty"), "{error}");
        let error = test_material_identity(root.path(), &["missing/**".into()], &runner("true"))
            .expect_err("empty match");
        assert!(error.contains("matched no regular files"), "{error}");
    }

    #[test]
    fn incremental_membership_uses_the_material_matcher_and_its_refusals() {
        let patterns = vec!["src/**".into(), "Cargo.toml".into()];
        assert!(test_material_includes_path(&patterns, "src/lib.rs").expect("source match"));
        assert!(test_material_includes_path(&patterns, "Cargo.toml").expect("file match"));
        assert!(!test_material_includes_path(&patterns, "README.md").expect("unrelated path"));
        for unsafe_pattern in ["../outside", "/absolute", "["] {
            let error = test_material_includes_path(&[unsafe_pattern.into()], "src/lib.rs")
                .expect_err("unsafe material pattern");
            assert!(
                error.contains("repository root") || error.contains("invalid material input"),
                "{unsafe_pattern}: {error}"
            );
        }
    }

    #[test]
    fn every_inherited_environment_value_changes_the_identity_without_being_disclosed() {
        let first = environment_identity_from([
            ("VISIBLE".into(), "one".into()),
            ("SECRET".into(), "credential-one".into()),
        ]);
        let changed = environment_identity_from([
            ("VISIBLE".into(), "one".into()),
            ("SECRET".into(), "credential-two".into()),
        ]);
        assert_ne!(first, changed);
        let rendered = hex(&first);
        assert!(!rendered.contains("credential"));
    }

    #[cfg(unix)]
    #[test]
    fn material_policy_refuses_escape_invalid_globs_and_symbolic_inputs() {
        use std::os::unix::fs::symlink;

        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        fs::write(root.path().join("source"), "content").expect("source");
        symlink("source", root.path().join("linked")).expect("symlink");

        for pattern in ["../outside", "/absolute", "["] {
            let error = test_material_identity(root.path(), &[pattern.into()], &runner("true"))
                .expect_err("unsafe or invalid pattern");
            assert!(
                error.contains("repository root") || error.contains("invalid material input"),
                "{pattern}: {error}"
            );
        }
        let error = test_material_identity(root.path(), &["linked".into()], &runner("true"))
            .expect_err("symbolic input");
        assert!(error.contains("symbolic link"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn tool_identity_covers_shebangs_file_arguments_and_rustup_state() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        fs::write(root.path().join("source"), "content").expect("source");
        fs::write(root.path().join("config"), "first").expect("config");
        let script = root.path().join("runner.sh");
        fs::write(&script, "#!/usr/bin/env sh\nexit 0\n").expect("script");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).expect("mode");

        let mut configured = runner("./runner.sh");
        configured.dir = Some(".".into());
        configured.run = vec!["./runner.sh".into(), "--config=config".into(), "{}".into()];
        let first = test_material_identity(root.path(), &["source".into()], &configured)
            .expect("complete identity");
        fs::write(root.path().join("config"), "second").expect("changed config");
        let changed = test_material_identity(root.path(), &["source".into()], &configured)
            .expect("changed identity");
        assert_ne!(first.toolchain_sha256, changed.toolchain_sha256);

        assert!(configured_file_argument("{}", root.path()).is_none());
        assert!(configured_file_argument("--config=", root.path()).is_none());
        assert_eq!(
            configured_file_argument("--config=config", root.path()),
            Some(root.path().join("config"))
        );
        assert_eq!(shebang_program(b"plain text"), None);
        assert_eq!(shebang_program(b"#!\xff\n"), None);
        assert_eq!(
            shebang_program(b"#!/usr/bin/env -S sh -eu\n"),
            Some("sh".into())
        );
        assert_eq!(shebang_program(b"#!/bin/sh\n"), Some("/bin/sh".into()));

        let mut empty = runner("true");
        empty.list.clear();
        empty.run.clear();
        assert!(toolchain_identity(root.path(), &empty).is_err());
        assert!(toolchain_identity(root.path(), &runner("./missing")).is_err());

        let rustup = root.path().join("rustup");
        fs::create_dir_all(rustup.join("update-hashes")).expect("rustup directories");
        fs::write(
            rustup.join("settings.toml"),
            "default_toolchain = 'stable'\n",
        )
        .expect("settings");
        let mut digest = Sha256::new();
        let error = hash_rustup_state_at(&mut digest, &rustup).expect_err("empty hashes");
        assert!(error.contains("is empty"), "{error}");
        fs::write(rustup.join("update-hashes/stable"), "digest").expect("update hash");
        hash_rustup_state_at(&mut digest, &rustup).expect("complete rustup identity");
        assert!(hash_external_file(&mut digest, &rustup).is_err());
        assert!(hash_external_file(&mut digest, &root.path().join("absent")).is_err());

        let mut hashed = BTreeSet::new();
        assert!(
            hash_executable(&mut digest, &rustup, root.path(), &mut hashed).is_err(),
            "directories are not executable identity files"
        );
        assert!(
            hash_executable(
                &mut digest,
                &root.path().join("absent"),
                root.path(),
                &mut hashed,
            )
            .is_err()
        );
    }

    #[test]
    fn stable_file_reads_detect_each_observable_race_boundary() {
        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let path = root.path().join("input");

        fs::write(&path, "before").expect("input");
        let before = fs::metadata(&path).expect("metadata");
        assert_eq!(
            read_stable_file(&path, &before, "input").expect("stable"),
            b"before"
        );

        fs::remove_file(&path).expect("remove");
        assert!(read_stable_file(&path, &before, "input").is_err());

        fs::write(&path, "before").expect("input");
        let before = fs::metadata(&path).expect("metadata");
        let error = read_stable_file_with(
            &path,
            &before,
            "input",
            || fs::remove_file(&path).expect("remove after read"),
            || {},
        )
        .expect_err("disappeared before metadata check");
        assert!(error.contains("rechecking"), "{error}");

        fs::write(&path, "before").expect("input");
        let before = fs::metadata(&path).expect("metadata");
        let error = read_stable_file_with(
            &path,
            &before,
            "input",
            || fs::write(&path, "longer value").expect("mutate after read"),
            || {},
        )
        .expect_err("metadata changed");
        assert!(error.contains("changed while"), "{error}");

        fs::write(&path, "before").expect("input");
        let before = fs::metadata(&path).expect("metadata");
        let error = read_stable_file_with(
            &path,
            &before,
            "input",
            || {},
            || fs::write(&path, "change").expect("mutate after metadata"),
        )
        .expect_err("bytes changed after metadata");
        assert!(error.contains("changed while"), "{error}");

        fs::write(&path, "before").expect("input");
        let before = fs::metadata(&path).expect("metadata");
        let error = read_stable_file_with(
            &path,
            &before,
            "input",
            || {},
            || fs::remove_file(&path).expect("remove after metadata"),
        )
        .expect_err("disappeared before confirmation read");
        assert!(error.contains("re-reading"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn non_file_inputs_are_ignored_but_empty_roots_are_refused() {
        let root = tempfile::Builder::new()
            .tempdir_in(aplan_core::fixture::fixture_root())
            .expect("fixture");
        let fifo = root.path().join("fifo");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .expect("mkfifo")
                .success()
        );
        let matcher = glob_set(&["fifo".into()]).expect("matcher");
        let mut files = BTreeMap::new();
        collect_inputs(root.path(), &fifo, &matcher, &mut files).expect("fifo is ignored");
        assert!(files.is_empty());
        assert!(
            test_material_identity(root.path(), &[String::new()], &runner("true"))
                .expect_err("rootless pattern")
                .contains("no repository-relative root")
        );
    }
}
