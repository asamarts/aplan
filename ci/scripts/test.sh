#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

export CARGO_BUILD_WARNINGS=deny

declare -a scope=(--workspace)
declare -a packages=()
while [[ "$#" -gt 0 ]]; do
    [[ "$1" == "--package" && "$#" -ge 2 && -n "$2" ]] || {
        echo "usage: ci/scripts/test.sh [--package NAME]..." >&2
        exit 2
    }
    packages+=("$2")
    shift 2
done
if [[ "${#packages[@]}" -gt 0 ]]; then
    scope=()
    for package in "${packages[@]}"; do
        scope+=(-p "$package")
    done
fi

# The shared gate has one admitted non-doctest runner. Refuse an absent or
# merely compatible binary before compiling anything: the measured contract is
# the checksum-pinned build in ci/nextest.env, not whatever `cargo nextest`
# happens to find on the host.
"$REPO_ROOT/ci/scripts/install-nextest.sh" check

# The CLI-surface audit resolves against invocations that actually happened
# (`A1-208`), and the spawn helper records them when told where (`A1-207`). It is
# recorded from the suite run this gate already does rather than from a second
# one: this repository pays for its own CI (the pre-public repository-boundary decision), and a gate that doubles
# the test time to measure the test time is one nobody keeps.
# Absolute, because a test binary runs with its *package* as the working
# directory and a relative path would land under `crates/aplan/`.
#
# And unique per run. A fixed path is shared state: two runs in one checkout --
# a second terminal, or a preflight beside a bare `test.sh` -- have one truncate
# the other's log mid-flight, and the audit then reports two thirds of the
# surface unexercised. Which is exactly what it did, once, and a gate that can
# accuse the suite of something the suite did not do is worse than no gate.
INVOCATIONS_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
mkdir -p "$INVOCATIONS_DIR"
INVOCATIONS="$(mktemp "$INVOCATIONS_DIR/cli-invocations.XXXXXXXX.jsonl")"
NEXTEST_LIST="$(mktemp "$INVOCATIONS_DIR/nextest-list.XXXXXXXX.json")"
CARGO_LIST="$(mktemp "$INVOCATIONS_DIR/cargo-list.XXXXXXXX.jsonl")"
NEXTEST_GROUPS="$(mktemp "$INVOCATIONS_DIR/nextest-groups.XXXXXXXX.txt")"

# The ordinary read path persists exact parsed-plan snapshots. A persistent
# self-hosted runner must not let a snapshot from an earlier job decide whether
# today's full correctness corpus reaches the loader and validator. Give this
# run an owner directory, but point aplan at an absent child: the first access
# is necessarily cold, while later accesses can still exercise publication,
# exact hits and concurrent readers/writers. Never inspect or remove the user's
# real cache.
TEST_CACHE_OWNER="$(mktemp -d "$INVOCATIONS_DIR/aplan-test-cache.XXXXXXXX")"
export APLAN_CACHE_DIR="$TEST_CACHE_OWNER/cache"
trap 'rm -f "$INVOCATIONS" "$NEXTEST_LIST" "$CARGO_LIST" "$NEXTEST_GROUPS"; rm -rf "$TEST_CACHE_OWNER"' EXIT
export APLAN_TEST_INVOCATIONS="$INVOCATIONS"

if [[ -e "$APLAN_CACHE_DIR" ]]; then
    echo "isolated test cache unexpectedly exists before the suite: $APLAN_CACHE_DIR" >&2
    exit 1
fi

echo "==> cargo nextest list ${scope[*]} --message-format json"
cargo nextest list "${scope[@]}" --message-format json > "$NEXTEST_LIST"

echo "==> cargo test --no-run --message-format json ${scope[*]}"
cargo test --no-run --message-format json "${scope[@]}" > "$CARGO_LIST"

"$REPO_ROOT/ci/scripts/nextest-inventory.py" \
    --nextest "$NEXTEST_LIST" --cargo "$CARGO_LIST"

# Resolve the mutex through nextest rather than trusting the TOML text. This
# catches a renamed/moved test, an over-broad filter and a stale registration.
# Package-scoped development runs require exactly the applicable subset; the
# complete full/release run requires all three workspace-source actors.
echo "==> cargo nextest show-config test-groups ${scope[*]}"
cargo nextest show-config test-groups \
    --groups workspace-source --no-pager --color never "${scope[@]}" > "$NEXTEST_GROUPS"

if [[ "$(grep -Fxc 'group: workspace-source (max threads = 1)' "$NEXTEST_GROUPS" || true)" -ne 1 ]]; then
    echo "nextest workspace-source group is absent or does not serialize its members" >&2
    cat "$NEXTEST_GROUPS" >&2
    exit 1
fi

declare -a expected_group=()
if [[ "${#packages[@]}" -eq 0 ]]; then
    expected_group=(
        timings::evidence_gate_corpus_exercises_the_rolestat_shape_and_fault_control
        timings::evidence_rollout_artifact_reconstructs_trust_and_keeps_direct_rollback
        untracked::the_generators_note_an_untracked_plan_file
    )
else
    for package in "${packages[@]}"; do
        case "$package" in
            aplan)
                expected_group+=(timings::evidence_gate_corpus_exercises_the_rolestat_shape_and_fault_control)
                expected_group+=(timings::evidence_rollout_artifact_reconstructs_trust_and_keeps_direct_rollback)
                ;;
            xtask)
                expected_group+=(untracked::the_generators_note_an_untracked_plan_file)
                ;;
        esac
    done
fi
mapfile -t expected_group < <(printf '%s\n' "${expected_group[@]}" | sed '/^$/d' | LC_ALL=C sort -u)
mapfile -t actual_group < <(sed -n 's/^          //p' "$NEXTEST_GROUPS" | LC_ALL=C sort -u)
if [[ "${#actual_group[@]}" -ne "${#expected_group[@]}" ]]; then
    group_matches=false
else
    group_matches=true
    for index in "${!expected_group[@]}"; do
        [[ "${actual_group[$index]}" == "${expected_group[$index]}" ]] || group_matches=false
    done
fi
if [[ "$group_matches" != true ]]; then
    echo "nextest workspace-source membership does not match the admitted contract" >&2
    printf 'expected: %s\n' "${expected_group[*]:-(none)}" >&2
    printf 'actual: %s\n' "${actual_group[*]:-(none)}" >&2
    exit 1
fi

config_sha256="$(python3 -c \
    'import hashlib, pathlib, sys; print(hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest())' \
    .config/nextest.toml)"
printf 'nextest-contract: config_sha256=%s group_members=%s\n' \
    "$config_sha256" "${#actual_group[@]}"

echo "==> cargo nextest run ${scope[*]} --test-threads num-cpus --retries 0 --flaky-result fail"
runner_started="$SECONDS"
cargo nextest run "${scope[@]}" \
    --test-threads num-cpus --retries 0 --flaky-result fail

# Nextest deliberately does not run rustdoc harnesses. Keep this as a separate
# Cargo stage with exactly the same workspace/package selection.
echo "==> cargo test ${scope[*]} --doc"
cargo test "${scope[@]}" --doc
printf 'test-latency: runner_plus_doctests_seconds=%s\n' "$((SECONDS - runner_started))"

echo "==> cargo xtask cli-surface --check"
audit_started="$SECONDS"
cargo run -q -p xtask -- cli-surface --check
printf 'test-latency: cli_surface_audit_seconds=%s\n' "$((SECONDS - audit_started))"
