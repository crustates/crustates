---
status: accepted
date: 2026-11-03
decision-makers: Miguel Martens
---

# Install media: a live Molt with our installer, built by image-builder

## Context and Problem Statement

Molt installs today by writing a disk image (`raw.zst`) onto the boot disk
from another computer or a live Linux stick, or as a VM disk. That is fine
for people who know `dd`, and it skips every choice: which disk, a pool, the
network, the admin secrets. A homelab owner expects a USB stick that boots,
checks the machine and asks a few questions. What do we put on that stick,
and how is it built?

## Decision Drivers

- It installs offline: the image is on the stick, signed as released.
- One installer for the screens (people) and an answers file (no one at the
  machine, and CI).
- The installed machine is exactly what a disk image would give: the same
  image, digest and boot loader, following its channel afterwards.
- Built by the release workflow on both architectures, with tools we already
  run (podman, a pinned builder container).
- Anaconda is not an option for long: `anaconda-iso` is deprecated, and its
  steps are not ours.

## Considered Options

- image-builder's `generic-iso` type: the Molt image booted live with our
  installer, the payload inside as an OCI layout
- image-builder's `bootc-installer` type (Anaconda with a bootc payload)
- Our own ISO build with xorriso and grub2-mkrescue
- No ISO: disk images only

## Decision Outcome

Chosen option: "`generic-iso` with our installer", because it boots the
installer we write on the image we ship, and image-builder handles the parts
that are easy to get wrong (El Torito, the EFI image, the hybrid MBR, the
squashfs, the media checksum).

- `molt/installer/Containerfile` builds the live system FROM the Molt image
  it installs: bootc takes the install configuration and the SELinux policy
  from the system it runs on. It adds dracut-live (live boot and the media
  check), the ISO tools image-builder runs inside it, `crust-installer` on
  tty1, and masks what must not run live (API, agent, console, health checks,
  pool imports).
- The payload is the image as an OCI layout in
  `/usr/lib/crustates/payload:molt`, copied with `skopeo copy
--preserve-digests` from the registry by digest for releases, so the
  installed machine records the digest the release workflow signed. An OCI
  layout keeps the compressed layers; `containers-storage` would need about
  as much memory as the image during the install (image-builder #2536).
- `crust-installer` installs with `bootc install to-disk --wipe` from that
  layout, with `--target-imgref ghcr.io/crustates/molt:<channel>`, then
  writes the hostname, the network, the channel and the hashes of the admin
  token and recovery PIN into the new system, and makes the ZFS pool.
  `crust-firstboot` gives the hashes their group at the first boot.
- image-builder runs as its upstream container, pinned by digest, with the
  `bootc-generic-iso` type name every version accepts. The boot menu comes
  from `/usr/lib/image-builder/bootc/iso.yaml` in the live image: install,
  basic graphics, rescue, check media. The kernel arguments are written out
  in each entry, because image-builder 87 and 88 treat `kernel_args`
  differently.
- An answers file (`answers.toml` on a stick labelled `CRUSTANS`, or QEMU's
  `fw_cfg`) installs without questions; the installer prints the token and
  PIN on the console. CI installs the test image this way on an empty VM
  with a mirrored pool and boots the result.
- Releases attach the ISO, its checksum and its Sigstore bundle, like the
  disk images.

### Consequences

- Good, because the stick installs offline what the release signed, by
  digest, and the machine then follows its channel.
- Good, because people and CI use the same installer and the same install
  code.
- Bad, because image-builder's menu entries are kernel command lines only:
  "boot from the first disk" and "firmware settings" cannot be entries. The
  installer offers the firmware settings when the hardware check fails.
- Bad, because the ISO holds the image twice (live system and payload),
  about 3 GB.
- Bad, because the arm64 ISO is built on releases but not boot-tested: the
  hosted arm64 runners have no KVM, and image-builder does not boot-test
  arm64 ISOs either.
- Bad, because bootc 1.16 (in the pinned base) has no `bootc install
mount`, so the installer finds the new deployment's `/etc` and `/var` on
  the root partition itself.

### Confirmation

`cargo xtask molt test-install` builds the test image and its ISO, installs
it on an empty VM from an answers file with a mirrored pool, and boots the
installed disk: the selftest must pass, the pool must be imported, and the
API must accept the token the installer printed. CI runs it on pull requests
that touch the installer.

## Pros and Cons of the Options

### `bootc-installer` (Anaconda)

- Good, because Anaconda is maintained by Fedora.
- Bad, because its steps, screens and kickstart are Anaconda's, not the
  design kit's, and image-builder's own docs prefer `generic-iso`.

### Our own ISO build

- Good, because every byte is ours.
- Bad, because BIOS and UEFI boot, the hybrid image and the squashfs are a
  lot to maintain for no gain.

### Disk images only

- Bad, because writing a raw image with `dd` from another computer is not a
  homelab install, and it asks nothing.

## More Information

- ADR 0002 (Molt), ADR 0016 (updates), ADR 0017 (channels), ADR 0018
  (console and recovery PIN)
- image-builder: <https://github.com/osbuild/image-builder> (v89.0.0 read
  for this decision); bootc install: `bootc-install-to-disk(8)`
- Code: `crates/crust-installer`, `molt/installer`, `xtask/src/molt.rs`
  (`iso`, `test-install`)
