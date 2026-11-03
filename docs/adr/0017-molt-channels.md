---
status: accepted
date: 2026-11-03
decision-makers: Miguel Martens
---

# Molt channels: stable, beta and development, promoted by digest

## Context and Problem Statement

Every Molt machine follows `beta` (ADR 0016), and every `v*` tag publishes to
`beta` (ADR 0014). People who want a machine that only gets releases others
have run for a while have nothing to follow. How does an image reach those
machines, and how does a machine pick what it follows?

## Decision Drivers

- `stable` runs exactly the bytes that ran on `beta`: no rebuild, the same
  digest and signatures.
- A person decides when a release goes to `stable`, and the release must have
  had time on `beta` first.
- A machine never moves to an older version without someone asking for it: a
  newer version may have changed data that the older one cannot read.
- A fresh install needs no choice to get a sensible channel.
- No scheduled workflow: Actions minutes are limited.

## Considered Options

- Tags in one repository, promoted by digest from a manual workflow
- A rebuild of the release for `stable`
- One repository per channel (`molt-stable`, `molt-beta`)
- Automatic promotion after seven days (scheduled workflow)

## Decision Outcome

Chosen option: "Tags in one repository, promoted by digest from a manual
workflow", because it ships the tested bytes, keeps one set of signatures and
SBOMs, and leaves the decision with a person.

- The channels are tags of `ghcr.io/crustates/molt`: `development` (every
  merge), `beta` (every `v*` release, pre-releases too) and `stable`.
- `promote.yaml` runs by hand on `development`, after the `release`
  environment's approval. It always refuses a SemVer pre-release. It also
  refuses a release published less than seven days ago, or a version older
  than the one `stable` holds, unless `force` is set with a reason; the
  reason goes into the release notes. It checks that `<version>` is the image
  the release workflow signed for `v<version>`, copies it to `:stable` with
  `skopeo copy --all --preserve-digests`, signs it again with
  `channel=stable`, and makes the GitHub release the latest full release.
  `dry-run` runs the checks only.
- Nodes accept the `promote.yaml` signature as well as `molt.yaml`'s
  (ADR 0016), from `development` only.
- The image's `/usr/lib/crustates/molt.toml` names no channel. A node without
  one in `/etc/crustates/molt.toml` picks at its first check: `stable` if it
  holds an image at least as new as the booted one, else `beta`. It writes
  the choice to `/etc`, so it keeps it. Only a missing `:stable` tag counts as
  "no stable"; any other registry error retries at the next check. The
  installer (M0 Part H) writes the channel itself.
- `crust molt channel set <channel>` changes it. It refuses a channel whose
  image has an older version than the booted one, or a version it cannot
  compare, unless `--allow-downgrade` is given. Then it checks the new
  channel; with the `auto` policy a newer image is staged at once.
- Versions compare by SemVer precedence (the `semver` crate); build metadata
  does not count.

### Consequences

- Good, because a `stable` machine runs bytes that `beta` machines ran for a
  week, under the same signatures.
- Good, because promotion is one approved click, and a mistyped older version
  is refused instead of sending every `stable` machine back.
- Good, because a fresh install from the newest release disk image stays on
  `beta` until `stable` catches up, rather than going back to an older
  `stable`.
- Bad, because promotion needs someone to remember it; nothing reminds them.
- Bad, because a machine that picked `beta` before `stable` existed stays on
  `beta` until someone runs `crust molt channel set stable`.
- Bad, because a machine switched to `:development` by hand with
  `bootc switch` follows `beta` at its next check, until
  `crust molt channel set development`: the agent follows its own setting,
  not bootc's image name.

### Confirmation

Agent unit tests cover the first-check choice (stable present, missing, older
than the booted image, and an unreachable registry), the downgrade checks of
`crust molt channel set`, and SemVer precedence. An API test sends a channel
change through a real API and agent. The checks in `promote.yaml` ran locally
against the real registry; the first real promotion, or a `dry-run` dispatch,
confirms the rest.

## Pros and Cons of the Options

### Tags in one repository, promoted by digest

- Good, because the digest, signatures and SBOM referrers stay the same.
- Bad, because a tag can be moved by anyone with write access to the
  package; nodes still check the signature, so a moved tag can only point at
  another Crustates-signed image.

### A rebuild for stable

- Bad, because the bytes on `stable` would never have run on `beta`.

### One repository per channel

- Good, because package permissions could differ per channel.
- Bad, because every image, signature and SBOM would exist twice, and the
  agent's repository setting would change with the channel.

### Automatic promotion after seven days

- Good, because nobody has to remember it.
- Bad, because a release with a known problem would go out unless someone
  stopped it, and a daily schedule costs Actions minutes.

## More Information

- ADR 0014 (GHCR; it said "a `stable` channel comes later"), ADR 0016
  (controlled updates)
- Code: `crates/crust-agent/src/molt.rs` (`set_channel`, `choose_channel`),
  `.github/workflows/promote.yaml`
