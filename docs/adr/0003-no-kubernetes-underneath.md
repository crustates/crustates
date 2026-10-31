---
status: accepted
date: 2026-10-30
decision-makers: Miguel Martens
---

# No Kubernetes underneath; a Kubernetes-shaped API instead

## Context and Problem Statement

Crustates manages VMs, apps, storage and networking declaratively: the user
states what they want and the platform makes it so. Kubernetes is the best
known system that works this way. Should Crustates run on Kubernetes, or
borrow its shape without running it?

## Decision Drivers

- Declarative resources (`spec` + `status`) and reconcile loops (plan principle
  3).
- Small footprint: the platform's memory budget is about 1 GB, and the minimum
  box has 8 GB of RAM.
- A boring core with nothing extra to run or upgrade (plan principle 4 and the
  Cast section).

## Considered Options

- Kubernetes (for example k3s) as the underlayer
- An own control plane with a Kubernetes-shaped API

## Decision Outcome

Chosen option: "An own control plane with a Kubernetes-shaped API", because it
keeps the declarative model users know from Kubernetes while the core stays one
small Rust control plane and one agent per host.

- Every resource has `metadata`, `spec` and `status`; reconcile loops drive
  the engines toward `spec` and report `status`.
- Kubernetes is a workload, not the underlayer: Crustates offers a one-click k3s
  or Talos cluster built from VMs.

### Consequences

- Good, because users who know Kubernetes recognise the resource shape.
- Good, because the platform fits in its memory budget on small machines.
- Bad, because we build the API machinery ourselves: watches, concurrency
  control, conditions and the reconcile framework.
- Bad, because tools that need Kubernetes are not available underneath: CRI-O
  for containers and CloudNativePG for databases are ruled out for that reason.

### Confirmation

The workspace has no Kubernetes dependency. Resources in `crust-types` follow
the `metadata` / `spec` / `status` shape.

## Pros and Cons of the Options

### Kubernetes as the underlayer

- Good, because its ecosystem (operators, CRI-O, CloudNativePG) comes for free.
- Bad, because it adds a large system to run and upgrade on every box.

### Own control plane, Kubernetes-shaped API

- Good, because the core stays small and fully under our control.
- Good, because Kubernetes is still available, as a workload.
- Bad, because the API machinery is our own code to write and test.

## More Information

- Plan: [Vision and positioning](../plan.md#vision-and-positioning) (principle
  3), [Tide: compute](../plan.md#tide-compute),
  [Hardware support](../plan.md#hardware-support)
- The plan states this decision as a principle. The drivers above come from
  other sections of the plan; it does not compare the two options directly.
