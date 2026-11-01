---
status: accepted
date: 2026-11-01
decision-makers: Miguel Martens
---

# Image hosting on GitHub Container Registry

## Context and Problem Statement

Molt and the platform ship as OCI images: bootc pulls OS updates from a
registry, and installers and disk images are built from the same images. Each
image is signed with cosign and carries an SBOM. The plan left open which
registry hosts them. Where do Crustates images live?

## Decision Drivers

- Free public hosting, and anonymous pulls without tight rate limits, since
  every Molt machine pulls its updates.
- Supports OCI artifacts next to the image: cosign signatures and SBOM
  attestations.
- Publishing from CI without extra long-lived credentials.
- Little to run or maintain: Crustates is maintained by one person for now.

## Considered Options

- GitHub Container Registry (`ghcr.io`)
- Quay.io
- Docker Hub
- A self-hosted registry (for example zot)

## Decision Outcome

Chosen option: "GitHub Container Registry", because it sits next to the code
and CI, publishes with the workflow's own `GITHUB_TOKEN`, and serves public
images and their signatures for free.

- The OS image is `ghcr.io/crustates/molt`; platform images follow as
  `ghcr.io/crustates/<name>`.
- Tags: `development` and `sha-<commit>` from the `development` branch;
  `<version>` and `beta` from `v*` tags. A `stable` channel comes later.
  Per-architecture images are tagged `sha-<commit>-amd64` and
  `sha-<commit>-arm64`; the other tags are multi-architecture.
- Images are signed keyless with cosign (GitHub OIDC). Each architecture's
  SPDX SBOM is attached as an OCI referrer (`oras discover` lists it) and its
  digest is signed too; a full SBOM is too large for the public Rekor log.
- Offline updates from a USB stick (plan) do not depend on the registry.

### Consequences

- Good, because publishing needs no secrets beyond what GitHub Actions gives
  every workflow.
- Good, because signatures, SBOMs and images sit in one place.
- Bad, because updates depend on GitHub's availability; a mirror can be added
  later without changing the image names users configure, if we put a
  Crustates domain in front.
- Neutral: packages must be set to public once, in the organisation's package
  settings.

### Confirmation

The `molt` workflow publishes to `ghcr.io/crustates/molt`, and
`cosign verify` with the workflow identity succeeds for every published tag.

## Pros and Cons of the Options

### GitHub Container Registry

- Good, because it uses `GITHUB_TOKEN` and supports OCI artifacts.
- Bad, because it ties distribution to GitHub.

### Quay.io

- Good, because Fedora's own bootc images live there.
- Bad, because publishing needs a robot account secret.

### Docker Hub

- Good, because it is the default registry for many tools.
- Bad, because anonymous pulls are rate limited, which hurts machines that
  check for updates.

### Self-hosted registry

- Good, because it is fully under our control.
- Bad, because it is one more service to run, secure and keep online.

## More Information

- Plan: [Molt: the operating system](../plan.md#molt-the-operating-system)
  (update delivery), [Open decisions and risks](../plan.md#open-decisions-and-risks)
- Workflow: `.github/workflows/molt.yaml`
