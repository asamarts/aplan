#!/usr/bin/env bash
# Run the TUI's three wall-clock gates on the reference host (`A2-164`, ADR-0067).
#
#   ci/scripts/reference-tui-perf.sh [--budgets reference|measure] [--rev REVISION]
#
# The reference host owns the TUI's wall-clock budgets. It is a quiet machine that runs one
# benchmark at a time through a queue. This script sends REVISION (HEAD by default) as committed.
# Then, on the host and inside one slot of its queue, it:
# - builds the gates in release;
# - waits until the one-minute load is at or under APLAN_REFERENCE_MAX_LOAD;
# - runs the in-process render gate, the layered-graph gate and the real-terminal gate, each
#   pinned to one CPU, with APLAN_TUI_BUDGETS set to --budgets: `reference`, the default, holds
#   each metric to its reference budget, and `measure` holds none and is how those budgets are
#   measured.
# Every metric comes back as a TUI_PERF line, in a file under target/reference-tui-perf/. The exit
# status is the gates'. ci/scripts/reference-tui-budgets.py turns five `measure` runs into budgets.
#
# Two settings reach the host, and nothing here names it:
#   APLAN_REFERENCE_HOST    the host's ssh destination.
#   APLAN_REFERENCE_QUEUE   a command prefix that the host's shell runs. It waits for the queue's
#                           slot and holds it while the command after it runs. The run's label is
#                           in $APLAN_REFERENCE_LABEL there.
# Four have defaults:
#   APLAN_REFERENCE_CPU         the CPU the gates are pinned to (2).
#   APLAN_REFERENCE_MAX_LOAD    the highest one-minute load to measure at (0.25).
#   APLAN_REFERENCE_QUIET_WAIT  seconds to wait for that load once the build is done (1800).
#   APLAN_REFERENCE_TOOLCHAIN   a directory on the host whose bin/ holds the Rust release that
#                               rust-toolchain.toml pins, for a host without rustup. Leave it empty
#                               to use the host's PATH.
#
# Exit status:
#   0    the gates passed.
#   1    a gate failed.
#   2    a usage or setup error.
#   3    the host did not get quiet.
#   any other: the queue's own status when it gave up.
set -euo pipefail
export CARGO_BUILD_WARNINGS=deny

# The gates, as `package|cargo target|test`, in the order they run.
GATES=(
    "aplan-tui|--lib|performance_tests::layered_graph_performance_gate_layout_and_semantic_hits"
    "aplan-tui|--lib|performance_tests::tui_performance_gate_render_interaction_and_graph_work"
    "aplan|--test integration|tui_performance::tui_performance_gate_startup_interaction_and_memory"
)
# The build directory under the host's home. The target directory in it is kept between runs, so
# only the first run builds from nothing.
REMOTE_TREE=aplan-reference

# The one-minute load. Anything that is not a clear "at or under the limit" counts as busy,
# including an unreadable load.
quiet_at() {
    [[ "$1" =~ ^[0-9]+(\.[0-9]+)?$ ]] && awk -v current="$1" -v limit="$2" \
        'BEGIN { exit !(current + 0 <= limit + 0) }'
}

