---
status: proposed
date: 2026-10-30
decision-makers: Miguel Martens
---

# Full VMs on QEMU, microVMs on Cloud Hypervisor via Kata runtime-rs

## Context and Problem Statement

ADR 0004 gives users two compute resources: VM and App. Full VMs must run
Windows and ISO installers (OPNsense, Home Assistant OS) with a display, USB,
GPU and PCI passthrough, Secure Boot and a vTPM. Apps run in microVMs that must
start fast and stay small. Which hypervisor runs each?

## Decision Drivers

- Full VMs: display, USB, Windows guests, GPU and PCI passthrough, UEFI with
  Secure Boot, vTPM.
- MicroVMs: small, fast, driven through containerd like a container.
- Prefer Rust components where they are mature.
- Run on amd64 and arm64, including boards without a GICv3-ITS.

## Considered Options

- QEMU with KVM for full VMs; Cloud Hypervisor via Kata runtime-rs for
  microVMs
- Cloud Hypervisor for everything
- Firecracker for microVMs
- QEMU's `microvm` machine type for microVMs

## Decision Outcome

Chosen option: "QEMU for full VMs; Cloud Hypervisor via Kata runtime-rs for
microVMs", because QEMU has the display, USB, Windows and GPU support that
Cloud Hypervisor lacks, and Kata runtime-rs (Rust, the default since Kata 4.0)
plugs Cloud Hypervisor microVMs into containerd.

- VMs run on QEMU with KVM: UEFI, vTPM (swtpm) with Secure Boot so Windows 11
  installs, the virtio driver ISO bundled, consoles through noVNC or SPICE.
- Headless Linux VMs may use Cloud Hypervisor instead.
- App microVMs run on Cloud Hypervisor through Kata Containers runtime-rs.
- On boards without a GICv3-ITS, microVMs fall back to QEMU `microvm` or to
  shared isolation.

### Consequences

- Good, because every VM feature users expect from Proxmox is covered.
- Good, because the microVM path is Rust end to end on the host side.
- Bad, because two hypervisors must be supported and tested.
- Bad, because no Raspberry Pi can run Cloud Hypervisor microVMs.

### Confirmation

The VM driver and the microVM runtime each sit behind a trait with contract
tests. VM-in-CI tests boot a UEFI VM on QEMU and an App microVM through Kata.

## Pros and Cons of the Options

### QEMU for VMs, Cloud Hypervisor via Kata for microVMs

- Good, because each engine does what it is best at.
- Bad, because two engines to maintain.

### Cloud Hypervisor for everything

- Good, because one Rust hypervisor.
- Bad, because it lacks the display, USB, Windows and GPU support full VMs
  need.

### Firecracker for microVMs

- Good, because it starts fast.
- Bad, because Kata only supports Firecracker through its deprecated Go
  runtime.
- Neutral: kept as a later option, driven directly, for short-lived untrusted
  jobs (CI runners, scheduled scripts, AI agent sandboxes) where shared volumes
  and GPUs are not needed.

### QEMU `microvm` for microVMs

- Good, because it runs on GICv2 boards.
- Neutral: used as the fallback where Cloud Hypervisor cannot run.

## More Information

- Plan: [Tide: compute](../plan.md#tide-compute),
  [Tech stack](../plan.md#tech-stack), [Later](../plan.md#later),
  [Open decisions and risks](../plan.md#open-decisions-and-risks)
- Status is "proposed" because the plan marks both the full-VM engine and the
  microVM engine as "Recommended", not decided.
