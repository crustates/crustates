---
status: accepted
date: 2026-10-30
decision-makers: Miguel Martens
---

# Reposition Crustates as a self-hosted homelab platform

## Context and Problem Statement

Crustates started as a hosted cloud (a PaaS). The new plan turns it into "your
own cloud, on your own hardware": one Rust platform for compute, storage and
networking that replaces the Proxmox + TrueNAS + Portainer stack in a homelab.
The brand and the product names carry over, but their meaning changes. What
does Crustates become, and what do its product names mean now?

## Decision Drivers

- Homelab and self-hosting users, small offices and edge sites want VMs, apps
  and storage without stitching together five tools.
- Fully open source and self-hosted: no cloud account required (plan principle
  5).
- Keep the existing Crustates brand and assets.

## Considered Options

- Keep the hosted cloud (PaaS) direction of the earlier plan
- Reposition as a self-hosted homelab platform, keeping the brand and giving
  the product names new meanings

## Decision Outcome

Chosen option: "Reposition as a self-hosted homelab platform", because it is
the product the plan describes: one coherent platform on the user's own
hardware, with no cloud account.

The product names now mean:

| Name      | Area               | Covers                                                     |
| --------- | ------------------ | ---------------------------------------------------------- |
| Crustates | Platform           | Everything; the CLI is `crust`                             |
| Tide      | Compute            | VMs and apps (microVMs, containers)                        |
| Reef      | Storage            | Pools, shares, S3, snapshots, backups                      |
| Current   | Networking         | Bridges, VLANs, eBPF policy, firewall, mesh, remote access |
| Molt      | OS lifecycle       | Atomic upgrades and rollback                               |
| Cast      | Cluster            | Joining nodes and sites                                    |
| Barnacle  | Plugins            | Sandboxed Wasm extensions                                  |
| Spawn     | GitOps and deploys | Git sources, drift, PR plans, push-to-deploy, previews     |

The AI assistant is called "Assistant" for now (see ADR 0011). Theme names are
for things users see; code names stay plain (`vm-driver`, not `tide-driver`).

### Consequences

- Good, because one product replaces several tools a homelab runs today.
- Good, because the brand kit (`crustates/brand`, `@crustates/brand`,
  `@crustates/ui`) is reused as it is.
- Bad, because the old PaaS code is removed and the API, auth, agent, TUI, MCP
  server, xtask and CI are built new in M0 and M1.
- Bad, because Proxmox, TrueNAS and Portainer in one product is a large scope;
  each milestone is held to one working machine.
- Neutral: positioning copy, TRADEMARK.md subtitles and baked-in assets (OG
  image, badges) change from "hosted cloud" to "your own cloud, on your own
  hardware".

### Confirmation

README, docs and CLI subcommands use the names above; crates and modules use
plain names.

## Pros and Cons of the Options

### Keep the hosted cloud direction

- Good, because it is the earlier plan.
- Bad, because it needs a cloud account, which conflicts with the fully open
  source and self-hosted principle of the new plan.

### Self-hosted homelab platform

- Good, because users keep full control of their hardware and data.
- Good, because the brand carries over; only the meaning of the names changes.
- Bad, because the PaaS code cannot be reused.

## More Information

- Plan: [Vision and positioning](../plan.md#vision-and-positioning),
  [Names and brand](../plan.md#names-and-brand),
  [Way of working and repo](../plan.md#way-of-working-and-repo)
- The plan does not record why the hosted direction was dropped beyond the
  self-hosted principle.
