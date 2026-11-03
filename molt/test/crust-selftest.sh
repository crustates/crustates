#!/bin/bash
# Checks a booted Molt test image. Runs at every boot as root and prints the
# admin token (CRUST-SELFTEST-TOKEN: crt_…) and one result line:
# CRUST-SELFTEST: PASS <version>, or CRUST-SELFTEST: FAIL <reason>.
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
[ "$(systemctl is-enabled bootc-fetch-apply-updates.timer 2>&1)" = masked ] ||
    fail "bootc-fetch-apply-updates.timer is not masked: the agent decides when to update"
/usr/libexec/crustates/cosign version >/dev/null 2>&1 || fail "the pinned cosign does not run"
grep -q boot_counter /boot/grub2/grub.cfg || fail "grub.cfg has no greenboot boot counter: no automatic rollback"

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
[ "$(systemctl show -P User crust-api.service)" = crustates ] || fail "crust-api does not run as crustates"

# The admin token and recovery PIN: on the first boot from what the console
# shows, which is gone after a reboot, so this test keeps a copy (test images
# only).
saved=/var/lib/crust-selftest
first=/run/crustates/firstboot.json
retry 60 test -s "$first" -o -s "$saved/token" || fail "no first-boot secrets within 60 s"
if [ -s "$first" ]; then
    [ "$(stat -c '%U %a' "$first")" = "root 600" ] || fail "$first is $(stat -c '%U %a' "$first"), want root 600"
    install -d -m 0700 "$saved"
    jq -r .token "$first" >"$saved/token"
    jq -r .pin "$first" >"$saved/pin"
fi
token=$(cat "$saved/token")
pin=$(cat "$saved/pin")
[[ "$pin" =~ ^[0-9]{9}$ ]] || fail "the recovery PIN is not nine digits"
echo "<2>CRUST-SELFTEST-TOKEN: $token" >/dev/kmsg
[ "$(stat -c '%U:%G %a' /var/lib/crustates/secrets)" = "root:crustates 750" ] ||
    fail "/var/lib/crustates/secrets is $(stat -c '%U:%G %a' /var/lib/crustates/secrets), want root:crustates 750"
for secret in admin-token recovery-pin; do
    [ "$(stat -c '%U:%G %a' /var/lib/crustates/secrets/$secret)" = "root:crustates 640" ] ||
        fail "the $secret hash is $(stat -c '%U:%G %a' /var/lib/crustates/secrets/$secret), want root:crustates 640"
done
code=$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8080/v1/nodes)
[ "$code" = 401 ] || fail "/v1/nodes without the token answered $code, want 401"

# The local console: on tty1 and tty2, no login prompts, its socket for root.
for unit in crust-console crust-console-logs; do
    retry 30 systemctl is-active --quiet "$unit" || fail "$unit is not running"
done
[ "$(systemctl is-enabled getty@tty1.service 2>&1)" = masked ] || fail "getty@ is not masked"
socket=/run/crustates/api.sock
[ "$(stat -c '%U:%G %a' "$socket")" = "crustates:crustates 660" ] ||
    fail "$socket is $(stat -c '%U:%G %a' "$socket" 2>&1), want crustates:crustates 660"
code=$(curl -s -o /dev/null -w '%{http_code}' --unix-socket "$socket" http://localhost/v1/nodes)
[ "$code" = 200 ] || fail "/v1/nodes through the local socket answered $code, want 200"
code=$(curl -s -o /dev/null -w '%{http_code}' --unix-socket "$socket" -H 'content-type: application/json' \
    -d "{\"pin\":\"$pin\",\"action\":\"selftest\"}" http://localhost/v1/local/pin)
[ "$code" = 204 ] || fail "the recovery PIN check answered $code, want 204"
code=$(setpriv --reuid=nobody --regid=nobody --clear-groups curl -s -o /dev/null -w '%{http_code}' --unix-socket "$socket" http://localhost/v1/nodes)
[ "$code" != 200 ] || fail "the local socket served a user who is not root"
[ -s /usr/lib/kbd/consolefonts/crustates-console-16.psfu ] || fail "the console font is missing"

# The boot splash is in the initramfs, ZFS is not.
initramfs="/usr/lib/modules/$(uname -r)/initramfs.img"
lsinitrd "$initramfs" | grep -q 'plymouth/themes/crustates/crustates.script' || fail "the initramfs has no Crustates splash"
! lsinitrd "$initramfs" | grep -q '/zfs.ko' || fail "the initramfs has the ZFS module"
export CRUST_TOKEN=$token
retry 60 sh -c 'crust node list | grep -qw Ready' || fail "no Ready node within 60 s: $(crust node list 2>&1 | tr '\n' ' ')"

# greenboot's verdict on this boot.
health_done() {
    case $(systemctl show -P ActiveState greenboot-healthcheck.service) in
        active | failed) return 0 ;;
        *) return 1 ;;
    esac
}
retry 360 health_done
systemctl is-active --quiet greenboot-healthcheck.service || fail "the greenboot health check failed: $(journalctl -b -u greenboot-healthcheck --no-pager | grep -E 'FAIL|error' | tail -3 | tr '\n' ' ')"

# shellcheck source=/dev/null
result "PASS $(. /usr/lib/os-release && echo "$IMAGE_VERSION")"
