#!/usr/bin/env bash
set -euo pipefail

if [[ -n "${GITHUB_ENV:-}" && -z "${APLAN_VALIDATION_SETUP_STARTED_EPOCH:-}" ]]; then
    printf 'APLAN_VALIDATION_SETUP_STARTED_EPOCH=%s\n' "$(date +%s)" >> "$GITHUB_ENV"
fi

# Install only what a clean standard GitHub-hosted runner lacks for one of this
# repository's gates. Workflow steps call this only when
# `runner.environment == 'github-hosted'`; tools shared by private and hosted
# paths, such as nextest, are prepared by an unconditional workflow step.

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
requested_profile="${1:-}"

resolve_profile() {
    case "$1" in
        development) echo test ;;
        full) echo coverage ;;
        base|test|coverage|msrv|release|latest-stable|portability) echo "$1" ;;
        *) return 1 ;;
    esac
}

# Validation profiles compose capabilities. Release runs every gate, including
# coverage, and also needs its publication toolchain. Keep that composition in
# one executable decision so a clean runner cannot prepare only the final
# artifact step while omitting a tool required by an earlier gate.
profile_needs_coverage() {
    case "$1" in
        coverage|release) return 0 ;;
        *) return 1 ;;
    esac
}

resolve_capabilities() {
    local resolved
    resolved="$(resolve_profile "$1")" || return 1
    if profile_needs_coverage "$resolved"; then
        echo coverage
    fi
    if [[ "$resolved" == "release" ]]; then
        echo release
    fi
}

if [[ "$requested_profile" == "--resolve-profile" ]]; then
    [[ "$#" -eq 2 ]] || {
        echo "usage: ci/scripts/prepare-hosted.sh --resolve-profile PROFILE" >&2
        exit 2
    }
    resolve_profile "$2" || {
        echo "error: unknown hosted preparation profile '$2'" >&2
        exit 2
    }
    exit 0
fi

if [[ "$requested_profile" == "--resolve-capabilities" ]]; then
    [[ "$#" -eq 2 ]] || {
        echo "usage: ci/scripts/prepare-hosted.sh --resolve-capabilities PROFILE" >&2
        exit 2
    }
    resolve_capabilities "$2" || {
        echo "error: unknown hosted preparation profile '$2'" >&2
        exit 2
    }
    exit 0
fi

[[ "$#" -eq 1 ]] || {
    echo "usage: ci/scripts/prepare-hosted.sh {development|full|base|test|coverage|msrv|release|latest-stable|portability}" >&2
    exit 2
}
profile="$(resolve_profile "$requested_profile")" || {
    echo "usage: ci/scripts/prepare-hosted.sh {development|full|base|test|coverage|msrv|release|latest-stable|portability}" >&2
    exit 2
}

# `python3` drives the committed read-performance harness from the ordinary
# workspace test suite. Standard hosted images currently provide it, but that
# is still an input to our gate and must fail here before repository work if a
# future image removes it.
if [[ "$profile" != "portability" ]]; then
    for command in rustup curl sha256sum tar python3 cmp install; do
        if ! command -v "$command" >/dev/null 2>&1; then
            echo "error: a GitHub-hosted runner is expected to provide '$command'" >&2
            exit 1
        fi
    done
else
    # The portability profile compiles only the shared TUI library suite and
    # one selected integration module. Requiring Linux-only archive and install
    # tools on Windows would turn an irrelevant image detail into the verdict.
    command -v rustup >/dev/null 2>&1 || {
        echo "error: a GitHub-hosted portability runner is expected to provide 'rustup'" >&2
        exit 1
    }
fi

# Ordinary private and hosted jobs must build the same artifact shape, so they
# install one fail-closed Cargo policy. The portability matrix intentionally
# uses each platform's native linker and keeps only the pinned Rust contract.
if [[ "$profile" != "portability" ]]; then
    "$REPO_ROOT/ci/scripts/install-cargo-ci-config.sh"
fi

# The MSRV and compatibility gates select their own intentionally different
# compilers. Every authoritative profile derives the exact compiler and its
# required components from the committed toolchain contract.
toolchain="$("$REPO_ROOT/ci/scripts/toolchain.sh" channel)"
components="$("$REPO_ROOT/ci/scripts/toolchain.sh" components)"
if [[ "$profile" != "msrv" && "$profile" != "latest-stable" ]]; then
    rustup toolchain install "$toolchain" \
        --profile minimal \
        --component "$components" \
        --no-self-update
    rustup default "$toolchain"
    "$REPO_ROOT/ci/scripts/toolchain.sh" verify
fi

if profile_needs_coverage "$profile"; then
    rustup component add llvm-tools-preview --toolchain "$toolchain"

    # Derive the pin from the private runner image. A second handwritten
    # version here would let the two environments measure coverage with
    # different tools while both workflows remained green.
    version="$(sed -n 's/^ARG LLVM_COV_VERSION=//p' "$REPO_ROOT/ci/Containerfile")"
    if [[ -z "$version" ]] || [[ "$(printf '%s\n' "$version" | wc -l)" -ne 1 ]]; then
        echo "error: ci/Containerfile must declare exactly one LLVM_COV_VERSION" >&2
        exit 1
    fi

    installed="$(cargo llvm-cov --version 2>/dev/null || true)"
    if [[ "$installed" != "cargo-llvm-cov ${version}" ]]; then
        case "$(uname -m)" in
            x86_64)
                arch=x86_64
                checksum_arg=LLVM_COV_X86_64_SHA256
                ;;
            aarch64|arm64)
                arch=aarch64
                checksum_arg=LLVM_COV_AARCH64_SHA256
                ;;
            *)
                echo "error: cargo-llvm-cov has no configured archive for $(uname -m)" >&2
                exit 1
                ;;
        esac
        checksum="$(sed -n "s/^ARG ${checksum_arg}=//p" "$REPO_ROOT/ci/Containerfile")"
        if [[ ! "$checksum" =~ ^[0-9a-f]{64}$ ]]; then
            echo "error: ci/Containerfile must declare one valid ${checksum_arg}" >&2
            exit 1
        fi
        work="$(mktemp -d)"
        trap 'rm -rf "$work"' EXIT
        curl --fail --location --silent --show-error --retry 3 \
            --output "$work/llvm-cov.tar.gz" \
            "https://github.com/taiki-e/cargo-llvm-cov/releases/download/v${version}/cargo-llvm-cov-${arch}-unknown-linux-gnu.tar.gz"
        printf '%s  %s\n' "$checksum" "$work/llvm-cov.tar.gz" | sha256sum -c -
        tar -xzf "$work/llvm-cov.tar.gz" -C "$work"
        install -m 0755 "$work/cargo-llvm-cov" "${CARGO_HOME:-$HOME/.cargo}/bin/cargo-llvm-cov"
    fi
    cargo llvm-cov --version
fi

if [[ "$profile" == "release" ]]; then
    rustup target add x86_64-unknown-linux-musl --toolchain "$toolchain"
    sudo apt-get update -qq
    sudo apt-get install -y --no-install-recommends musl-tools
    for command in gh jq; do
        command -v "$command" >/dev/null 2>&1 || {
            echo "error: a GitHub-hosted release runner is expected to provide '$command'" >&2
            exit 1
        }
    done
fi

echo "hosted runner prepared for ${requested_profile} (${profile})"
