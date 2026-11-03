#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

# Does this workspace's own source compile with the compiler it promises?
# (`A1-330`)
#
# `cargo xtask msrv` checks the other half: that no dependency in the locked
# graph declares a floor above ours. It needs no toolchain, because every
# package states what it wants and cargo has already resolved the graph. What it
# cannot see is us. Nothing stops a `1.92` feature landing in `aplan-core`
# tomorrow -- every dependency would still declare `1.88`, that gate would stay
# green, and the first to learn otherwise would be a source adopter on the
# promised compiler. That is `A1-327`'s defect moved from the dependencies to us.
#
# **The toolchain is derived, never written down twice.** A second copy of the
# MSRV is the same defect one file over, which is the whole subject of `A1-327`:
# this reads `rust-version` out of `Cargo.toml` and asks for exactly that.
MSRV="$(sed -n 's/^rust-version = "\([0-9.]*\)"/\1/p' Cargo.toml | head -1)"
if [[ -z "$MSRV" ]]; then
    echo "error: no rust-version in Cargo.toml, so there is no promise to check" >&2
    exit 1
fi

# **No dev-dependencies**, which is not a shortcut. `rust-version` is a promise
# to somebody running `cargo build` on this as a dependency, and that never
# compiles a test-only crate -- the same boundary `cargo xtask msrv` draws when
# it excludes dev-only packages from the graph's floor. A dev-dependency needing
# more is a real problem for contributors, and the ordinary test job meets it
# directly by failing to build.
#
# Its own target directory: switching compilers in a shared one invalidates
# every artifact, so this would silently double the cost of the next stable
# build. Measured on this workspace: 27s cold, under a second warm.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}/msrv"
# Cargo at the 1.88 consumer floor predates `build.warnings` and silently
# ignores `CARGO_BUILD_WARNINGS`. This isolated target therefore keeps the one
# intentional rustflag exception (`A1-631`).
export RUSTFLAGS="-D warnings"

if ! rustup toolchain list 2>/dev/null | grep -q "^${MSRV}"; then
    if ! command -v rustup >/dev/null 2>&1; then
        echo "error: rustc ${MSRV} is needed to check the declared MSRV and rustup is not" \
             "installed, so this gate cannot run. A gate that cannot run must say so rather" \
             "than pass." >&2
        exit 1
    fi
    echo "==> rustup toolchain install ${MSRV} (the declared MSRV, not installed here)"
    rustup toolchain install "${MSRV}" --profile minimal --no-self-update
fi

echo "==> cargo +${MSRV} check --locked --workspace (the declared MSRV)"
cargo "+${MSRV}" check --locked --workspace
echo "msrv-build: this workspace's own source compiles with rustc ${MSRV}"
