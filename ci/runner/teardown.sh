#!/usr/bin/env bash
set -euo pipefail

# Stop and remove the runner container and its reboot-persistence unit.
# Pass --purge to also drop the cargo cache volumes (registry + target).
# The config volume (registration identity) is kept unless purged, so a
# re-setup re-attaches the same runner without a new token.

CONTAINER_NAME="${CONTAINER_NAME:-aplan-runner}"

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
    echo "==> Volumes purged (config/identity volume kept; remove manually to fully reset)"
fi

echo "==> Teardown complete"
