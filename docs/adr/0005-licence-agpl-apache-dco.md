---
status: accepted
date: 2026-10-30
decision-makers: Miguel Martens
---

# Licence: AGPLv3 platform, Apache-2.0 SDKs and plugin interfaces, DCO

## Context and Problem Statement

Crustates is fully open source and self-hosted. Its licence has to keep it
free to run anywhere, stop someone from selling a hosted version with private
changes, and still let plugins and integrations use any licence. Contributors
need a way to state they may submit their code. Which licences, and CLA or DCO?

## Decision Drivers

- Fully open source: no source-available licence.
- No hosted Crustates with private changes.
- Plugins, SDK users and integrations may use any licence.
- Keep contributors: LXD's switch to AGPL plus a CLA pushed contributors to
  Incus.

## Considered Options

- AGPLv3 platform, Apache-2.0 for what others build against, DCO
- AGPLv3 with a CLA (Proxmox VE, LXD)
- Apache-2.0 for everything, no CLA (Incus)
- Open core: GPL/LGPL plus proprietary parts (TrueNAS Community Edition)
- Source-available: BSL 1.1 (Dockhand), PolyForm Noncommercial (WolfStack),
  FSL

## Decision Outcome

Chosen option: "AGPLv3 platform, Apache-2.0 for what others build against,
DCO", because the AGPL keeps the platform open including when hosted, Apache-2.0
leaves integrations free, and a DCO keeps the bar for contributors low.

- The platform is licensed under `AGPL-3.0-or-later`.
- Apache-2.0 covers the API spec, client SDKs (`sdk/`), plugin interfaces
  (`wit/`) and the OpenTofu provider.
- Every commit is signed off under the
  [Developer Certificate of Origin](https://developercertificate.org); there is
  no CLA.
- Brand assets are CC BY 4.0 with a trademark policy (TRADEMARK.md in
  `crustates/brand`).

### Consequences

- Good, because anyone can run Crustates anywhere for free; obligations only
  start when someone modifies it and distributes or hosts it.
- Good, because a cloud provider cannot sell hosted Crustates with private
  changes.
- Good, because plugins and integrations can use any licence.
- Bad, because relicensing later needs every contributor's consent. "Or later"
  lets a future AGPL version apply without that consent.
- Neutral: the same split as NetBird (permissive client, AGPLv3 server).

### Confirmation

`LICENSE` at the root is the AGPLv3; `sdk/LICENSE` and `wit/LICENSE` are
Apache-2.0. Every crate's `license` field matches its folder, cargo-deny only
allows dependency licences compatible with the AGPL, and a pull request with
unsigned commits cannot be merged.

## Pros and Cons of the Options

### AGPLv3 + Apache-2.0 + DCO

- Good, because it is what the closest peer, Proxmox, builds a business on.
- Good, because it protects against hosted forks with private changes.
- Bad, because relicensing needs every contributor's consent.

### AGPLv3 with a CLA

- Good, because the project can relicense later.
- Bad, because LXD's switch to a CLA pushed contributors away to Incus.

### Apache-2.0 for everything

- Good, because it is the most permissive and needs no CLA.
- Bad, because a cloud provider could sell hosted Crustates with private
  changes.

### Open core

- Good, because proprietary parts can fund development.
- Bad, because proprietary parts break the fully open source principle.

### Source-available (BSL, PolyForm, FSL)

- Good, because it restricts commercial use.
- Bad, because it is not open source, which puts off homelab users and
  distributions.

## More Information

- Plan: [Licence and base OS](../plan.md#licence-and-base-os),
  [Open decisions and risks](../plan.md#open-decisions-and-risks)
- The plan says "AGPLv3". "Or later" was chosen when the first ADRs were
  written, to soften the relicensing trade-off of the DCO.
