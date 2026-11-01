#!/bin/bash
# Checks a booted Molt test image. Runs once at boot as root and prints one
# result line: CRUST-SELFTEST: PASS, or CRUST-SELFTEST: FAIL <reason>.
set -uo pipefail

# Prints the result line to the journal and to every kernel console. On
# arm64 /dev/console is not always the serial port, but /dev/kmsg reaches it.
result() {
    echo "CRUST-SELFTEST: $*"
    echo "<2>CRUST-SELFTEST: $*" >/dev/kmsg
}

fail() {
    result "FAIL $*"
    exit 1
}

# Retries "$@" once a second for up to $1 seconds.
retry() {
    local seconds=$1
    shift
    for _ in $(seq "$seconds"); do
        "$@" >/dev/null 2>&1 && return 0
        sleep 1
    done
    return 1
}

[ "$(getenforce)" = Enforcing ] || fail "SELinux is $(getenforce), want Enforcing"
grep -qx 'VARIANT_ID=molt' /usr/lib/os-release || fail "os-release has no VARIANT_ID=molt"
grep -q '^IMAGE_VERSION=.' /usr/lib/os-release || fail "os-release has no IMAGE_VERSION"
! command -v sshd >/dev/null || fail "sshd is installed"
nft list table inet crustates >/dev/null || fail "the crustates nftables table is not loaded"
! touch /usr/.crust-selftest 2>/dev/null || fail "/usr is writable"

retry 60 curl -fsS http://127.0.0.1:8080/healthz || fail "/healthz did not answer within 60 s"
[ -f /var/lib/crustates/crust.db ] || fail "no database in /var/lib/crustates"
retry 60 sh -c 'crust node list | grep -qw Ready' || fail "no Ready node within 60 s: $(crust node list 2>&1 | tr '\n' ' ')"

result PASS