# The half that runs on the host, inside the queue's slot. Progress goes to stderr. Stdout carries
# only the results, which the local half writes to its result file.
on_host() {
    local stage=$1 commit=$2 source=$3 budgets=$4 cpu=$5 max_load=$6 quiet_wait=$7 toolchain=$8
    if [[ ! "$stage" =~ ^[A-Za-z0-9_.-]+$ ]]; then
        echo "the stage must be a plain directory name, not $stage" >&2
        exit 2
    fi
    cd ~
    # shellcheck disable=SC2064 # the stage is fixed now
    trap "rm -rf ~/$stage" EXIT
    if [[ -n "$toolchain" ]]; then
        export PATH="$toolchain/bin:$PATH"
    fi
    mkdir -p "$REMOTE_TREE"
    cd "$REMOTE_TREE"
    # A file an earlier revision had and this one does not must not stay to be built.
    find . -path ./target -prune -o ! -type d -exec rm -f {} +
    # Every file gets the time of extraction, not its commit's time: cargo rebuilds what is newer
    # than its last build, and a commit older than the previous run's build would otherwise be
    # measured with that run's binaries. A2-164's doubling proof caught exactly that.
    tar -xmzf ~/"$stage/tree.tar.gz"

    local channel rustc_version
    channel="$(sed -n 's/^channel = "\(.*\)"$/\1/p' rust-toolchain.toml)"
    rustc_version="$(rustc --version)"
    if [[ "$rustc_version" != "rustc $channel "* ]]; then
        echo "the host's rustc is '$rustc_version', not the pinned $channel: set APLAN_REFERENCE_TOOLCHAIN" >&2
        exit 2
    fi
    echo "REFERENCE commit=$commit source=$source budgets=$budgets cpu=$cpu max_load=$max_load rustc=${rustc_version// /_}"

    # Build first, so that compiling, and its load, are over before anything is measured.
    echo "building the gates in release" >&2
    cargo test --release --locked -p aplan-tui --lib --no-run >&2
    cargo test --release --locked -p aplan --test integration --no-run >&2

    # Every gate runs, so that one failure does not hide the metrics of the others, and each
    # starts once the load is down again: a gate keeps one CPU busy, which leaves a load near 1.
    local status=0 gate package target test gate_status log=~/"$stage/gate.log" load deadline
    for gate in "${GATES[@]}"; do
        IFS='|' read -r package target test <<<"$gate"
        deadline=$((SECONDS + quiet_wait))
        while :; do
            load="$(cut -d' ' -f1 /proc/loadavg 2>/dev/null)" || load=unreadable
            quiet_at "$load" "$max_load" && break
            if ((SECONDS >= deadline)); then
                echo "the host's load is $load after $quiet_wait seconds, over $max_load" >&2
                exit 3
            fi
            sleep 15
        done
        echo "REFERENCE gate=$test load_before=$load"
        echo "==> $test at a load of $load" >&2
        gate_status=0
        # shellcheck disable=SC2086 # the target is two words
        APLAN_TUI_BUDGETS="$budgets" taskset -c "$cpu" \
            cargo test -q --release --locked -p "$package" $target "$test" -- \
            --ignored --exact --nocapture --test-threads=1 >"$log" 2>&1 || gate_status=$?
        cat "$log" >&2
        grep -a '^TUI_PERF ' "$log" || true
        echo "REFERENCE gate=$test status=$gate_status"
        if ((gate_status != 0)); then
            grep -a 'exceeded the' "$log" | sed 's/^/REFERENCE exceeded=/' || true
            status=1
        fi
    done
    echo "REFERENCE load_after=$(cut -d' ' -f1 /proc/loadavg)"
    exit "$status"
}

# The local half's cleanup: stop the run on the host if it still waits or runs there, through the
# PID and start time its queue client wrote to the stage, and remove the stage.
# shellcheck disable=SC2016 # expanded on the host
STOP_REMOTE='
alive() { [[ "$(cut -d" " -f3,22 "/proc/$pid/stat" 2>/dev/null)" =~ ^[^ZX]\ ([0-9]+)$ && "${BASH_REMATCH[1]}" == "$start" ]]; }
if read -r pid start 2>/dev/null <~/"$1/client.pid" && alive && kill -TERM "$pid" 2>/dev/null; then
    for _ in $(seq 120); do alive || break; sleep 0.5; done
fi
rm -rf ~/"$1"'
host="" stage="" ssh_pid=""
# shellcheck disable=SC2329 # run by the EXIT trap
cleanup() {
    if [[ -n "$stage" ]]; then
        ssh -o BatchMode=yes "$host" "bash -c $(printf %q "$STOP_REMOTE") stop $stage" </dev/null || true
    fi
    if [[ -n "$ssh_pid" ]]; then kill "$ssh_pid" 2>/dev/null || true; fi
}

usage() {
    echo "usage: ci/scripts/reference-tui-perf.sh [--budgets reference|measure] [--rev REVISION]" >&2
    exit 2
}

