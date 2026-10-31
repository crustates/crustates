# Crustates

Crustates is a homelab platform in Rust: "your own cloud, on your own
hardware". One platform runs VMs, apps, storage and networking on amd64 and
arm64. It has a read-only, image-based OS and one declarative API that the web
UI, CLI, TUI, MCP server, OpenTofu provider and AI assistant all use. It
replaces a Proxmox + TrueNAS + Portainer stack. The full plan, with milestones
and open decisions, is in [`docs/plan.md`](docs/plan.md).

The platform is AGPL-3.0. `sdk/` and `wit/` are Apache-2.0.

## Product names

Use these names in docs, UI text, CLI subcommands and commit scopes (lower
case in scopes):

| Name      | Area                                                      |
| --------- | --------------------------------------------------------- |
| Tide      | Compute: VMs, microVMs and apps                           |
| Reef      | Storage: pools, shares, S3, snapshots, backups            |
| Current   | Networking: bridges, VLANs, firewall, eBPF, ingress, DNS  |
| Molt      | The read-only OS and its upgrades                         |
| Cast      | Clusters: joining nodes and sites                         |
| Barnacle  | Plugins: sandboxed Wasm components, interfaces in `wit/`  |
| Spawn     | GitOps and push-to-deploy                                 |
| Assistant | The AI assistant                                          |

Code names stay plain: a crate or module is `vm-driver`, not `tide-driver`.

## Architecture

- **Control plane** (Rust): axum REST API with OpenAPI and watch streams,
  state in SQLite (Raft-replicated later), reconcile loops.
- **Node agent** (Rust): one per host, talks to the control plane over gRPC
  (tonic) with mTLS.
- **Engines**, each behind a trait: QEMU for full VMs, Cloud Hypervisor via
  Kata runtime-rs for microVMs, containerd + crun for shared containers, ZFS
  and Btrfs for storage, Caddy for ingress and TLS, nftables + Aya eBPF for
  the firewall and workload policy.
- **Resources** are declarative and Kubernetes-shaped (`metadata`, `spec`,
  `status`) with no Kubernetes underneath: users write `spec`, reconcilers
  drive the engines and write `status`.
- **Clients** (web UI, `crust` CLI and TUI, MCP server, OpenTofu provider,
  assistant) all use the same public API.

## Repo layout

Today (milestone M0):

| Path                 | What                                             |
| -------------------- | ------------------------------------------------ |
| `docs/plan.md`       | Master plan                                      |
| `docs/adr/`          | Architecture decision records (MADR)             |
| `sdk/`               | Client SDK (Apache-2.0), empty for now           |
| `wit/`               | Barnacle plugin interfaces (Apache-2.0), empty   |
| `.github/workflows/` | `ci` (the required check) and `docs` (Pages)     |

The workspace skeleton adds:

| Path                     | What                                             |
| ------------------------ | ------------------------------------------------ |
| `Cargo.toml`             | Workspace: edition 2024, shared deps and lints   |
| `crates/crust-types`     | Resource model: `Resource<Spec, Status>`, `Node` |
| `crates/crust-store`     | `Store` trait, SQLite store, contract tests      |
| `crates/crust-proto`     | gRPC protocol between agent and control plane    |
| `crates/crust-reconcile` | Reconcile loops, node heartbeat reconciler       |
| `crates/crust-api`       | Control plane: REST API, `/docs`, agent service  |
| `crates/crust-agent`     | Node agent                                       |
| `crates/crust-cli`       | The `crust` CLI                                  |
| `xtask/`                 | `cargo xtask` build tasks                        |

Update both tables when the layout changes.

## Build and test

Once the workspace skeleton lands:

- `cargo xtask ci`: what CI runs (fmt, clippy with warnings as errors, tests,
  cargo-deny). Run it before you push.
- `cargo test -p <crate>`: one crate's tests.
- `cargo run -p crust-api`: the control plane; the API reference is at
  `/docs`.
- `cargo run -p crust-agent`: registers this machine as a node.
- `cargo xtask openapi`: regenerates the OpenAPI spec.

## Rules

- Rust stable, pinned in `rust-toolchain.toml`. Change the toolchain only
  there.
- Every external engine (hypervisor, storage, network, OS updater, state
  store) sits behind a trait with contract tests: one shared test suite that
  every implementation, real or fake, must pass. Tests use fakes.
- Errors: `thiserror` enums in library crates, `anyhow` only in binaries.
- Outside tests, handle errors with `?` or a `match`; `unwrap()` is for tests
  only.
- Log with `tracing` and structured fields:
  `info!(node = %name, "node registered")`.
- Every public type, trait and function has a doc comment.
- `unsafe` code is forbidden unless an ADR allows it for that crate.
- Secrets live in GitHub environment secrets. The repo, tests and fixtures
  never contain a secret.
- GitHub Actions: workflow files end in `.yaml`, actions are pinned by full
  commit SHA with the version in a comment, and `permissions` stay minimal.
- Commits: Conventional Commits with sign-off (`git commit -s`), for example
  `feat(tide): add VM snapshots`.
- Pull requests target `development` and are squash-merged, so the PR title
  is the commit message.

## Decisions

Decisions live in `docs/adr/` as numbered MADR records. Read the ADRs for the
area you touch before you change it. A new engine, a new external dependency,
or a change to the API or the on-disk format needs an ADR first: copy
`docs/adr/0000-template.md` to the next free number and open a PR that holds
only the ADR (see `CONTRIBUTING.md`).

When the plan or an ADR is unclear or contradicts itself, list the question in
the PR description and leave that part out.
