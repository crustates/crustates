# Crustates

Crustates is a homelab platform in Rust: "your own cloud, on your own
hardware". It runs VMs, apps, storage and networking, with a read-only OS and a
declarative API. The platform is AGPL-3.0. `sdk/` and `wit/` are Apache-2.0.

## Product names

Use these names in code, docs and commit scopes (lower case in scopes):

- **Tide**: compute (VMs, microVMs, apps)
- **Reef**: storage
- **Current**: networking
- **Molt**: the read-only OS and upgrades
- **Cast**: clusters
- **Barnacle**: plugins (interfaces in `wit/`)
- **Spawn**: GitOps
- **Assistant**: the AI assistant

## Layout

Milestone M0: only `docs/`, `sdk/` and `wit/` exist so far. The Cargo
workspace layout comes with its ADR in `docs/adr/`. Update this section when
that ADR is merged.

## Rules

- Rust stable, pinned in `rust-toolchain.toml`. Change the toolchain only there.
- Every engine (hypervisor, storage, network, OS updater) sits behind a trait,
  so it can be swapped and faked in tests.
- Commits: Conventional Commits with sign-off (`git commit -s`), for example
  `feat(tide): add VM snapshots`.
- Secrets live in GitHub environment secrets. The repo, tests and fixtures
  never contain a secret.
- Pull requests target `development`. Bigger changes start with an ADR (see
  `CONTRIBUTING.md`).