local_half() {
    local budgets=reference rev=HEAD
    while (($#)); do
        case "$1" in
        --budgets)
            budgets=${2:-}
            shift 2 || usage
            ;;
        --rev)
            rev=${2:-}
            shift 2 || usage
            ;;
        *) usage ;;
        esac
    done
    case "$budgets" in
    reference | measure) ;;
    *) usage ;;
    esac
    host=${APLAN_REFERENCE_HOST:-}
    local queue=${APLAN_REFERENCE_QUEUE:-}
    local cpu=${APLAN_REFERENCE_CPU:-2} max_load=${APLAN_REFERENCE_MAX_LOAD:-0.25}
    local quiet_wait=${APLAN_REFERENCE_QUIET_WAIT:-1800} toolchain=${APLAN_REFERENCE_TOOLCHAIN:-}
    if [[ -z "$host" || -z "$queue" ]]; then
        echo "set APLAN_REFERENCE_HOST and APLAN_REFERENCE_QUEUE: see the head of $0" >&2
        exit 2
    fi
    if [[ ! "$cpu" =~ ^[0-9]+$ || ! "$quiet_wait" =~ ^[0-9]+$ || ! "$max_load" =~ ^[0-9]+(\.[0-9]+)?$ ]]; then
        echo "APLAN_REFERENCE_CPU and APLAN_REFERENCE_QUIET_WAIT must be whole numbers, APLAN_REFERENCE_MAX_LOAD a number" >&2
        exit 2
    fi

    local root commit source
    root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
    commit="$(git -C "$root" rev-parse --verify "$rev^{commit}")"
    # What the gates measure, which survives a rebase or a commit to the plan: the crates, the
    # workspace manifest and the lockfile, as git names them.
    source="$(git -C "$root" rev-parse "$commit:crates" "$commit:Cargo.toml" "$commit:Cargo.lock" |
        cut -c1-12 | paste -sd- -)"
    if [[ -n "$(git -C "$root" status --porcelain --untracked-files=no)" ]]; then
        echo "note: this measures $commit as committed, without the uncommitted changes here" >&2
    fi
    local out="$root/target/reference-tui-perf"
    mkdir -p "$out"
    local result
    result="$out/${commit:0:12}-$budgets-$(date -u +%Y%m%dT%H%M%SZ).txt"

    # The tree goes to a stage of its own on the host, so that runs waiting in the queue cannot
    # overwrite each other's. Sending it is light, so it happens before the run takes the slot.
    trap cleanup EXIT
    local made
    made="$(ssh -o BatchMode=yes "$host" 'mktemp -d ~/aplan-reference.stage.XXXXXX' </dev/null)" || made=""
    made="${made##*/}"
    if [[ ! "$made" =~ ^[A-Za-z0-9_.-]+$ ]]; then
        echo "$host gave no directory to send the tree to" >&2
        exit 2
    fi
    stage=$made
    trap 'exit 129' HUP
    trap 'exit 130' INT
    trap 'exit 143' TERM

    git -C "$root" archive --format=tar.gz "$commit" |
        ssh -o BatchMode=yes "$host" "cat >~/$stage/tree.tar.gz"
    git -C "$root" show "$commit:ci/scripts/reference-tui-perf.sh" |
        ssh -o BatchMode=yes "$host" "cat >~/$stage/run.sh"

    local label="aplan tui-perf ${commit:0:12} $budgets"
    local status=0
    echo "measuring $commit on $host; results in $result" >&2
    # In the background, so that a signal reaches the traps above at once rather than after the
    # queue's wait. The host's shell writes its PID, which becomes the queue client's.
    # shellcheck disable=SC2016 # $$ is the host's shell
    ssh -o BatchMode=yes "$host" 'echo $$ "$(cut -d" " -f22 /proc/$$/stat)" >~/'"$stage"'/client.pid && '"\
export APLAN_REFERENCE_LABEL=$(printf %q "$label") && exec $queue \
bash ~/$stage/run.sh --on-host $(printf '%q ' "$stage" "$commit" "$source" "$budgets" "$cpu" "$max_load" "$quiet_wait" "$toolchain")" \
        </dev/null >"$result" &
    ssh_pid=$!
    wait "$ssh_pid" || status=$?
    ssh_pid=""

    local metrics
    metrics="$(grep -c '^TUI_PERF ' "$result" || true)"
    case "$status" in
    0) echo "the gates passed on $host: $metrics metrics in $result" >&2 ;;
    1)
        echo "a gate failed on $host: $metrics metrics in $result" >&2
        sed -n 's/^REFERENCE exceeded=//p' "$result" >&2
        ;;
    3) echo "$host did not get quiet enough to measure" >&2 ;;
    *) echo "the run on $host ended with status $status" >&2 ;;
    esac
    exit "$status"
}

if [[ "${1:-}" == "--on-host" ]]; then
    shift
    on_host "$@"
else
    local_half "$@"
fi
