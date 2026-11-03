#!/usr/bin/env bash
# Observational compatibility against Rust's moving stable channel (ADR-0052).
# This is deliberately not a validation profile and never builds a release.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

for command in rustup rustc cargo; do
    command -v "$command" >/dev/null 2>&1 || {
        echo "latest-stable: required command '$command' is unavailable" >&2
        exit 1
    }
done

pinned="$("$SCRIPT_DIR/toolchain.sh" channel)"
components="$("$SCRIPT_DIR/toolchain.sh" components)"
rustup toolchain install stable \
    --profile minimal \
    --component "$components" \
    --no-self-update

latest_details="$(rustup run stable rustc --version --verbose)"
latest="$(sed -n 's/^release: //p' <<<"$latest_details")"
host="$(sed -n 's/^host: //p' <<<"$latest_details")"
commit="$(sed -n 's/^commit-hash: //p' <<<"$latest_details")"
[[ -n "$latest" && -n "$host" && -n "$commit" ]] || {
    echo "latest-stable: rustc returned an incomplete identity" >&2
    printf '%s\n' "$latest_details" >&2
    exit 1
}

echo "latest-stable: pinned=$pinned observed=$latest host=$host commit=$commit components=$components"
if [[ "$latest" == "$pinned" ]]; then
    echo "latest-stable: the compatibility compiler currently equals the pin"
else
    echo "latest-stable: review whether rust-toolchain.toml should move from $pinned to $latest"
    if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
        echo "::notice title=Rust toolchain upgrade available::Pinned $pinned; latest stable is $latest. Review the compatibility result and make an explicit upgrade decision."
    fi
fi

# RUSTUP_TOOLCHAIN intentionally overrides the exact repository file only in
# this observational process. Authoritative entry points reject this moving
# name through toolchain.sh verify.
export RUSTUP_TOOLCHAIN=stable
export CARGO_BUILD_WARNINGS=deny

echo "==> latest-stable: cargo fmt --all --check"
cargo fmt --all --check
echo "==> latest-stable: cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings
echo "==> latest-stable: cargo doc --workspace --no-deps"
RUSTDOCFLAGS="${RUSTDOCFLAGS:--D warnings}" cargo doc --workspace --no-deps
echo "==> latest-stable: cargo test --workspace"
cargo test --workspace
echo "latest-stable: compatibility checks passed on Rust $latest; this is not a full or release receipt"
