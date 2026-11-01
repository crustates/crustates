---
status: accepted
date: 2026-11-01
decision-makers: Miguel Martens
---

# Rust workspace layout

## Context and Problem Statement

M0 starts the Rust code: a resource model, a state store, the control plane
API, the node agent, reconcile loops and the `crust` CLI. They share types and
build settings, and later milestones add drivers for every engine. How is the
Cargo workspace laid out, and which libraries does it build on?

## Decision Drivers

- One place for dependency versions, lints and the toolchain.
- Crates small enough that a driver or the agent does not pull in the API
  server.
- Every engine behind a trait with contract tests (plan principle 2).
- No tools beyond `cargo` and `rustup` to build and test.

## Considered Options

- One Cargo workspace with one crate per component under `crates/`
- One crate with modules
- One repository per component

## Decision Outcome

Chosen option: "One Cargo workspace with one crate per component", because it
keeps one lockfile, one lint set and one CI run while each component builds and
tests on its own.

```text
Cargo.toml             workspace: edition 2024, resolver 3, shared deps and lints
.cargo/config.toml     `cargo xtask` alias
deny.toml              cargo-deny: licences, advisories, sources
crates/
  crust-types/         resource model: Resource<Spec, Status>, Node
  crust-store/         Store trait, SQLite store, contract tests
  crust-proto/         gRPC protocol between agent and control plane
  crust-reconcile/     reconcile loops and the node heartbeat reconciler
  crust-api/           control plane: REST API, /docs, agent gRPC service
  crust-agent/         node agent
  crust-cli/           the `crust` binary
xtask/                 build tasks: `cargo xtask ci`, `cargo xtask openapi`
```

- Crates are named `crust-*` with plain names, not product names.
- Shared versions live in `[workspace.dependencies]`; lints in
  `[workspace.lints]`: clippy `pedantic` and `unwrap_used` warn, `missing_docs`
  warns, `unsafe_code` is forbidden. CI turns warnings into errors.
- Libraries: tokio, axum, utoipa with the Scalar reference, sqlx (SQLite),
  tonic with protox (so no `protoc` is needed), reqwest, clap, figment for
  config, sysinfo for node facts, jiff for timestamps, thiserror in libraries
  and anyhow in binaries, tracing for logs.
- The control plane hosts the agent gRPC service and agents connect to it, so
  agents work behind NAT. mTLS follows later
  ([plan: security model](../plan.md#security-model)).
- The OpenAPI spec is written to `sdk/openapi.json`, because the API spec is
  Apache-2.0 like the rest of `sdk/` (ADR 0005).

### Consequences

- Good, because each crate's dependencies stay small and explicit.
- Good, because the store contract tests live next to the trait and any new
  store implementation runs them.
- Bad, because cross-crate changes touch several `Cargo.toml` files.

### Confirmation

`cargo xtask ci` runs fmt, clippy, tests and cargo-deny on the whole workspace,
locally and in CI.

## Pros and Cons of the Options

### One workspace, one crate per component

- Good, because components build and test on their own with shared settings.
- Bad, because there are more crates to keep in sync.

### One crate with modules

- Good, because there is one `Cargo.toml`.
- Bad, because the agent and the CLI would compile the API server and its
  dependencies.

### One repository per component

- Good, because each component is versioned on its own.
- Bad, because shared types and cross-cutting changes need several pull
  requests, which the plan's monorepo rules out.

## More Information

- Plan: [Tech stack](../plan.md#tech-stack),
  [Way of working and repo](../plan.md#way-of-working-and-repo)
- `crust-proto` is not in the original M0 brief; it exists so the agent and the
  control plane share the protocol without depending on each other.
