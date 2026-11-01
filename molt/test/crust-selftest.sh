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

# Writes failed units and boot warnings to the serial consoles, so a failed
# boot test can be read from its serial log alone.
diagnose() {
    local tty
    for tty in $(tr ' ' '\n' </sys/class/tty/console/active | grep -E '^tty(S|AMA)[0-9]+$'); do
        {
            echo "----- crust-selftest diagnostics -----"
            systemctl --failed --no-pager
            journalctl -b -p warning --no-pager | tail -n 60
            echo "----- end of diagnostics -----"
        } >"/dev/$tty" 2>&1
    done
}

fail() {
    diagnose
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

# Storage: ZFS from the signed out-of-tree module, Btrfs from the kernel.
modprobe zfs || fail "modprobe zfs failed"
signer=$(modinfo -F signer zfs)
[[ "$signer" == *Crustates* ]] || fail "zfs.ko is signed by '${signer}', not the Crustates key"
[ -s /usr/share/crustates/keys/crustates-module-signing.der ] || fail "the module-signing certificate is missing"
work=/var/tmp/crust-selftest
mkdir -p "$work"
truncate -s 256M "$work/zfs-vdev"
zpool create -m none selftest "$work/zfs-vdev" || fail "zpool create on a file vdev failed"
zpool list -H -o health selftest | grep -qx ONLINE || fail "the selftest pool is not ONLINE"
zpool destroy selftest
truncate -s 256M "$work/btrfs.img"
mkfs.btrfs -q "$work/btrfs.img" || fail "mkfs.btrfs failed"
mkdir -p "$work/btrfs"
mount -o loop "$work/btrfs.img" "$work/btrfs" || fail "mounting Btrfs on a loop device failed"
touch "$work/btrfs/ok" && umount "$work/btrfs"
rm -rf "$work"

retry 60 curl -fsS http://127.0.0.1:8080/healthz || fail "/healthz did not answer within 60 s"
[ -f /var/lib/crustates/crust.db ] || fail "no database in /var/lib/crustates"
retry 60 sh -c 'crust node list | grep -qw Ready' || fail "no Ready node within 60 s: $(crust node list 2>&1 | tr '\n' ' ')"

result PASS
