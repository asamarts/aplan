#!/usr/bin/env bash
# Profile-neutral validation entry point. Its data comes only from the generated
# form of ci/validation.yml; `cargo xtask validation --check` gates that copy.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
# shellcheck source-path=SCRIPTDIR
# shellcheck source=validation-generated.sh
# Generated beside this script before execution.
# shellcheck disable=SC1091
source "$SCRIPT_DIR/validation-generated.sh"

usage() {
    local code="${1:-2}"
    cat >&2 <<'EOF'
usage:
  ci/scripts/validate.sh --gate NAME
  ci/scripts/validate.sh --profile full|release [--plan]
  ci/scripts/validate.sh --profile development [--select GATE]... [--package NAME]... [--widen REASON]... [--plan]
EOF
    exit "$code"
}

contains() {
    local needle="$1"
    shift
    local value
    for value in "$@"; do
        if [[ "$value" == "$needle" ]]; then
            return 0
        fi
    done
    return 1
}

print_set() {
    local label="$1"
    shift
    if [[ "$#" -eq 0 ]]; then
        echo "validation: ${label}: (none)"
    else
        echo "validation: ${label}: $*"
    fi
}

run_gate() {
    local name="$1"
    local command="${VALIDATION_GATE_COMMANDS[$name]-}"
    local input_class="${VALIDATION_GATE_INPUT_CLASSES[$name]-}"
    local started finished status outcome
    local package
    local -a command_args=()
    if [[ -z "$command" ]]; then
        echo "validation: unknown gate '$name'" >&2
        exit 2
    fi
    if [[ -z "$input_class" ]]; then
        echo "validation: gate '$name' has no input class" >&2
        exit 2
    fi
    if [[ "$profile" == "development" && "$input_class" == "rust-workspace" \
        && "${#packages[@]}" -gt 0 && "${#widening_reasons[@]}" -eq 0 ]]; then
        for package in "${packages[@]}"; do
            command_args+=(--package "$package")
        done
    fi
    echo "=================== validation: ${name} ==================="
    started="$(date +%s)"
    # Receipt controls belong to this dispatcher. A gate can run tests which
    # themselves exercise validation; inheriting the outer artifact path or
    # job summary would make those nested fixtures contend with their parent.
    if (
        unset APLAN_VALIDATION_INPUT_KIND
        unset APLAN_VALIDATION_BASE_REV APLAN_VALIDATION_HEAD_REV
        unset APLAN_VALIDATION_QUEUE_SECONDS APLAN_VALIDATION_SETUP_SECONDS
        unset APLAN_VALIDATION_SETUP_STARTED_EPOCH APLAN_VALIDATION_RECEIPT
        unset GITHUB_STEP_SUMMARY
        "$REPO_ROOT/$command" "${command_args[@]}"
    ); then
        status=0
        outcome=passed
    else
        status=$?
        outcome=failed
    fi
    finished="$(date +%s)"
    gate_names+=("$name")
    gate_outcomes+=("$outcome")
    gate_durations+=("$((finished - started))")
    printf 'validation: gate=%s outcome=%s duration_seconds=%s\n' \
        "$name" "$outcome" "$((finished - started))"
    return "$status"
}

write_release_receipt() {
    local status="$1"
    local outcome=failed
    local summary="${GITHUB_STEP_SUMMARY:-}"
    local index

    [[ "$release_receipt_active" -eq 1 ]] || return 0
    if [[ "$status" -eq 0 ]]; then
        outcome=passed
    fi
    printf 'validation: release receipt outcome=%s source=%s version=%s tag=%s inventory_sha256=%s\n' \
        "$outcome" "$release_source" "$release_version" "$release_tag" "$release_inventory"

    [[ -n "$summary" ]] || return 0
    {
        echo "### aplan release validation"
        echo
        echo "- Outcome: ${outcome}"
        echo "- Source: \`${release_source}\`"
        echo "- Version: \`${release_version}\`"
        echo "- Tag: \`${release_tag}\`"
        echo "- Gate inventory: \`sha256:${release_inventory}\`"
        echo "- Toolchain: \`${release_toolchain:-unverified}\`"
        echo
        echo "| Gate | Outcome | Duration (seconds) |"
        echo "|---|---:|---:|"
        for index in "${!gate_names[@]}"; do
            printf "| \`%s\` | %s | %s |\n" \
                "${gate_names[$index]}" "${gate_outcomes[$index]}" "${gate_durations[$index]}"
        done
    } >>"$summary"
}

