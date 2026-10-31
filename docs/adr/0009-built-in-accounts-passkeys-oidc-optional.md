---
status: proposed
date: 2026-10-30
decision-makers: Miguel Martens
---

# Built-in accounts with passkeys, OIDC optional

## Context and Problem Statement

The API is the only way into a Crustates box, so who may call it, and as which
role, is the core of its security. A fresh homelab has no identity provider,
but some users already run one. How do users log in?

## Decision Drivers

- Works on a fresh install with no other service and no cloud account.
- Strong login by default.
- Users who already run an identity provider can use it.
- Role-based access and an audit log of every change.

## Considered Options

- Built-in accounts with passkeys or TOTP, OIDC optional
- OIDC only, with an external identity provider

## Decision Outcome

Chosen option: "Built-in accounts with passkeys or TOTP, OIDC optional",
because it works out of the box and still lets users plug in their own identity
provider.

- The first-run wizard creates the admin account.
- Users log in with passkeys or TOTP; OIDC login is optional.
- Access is role-based, and every change lands in the audit log.
- The local recovery console on the machine's own screen can reset the admin
  password.

### Consequences

- Good, because nothing else has to run before the first login.
- Good, because passkeys give phishing-resistant login by default.
- Bad, because Crustates stores and protects credentials itself.

### Confirmation

Accounts, roles and the audit log ship in M4. Auth sits in front of every API
route, and tests cover passkey, TOTP and OIDC login.

## Pros and Cons of the Options

### Built-in accounts, OIDC optional

- Good, because it works on a fresh box.
- Bad, because there is credential storage to secure.

### OIDC only

- Good, because no credentials are stored on the box.
- Bad, because a fresh install would need an identity provider before anyone
  can log in, which conflicts with "no cloud account required".

## More Information

- Plan: [Security model](../plan.md#security-model),
  [Milestones](../plan.md#milestones) (M4),
  [Open decisions and risks](../plan.md#open-decisions-and-risks)
- Status is "proposed" because the plan marks accounts as "Recommended".
- Open: the plan names passkeys "or TOTP", and the recovery console resets an
  admin password. Is a password always required with TOTP, and can a passkey
  replace the password entirely?
