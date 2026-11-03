use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest as _, Sha256};

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let workspace = manifest
        .parent()
        .and_then(Path::parent)
        .expect("aplan crate is inside the workspace");
    let mut inputs = Vec::new();
    collect_sources(workspace, workspace, &mut inputs);
    inputs.sort();

    let mut digest = Sha256::new();
    digest.update(b"aplan-cache-writer-v1\0");
    for relative in inputs {
        println!(
            "cargo:rerun-if-changed={}",
            workspace.join(&relative).display()
        );
        digest.update(relative.as_os_str().as_encoded_bytes());
        digest.update([0]);
        digest.update(fs::read(workspace.join(relative)).expect("read cache identity input"));
        digest.update([0]);
    }

    let mut environment = env::vars_os()
        .filter(|(name, _)| {
            let name = name.to_string_lossy();
            name == "HOST"
                || name == "TARGET"
                || name == "PROFILE"
                || name == "OPT_LEVEL"
                || name == "DEBUG"
                || name == "RUSTC"
                || name == "RUSTC_WRAPPER"
                || name == "RUSTC_WORKSPACE_WRAPPER"
                || name == "RUSTFLAGS"
                || name == "CARGO_ENCODED_RUSTFLAGS"
                || name.starts_with("CARGO_CFG_")
                || name.starts_with("CARGO_FEATURE_")
        })
        .collect::<Vec<_>>();
    environment.sort_by(|left, right| left.0.cmp(&right.0));
    for (name, value) in environment {
        println!("cargo:rerun-if-env-changed={}", name.to_string_lossy());
        digest.update(name.as_encoded_bytes());
        digest.update([0]);
        digest.update(value.as_encoded_bytes());
        digest.update([0]);
    }

    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let rustc_identity = Command::new(rustc)
        .arg("-Vv")
        .output()
        .expect("run rustc to identify the cache writer");
    assert!(rustc_identity.status.success(), "rustc -Vv failed");
    digest.update(b"rustc\0");
    digest.update(&rustc_identity.stdout);

    println!(
        "cargo:rustc-env=APLAN_CACHE_WRITER_IDENTITY=semantic-sha256:{}",
        hex(&digest.finalize())
    );
}

fn collect_sources(workspace: &Path, directory: &Path, output: &mut Vec<PathBuf>) {
    let mut entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
        .map(|entry| entry.expect("read workspace entry").path())
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
        let relative = path.strip_prefix(workspace).expect("workspace child");
        if path.is_dir() {
            if matches!(
                relative
                    .components()
                    .next()
                    .and_then(|part| part.as_os_str().to_str()),
                Some("target" | ".git" | "aplan" | "docs" | "ci")
            ) {
                continue;
            }
            collect_sources(workspace, &path, output);
            continue;
        }
        let file_name = path.file_name().and_then(|name| name.to_str());
        let is_manifest = matches!(file_name, Some("Cargo.toml" | "Cargo.lock" | "build.rs"));
        let is_rust = path.extension().is_some_and(|extension| extension == "rs");
        if is_manifest || is_rust {
            output.push(relative.to_path_buf());
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}
