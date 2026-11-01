#!/bin/bash
# Signs every kernel module under /out with the Crustates module-signing key
# and puts the matching certificate into the image.
set -euo pipefail

kernel=$(cat /kmods/kernel.lock).$(uname -m)
key=/run/secrets/module-signing-key
cert=/run/secrets/module-signing-cert
sign_file="/usr/src/kernels/${kernel}/scripts/sign-file"
for secret in "$key" "$cert"; do
    if [ ! -s "$secret" ]; then
        echo "sign.sh: $secret is missing; pass the key with podman build --secret (cargo xtask molt build does)" >&2
        exit 1
    fi
done

# /out is copied over the image root. A directory in /out where the image has
# a symlink (Fedora's /usr/sbin -> bin, /lib -> usr/lib, ...) would replace the
# symlink and hide everything behind it, so refuse that.
while read -r path; do
    target=${path#/out}
    if [ -L "$target" ]; then
        echo "sign.sh: /out$target is a directory, but $target is a symlink to $(readlink "$target") in the image; install into the symlink's target instead" >&2
        exit 1
    fi
done < <(find /out -mindepth 1 -maxdepth 3 -type d)

mapfile -t modules < <(find /out/usr/lib/modules -name '*.ko')
if [ "${#modules[@]}" -eq 0 ]; then
    echo "sign.sh: no modules under /out/usr/lib/modules" >&2
    exit 1
fi
for module in "${modules[@]}"; do
    "$sign_file" sha512 "$key" "$cert" "$module"
    # sign-file appends this marker; its absence means the module is unsigned.
    tail -c 28 "$module" | grep -q '~Module signature appended~' || {
        echo "sign.sh: $module is not signed" >&2
        exit 1
    }
done
echo "signed ${#modules[@]} modules for ${kernel}: $(openssl x509 -inform DER -in "$cert" -noout -subject)"
install -D -m 0644 "$cert" /out/usr/share/crustates/keys/crustates-module-signing.der
