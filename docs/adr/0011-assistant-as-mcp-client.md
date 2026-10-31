---
status: proposed
date: 2026-10-30
decision-makers: Miguel Martens
---

# AI assistant as an MCP client of the public API

## Context and Problem Statement

Crustates ships an assistant users can ask about their setup ("why is Jellyfin
slow?") and tell to do things ("snapshot this VM"). An assistant that can act
on the box must never see or do more than the user may, and must not leak data
off the network without the user choosing that. How does the assistant reach
the platform, which model does it use, and how are its actions controlled?

## Decision Drivers

- It can only see and do what the API allows the user.
- Privacy: nothing leaves the network unless the user opts in.
- Models can be confidently wrong, small local models more so.
- Logs, app output, file names and web content may contain prompt injections.

## Considered Options

- An MCP client of the public API, with a model the user brings
- A separate assistant backend with its own access to the platform

## Decision Outcome

Chosen option: "An MCP client of the public API, with a model the user
brings", because it uses the same API tools as the CLI, the web UI and the
external MCP server, so it can never do more than the user.

### Models

- Nothing is configured out of the box; the assistant stays off until the user
  picks a provider.
- Local first: a model on the box (Ollama or llama.cpp as a catalog App, with
  GPU passthrough if present) or elsewhere on the LAN. Setup suggests a local
  model sized to the hardware.
- Remote, opt-in: any OpenAI-compatible endpoint or the Anthropic API with the
  user's own key, stored as a platform secret.
- The UI always shows which provider and model answered and whether data left
  the network.

### Grounding

Live state through the API tools, plus retrieval over the
Crustates docs, ADRs and the user's resource specs, embedded locally
(sqlite-vec). Answers cite their sources with links into the UI.

### Safety

- It acts as the user, with the user's role and never more; a per-role
  allowlist decides which tools it may call.
- Read-only by default. Every change is first shown as a plan (resource diff,
  dry run through the reconciler, what it affects) and only applied after the
  user confirms.
- Destructive actions need the user to type the resource name.
- Every action is audited "via assistant" with the prompt that led to it;
  risky changes take a snapshot first.
- Text from logs, app output, file names and web content is untrusted and can
  never trigger an action on its own. No autonomous actions.
- Rate limits and a token budget per user.

### Consequences

- Good, because the assistant's permissions are exactly the user's.
- Good, because local by default keeps data on the network.
- Good, because external agents (Claude Code, Claude Desktop and others) get
  the same tools through the MCP server.
- Bad, because answer quality depends on the model the user picks.
- Bad, because every action costs a confirmation step.

### Confirmation

M4 ships the read-only assistant (chat drawer and `crust ask`). M6 adds
actions with plan-and-confirm. Tests check that the assistant cannot call a
tool outside the user's role allowlist.

## Pros and Cons of the Options

### MCP client of the public API

- Good, because there is one permission model for all clients.
- Good, because the same tools serve external agents.
- Bad, because it can only do what the API exposes as tools.

### Separate assistant backend

- Good, because it could reach data the API does not expose.
- Bad, because it is a separate brain bolted on, with its own access to
  secure.

## More Information

- Plan: [AI assistant](../plan.md#ai-assistant),
  [Security model](../plan.md#security-model),
  [Tech stack](../plan.md#tech-stack),
  [Open decisions and risks](../plan.md#open-decisions-and-risks)
- Status is "proposed" because the plan marks the assistant's model default as
  "Recommended". This ADR reads "local by default" as "off until the user picks
  a provider, with a local model suggested first".
- Open: the assistant's name, plain "Assistant" or a crab name such as Claw or
  Pincer.
