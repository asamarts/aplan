#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=ci/deny.env
source "$SCRIPT_DIR/../deny.env"

usage() {
    echo "usage: ci/scripts/install-cargo-deny.sh check|install [INSTALL_DIR]" >&2
    exit 2
}

[[ "$#" -ge 1 && "$#" -le 2 ]] || usage
mode="$1"
case "$mode" in
    check) [[ "$#" -eq 1 ]] || usage ;;
    install) ;;
    *) usage ;;
esac

if [[ ! "$CARGO_DENY_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "error: ci/deny.env contains an invalid cargo-deny version" >&2
    exit 1
fi
for value in "$CARGO_DENY_X86_64_UNKNOWN_LINUX_MUSL_SHA256" \
    "$CARGO_DENY_AARCH64_UNKNOWN_LINUX_MUSL_SHA256" \
    "$CARGO_DENY_X86_64_APPLE_DARWIN_SHA256" \
    "$CARGO_DENY_AARCH64_APPLE_DARWIN_SHA256"; do
    if [[ ! "$value" =~ ^[0-9a-f]{64}$ ]]; then
        echo "error: ci/deny.env contains an invalid cargo-deny checksum" >&2
        exit 1
    fi
done

platform="$(uname -s)"
machine="$(uname -m)"
case "$platform:$machine" in
    Linux:x86_64)
        target=x86_64-unknown-linux-musl
        checksum="$CARGO_DENY_X86_64_UNKNOWN_LINUX_MUSL_SHA256"
        ;;
    Linux:aarch64|Linux:arm64)
        target=aarch64-unknown-linux-musl
        checksum="$CARGO_DENY_AARCH64_UNKNOWN_LINUX_MUSL_SHA256"
        ;;
    Darwin:x86_64)
        target=x86_64-apple-darwin
        checksum="$CARGO_DENY_X86_64_APPLE_DARWIN_SHA256"
        ;;
    Darwin:aarch64|Darwin:arm64)
        target=aarch64-apple-darwin
        checksum="$CARGO_DENY_AARCH64_APPLE_DARWIN_SHA256"
        ;;
    *)
        echo "error: cargo-deny ${CARGO_DENY_VERSION} has no configured archive for ${platform}/${machine}" >&2
        exit 1
        ;;
esac

check_installed() {
    local binary="$1" observed
    if [[ ! -x "$binary" ]]; then
        echo "error: cargo-deny ${CARGO_DENY_VERSION} is required but '${binary}' is not executable" >&2
        return 1
    fi
    observed="$("$binary" --version 2>&1)" || {
        echo "error: cannot query cargo-deny at '${binary}'" >&2
        printf '%s\n' "$observed" >&2
        return 1
    }
    if [[ "$observed" != "cargo-deny ${CARGO_DENY_VERSION}" ]]; then
        echo "error: cargo-deny at '${binary}' is not the pinned build" >&2
        echo "expected cargo-deny ${CARGO_DENY_VERSION}; observed ${observed}" >&2
        return 1
    fi
    printf 'cargo-deny: version=%s target=%s\n' "$CARGO_DENY_VERSION" "$target"
}

if [[ "$mode" == "check" ]]; then
    binary="$(command -v cargo-deny || true)"
    if [[ -z "$binary" ]]; then
        echo "error: cargo-deny ${CARGO_DENY_VERSION} is required; run ci/scripts/install-cargo-deny.sh install" >&2
        exit 1
    fi
    check_installed "$binary"
    exit 0
fi

install_dir="${2:-${CARGO_HOME:-$HOME/.cargo}/bin}"
binary="$install_dir/cargo-deny"
if check_installed "$binary" >/dev/null 2>&1; then
    check_installed "$binary"
    exit 0
fi

for command in curl tar install mktemp; do
    command -v "$command" >/dev/null 2>&1 || {
        echo "error: installing cargo-deny requires '${command}'" >&2
        exit 1
    }
done

verify_checksum() {
    local expected="$1" archive="$2" observed
    if command -v sha256sum >/dev/null 2>&1; then
        printf '%s  %s\n' "$expected" "$archive" | sha256sum -c -
        return
    fi
    if command -v shasum >/dev/null 2>&1; then
        observed="$(shasum -a 256 "$archive" | awk '{print $1}')"
        [[ "$observed" == "$expected" ]] || {
            echo "${archive}: computed checksum did NOT match" >&2
            return 1
        }
        printf '%s: OK\n' "$archive"
        return
    fi
    echo "error: installing cargo-deny requires 'sha256sum' or 'shasum'" >&2
    return 1
}

scratch_root="${RUNNER_TEMP:-${APLAN_SCRATCH_ROOT:-${HOME}/.tmp}}"
[[ -d "$scratch_root" ]] || {
    echo "error: cargo-deny scratch directory does not exist: ${scratch_root}" >&2
    exit 1
}
work="$(mktemp -d "$scratch_root/aplan-cargo-deny.XXXXXXXX")"
trap 'rm -rf "$work"' EXIT
archive="cargo-deny-${CARGO_DENY_VERSION}-${target}.tar.gz"
directory="cargo-deny-${CARGO_DENY_VERSION}-${target}"
curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error --retry 3 \
    --output "$work/$archive" \
    "https://github.com/EmbarkStudios/cargo-deny/releases/download/${CARGO_DENY_VERSION}/${archive}"
verify_checksum "$checksum" "$work/$archive"
tar -xzf "$work/$archive" -C "$work" "$directory/cargo-deny"
install -d "$install_dir"
install -m 0755 "$work/$directory/cargo-deny" "$binary"
check_installed "$binary"
