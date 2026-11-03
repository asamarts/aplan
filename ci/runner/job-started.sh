#!/usr/bin/env bash
# GitHub's runner invokes this hook before every job. It bounds the persistent
# Cargo target without ever treating an untrusted name or a symlink as build
# output. A path argument exists for the hermetic regression test; production
# supplies none and therefore operates only on the image's fixed target mount.
set -euo pipefail

TARGET_MOUNT="${1:-/home/runner/cargo-target}"
TARGET_CAP_GIB="${TARGET_CAP_GIB:-100}"

if [[ ! "$TARGET_CAP_GIB" =~ ^[0-9]+$ ]] || (( 10#$TARGET_CAP_GIB > 8191 )); then
    echo "job-started: TARGET_CAP_GIB must be an integer from 0 through 8191" >&2
    exit 1
fi
if [[ -L "$TARGET_MOUNT" ]]; then
    echo "job-started: refusing symlink target mount: $TARGET_MOUNT" >&2
    exit 1
fi
if [[ ! -e "$TARGET_MOUNT" ]]; then
    exit 0
fi
if [[ ! -d "$TARGET_MOUNT" ]]; then
    echo "job-started: target mount is not a directory: $TARGET_MOUNT" >&2
    exit 1
fi

# A dead parent PID does not prove its process tree is dead, and a live PID may
# have been reused. The versioned owner record binds PID, boot and start time;
# Cargo and its descendants inherit a unique lease. The reclaimer retains any
# missing, malformed, reused or uninspectable identity and removes only an exact
# real allocation whose owner, child, process group and lease are all gone.
HOOK_DIR="${BASH_SOURCE[0]%/*}"
/usr/bin/python3 "$HOOK_DIR/reclaim-coverage.py" "$TARGET_MOUNT"

used_bytes="$(/usr/bin/du -s -B1 -- "$TARGET_MOUNT" 2>/dev/null)" || {
    echo "job-started: could not measure target mount: $TARGET_MOUNT" >&2
    exit 1
}
used_bytes="${used_bytes%%[[:space:]]*}"
if [[ ! "$used_bytes" =~ ^[0-9]+$ ]]; then
    echo "job-started: du returned a non-numeric size: $used_bytes" >&2
    exit 1
fi
cap_bytes=$((10#$TARGET_CAP_GIB * 1024 * 1024 * 1024))
if (( used_bytes <= cap_bytes )); then
    exit 0
fi

echo "job-started: target uses $used_bytes bytes (> $cap_bytes byte cap); clearing build output"

# These are Cargo's documented target-root outputs and the target triples this
# repository builds. Do not turn this into "everything below the mount": live
# coverage parents, CACHEDIR.TAG and an unknown entry are retained for review.
for candidate in \
    "$TARGET_MOUNT/debug" \
    "$TARGET_MOUNT/release" \
    "$TARGET_MOUNT/doc" \
    "$TARGET_MOUNT/tmp" \
    "$TARGET_MOUNT/package" \
    "$TARGET_MOUNT/llvm-cov-target" \
    "$TARGET_MOUNT/msrv" \
    "$TARGET_MOUNT"/*-unknown-* \
    "$TARGET_MOUNT"/wasm32-* \
    "$TARGET_MOUNT"/aarch64-*; do
    [[ -d "$candidate" && ! -L "$candidate" ]] || continue
    echo "job-started: removing build output ${candidate##*/}"
    /bin/rm -rf -- "$candidate"
done
