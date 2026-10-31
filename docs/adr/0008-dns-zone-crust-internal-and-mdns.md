---
status: accepted
date: 2026-10-30
decision-makers: Miguel Martens
---

# Default DNS zone `crust.internal` plus mDNS

## Context and Problem Statement

Every app and VM gets a DNS name automatically. The names need a zone that can
never clash with a public domain, and they should resolve on the LAN even
before the user sets up any DNS. Which zone is the default, and how do names
resolve without DNS setup?

## Decision Drivers

- The zone must never be delegated publicly.
- Names should resolve on the LAN with no DNS setup.
- Users with their own domain can use it instead.

## Considered Options

- `crust.internal`
- `home.arpa`
- A made-up top-level domain such as `.crust`

## Decision Outcome

Chosen option: "`crust.internal`", because ICANN has reserved `.internal` for
private use and will never delegate it, and the name carries the brand.

- The zone is set in the first-run wizard and can be changed later;
  `crust.internal` is the default and `home.arpa` works too.
- Every app and VM gets `<name>.crust.internal`, signed by the local CA (ADR
  0007).
- With the user's own domain (for example `home.example.com`), Crustates
  switches to Let's Encrypt via DNS-01.
- mDNS announces every app as `<name>.local` as well.

### Consequences

- Good, because names never clash with public domains.
- Good, because `.local` names work on the LAN with no DNS setup.
- Bad, because `crust.internal` names only resolve for clients that use the
  Crustates DNS integration.

### Confirmation

A new app resolves as `<name>.crust.internal` through the DNS integration and
as `<name>.local` over mDNS.

## Pros and Cons of the Options

### `crust.internal`

- Good, because `.internal` is reserved for private use.
- Good, because it names the platform.

### `home.arpa`

- Good, because it is reserved for home networks too.
- Neutral: allowed as a user choice, not the default.

### A made-up TLD such as `.crust`

- Bad, because it could be delegated publicly one day.

## More Information

- Plan: [Current: networking](../plan.md#current-networking)
