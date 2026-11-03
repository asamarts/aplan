#!/usr/bin/env bash
# Change-aware development validation for ordinary handoff. The explicit
# profile selector is also CI's one-change rollback route to full validation;
# `--full` remains its convenient local spelling.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
export CARGO_BUILD_WARNINGS=deny

usage() {
    echo "usage: ci/scripts/preflight.sh [--profile development|full] [--base REV --head REV | [--pre-push FROM TO]...]" >&2
    echo "       ci/scripts/preflight.sh --full" >&2
    exit "${1:-2}"
}

profile=development
profile_supplied=0
full_alias=0
base=""
head=""
declare -a updates=()
while [[ "$#" -gt 0 ]]; do
    case "$1" in
        --profile)
            [[ "$#" -ge 2 && "$profile_supplied" -eq 0 ]] || usage
            case "$2" in
                development|full) profile="$2" ;;
                *) usage ;;
            esac
            profile_supplied=1
            shift 2
            ;;
        --full)
            [[ "$profile_supplied" -eq 0 ]] || usage
            profile=full
            profile_supplied=1
            full_alias=1
            shift
            ;;
        --pre-push)
            [[ "$#" -ge 3 && -n "$2" && -n "$3" ]] || usage
            updates+=("$2" "$3")
            shift 3
            ;;
        --base)
            [[ "$#" -ge 2 && -n "$2" && -z "$base" ]] || usage
            base="$2"
            shift 2
            ;;
        --head)
            [[ "$#" -ge 2 && -n "$2" && -z "$head" ]] || usage
            head="$2"
            shift 2
            ;;
        -h|--help)
            usage 0
            ;;
        *)
            usage
            ;;
    esac
done

is_null_revision() {
    case "$1" in
        0000000000000000000000000000000000000000|0000000000000000000000000000000000000000000000000000000000000000)
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

# GitHub uses an all-zero `before` object ID when a pushed branch did not exist.
# There is no honest diff to classify in that case: the complete checked-out
# tree is new to the remote. Bind the head exactly and widen to the full profile.
if [[ -n "$base" ]] && is_null_revision "$base"; then
    [[ -n "$head" && "${#updates[@]}" -eq 0 && "$full_alias" -eq 0 ]] || usage
    if ! APLAN_VALIDATION_HEAD_REV="$(git -C "$REPO_ROOT" rev-parse --verify "${head}^{commit}" 2>/dev/null)"; then
        echo "validation: initial-push head is not a commit" >&2
        exit 2
    fi
    checked_out="$(git -C "$REPO_ROOT" rev-parse --verify 'HEAD^{commit}')"
    if [[ "$APLAN_VALIDATION_HEAD_REV" != "$checked_out" ]]; then
        echo "validation: initial-push head is not the checked-out source" >&2
        exit 2
    fi
    printf 'validation: classifier input=initial-push update_count=0\n'
    printf 'validation: initial push has no base commit; running full validation\n'
    export APLAN_VALIDATION_HEAD_REV
    export APLAN_VALIDATION_INPUT_KIND=working-tree
    profile=full
    base=""
    head=""
fi

if [[ "$profile" == "full" ]]; then
    [[ "${#updates[@]}" -eq 0 ]] || usage
    if [[ "$full_alias" -eq 1 && ( -n "$base" || -n "$head" ) ]]; then
        usage
    fi
    if [[ -n "$base" || -n "$head" ]]; then
        [[ -n "$base" && -n "$head" ]] || usage
        APLAN_VALIDATION_BASE_REV="$(git -C "$REPO_ROOT" rev-parse --verify "${base}^{commit}")"
        APLAN_VALIDATION_HEAD_REV="$(git -C "$REPO_ROOT" rev-parse --verify "${head}^{commit}")"
        checked_out="$(git -C "$REPO_ROOT" rev-parse --verify 'HEAD^{commit}')"
        if [[ "$APLAN_VALIDATION_HEAD_REV" != "$checked_out" ]]; then
            echo "validation: full rollback head is not the checked-out source" >&2
            exit 2
        fi
        export APLAN_VALIDATION_BASE_REV APLAN_VALIDATION_HEAD_REV
        export APLAN_VALIDATION_INPUT_KIND=base-head
    fi
    exec "$SCRIPT_DIR/validate.sh" --profile full
fi

