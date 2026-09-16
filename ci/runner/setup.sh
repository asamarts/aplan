#!/usr/bin/env bash
set -euo pipefail

# Build the runner image, create the persistent volumes, start the runner
# container, and install a systemd user unit so it survives reboot (the pre-public repository-boundary decision).

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CI_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Load environment.
ENV_FILE="${CI_DIR}/.env"
if [[ ! -f "$ENV_FILE" ]]; then
    echo "Error: ${ENV_FILE} not found."
    echo "Copy ci/env.example to ci/.env and fill in the values."
    exit 1
fi
set -a
# shellcheck disable=SC1090  # .env is deployment-local, resolved at runtime.
source "$ENV_FILE"
set +a

: "${GITHUB_REPO_URL:?GITHUB_REPO_URL is required in .env}"
: "${GITHUB_TOKEN:?GITHUB_TOKEN is required in .env}"

RUNNER_IMAGE="${RUNNER_IMAGE:-aplan-runner}"
CONTAINER_NAME="${CONTAINER_NAME:-aplan-runner}"

# Fork-bomb guard: 8x podman's 2048 default, generous for a cargo build but
# still a ceiling. Verified after start so a future re-register cannot silently
# drop it.
PIDS_LIMIT="16384"

# Where CARGO_TARGET_DIR lives inside the container. MUST match the ENV in
# ci/Containerfile: the volume is mounted here so builds stay incremental across
# jobs, and it is OUTSIDE the checkout so actions/checkout's `git clean -ffdx`
# cannot wipe it.
TARGET_MOUNT="/home/runner/cargo-target"

echo "==> Building runner image: ${RUNNER_IMAGE}"
podman build -t "${RUNNER_IMAGE}" -f "${CI_DIR}/Containerfile" "${CI_DIR}"

echo "==> Creating volumes"
podman volume create aplan-runner-config 2>/dev/null || true
podman volume create aplan-runner-registry 2>/dev/null || true
podman volume create aplan-runner-target 2>/dev/null || true

echo "==> Starting runner container: ${CONTAINER_NAME}"
# `--network pasta:--dns-host,...` pins the resolver this container's DNS is
# forwarded to, and it is stored in the container, so it survives a restart.
#
# Without it, `pasta` reads the host's /etc/resolv.conf at every start and keeps
# that answer for the container's life. Two things rewrite that file here:
# dhcpcd, and tailscaled whenever `--accept-dns` changes -- to the single line
# `nameserver 100.100.100.100`, its own resolver. When Tailscale DNS was
# switched off on 2026-09-11 that resolver stopped answering and 11 of 14 runner
# containers on this host lost DNS at once, while keeping full connectivity, so
# it read as "GitHub is down" rather than as a resolver that had gone away.
#
# Verified rather than assumed: a container created with this option keeps
# resolving across `podman restart`, across Tailscale DNS being toggled off and
# on, and when the host file names a resolver that no longer answers. The
# equivalent setting in ~/.config/containers/containers.conf is NOT enough --
# it is not applied when a single container is restarted on its own.
DNS_HOST="${RUNNER_DNS_HOST:-192.168.8.1}"

podman run -d \
    --name "${CONTAINER_NAME}" \
    --restart unless-stopped \
    --network "pasta:--dns-host,${DNS_HOST}" \
    --pids-limit "${PIDS_LIMIT}" \
    -e GITHUB_REPO_URL="${GITHUB_REPO_URL}" \
    -e GITHUB_TOKEN="${GITHUB_TOKEN}" \
    -e RUNNER_NAME="${RUNNER_NAME:-aplan-runner}" \
    -e RUNNER_LABELS="${RUNNER_LABELS:-self-hosted,linux,aplan}" \
    -v aplan-runner-config:/home/runner/_config \
    -v aplan-runner-registry:/usr/local/cargo/registry \
    -v aplan-runner-target:"${TARGET_MOUNT}" \
    "${RUNNER_IMAGE}"

# Fail loudly if the PID ceiling did not take.
_actual_pids="$(podman inspect --format '{{.HostConfig.PidsLimit}}' "${CONTAINER_NAME}" 2>/dev/null || echo '?')"
if [[ "${_actual_pids}" != "${PIDS_LIMIT}" ]]; then
    echo "ERROR: ${CONTAINER_NAME} PidsLimit is '${_actual_pids}', expected ${PIDS_LIMIT}." >&2
    exit 1
fi
echo "==> Verified PidsLimit=${PIDS_LIMIT}"

# --- Reboot persistence ------------------------------------------------------
# `--restart unless-stopped` plus podman-restart.service start the container on
# a normal reboot, but only if the container OBJECT survives. A hard reboot that
# clears it would leave the runner permanently offline. A systemd user unit
# closes that gap: it guarantees a boot start AND recreates the container (by
# re-running this script in recovery mode) if it is gone. Skipped when invoked
# as recovery so the unit's own ExecStartPre does not re-enter systemctl.
if [[ -z "${APLAN_RUNNER_RECOVERY:-}" ]]; then
    echo "==> Installing systemd user unit (reboot persistence)"
    REPO_ROOT="$(cd "${CI_DIR}/.." && pwd)"
    UNIT_DIR="${HOME}/.config/systemd/user"
    mkdir -p "${UNIT_DIR}"
    cat > "${UNIT_DIR}/container-${CONTAINER_NAME}.service" <<UNIT
[Unit]
Description=${CONTAINER_NAME} self-hosted GitHub Actions runner (podman)
Documentation=${GITHUB_REPO_URL}/tree/main/ci/runner
After=network-online.target podman-restart.service
Wants=network-online.target

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStartPre=/bin/sh -c 'podman container exists ${CONTAINER_NAME} || (cd ${REPO_ROOT} && APLAN_RUNNER_RECOVERY=1 bash ci/runner/setup.sh)'
ExecStart=/usr/bin/podman start ${CONTAINER_NAME}

[Install]
WantedBy=default.target
UNIT
    systemctl --user daemon-reload 2>/dev/null || true
    systemctl --user enable "container-${CONTAINER_NAME}.service" 2>/dev/null || true
    # Linger so the unit starts at boot without an active login session.
    loginctl enable-linger "$(id -un)" >/dev/null 2>&1 || true
    echo "==> Enabled container-${CONTAINER_NAME}.service (runner survives reboot)"
fi

echo "==> Runner started. Follow logs with: podman logs -f ${CONTAINER_NAME}"