write_profile_receipt() {
    local status="$1"
    local outcome=failed
    local runner_environment="${RUNNER_ENVIRONMENT:-local}"
    local runner_os="${RUNNER_OS:-unknown}"
    local runner_arch="${RUNNER_ARCH:-unknown}"
    local -a args=()
    local index

    [[ "$profile_receipt_active" -eq 1 ]] || return 0
    if [[ "$status" -eq 0 ]]; then
        outcome=passed
    fi
    args=(
        --profile "$profile"
        --assurance "$assurance"
        --outcome "$outcome"
        --source "$profile_source"
        --dirty "$profile_dirty"
        --inventory-sha256 "$profile_inventory"
        --toolchain "$profile_toolchain"
        --input-kind "${APLAN_VALIDATION_INPUT_KIND:-direct}"
        --runner-environment "$runner_environment"
        --runner-os "$runner_os"
        --runner-arch "$runner_arch"
    )
    for name in "${selected[@]}"; do args+=(--selected "$name"); done
    for name in "${widened[@]}"; do args+=(--widened "$name"); done
    for name in "${deferred[@]}"; do args+=(--deferred "$name"); done
    for name in "${packages[@]}"; do args+=(--package "$name"); done
    for name in "${widening_reasons[@]}"; do args+=(--widening-reason "$name"); done
    for index in "${!gate_names[@]}"; do
        args+=(--gate "${gate_names[$index]}" "${gate_outcomes[$index]}" "${gate_durations[$index]}")
    done
    if [[ -n "${APLAN_VALIDATION_BASE_REV:-}" ]]; then
        args+=(--base "$APLAN_VALIDATION_BASE_REV")
    fi
    if [[ -n "${APLAN_VALIDATION_HEAD_REV:-}" ]]; then
        args+=(--head "$APLAN_VALIDATION_HEAD_REV")
    fi
    if [[ -n "${APLAN_VALIDATION_QUEUE_SECONDS:-}" ]]; then
        args+=(--queue-seconds "$APLAN_VALIDATION_QUEUE_SECONDS")
    fi
    if [[ -n "${APLAN_VALIDATION_SETUP_SECONDS:-}" ]]; then
        args+=(--setup-seconds "$APLAN_VALIDATION_SETUP_SECONDS")
    fi
    if [[ -n "${APLAN_VALIDATION_RECEIPT:-}" ]]; then
        args+=(--output "$APLAN_VALIDATION_RECEIPT")
    fi
    if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
        args+=(--summary "$GITHUB_STEP_SUMMARY")
    fi
    if [[ "$profile" == "release" ]]; then
        args+=(--release-version "$release_version" --release-tag "$release_tag")
    fi
    "$SCRIPT_DIR/validation-receipt.py" "${args[@]}"
}

validation_receipts_on_exit() {
    local status=$?
    trap - EXIT
    if ! write_profile_receipt "$status"; then
        echo "validation: profile receipt publication failed" >&2
        if [[ "$status" -eq 0 ]]; then
            status=2
        fi
    fi
    if ! write_release_receipt "$status"; then
        echo "validation: release receipt publication failed" >&2
        if [[ "$status" -eq 0 ]]; then
            status=2
        fi
    fi
    exit "$status"
}

profile=""
gate=""
plan=0
declare -a requested=()
declare -a packages=()
declare -a widening_reasons=()
declare -a gate_names=()
declare -a gate_outcomes=()
declare -a gate_durations=()
release_receipt_active=0
profile_receipt_active=0
release_source=""
release_version=""
release_tag=""
release_inventory=""
release_toolchain=""
profile_source=""
profile_dirty=""
profile_inventory=""
profile_toolchain=""

