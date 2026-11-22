use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use sha2::{Digest as _, Sha256};

mod build_support;

fn main() {
    println!("cargo:rerun-if-env-changed=CARGO_MANIFEST_DIR");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");

    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let (root, inputs) = build_support::collect_cache_identity_inputs(&manifest);

    let mut digest = Sha256::new();
    digest.update(b"aplan-cache-writer-v2\0");
    if let Some(version) = env::var_os("CARGO_PKG_VERSION") {
        digest.update(b"version\0");
        digest.update(version.as_encoded_bytes());
        digest.update([0]);
    }
    for relative in inputs {
        let full = root.join(&relative);
        println!("cargo:rerun-if-changed={}", full.display());
        let normalized = build_support::portable_relative_path(&relative);
        digest.update(normalized.as_bytes());
        digest.update([0]);
        digest.update(fs::read(&full).expect("read cache identity input"));
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

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}
