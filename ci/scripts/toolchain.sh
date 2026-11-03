#!/usr/bin/env bash
# Read and verify the repository's one development/release toolchain contract.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
DEFAULT_MANIFEST="$REPO_ROOT/rust-toolchain.toml"

usage() {
    cat >&2 <<'EOF'
usage: ci/scripts/toolchain.sh {contract|channel|components|cache-key|verify} [rust-toolchain.toml]
EOF
    exit 2
}

read_contract() {
    local manifest="$1"
    local -a channel_lines component_lines profile_lines components
    local component_line component_csv component seen_csv line

    [[ -f "$manifest" ]] || {
        echo "toolchain: contract file does not exist: $manifest" >&2
        return 1
    }
    channel_lines=()
    while IFS= read -r line; do
        channel_lines+=("$line")
    done < <(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)"[[:space:]]*$/\1/p' "$manifest")
    if [[ "${#channel_lines[@]}" -ne 1 ]]; then
        echo "toolchain: $manifest must declare exactly one channel" >&2
        return 1
    fi
    TOOLCHAIN_CHANNEL="${channel_lines[0]}"
    if ! [[ "$TOOLCHAIN_CHANNEL" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        echo "toolchain: channel must be an exact Rust release, not '$TOOLCHAIN_CHANNEL'" >&2
        return 1
    fi

    profile_lines=()
    while IFS= read -r line; do
        profile_lines+=("$line")
    done < <(sed -n 's/^[[:space:]]*profile[[:space:]]*=[[:space:]]*"\([^"]*\)"[[:space:]]*$/\1/p' "$manifest")
    if [[ "${#profile_lines[@]}" -ne 1 || "${profile_lines[0]}" != "minimal" ]]; then
        echo "toolchain: $manifest must declare exactly one minimal profile" >&2
        return 1
    fi

    component_lines=()
    while IFS= read -r line; do
        component_lines+=("$line")
    done < <(sed -n '/^[[:space:]]*components[[:space:]]*=/p' "$manifest")
    if [[ "${#component_lines[@]}" -ne 1 ]]; then
        echo "toolchain: $manifest must declare exactly one components list" >&2
        return 1
    fi
    component_line="${component_lines[0]}"
    if ! grep -Eq '^[[:space:]]*components[[:space:]]*=[[:space:]]*\["[a-z0-9_-]+"(,[[:space:]]*"[a-z0-9_-]+")*\][[:space:]]*$' <<<"$component_line"; then
        echo "toolchain: components must be one non-empty TOML string array" >&2
        return 1
    fi
    component_csv="$(sed -E 's/^[^[]*\[//; s/\][[:space:]]*$//; s/[[:space:]\"]//g' <<<"$component_line")"
    IFS=',' read -r -a components <<<"$component_csv"
    seen_csv=","
    for component in "${components[@]}"; do
        if [[ "$seen_csv" == *",${component},"* ]]; then
            echo "toolchain: component '$component' is declared more than once" >&2
            return 1
        fi
        seen_csv="${seen_csv}${component},"
    done
    for component in rustfmt clippy; do
        if [[ "$seen_csv" != *",${component},"* ]]; then
            echo "toolchain: required component '$component' is missing" >&2
            return 1
        fi
    done
    TOOLCHAIN_COMPONENTS="$component_csv"
    TOOLCHAIN_CACHE_KEY="rust-${TOOLCHAIN_CHANNEL}-${TOOLCHAIN_COMPONENTS//,/-}"
}

command="${1:-}"
[[ "$#" -ge 1 && "$#" -le 2 ]] || usage
manifest="${2:-$DEFAULT_MANIFEST}"
read_contract "$manifest"

case "$command" in
    contract)
        printf 'channel=%s components=%s cache-key=%s\n' \
            "$TOOLCHAIN_CHANNEL" "$TOOLCHAIN_COMPONENTS" "$TOOLCHAIN_CACHE_KEY"
        ;;
    channel)
        printf '%s\n' "$TOOLCHAIN_CHANNEL"
        ;;
    components)
        printf '%s\n' "$TOOLCHAIN_COMPONENTS"
        ;;
    cache-key)
        printf '%s\n' "$TOOLCHAIN_CACHE_KEY"
        ;;
    verify)
        [[ "$manifest" == "$DEFAULT_MANIFEST" ]] || {
            echo "toolchain: verify accepts only the repository contract" >&2
            exit 2
        }
        command -v rustup >/dev/null 2>&1 || {
            echo "toolchain: rustup is required to verify the pinned toolchain" >&2
            exit 1
        }
        cd "$REPO_ROOT"
        active="$(rustup show active-toolchain | awk '{print $1}')"
        case "$active" in
            "$TOOLCHAIN_CHANNEL"|"$TOOLCHAIN_CHANNEL"-*) ;;
            *)
                echo "toolchain: active toolchain is '$active', expected '$TOOLCHAIN_CHANNEL' from rust-toolchain.toml" >&2
                exit 1
                ;;
        esac
        details="$(rustc --version --verbose)"
        release="$(sed -n 's/^release: //p' <<<"$details")"
        host="$(sed -n 's/^host: //p' <<<"$details")"
        commit="$(sed -n 's/^commit-hash: //p' <<<"$details")"
        if [[ "$release" != "$TOOLCHAIN_CHANNEL" || -z "$host" || -z "$commit" ]]; then
            echo "toolchain: rustc identity does not match the exact '$TOOLCHAIN_CHANNEL' contract" >&2
            printf '%s\n' "$details" >&2
            exit 1
        fi
        installed="$(rustup component list --toolchain "$TOOLCHAIN_CHANNEL" --installed)"
        IFS=',' read -r -a components <<<"$TOOLCHAIN_COMPONENTS"
        for component in "${components[@]}"; do
            if ! grep -qE "^${component}(-${host})?([[:space:]]|$)" <<<"$installed"; then
                echo "toolchain: required component '$component' is not installed for $TOOLCHAIN_CHANNEL" >&2
                exit 1
            fi
        done
        printf 'toolchain: channel=%s host=%s commit=%s components=%s cache-key=%s\n' \
            "$release" "$host" "$commit" "$TOOLCHAIN_COMPONENTS" "$TOOLCHAIN_CACHE_KEY"
        ;;
    *) usage ;;
esac
