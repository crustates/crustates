#!/bin/bash
# greenboot health check: Crustates works on this boot. When it fails on the
# first boot of a new Molt image, greenboot rolls back to the previous image.
set -uo pipefail

# The first boot of a new image is a trial: a failure here rolls it back.
trial=false
grub2-editenv list 2>/dev/null | grep -q '^greenboot_next_deployment_id=' && trial=true

# On a trial boot the console shows each check as it goes
# (/run/crustates/health.json, ADR 0018).
names=("crust-api" "crust-agent" "ZFS pools" "network")
states=(waiting waiting waiting waiting)
details=("" "" "" "")
progress() {
    [ "$trial" = true ] || return 0
    local state=$1 i checks=()
    for i in "${!names[@]}"; do
        checks+=("$(jq -nc --arg n "${names[$i]}" --arg s "${states[$i]}" --arg d "${details[$i]}" '{name: $n, state: $s, detail: $d}')")
    done
    install -d -m 0770 -o root -g crustates /run/crustates
    printf '%s\n' "${checks[@]}" | jq -sc --arg state "$state" '{state: $state, deadline: 300, checks: .}' \
        >/run/crustates/health.json.new && mv /run/crustates/health.json.new /run/crustates/health.json
}
# Marks check $1 as $2 with detail $3.
mark() {
    states[$1]=$2
    details[$1]=$3
    progress running
}
failed() {
    mark "$1" failed "$2"
    progress failed
    echo "FAIL: $2"
    exit 1
}

# Test images only (molt-test-bad): fail, to test the rollback.
if [ -e /usr/lib/crustates/test-fail-health ]; then
    failed 0 "this test image fails its health check on purpose"
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

mark 0 running starting
wait_for "the API answers on port 8080" curl -fsS --max-time 5 http://127.0.0.1:8080/healthz ||
    failed 0 "did not answer in 300 s"
mark 0 ok ready
mark 1 running starting
wait_for "the agent runs" systemctl is-active --quiet crust-agent.service || failed 1 "did not start in 300 s"
mark 1 ok running
for unit in zfs-import-cache.service zfs-import-scan.service zfs-mount.service; do
    if systemctl is-failed --quiet "$unit"; then
        failed 2 "$unit failed"
    fi
done
echo "ok: no ZFS import failed"
mark 2 ok "no import failed"
# A network that is down only rolls back a new image (a trial boot); on an
# ordinary boot it is more likely a cable than the image.
mark 3 running "waiting for a default route"
if ! wait_for "a default route with carrier" network; then
    [ "$trial" = true ] && failed 3 "no default route with carrier in 300 s"
    echo "no network, but this is not the first boot of a new image"
fi
mark 3 ok up
progress ok
