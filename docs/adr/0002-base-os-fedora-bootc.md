---
status: accepted
date: 2026-10-30
decision-makers: Miguel Martens
---

# Base OS: Fedora bootc with ZFS and Btrfs

## Context and Problem Statement

Molt is the read-only, image-based OS that Crustates ships: the whole OS,
kernel and initrd (or UKI) as an OCI image, with atomic A/B upgrades and
rollback. Reef needs both ZFS and Btrfs as first-class pool types. Btrfs lives
in the kernel; ZFS is an out-of-tree module that must match the kernel exactly.
Which base OS and kernel give us both, without the module breaking on updates?

## Decision Drivers

- ZFS and Btrfs both first-class.
- Read-only root and atomic upgrades with rollback.
- No DKMS-style breakage when the kernel updates.
- Secure Boot with signed out-of-tree modules.
- Small lag between a new kernel and OpenZFS support.

## Considered Options

- Fedora bootc with Fedora's own kernel, pinned per release
- CentOS Stream 10 bootc
- A separately built LTS kernel
- A package-based distribution with ZFS through DKMS

## Decision Outcome

Chosen option: "Fedora bootc with Fedora's own kernel, pinned per release",
because Fedora has Btrfs in its kernel, OpenZFS follows Fedora kernels closely,
and an image build lets CI refuse any kernel ZFS cannot build against.

- The OS is a bootc image built from a Containerfile and pulled from a
  registry. The root is read-only with composefs and fs-verity.
- The kernel is pinned per Molt release.
- CI compiles the ZFS module against that exact kernel. An image only ships if
  ZFS builds and both pool types pass tests. This is the approach uCore already
  uses on Fedora bootc.
- The ZFS module and other out-of-tree modules are signed with the Crustates
  key; Secure Boot machines enrol that key once.
- bcachefs is an opt-in experimental driver only.

### Consequences

- Good, because Btrfs works with no extra module.
- Good, because a kernel that breaks ZFS never reaches users.
- Good, because hardware support is Fedora's: the mainline kernel plus
  linux-firmware.
- Bad, because a new Fedora kernel can land before OpenZFS supports it; pinning
  and the CI gate hold the kernel back until it does.
- Bad, because Secure Boot users must enrol the Crustates key once.
- Bad, because ZFS is CDDL-licensed. Shipping its module in an image is common
  practice (uCore does it) but debated legally. The AGPL userland is not
  affected.

### Confirmation

The Molt image build in CI compiles ZFS against the pinned kernel, signs the
module, and boots the image in a VM-in-CI test with a ZFS and a Btrfs pool.

## Pros and Cons of the Options

### Fedora bootc, Fedora kernel pinned per release

- Good, because Btrfs is in the kernel.
- Good, because OpenZFS 2.4.4 supports kernels up to Linux 7.2, so the lag is
  small.
- Good, because uCore proves the build-ZFS-in-CI approach on the same base.
- Bad, because Fedora kernels move fast, so pinning is required.

### CentOS Stream 10 bootc

- Bad, because it does not ship Btrfs by default: Red Hat enables it only as a
  module in `kernel-modules-internal`, which is not installed by default.

### A separately built LTS kernel

- Good, because an LTS kernel changes less often.
- Bad, because we would build and maintain the kernel ourselves.
- Neutral: kept as a later option (uCore offers an LTS-kernel stream for
  servers).

### Package-based distribution with ZFS through DKMS

- Bad, because DKMS rebuilds the module on the user's machine after a kernel
  update and can fail there: the classic ZFS breakage.
- Bad, because it gives no read-only root or atomic rollback.

## More Information

- Plan: [Molt: the operating system](../plan.md#molt-the-operating-system),
  [Licence and base OS](../plan.md#licence-and-base-os),
  [Hardware support](../plan.md#hardware-support),
  [Open decisions and risks](../plan.md#open-decisions-and-risks)
- Still to verify: whether the bootc image boots on a Raspberry Pi.
