---
status: accepted
date: 2026-10-30
decision-makers: Miguel Martens
---

# Ingress: Caddy with three certificate modes

## Context and Problem Statement

Every app and VM gets a name such as `jellyfin.crust.internal` (ADR 0008) and
should be reachable over HTTPS. Most homelabs are offline or private, some own
a domain, and a few apps are deliberately exposed to the internet. Which
reverse proxy handles HTTP routing and TLS, and how do certificates work in
each of these cases?

## Decision Drivers

- Automatic TLS with no manual certificate work.
- Works offline, with nothing exposed.
- Real certificates when the user owns a domain.
- Configurable through an API by the Current driver.

## Considered Options

- Caddy
- River (Rust, on Pingora)

## Decision Outcome

Chosen option: "Caddy", because it has automatic TLS, a built-in local CA and
an admin API that the Current driver configures.

Certificates come in three modes:

1. **Local CA** (default, works offline): Caddy's built-in CA signs internal
   names. Devices get the root certificate by QR code or from a download page.
2. **Let's Encrypt via DNS-01**: real certificates for internal names under a
   domain the user owns, with nothing exposed. The Molt image ships a Caddy
   build with common DNS providers. Crustates switches to this mode when the
   user sets their own domain.
3. **Let's Encrypt via HTTP-01**: for apps the user deliberately exposes to the
   internet.

Layers: eBPF and nftables handle network traffic, Caddy handles HTTP routing
and TLS, the DNS integration (goethite) makes names resolve.

### Consequences

- Good, because every app gets HTTPS from the first start, even offline.
- Good, because owning a domain upgrades to real certificates with nothing
  exposed.
- Bad, because with the local CA each device must trust the root certificate
  once.
- Bad, because the Molt image carries a custom Caddy build with DNS providers.

### Confirmation

The ingress driver sits behind a trait with contract tests. An integration test
reaches an app over HTTPS in local-CA mode.

## Pros and Cons of the Options

### Caddy

- Good, because automatic TLS, a local CA and an admin API are built in.
- Bad, because it is Go, not Rust (acceptable: the plan reuses proven engines).

### River

- Good, because it is Rust, on Pingora.
- Bad, because its development is paused.

## More Information

- Plan: [Current: networking](../plan.md#current-networking),
  [Open decisions and risks](../plan.md#open-decisions-and-risks)
- Revisit River if its development resumes.
