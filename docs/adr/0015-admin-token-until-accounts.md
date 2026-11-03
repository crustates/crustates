---
status: accepted
date: 2026-11-03
decision-makers: Miguel Martens
---

# An admin token for the API until accounts arrive

## Context and Problem Statement

The API is the only way into a Molt machine. Until now it had no login, so
anyone on the network could call it. Accounts with passkeys and roles
(ADR 0009) come in M4, but M0 already adds commands that change the machine:
upgrade, roll back, enrol a Secure Boot key. How is the API protected until
accounts exist?

## Decision Drivers

- No open API on a fresh install, from M0 on.
- Works without any other service, cloud account or TLS certificate.
- Nothing secret in the repository, the image or a disk image: each machine
  makes its own secret.
- A stolen database or disk image must not reveal the secret.
- Simple to replace with accounts in M4.

## Considered Options

- One admin token per machine, made at first boot, sent as a bearer token
- A password set at first boot
- Built-in accounts now (ADR 0009 brought forward)
- No protection until M4, "trusted networks only"

## Decision Outcome

Chosen option: "One admin token per machine, made at first boot", because it
closes the API now with the least code, and accounts can replace it without
changing how clients send credentials (`Authorization: Bearer`).

- On its first boot the machine makes a random token,
  `crt_XXXXX-XXXXX-XXXXX-XXXXX`: 20 characters of Crockford base32, 100
  bits. Typing is forgiving: any case, dashes optional, `I`/`L` read as `1`
  and `O` as `0`.
- Only its argon2id hash (default parameters, a PHC string) is stored, in
  `/var/lib/crustates/secrets/admin-token`. The directory is `root:crustates`
  `0750`, the file `0640`: root writes it, the API (user `crustates`) can
  only read it.
- The token itself is shown once, on the machine's console, until the next
  reboot (`/run/issue.d`, a tmpfs). It is never logged. The local console
  (Part G of M0) replaces this with its first-boot screen and can make a new
  token after a PIN.
- Every `/v1/` request needs the token; `/healthz`, `/docs` and the OpenAPI
  description stay open. The API reads the hash file on every request, so a
  new token takes effect without a restart, and keeps the last verified token
  in memory so argon2 runs once per token, not per request. At most two
  argon2 checks run at once.
- Ten wrong tokens from one address (an IPv6 /64) within a minute answer
  `429` for the rest of that minute.
- `crust login` checks a token and saves it in `~/.config/crust/config.toml`
  (mode `0600`) together with the API's URL; the CLI sends it only to that
  URL.
- Every change made through the API is written to an audit log in the state
  database (`GET /v1/audit`), with the principal `admin token`.

### Consequences

- Good, because a fresh install no longer exposes an open API.
- Good, because no secret is in the repository, the image or a release.
- Good, because clients already send a bearer token, as they will with
  accounts.
- Bad, because there is one principal: everyone with the token is admin.
- Bad, because the token travels over plain HTTP until the API gets TLS
  (ADR 0007); on an untrusted network it can be sniffed.
- Bad, because a lost token cannot be recovered before the local console
  lands; until then the fix is a reinstall.

### Confirmation

API tests check `401` without or with a wrong token, `429` after ten wrong
tokens, `503` before a token exists, and that typed variants of the token
work. The Molt boot test checks the file modes, that `/v1/nodes` answers
`401` without the token, and uses the token printed on the console.

## Pros and Cons of the Options

### One admin token per machine

- Good, because it is a few hundred lines and needs no other service.
- Bad, because it is all or nothing.

### A password set at first boot

- Good, because people know passwords.
- Bad, because chosen passwords are weaker than 100 random bits, and the
  first boot has no keyboard prompt on a headless machine.

### Built-in accounts now

- Good, because it is the end state.
- Bad, because passkeys need TLS and a web UI, which come later; it would
  hold M0 back by a milestone.

### No protection until M4

- Bad, because M0 adds commands that change the OS, and "only on trusted
  networks" is advice, not protection.

## More Information

- ADR 0009 (accounts), ADR 0007 (TLS)
- Code: `crates/crust-api/src/auth.rs`, `molt/rootfs/usr/libexec/crustates/crust-firstboot`
