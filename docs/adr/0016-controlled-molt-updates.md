---
status: accepted
date: 2026-11-03
decision-makers: Miguel Martens
---

# Molt updates under Crustates' control, with automatic rollback

## Context and Problem Statement

Molt (ADR 0002) is a bootc image. The base image enables
`bootc-fetch-apply-updates.timer`, which pulls the tracked tag every eight
hours and reboots into whatever it finds, at any time of day, without
checking a signature and without a health check. Homelab owners need to know
when their machine reboots, and a bad image must not stay. How do Molt
machines update?

## Decision Drivers

- Only images signed by the Crustates release workflows are installed.
- The owner chooses: updates by themselves in a nightly window, or only when
  asked.
- An image that does not boot, or boots but breaks Crustates, is rolled back
  without anyone at the machine, and is not installed again.
- The state is visible through the API: what runs, what is staged, what is
  available, what went wrong.
- No new long-running service beyond the agent.

## Considered Options

- The agent drives bootc; greenboot checks health; the boot counter is armed
  when an image is staged
- bootc's timer plus a containers signature policy
- Our own health-check unit instead of greenboot

## Decision Outcome

Chosen option: "The agent drives bootc; greenboot checks health", because it
is the only option that checks our keyless signatures, and greenboot already
does the boot-counting and rollback that a homemade unit would have to
re-implement.

- `bootc-fetch-apply-updates.timer` is masked. The agent checks the channel
  (the tag in `/usr/lib/crustates/molt.toml`, overridable in
  `/etc/crustates/molt.toml`) 5 to 20 minutes after start and then daily.
- A check resolves the tag to a digest with skopeo and skips digests that
  are booted, staged or marked bad. It verifies a new image with a pinned
  cosign: the certificate must name the `molt.yaml` or `promote.yaml`
  workflow of `crustates/crustates` on `development` or a `v*` tag, issued by
  GitHub Actions. Then `bootc switch <repository>@<digest>` stages it. The
  machine tracks the channel through the agent, not through bootc's own
  image reference.
- Policy `auto` (the default) stages new images and reboots into them inside
  the window (default `03:00-05:00` local time). Policy `manual` only reports
  them; `crust molt upgrade` stages one, `--now` also reboots.
- Health: greenboot (the Rust rewrite, `greenboot` 0.16 in Fedora 45) runs
  `required.d/50-crustates.sh` on every boot: the API answers, the agent
  runs, no ZFS import failed, and on the first boot of a new image a default
  route with carrier, within five minutes.
- greenboot arms its GRUB boot counter only after a health check fails in
  userspace, so a kernel that panics before userspace would boot forever.
  `crust-boot-trial.service` therefore sets `boot_counter=1` when an image is
  staged, next to greenboot's own trigger, and the kernel runs with
  `panic=10`. The new image gets one boot: if it panics or fails its health
  check, the machine is back on the previous image after at most one more
  reboot.
- When the health check fails on a new image, greenboot's `red.d` runs
  `crust-agent molt-mark-bad`; the agent never stages that digest again.
- Commands reach the agent over a stream it opens to the control plane
  (`NodeService.Commands`), so the agent keeps dialling out and the control
  plane needs no access to it. The agent reports the Molt status in every
  heartbeat; the API shows it in `Node.status.molt`.

### Consequences

- Good, because nothing unsigned is installed, and the owner decides when the
  machine reboots.
- Good, because a broken image costs one or two reboots, not a trip to the
  machine.
- Good, because greenboot and bootc stay upstream's code.
- Bad, because the agent runs as root with little sandboxing: bootc,
  mokutil and reboots need it.
- Bad, because the boot counter lives in `grub.cfg`, which bootupd writes only
  at install time: machines installed before this change roll back only after
  a reinstall. `crust molt status` shows whether a machine has it.
- Bad, because cosign is a downloaded binary, not a Fedora package; its
  version and checksum are pinned in the Containerfile.

### Confirmation

`cargo xtask molt test --upgrade` boots image A, upgrades it through the API
to image B, then to an image whose health check fails; greenboot must roll
back to B, and the agent must refuse the failed image afterwards. Unit tests
cover the policy, the window, bad digests and signature failures with a fake
OS.

## Pros and Cons of the Options

### The agent drives bootc

- Good, because it can check keyless (Fulcio) signatures by workflow identity.
- Good, because the API sees and controls every step.
- Bad, because it is code we maintain.

### bootc's timer plus a signature policy

- Good, because it is built in.
- Bad, because the containers `policy.json` matches Fulcio certificates by
  e-mail only (`subjectEmail`), not by workflow URI, and cannot read cosign's
  newer bundle format yet (containers/container-libs#388, open on
  2026-11-01).
- Bad, because it reboots whenever an update lands.

### Our own health-check unit

- Good, because it would be exactly what we need.
- Bad, because rolling back safely needs boot counting in GRUB, which
  greenboot already ships and Fedora already packages.

## More Information

- ADR 0002 (Fedora bootc), ADR 0014 (GHCR)
- greenboot-rs: <https://github.com/fedora-iot/greenboot-rs> (0.16.4 read for
  this decision)
- Code: `crates/crust-agent/src/molt.rs`, `crates/crust-agent/src/bootc.rs`,
  `molt/rootfs/usr/lib/greenboot/`, `molt/rootfs/usr/libexec/crustates/crust-boot-trial`
