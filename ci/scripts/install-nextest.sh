#!/usr/bin/env bash
set -euo pipefail

# On a self-hosted job this is the first setup command. A hosted preparation
# step writes the same variable earlier, so it includes its work as well.
if [[ -n "${GITHUB_ENV:-}" && -z "${APLAN_VALIDATION_SETUP_STARTED_EPOCH:-}" ]]; then
    printf 'APLAN_VALIDATION_SETUP_STARTED_EPOCH=%s\n' "$(date +%s)" >> "$GITHUB_ENV"
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../nextest.env
source "$SCRIPT_DIR/../nextest.env"

usage() {
    echo "usage: ci/scripts/install-nextest.sh check|install [INSTALL_DIR]" >&2
    exit 2
}

[[ "$#" -ge 1 && "$#" -le 2 ]] || usage
mode="$1"
case "$mode" in
    check) [[ "$#" -eq 1 ]] || usage ;;
    install) [[ "$#" -le 2 ]] || usage ;;
    *) usage ;;
esac

for value in "$NEXTEST_X86_64_UNKNOWN_LINUX_GNU_SHA256" \
    "$NEXTEST_AARCH64_UNKNOWN_LINUX_GNU_SHA256" \
    "$NEXTEST_UNIVERSAL_APPLE_DARWIN_SHA256"; do
    if [[ ! "$value" =~ ^[0-9a-f]{64}$ ]]; then
        echo "error: ci/nextest.env contains an invalid cargo-nextest checksum" >&2
        exit 1
    fi
done
if [[ ! "$NEXTEST_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ \
    || ! "$NEXTEST_COMMIT" =~ ^[0-9a-f]{40}$ ]]; then
    echo "error: ci/nextest.env contains an invalid cargo-nextest version or commit" >&2
    exit 1
fi

platform="$(uname -s)"
machine="$(uname -m)"
case "$platform:$machine" in
    Linux:x86_64)
        target=x86_64-unknown-linux-gnu
        checksum="$NEXTEST_X86_64_UNKNOWN_LINUX_GNU_SHA256"
        ;;
    Linux:aarch64|Linux:arm64)
        target=aarch64-unknown-linux-gnu
        checksum="$NEXTEST_AARCH64_UNKNOWN_LINUX_GNU_SHA256"
        ;;
    Darwin:x86_64|Darwin:arm64)
        target=universal-apple-darwin
        checksum="$NEXTEST_UNIVERSAL_APPLE_DARWIN_SHA256"
        ;;
    *)
        echo "error: cargo-nextest ${NEXTEST_VERSION} has no configured archive for ${platform}/${machine}" >&2
        exit 1
        ;;
esac

check_installed() {
    local binary="$1" metadata release commit
    if [[ ! -x "$binary" ]]; then
        echo "error: cargo-nextest ${NEXTEST_VERSION} is required but '${binary}' is not executable" >&2
        return 1
    fi
    metadata="$($binary --version 2>&1)" || {
        echo "error: cannot query cargo-nextest at '${binary}'" >&2
        printf '%s\n' "$metadata" >&2
        return 1
    }
    release="$(sed -n 's/^release: //p' <<<"$metadata")"
    commit="$(sed -n 's/^commit-hash: //p' <<<"$metadata")"
    if [[ "$release" != "$NEXTEST_VERSION" || "$commit" != "$NEXTEST_COMMIT" ]]; then
        echo "error: cargo-nextest at '${binary}' is not the pinned build" >&2
        echo "expected release ${NEXTEST_VERSION}, commit ${NEXTEST_COMMIT}" >&2
        printf 'observed:\n%s\n' "$metadata" >&2
        return 1
    fi
    printf 'cargo-nextest: release=%s commit=%s target=%s\n' \
        "$NEXTEST_VERSION" "$NEXTEST_COMMIT" "$target"
}

if [[ "$mode" == check ]]; then
    binary="$(command -v cargo-nextest || true)"
    if [[ -z "$binary" ]]; then
        echo "error: cargo-nextest ${NEXTEST_VERSION} is required; run ci/scripts/install-nextest.sh install" >&2
        exit 1
    fi
    check_installed "$binary"
    exit 0
fi

install_dir="${2:-${CARGO_HOME:-$HOME/.cargo}/bin}"
binary="$install_dir/cargo-nextest"
if check_installed "$binary" >/dev/null 2>&1; then
    check_installed "$binary"
    exit 0
fi

for command in curl tar install mktemp; do
    if ! command -v "$command" >/dev/null 2>&1; then
        echo "error: installing cargo-nextest requires '${command}'" >&2
        exit 1
    fi
done

verify_checksum() {
    local expected="$1" archive="$2" observed
    if command -v sha256sum >/dev/null 2>&1; then
        printf '%s  %s\n' "$expected" "$archive" | sha256sum -c -
        return
    fi
    if command -v shasum >/dev/null 2>&1; then
        observed="$(shasum -a 256 "$archive" | awk '{print $1}')"
        if [[ "$observed" == "$expected" ]]; then
            printf '%s: OK\n' "$archive"
            return
        fi
        echo "${archive}: computed checksum did NOT match" >&2
        return 1
    fi
    echo "error: installing cargo-nextest requires 'sha256sum' or 'shasum'" >&2
    return 1
}

scratch_root="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
if [[ ! -d "$scratch_root" ]]; then
    echo "error: cargo-nextest scratch directory does not exist: ${scratch_root}" >&2
    exit 1
fi
work="$(mktemp -d "$scratch_root/aplan-nextest.XXXXXXXX")"
trap 'rm -rf "$work"' EXIT
archive="cargo-nextest-${NEXTEST_VERSION}-${target}.tar.gz"
curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error --retry 3 \
    --output "$work/$archive" \
    "https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-${NEXTEST_VERSION}/${archive}"
verify_checksum "$checksum" "$work/$archive"
tar -xzf "$work/$archive" -C "$work" cargo-nextest
install -d "$install_dir"
install -m 0755 "$work/cargo-nextest" "$binary"
check_installed "$binary"