while [[ "$#" -gt 0 ]]; do
    case "$1" in
        --profile)
            [[ "$#" -ge 2 && -z "$profile" ]] || usage
            profile="$2"
            shift 2
            ;;
        --gate)
            [[ "$#" -ge 2 && -z "$gate" ]] || usage
            gate="$2"
            shift 2
            ;;
        --select)
            [[ "$#" -ge 2 ]] || usage
            requested+=("$2")
            shift 2
            ;;
        --package)
            [[ "$#" -ge 2 && -n "$2" ]] || usage
            packages+=("$2")
            shift 2
            ;;
        --widen)
            [[ "$#" -ge 2 && -n "$2" ]] || usage
            widening_reasons+=("$2")
            shift 2
            ;;
        --plan)
            [[ "$plan" -eq 0 ]] || usage
            plan=1
            shift
            ;;
        -h|--help)
            usage 0
            ;;
        *)
            echo "validation: unknown option '$1'" >&2
            usage
            ;;
    esac
done

if [[ -n "${PREFLIGHT_SKIP:-}" ]]; then
    echo "validation: PREFLIGHT_SKIP cannot subtract gates from a named profile" >&2
    exit 2
fi

export CARGO_BUILD_WARNINGS=deny

if [[ -n "$gate" ]]; then
    [[ -z "$profile" && "$plan" -eq 0 && "${#requested[@]}" -eq 0 \
        && "${#packages[@]}" -eq 0 && "${#widening_reasons[@]}" -eq 0 ]] || usage
    contains "$gate" "${VALIDATION_ALL_GATES[@]}" || {
        echo "validation: unknown gate '$gate'" >&2
        exit 2
    }
    echo "validation: one inventory gate; this is not a profile verdict"
    if [[ "$gate" != "msrv-build" ]]; then
        "$SCRIPT_DIR/toolchain.sh" verify
    fi
    run_gate "$gate"
    exit 0
fi

[[ -n "$profile" ]] || usage
declare -a selected=()
declare -a widened=()
declare -a deferred=()

case "$profile" in
    full)
        [[ "${#requested[@]}" -eq 0 && "${#packages[@]}" -eq 0 \
            && "${#widening_reasons[@]}" -eq 0 ]] || {
            echo "validation: full does not accept selection or subtraction" >&2
            exit 2
        }
        selected=("${VALIDATION_FULL_GATES[@]}")
        assurance="$VALIDATION_FULL_ASSURANCE"
        ;;
    release)
        [[ "${#requested[@]}" -eq 0 && "${#packages[@]}" -eq 0 \
            && "${#widening_reasons[@]}" -eq 0 ]] || {
            echo "validation: release does not accept selection or subtraction" >&2
            exit 2
        }
        [[ "$VALIDATION_RELEASE_REQUIRES" == "full" && "$VALIDATION_RELEASE_SAME_SOURCE" -eq 1 ]] || {
            echo "validation: generated release ordering is invalid" >&2
            exit 2
        }
        selected=("${VALIDATION_RELEASE_GATES[@]}")
        assurance="$VALIDATION_RELEASE_ASSURANCE"
        ;;
    development)
        assurance="$VALIDATION_DEVELOPMENT_ASSURANCE"
        declare -A requested_once=()
        declare -A package_once=()
        for name in "${requested[@]}"; do
            contains "$name" "${VALIDATION_DEVELOPMENT_SELECTABLE[@]}" || {
                echo "validation: development cannot select unknown or non-selectable gate '$name'" >&2
                exit 2
            }
            [[ -z "${requested_once[$name]-}" ]] || {
                echo "validation: development gate '$name' was selected twice" >&2
                exit 2
            }
            requested_once[$name]=1
        done
        for name in "${packages[@]}"; do
            [[ "$name" =~ ^[A-Za-z0-9][A-Za-z0-9_-]*$ ]] || {
                echo "validation: invalid development package '$name'" >&2
                exit 2
            }
            [[ -z "${package_once[$name]-}" ]] || {
                echo "validation: development package '$name' was selected twice" >&2
                exit 2
            }
            package_once[$name]=1
        done
        for name in "${VALIDATION_ALL_GATES[@]}"; do
            if contains "$name" "${VALIDATION_DEVELOPMENT_ALWAYS[@]}"; then
                selected+=("$name")
            elif [[ -n "${requested_once[$name]-}" ]]; then
                selected+=("$name")
            elif contains "$name" "${VALIDATION_DEVELOPMENT_SELECTABLE[@]}" && [[ "${#widening_reasons[@]}" -gt 0 ]]; then
                widened+=("$name")
            else
                deferred+=("$name")
            fi
        done
        ;;
    *)
        echo "validation: unknown profile '$profile'" >&2
        exit 2
        ;;
