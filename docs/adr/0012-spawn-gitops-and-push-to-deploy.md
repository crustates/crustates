---
status: accepted
date: 2026-10-30
decision-makers: Miguel Martens
---

# Spawn: pull-based GitOps plus push-to-deploy builds

## Context and Problem Statement

Users want to keep their whole setup in Git and to deploy their own code on
every push. Homelabs sit behind NAT, and opening a port for a Git host is a
security risk. How does Crustates follow Git, and how does it build and deploy
code?

## Decision Drivers

- Nothing on the internet has to reach the box.
- Git changes pass the same role checks and plans as CLI and UI changes.
- Secrets can live in the repo without being readable there.
- Deploy from a Dockerfile, or from source with no config.

## Considered Options

- Pull-based GitOps over the public API, plus push-to-deploy builds on the box
  with BuildKit and Railpack
- Push-based deploys, where a Git host or CI reaches into the box

## Decision Outcome

Chosen option: "Pull-based GitOps over the public API, plus push-to-deploy
builds", because the box only makes outbound connections and Git becomes one
more client of the same API.

### GitOps for the whole setup

- A `GitSource` points at a repo, branch and folder on GitHub (through a GitHub
  App), Forgejo/Gitea, GitLab, or plain git over https or ssh with a deploy
  key. Crustates pulls every few minutes and on webhooks relayed to it, with no
  port forwarding.
- The folder holds YAML for any API resource. Reconcilers apply it exactly like
  a CLI or UI change.
- Changes made in the UI show as drift: "revert to Git" or "adopt into Git
  (open a PR)". A prune policy decides whether resources removed from Git are
  deleted or only flagged.
- Every pull request gets a plan: the GitHub App posts the dry-run diff as a
  check. Merge to apply; `git revert` to roll back.
- Secrets stay encrypted in the repo with SOPS + age; the private key never
  leaves the node.
- Image update PRs for apps that pin versions.
- For VMs, Git manages the spec; what runs inside stays with cloud-init or
  Ansible.

### Push-to-deploy for the user's own code

- Every push to a connected app repo builds on the box with BuildKit, from a
  Dockerfile or with Railpack, into a local registry (zot), then rolls out with
  health checks and one-click rollback.
- Preview apps per pull request at `pr-42.<app>.crust.internal`, removed when
  the PR closes.
- An optional self-hosted GitHub Actions or Forgejo runner runs as an app in
  its own microVM.

### Safety

Audit entries are tagged "via git" with the commit, deploy keys and
GitHub App tokens are scoped to single repos, and a source can be paused.

### Consequences

- Good, because no inbound access is needed.
- Good, because Git, the CLI, the UI and the assistant share one plan and one
  permission model.
- Bad, because changes apply after the next pull or relayed webhook, not
  instantly.
- Bad, because webhooks need a relay service.

### Confirmation

M2 ships Git sources and push-to-deploy builds (Dockerfile and Railpack). M6
adds PR plans through a GitHub App, preview apps and image update PRs. Tests
apply a Git folder through the public API and check the audit tag.

## Pros and Cons of the Options

### Pull-based GitOps plus builds on the box

- Good, because the box stays unreachable from the internet.
- Good, because it reuses the reconcilers and the API.
- Bad, because of the polling delay and the webhook relay.

### Push-based deploys

- Good, because changes apply immediately.
- Bad, because something on the internet must reach the box, which needs port
  forwarding or an exposed endpoint.

## More Information

- Plan: [Spawn: GitOps and push-to-deploy](../plan.md#spawn-gitops-and-push-to-deploy),
  [Security model](../plan.md#security-model),
  [Tech stack](../plan.md#tech-stack),
  [Milestones](../plan.md#milestones)
- Open: the prune default for resources removed from Git. The plan recommends
  "warn until the user opts in" but marks it as an open decision.
