---
status: accepted
date: 2026-10-30
decision-makers: Miguel Martens
---

# Two compute resources, VM and App, with microVM isolation by default

## Context and Problem Statement

Users start workloads in two ways: from an ISO or disk image (Windows, Home
Assistant OS, OPNsense) or from an OCI image or Compose app. Crustates wants to
be secure by default, so an app should not share the host kernel unless the
user asks for it. How many compute resources do users see, and how isolated is
an app by default?

## Decision Drivers

- Secure by default: every app runs in its own microVM.
- Few concepts for users.
- An escape hatch where microVMs cost too much or the hardware cannot run them.

## Considered Options

- Two resources (VM and App) with isolation as a setting on App
- A separate resource or product per runtime (VM, microVM, container)
- Apps as shared-kernel containers by default, microVMs opt-in

## Decision Outcome

Chosen option: "Two resources with isolation as a setting on App", because
users think in terms of "an OS image" or "an app", and isolation is a property
of how an app runs, not a different product.

| User starts                 | Resource | Default runtime                                 | Option                                     |
| --------------------------- | -------- | ----------------------------------------------- | ------------------------------------------ |
| An ISO or disk image        | VM       | QEMU with KVM: UEFI, vTPM, VNC, USB             | Cloud Hypervisor (headless)                |
| An OCI image or Compose app | App      | MicroVM via Kata runtime-rs on Cloud Hypervisor | `isolation: shared` runs a plain container |

- containerd is the only container manager. Per app it picks Kata runtime-rs
  (microVM, default), crun (shared isolation) or later runwasi (Wasm).
- Compose files are imported into App resources.
- The engines behind these runtimes are recorded in ADR 0006.

### Consequences

- Good, because each app gets its own kernel without the user doing anything.
- Good, because users learn two resources, not one per runtime.
- Bad, because each app microVM costs tens of MB of RAM on top of the app;
  `isolation: shared` is the escape hatch.
- Bad, because microVMs need VT-x or AMD-V. Without them, apps fall back to
  shared containers, and the installer's hardware check says so.
- Bad, because Cloud Hypervisor on arm64 needs a GICv3-ITS interrupt
  controller, which no Raspberry Pi has. There, microVMs fall back to QEMU's
  `microvm` machine type or to shared isolation.

### Confirmation

The API has exactly two compute resources, VM and App. A new App without an
`isolation` field runs in a microVM.

## Pros and Cons of the Options

### Two resources, isolation as a setting

- Good, because it matches how users start workloads.
- Good, because changing isolation does not change the resource type.
- Bad, because one App resource must cover two quite different runtimes.

### A resource per runtime

- Good, because each resource maps to one engine.
- Bad, because users must learn the runtime differences up front.

### Shared containers by default

- Good, because containers use less memory.
- Bad, because apps share the host kernel unless the user opts out, which is
  not secure by default.

## More Information

- Plan: [Tide: compute](../plan.md#tide-compute),
  [Security model](../plan.md#security-model),
  [Open decisions and risks](../plan.md#open-decisions-and-risks)
- containerd as the container manager and crun as the OCI runtime are marked
  "Recommended" in the plan. crun can be replaced by youki once proven.
