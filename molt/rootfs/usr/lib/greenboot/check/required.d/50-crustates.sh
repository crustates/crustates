#!/bin/bash
# greenboot health check: Crustates works on this boot. When it fails on the
# first boot of a new Molt image, greenboot rolls back to the previous image.
set -uo pipefail

# Test images only (molt-test-bad): fail, to test the rollback.
if [ -e /usr/lib/crustates/test-fail-health ]; then
    echo "FAIL: this test image fails its health check on purpose"
    exit 1
fi

deadline=$((SECONDS + 300))
# Retries "$@" until it works or five minutes have passed since the start.
wait_for() {
    local what=$1
    shift
    until "$@" >/dev/null 2>&1; do
        if [ "$SECONDS" -ge "$deadline" ]; then
            echo "FAIL: $what"
            return 1
        fi
        sleep 2
    done
    echo "ok: $what"
}

network() {
    # A default route on a link that has carrier.
    ip route show default | grep -v linkdown | grep -q . ||
        ip -6 route show default | grep -v linkdown | grep -q .
}

wait_for "the API answers on port 8080" curl -fsS --max-time 5 http://127.0.0.1:8080/healthz || exit 1
wait_for "the agent runs" systemctl is-active --quiet crust-agent.service || exit 1
for unit in zfs-import-cache.service zfs-import-scan.service zfs-mount.service; do
    if systemctl is-failed --quiet "$unit"; then
        echo "FAIL: $unit failed"
        exit 1
    fi
done
echo "ok: no ZFS import failed"
# A network that is down only rolls back a new image (a trial boot); on an
# ordinary boot it is more likely a cable than the image.
if ! wait_for "a default route with carrier" network; then
    if grub2-editenv list | grep -q '^greenboot_next_deployment_id='; then
        exit 1
    fi
    echo "no network, but this is not the first boot of a new image"
fi
