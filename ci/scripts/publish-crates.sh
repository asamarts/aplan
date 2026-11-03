#!/usr/bin/env bash
set -euo pipefail

# Publish every public workspace package in dependency order. A rerun packages
# the exact source again and accepts an existing version only when crates.io
# records the same archive checksum.

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

PUBLISH_ORDER=(
    aplan-core
    aplan-output
    aplan-config
    aplan-evidence
    aplan-interop
    aplan-attest
    aplan-app
    aplan-tui
    aplan-mcp
    aplan
)

release_source="$(ci/scripts/release-source.sh "$REPO_ROOT")"
export APLAN_RELEASE_SHA="$release_source"
version="$(ci/scripts/current-version.sh)"

if [[ -z "${TAG:-}" || "$TAG" != "v${version}" || "$version" == *-* ]]; then
    echo "ERROR: crates.io publication requires TAG=v${version} and a release version" >&2
    exit 1
fi
ci/scripts/check-release-changelog.sh "$version"

for command in cargo curl jq sha256sum; do
    command -v "$command" >/dev/null 2>&1 || {
        echo "ERROR: crates.io publication requires '$command'" >&2
        exit 1
    }
done

registry_checksum() {
    local crate="$1"
    local crate_version="$2"
    local response status body
    response="$(curl --silent --show-error --retry 3 \
        --user-agent 'aplan-release/1 (https://github.com/asamarts/aplan)' \
        --write-out $'\n%{http_code}' \
        "https://crates.io/api/v1/crates/${crate}/${crate_version}")"
    status="${response##*$'\n'}"
    body="${response%$'\n'*}"
    case "$status" in
        200)
            if ! jq --exit-status --raw-output '.version.checksum' <<<"$body"; then
                echo "ERROR: crates.io returned no checksum for ${crate}@${crate_version}" >&2
                return 2
            fi
            ;;
        404) return 1 ;;
        *)
            echo "ERROR: crates.io returned HTTP ${status} for ${crate}@${crate_version}" >&2
            return 2
            ;;
    esac
}

wait_for_registry() {
    local crate="$1"
    local crate_version="$2"
    local expected="$3"
    local attempts="${APLAN_REGISTRY_ATTEMPTS:-60}"
    local interval="${APLAN_REGISTRY_INTERVAL_SECONDS:-5}"
    local observed="" status
    for ((attempt = 1; attempt <= attempts; attempt++)); do
        set +e
        observed="$(registry_checksum "$crate" "$crate_version")"
        status=$?
        set -e
        case "$status" in
            0)
                if [[ "$observed" == "$expected" ]]; then
                    if cargo info "${crate}@${crate_version}" \
                        --registry crates-io >/dev/null 2>&1; then
                        echo "==> crates.io confirms and resolves ${crate}@${crate_version} (${expected})"
                        return 0
                    fi
                    echo "==> ${crate}@${crate_version} has the expected checksum but is not yet resolvable"
                    sleep "$interval"
                    continue
                fi
                echo "ERROR: crates.io checksum for ${crate}@${crate_version} is ${observed}, expected ${expected}" >&2
                return 1
                ;;
            1) ;;
            *) return "$status" ;;
        esac
        sleep "$interval"
    done
    echo "ERROR: crates.io did not expose ${crate}@${crate_version} after ${attempts} checks" >&2
    return 1
}

package_dir="${CARGO_TARGET_DIR:-target}/package"
for crate in "${PUBLISH_ORDER[@]}"; do
    echo "==> Packaging ${crate}@${version}"
    cargo package --locked --no-verify --registry crates-io -p "$crate"
    archive="${package_dir}/${crate}-${version}.crate"
    if [[ ! -f "$archive" ]]; then
        echo "ERROR: cargo did not produce ${archive}" >&2
        exit 1
    fi
    expected="$(sha256sum "$archive" | awk '{print $1}')"
    set +e
    observed="$(registry_checksum "$crate" "$version")"
    status=$?
    set -e
    case "$status" in
        0)
            if [[ "$observed" != "$expected" ]]; then
                echo "ERROR: existing ${crate}@${version} has checksum ${observed}, expected ${expected}" >&2
                exit 1
            fi
            echo "==> ${crate}@${version} already matches crates.io (${expected})"
            wait_for_registry "$crate" "$version" "$expected"
            continue
            ;;
        1) ;;
        *) exit "$status" ;;
    esac

    echo "==> Publishing ${crate}@${version}"
    cargo publish --locked --registry crates-io -p "$crate"
    wait_for_registry "$crate" "$version" "$expected"
done

source_after="$(ci/scripts/release-source.sh "$REPO_ROOT")"
if [[ "$source_after" != "$release_source" ]]; then
    echo "ERROR: release source changed during crates.io publication" >&2
    exit 1
fi
