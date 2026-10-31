---
status: accepted
date: 2026-10-30
decision-makers: Miguel Martens
---

# Platform state in SQLite (Raft-replicated later), not PostgreSQL

## Context and Problem Statement

The control plane stores every resource and its status. Crustates starts on a
single node and grows to multi-node clusters across sites (Cast) later. Where
does the platform state live?

## Decision Drivers

- Nothing extra to run or upgrade on the box.
- Platform backup and restore must be simple, so a dead boot disk is not fatal.
- A homelab has few concurrent writes.
- A path to multi-node replication later.

## Considered Options

- SQLite embedded in the binary, Raft-replicated later
- PostgreSQL

## Decision Outcome

Chosen option: "SQLite embedded in the binary, Raft-replicated later", because
it runs inside the control plane with nothing extra to manage, and platform
backup is one file.

- Single node: SQLite through `sqlx`, with migrations.
- Multi-node (M6): Raft for the state with SQLite underneath; openraft is the
  recommended Raft library. Nodes join with `crust cast join`.
- Incus takes the same route with Raft-replicated SQLite (cowsql).

### Consequences

- Good, because there is no database server to install, tune or upgrade.
- Good, because platform backup is one file.
- Bad, because SQLite allows one writer at a time. Heavy concurrent writes are
  where PostgreSQL would pay off, and a homelab does not have them.
- Bad, because multi-node needs Raft replication that we build on top.

### Confirmation

The `Store` trait in `crust-store` has a SQLite implementation and a contract
test suite. The Raft-backed store must pass the same suite.

## Pros and Cons of the Options

### SQLite, Raft-replicated later

- Good, because it is embedded: nothing extra to run or upgrade.
- Good, because backup is one file.
- Bad, because replication is our own work.

### PostgreSQL

- Good, because it handles heavy concurrent writes.
- Bad, because it is one more service to run, upgrade and back up.
- Bad, because a homelab does not have the write load that would justify it.

## More Information

- Plan: [Cast: clustering](../plan.md#cast-clustering),
  [Tech stack](../plan.md#tech-stack),
  [Open decisions and risks](../plan.md#open-decisions-and-risks)
- The choice of openraft is "Recommended" in the plan, not decided; it gets its
  own ADR when multi-node work starts.