declare -a selector_args=(--working-tree)
input=working-tree
if [[ -n "$base" || -n "$head" ]]; then
    [[ -n "$base" && -n "$head" && "${#updates[@]}" -eq 0 ]] || usage
    selector_args=(--base "$base" --head "$head")
    input="base-head"
elif [[ "${#updates[@]}" -gt 0 ]]; then
    selector_args=()
    input="pre-push"
    for ((index = 0; index < ${#updates[@]}; index += 2)); do
        selector_args+=(--pre-push "${updates[$index]}" "${updates[$((index + 1))]}")
    done
fi
printf 'validation: classifier input=%s update_count=%s\n' "$input" "$((${#updates[@]} / 2))"

selection_file="$(mktemp "${TMPDIR:-/tmp}/aplan-validation-selection.XXXXXXXX")"
trap 'rm -f "$selection_file"' EXIT
set +e
classifier_started_epoch="$(date +%s)"
(
    cd "$REPO_ROOT"
    python3 "$SCRIPT_DIR/validation-select.py" "${selector_args[@]}" --format arguments --timings
) >"$selection_file"
classifier_status=$?
classifier_duration_seconds="$(($(date +%s) - classifier_started_epoch))"
set -e
if [[ "$classifier_status" -ne 0 ]]; then
    echo "validation: classifier outcome=failed duration_seconds=$classifier_duration_seconds; no development profile verdict was produced" >&2
    exit "$classifier_status"
fi

declare -a records=()
mapfile -d '' -t records <"$selection_file"
if (( ${#records[@]} % 2 != 0 )); then
    echo "validation: classifier emitted a truncated argument record" >&2
    exit 2
fi

declare -a validation_args=(--profile development)
no_changes=""
selector_schema=""
for ((index = 0; index < ${#records[@]}; index += 2)); do
    kind="${records[$index]}"
    value="${records[$((index + 1))]}"
    case "$kind" in
        schema)
            [[ -z "$selector_schema" && "$value" == "2" ]] || {
                echo "validation: classifier emitted an unsupported argument schema" >&2
                exit 2
            }
            selector_schema="$value"
            ;;
        gate)
            [[ -n "$value" ]] || {
                echo "validation: classifier emitted an empty gate" >&2
                exit 2
            }
            validation_args+=(--select "$value")
            ;;
        package)
            [[ -n "$value" ]] || {
                echo "validation: classifier emitted an empty package" >&2
                exit 2
            }
            validation_args+=(--package "$value")
            ;;
        widen)
            [[ -n "$value" ]] || {
                echo "validation: classifier emitted an empty widening reason" >&2
                exit 2
            }
            validation_args+=(--widen "$value")
            ;;
        no-changes)
            [[ -z "$no_changes" && ( "$value" == "true" || "$value" == "false" ) ]] || {
                echo "validation: classifier emitted an invalid no-changes record" >&2
                exit 2
            }
            no_changes="$value"
            ;;
        *)
            echo "validation: classifier emitted unknown record '$kind'" >&2
            exit 2
            ;;
    esac
done
[[ "$selector_schema" == "2" ]] || {
    echo "validation: classifier omitted its argument schema" >&2
    exit 2
}
[[ -n "$no_changes" ]] || {
    echo "validation: classifier omitted its no-changes record" >&2
    exit 2
}
printf 'validation: classifier outcome=passed no_changes=%s duration_seconds=%s launcher=python3 rust_build_processes=0\n' \
    "$no_changes" "$classifier_duration_seconds"

export APLAN_VALIDATION_INPUT_KIND="$input"
if [[ "$input" == "base-head" ]]; then
    APLAN_VALIDATION_BASE_REV="$(git -C "$REPO_ROOT" rev-parse --verify "${base}^{commit}")"
    APLAN_VALIDATION_HEAD_REV="$(git -C "$REPO_ROOT" rev-parse --verify "${head}^{commit}")"
    export APLAN_VALIDATION_BASE_REV APLAN_VALIDATION_HEAD_REV
elif [[ "$input" == "working-tree" ]]; then
    APLAN_VALIDATION_HEAD_REV="$(git -C "$REPO_ROOT" rev-parse --verify 'HEAD^{commit}')"
    export APLAN_VALIDATION_HEAD_REV
fi

rm -f "$selection_file"
trap - EXIT
exec "$SCRIPT_DIR/validate.sh" "${validation_args[@]}"
