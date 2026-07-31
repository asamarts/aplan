//! `aplan check` must stay hermetic (HLD §5.3): no subprocess, no socket, no
//! git. That property is what lets it run on every keystroke, in a pre-commit
//! hook, and offline, and it is the line that separates the fast L0 path from
//! the repo-opening `verify`.
//!
//! We prove it structurally. The check path lives entirely in `aplan-core`
//! (load + validate) and `aplan-output` (render); neither may name a process,
//! network, or git API. With `unsafe_code = forbid` set workspace-wide there is
//! no raw-syscall escape hatch, so "names none of these APIs" is equivalent to
//! "cannot spawn or connect", a real guarantee, not a proxy. `verify`'s git
//! access lives in `aplan-evidence`, which is deliberately not on this list.

use std::path::{Path, PathBuf};

/// APIs that could spawn a process or open a socket. Specific paths, so a
/// comment mentioning "subprocess" in prose does not trip the check.
const FORBIDDEN: &[&str] = &[
    "std::process",
    "process::Command",
    "std::net",
    "TcpStream",
    "TcpListener",
    "UdpSocket",
    "std::os::unix::net",
    "git2",
];

fn rs_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).expect("read crate src").flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(rs_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}

/// The part of a line that is code, i.e. before any `//` (which also covers the
/// `///` and `//!` doc forms). Crude but sufficient, the crates use no block
/// comments, and a forbidden API only matters when it is actually called.
fn code_of(line: &str) -> &str {
    line.split_once("//").map_or(line, |(code, _)| code)
}

#[test]
fn check_path_names_no_process_or_socket_api() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offences = Vec::new();

    for crate_name in ["aplan-core", "aplan-output"] {
        let src = manifest.join("..").join(crate_name).join("src");
        for file in rs_files(&src) {
            let text = std::fs::read_to_string(&file).expect("read source");
            for (n, line) in text.lines().enumerate() {
                let code = code_of(line);
                for needle in FORBIDDEN {
                    if code.contains(needle) {
                        offences.push(format!(
                            "{}:{} names `{needle}`, the check path must stay hermetic",
                            file.display(),
                            n + 1
                        ));
                    }
                }
            }
        }
    }

    assert!(
        offences.is_empty(),
        "`aplan check` hermeticity broken:\n  {}",
        offences.join("\n  ")
    );
}
