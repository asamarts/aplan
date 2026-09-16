#!/usr/bin/env bash
set -euo pipefail

# Stop and remove the runner container and its reboot-persistence unit.
# Pass --purge to also drop the cargo cache volumes (registry + target).
# The config volume (registration identity) is kept unless purged, so a
# re-setup re-attaches the same runner without a new token.

CONTAINER_NAME="${CONTAINER_NAME:-aplan-runner}"

# Load ci/.env if it is there, for `GITHUB_REPO_URL` (used by --purge to
# deregister). Absent is fine: every step below degrades to a printed
# instruction rather than failing.
_ENV_FILE="$(cd "$(dirname "$0")/.." && pwd)/.env"
if [[ -f "$_ENV_FILE" ]]; then
    set -a
    # shellcheck disable=SC1090  # .env is deployment-local, resolved at runtime.
    source "$_ENV_FILE"
    set +a
fi

# Remove the systemd unit first so it does not recreate the container we are
# about to remove. The unit has no ExecStop, so disabling it never stops a
# running container on its own.
echo "==> Removing systemd user unit"
systemctl --user disable --now "container-${CONTAINER_NAME}.service" 2>/dev/null || true
rm -f "${HOME}/.config/systemd/user/container-${CONTAINER_NAME}.service"
systemctl --user daemon-reload 2>/dev/null || true

echo "==> Stopping runner container: ${CONTAINER_NAME}"
podman stop -t 30 "${CONTAINER_NAME}" 2>/dev/null || true

echo "==> Removing container"
podman rm "${CONTAINER_NAME}" 2>/dev/null || true

if [[ "${1:-}" == "--purge" ]]; then
    echo "==> Purging cache volumes"
    podman volume rm aplan-runner-registry 2>/dev/null || true
    podman volume rm aplan-runner-target 2>/dev/null || true
    echo "==> Volumes purged (config/identity volume kept; remove it with"
    echo "    'podman volume rm aplan-runner-config' to fully reset)"

    # Deregistration lives here, not in the container's signal handler (A1-053).
    # A SIGTERM arrives on any ordinary restart or host shutdown, and removing the
    # registration there invalidates the persisted identity and leaves the runner
    # looping on credentials the server has deleted. Only an explicit --purge
    # means "this runner is going away".
    #
    # Through the API rather than `config.sh remove`, which needs a registration
    # token that has almost certainly expired by teardown time. Best effort: it
    # needs `gh` authenticated for this repository, and if it does not run the
    # runner simply shows as offline and can be removed in the UI.
    if command -v gh >/dev/null 2>&1; then
        _repo="${GITHUB_REPO_URL:-}"
        _slug="${_repo#https://github.com/}"
        if [[ -n "${_slug}" ]]; then
            _id="$(gh api "repos/${_slug}/actions/runners" \
                     --jq ".runners[] | select(.name==\"${CONTAINER_NAME}\") | .id" 2>/dev/null || true)"
            if [[ -n "${_id}" ]]; then
                echo "==> Deregistering runner '${CONTAINER_NAME}' (id ${_id}) from ${_slug}"
                gh api -X DELETE "repos/${_slug}/actions/runners/${_id}" >/dev/null 2>&1 \
                    && echo "==> Deregistered" \
                    || echo "==> Could not deregister; remove it in Settings -> Actions -> Runners"
            fi
        else
            echo "==> GITHUB_REPO_URL not set; remove the runner in Settings -> Actions -> Runners"
        fi
    else
        echo "==> gh not found; remove the runner in Settings -> Actions -> Runners"
    fi
fi

echo "==> Teardown complete"
