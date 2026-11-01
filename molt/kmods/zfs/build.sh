#!/bin/bash
# Builds the OpenZFS kernel modules and userland for the kernel in
# kernel.lock and installs them into /out.
set -euo pipefail
# shellcheck source-path=SCRIPTDIR
source "$(dirname "$0")/zfs.env"

kernel=$(cat /kmods/kernel.lock).$(uname -m)
cd /tmp
curl -sSfL -o zfs.tar.gz "https://github.com/openzfs/zfs/releases/download/zfs-${ZFS_VERSION}/zfs-${ZFS_VERSION}.tar.gz"
echo "${ZFS_SHA256}  zfs.tar.gz" | sha256sum --check --quiet
tar -xzf zfs.tar.gz
cd "zfs-${ZFS_VERSION}"

# Refuse kernels this release does not support, before spending time on a
# build that would fail or, worse, build and misbehave.
max=$(awk '/^Linux-Maximum:/ { print $2 }' META)
min=$(awk '/^Linux-Minimum:/ { print $2 }' META)
series=$(echo "$kernel" | cut -d. -f1,2)
if [ "$(printf '%s\n' "$min" "$series" "$max" | sort -V | sed -n 2p)" != "$series" ]; then
    echo "OpenZFS ${ZFS_VERSION} supports Linux ${min} to ${max}, but kernel.lock has ${kernel}." >&2
    echo "Wait for an OpenZFS release that supports Linux ${series}, or keep the current kernel." >&2
    exit 1
fi

./configure \
    --prefix=/usr --sysconfdir=/etc --localstatedir=/var --libdir=/usr/lib64 \
    --sbindir=/usr/bin \
    --with-linux="/usr/src/kernels/${kernel}" --with-linux-obj="/usr/src/kernels/${kernel}" \
    --with-udevdir=/usr/lib/udev --with-mounthelperdir=/usr/bin \
    --with-systemdunitdir=/usr/lib/systemd/system \
    --with-systemdpresetdir=/usr/lib/systemd/system-preset \
    --with-systemdmodulesloaddir=/usr/lib/modules-load.d \
    --with-dracutdir=/usr/lib/dracut \
    --enable-systemd --disable-sysvinit --disable-pyzfs
make -j"$(nproc)"
make install DESTDIR=/out
# Kbuild installs modules under /lib, a symlink to usr/lib in the image.
if [ -d /out/lib/modules ]; then
    mkdir -p /out/usr/lib
    cp -a /out/lib/modules /out/usr/lib/
    rm -rf /out/lib
fi
# Headers, static libraries and libtool files are for building, not running.
rm -rf /out/usr/include /out/usr/src /out/usr/lib64/*.a /out/usr/lib64/*.la \
    /out/usr/share/initramfs-tools /out/var
ls /out/usr/lib/modules/"${kernel}"/extra/
