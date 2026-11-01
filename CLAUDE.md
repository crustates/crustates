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

| Path                     | What                                             |
| ------------------------ | ------------------------------------------------ |
| `docs/plan.md`           | Master plan                                      |
| `docs/adr/`              | Architecture decision records (MADR)             |
| `crates/crust-types`     | Resource model: `Resource<Spec, Status>`, `Node` |
| `crates/crust-store`     | `Store` trait, SQLite store, contract tests      |
| `crates/crust-proto`     | gRPC protocol between agent and control plane    |
| `crates/crust-reconcile` | Reconcile loops, node heartbeat reconciler       |
| `crates/crust-api`       | Control plane: REST API, `/docs`, agent service  |
| `crates/crust-agent`     | Node agent                                       |
| `crates/crust-cli`       | The `crust` CLI                                  |
| `xtask/`                 | `cargo xtask` build tasks                        |
| `molt/`                  | Molt OS image: Containerfile, rootfs, selftest   |
| `sdk/`                   | Client SDK and `openapi.json` (Apache-2.0)       |
| `wit/`                   | Barnacle plugin interfaces (Apache-2.0), empty   |
| `.github/workflows/`     | `ci` (the required check) and `docs` (Pages)     |

Update this table when the layout changes. ADR 0013 records why it looks like
this.

## Build and test

- `cargo xtask ci`: what CI runs (fmt, clippy with warnings as errors, tests,
  cargo-deny). Run it before you push.
- `cargo test -p <crate>`: one crate's tests.
- `cargo run -p crust-api`: the control plane on `127.0.0.1:8080`, API
  reference at `/docs`, agents on `127.0.0.1:8081`, state in `./crust.db`.
  Settings: `crust-api.toml` or `CRUST_*` variables (`crates/crust-api/src/config.rs`).
- `cargo run -p crust-agent`: registers this machine as a node
  (`CRUST_AGENT_*` variables).
- `cargo run -p crust-cli -- node list`: the CLI against the local API.
- `cargo xtask openapi`: regenerates `sdk/openapi.json`; run it after changing
  the API and commit the result.
- `cargo xtask molt build | run | test`: builds the Molt image with podman,
  boots it in QEMU (API on `localhost:18080`), or boots the test image and
  waits for `CRUST-SELFTEST: PASS`. Needs podman with KVM; on macOS a rootful
  podman machine with 8 GB of memory.
- `molt/kernel.lock` pins the image kernel. A base image with another kernel
  fails the build; bump the lock in its own PR after checking OpenZFS supports
  that kernel.

A new `Store` implementation runs the shared contract tests from
`crust_store::contract::run` (feature `contract`).

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