esac

echo "validation: profile=$profile assurance=$assurance schema=$VALIDATION_SCHEMA"
print_set "selected gates" "${selected[@]}"
print_set "widened gates" "${widened[@]}"
print_set "deferred gates" "${deferred[@]}"
print_set "selected packages" "${packages[@]}"
print_set "widening reasons" "${widening_reasons[@]}"

if [[ "$plan" -eq 1 ]]; then
    exit 0
fi

if [[ -n "${APLAN_VALIDATION_SETUP_STARTED_EPOCH:-}" ]]; then
    if ! [[ "$APLAN_VALIDATION_SETUP_STARTED_EPOCH" =~ ^[0-9]{1,10}$ ]]; then
        echo "validation: setup start must be an epoch-second integer" >&2
        exit 2
    fi
    setup_finished="$(date +%s)"
    if [[ "$setup_finished" -lt "$APLAN_VALIDATION_SETUP_STARTED_EPOCH" ]]; then
        echo "validation: setup start is later than the validation clock" >&2
        exit 2
    fi
    export APLAN_VALIDATION_SETUP_SECONDS="$((setup_finished - APLAN_VALIDATION_SETUP_STARTED_EPOCH))"
fi

if [[ "$profile" == "release" ]]; then
    release_source="$("$SCRIPT_DIR/release-source.sh" "$REPO_ROOT")" || exit 2
    export APLAN_RELEASE_SHA="$release_source"
    release_version="$("$SCRIPT_DIR/current-version.sh")"
    release_tag="${TAG:-none}"
    if [[ -n "${TAG:-}" ]]; then
        if [[ "$release_version" == *-* ]]; then
            echo "validation: tagged release cannot use pre-release workspace version '$release_version'" >&2
            exit 2
        fi
        if [[ "$TAG" != "v${release_version}" ]]; then
            echo "validation: tag '$TAG' does not match workspace version '$release_version'" >&2
            exit 2
        fi
    fi
    release_inventory="$(sha256sum "$REPO_ROOT/ci/validation.yml" | awk '{print $1}')"
    release_receipt_active=1
    # Preserve the pre-existing failed-release receipt even when toolchain
    # verification refuses before a general profile receipt can be truthful.
    trap validation_receipts_on_exit EXIT
fi

profile_source="$(git -C "$REPO_ROOT" rev-parse --verify 'HEAD^{commit}')" || {
    echo "validation: cannot resolve the validation source commit" >&2
    exit 2
}
profile_source="${profile_source,,}"
if ! [[ "$profile_source" =~ ^[0-9a-f]{40}$ ]]; then
    echo "validation: source is not one full commit ID" >&2
    exit 2
fi
if [[ -n "$(git -C "$REPO_ROOT" status --porcelain=v1 --untracked-files=normal)" ]]; then
    profile_dirty=true
else
    profile_dirty=false
fi
profile_inventory="$(sha256sum "$REPO_ROOT/ci/validation.yml" | awk '{print $1}')"

# A profile verdict is meaningful only for the compiler that defines it. The
# MSRV gate below deliberately selects the consumer floor itself; everything
# else runs under this exact development/release identity.
profile_toolchain="$("$SCRIPT_DIR/toolchain.sh" verify)"
printf '%s\n' "$profile_toolchain"
if [[ "$profile" == "release" ]]; then
    release_toolchain="$profile_toolchain"
fi
profile_receipt_active=1
trap validation_receipts_on_exit EXIT

for name in "${selected[@]}" "${widened[@]}"; do
    run_gate "$name"
done

if [[ "$profile" == "release" ]]; then
    source_after="$("$SCRIPT_DIR/release-source.sh" "$REPO_ROOT")" || {
        echo "validation: release source changed during full validation" >&2
        exit 1
    }
    if [[ "$release_source" != "$source_after" ]]; then
        echo "validation: release source identity changed during full validation" >&2
        exit 1
    fi
fi

echo "validation: profile=$profile passed ${#selected[@]} selected and ${#widened[@]} widened gate(s)"
