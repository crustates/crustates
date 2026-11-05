---
status: accepted
date: 2026-11-03
decision-makers: Miguel Martens
---

# A console on the machine's screen, guarded by a recovery PIN

## Context and Problem Statement

Molt has no SSH and no login (ADR 0002, ADR 0015): the API is the only way
in, and it needs the network and the admin token. When the network is wrong,
the token is lost, or an update misbehaves, the owner stands in front of the
machine with a screen and a keyboard and can do nothing but reinstall. The
first boot shows the token above a login prompt that no one can use. What
does the machine's own screen offer, and who may use it?

## Decision Drivers

- Recovery without the network: see and fix the address, roll back, reboot.
- A person at the keyboard must not get everything at once: someone who
  walks past the machine should not reset the network or the token.
- No secret stored in plain text, and no shell.
- It works on every screen the Linux console runs on, from 80×25 up, with
  the 16 colours and 256 glyphs a console font has.
- It uses the API where the API owns the state (Molt, the audit log), so its
  actions are recorded like any other.

## Considered Options

- A console program on tty1 that talks to the API through a root-only
  socket, with a recovery PIN for risky actions
- The same console with the admin token stored on the machine
- A login prompt with a root password
- No console: reinstall to recover

## Decision Outcome

Chosen option: "a console on tty1, a root-only socket and a recovery PIN",
because it recovers without the network or the token and stores no secret,
and the PIN stops the casual passer-by without asking the owner for more
than nine digits.

- `crust-console` runs on tty1 instead of a login prompt and restarts when
  it stops; `crust-console --logs` shows the system log, read-only, on tty2.
  `getty@`, `autovt@` and `serial-getty@` are masked, and logind starts no
  other terminals.
- The API listens on `/run/crustates/api.sock` (mode 0660, user and group
  `crustates`) besides its TCP port. It serves only clients whose peer user
  is root, as the principal `local console`, without a token: root on the
  machine can read and change everything anyway.
- On the first boot, `crust-firstboot` makes the admin token and a recovery
  PIN of nine random digits (about 30 bits). Their argon2id hashes go next to
  each other in `/var/lib/crustates/secrets` (root writes, the API reads).
  The secrets themselves go into `/run/crustates/firstboot.json` (root only,
  a tmpfs); the console shows them, with a QR code of the API's address,
  until someone presses enter or the machine reboots. This replaces the
  message in `/run/issue.d` (ADR 0015).
- Risky actions ask for the PIN first: reset the network, change it, roll
  back, make a new admin token, reboot, shut down. The console sends it to
  `POST /v1/local/pin`, which only the local console may call. Every attempt
  goes into the audit log, and five wrong PINs within 15 minutes, with no
  right one since, lock it for 15 minutes from the last attempt. The lock is
  counted from the audit log, so restarting the API does not reset it.
- The console reads the machine itself (sysfs, `ip`, `nmcli`, `systemctl`,
  `smartctl`, `journalctl`) and asks the API for the Molt status. It rolls
  back through the API (ADR 0016) and makes a new token with
  `crust-api new-admin-token`.
- Network changes go through NetworkManager with a checkpoint: they revert
  after 60 seconds unless confirmed on the console. A hostname change is not
  part of the checkpoint; the agent keeps the node name it registered with
  first, so renaming the host does not add a second node.
- After an update, greenboot's health check writes its progress to
  `/run/crustates/health.json` on the first boot of the new image, and the
  console shows it.
- The screens use a font of our own, `crustates-console`: Lat2-Terminus16
  (SIL Open Font License) with eight rarely used glyphs replaced by the half
  blocks, circles and check marks the screens need, made from the base
  image's font at build time, in 8×16 and, for screens wider than 240
  columns, 16×32. Characters the loaded font lacks are replaced. The ten
  colours of the design kit are loaded into the console's palette; the five
  used as backgrounds sit in its first eight entries.
- The boot splash is a Plymouth script theme with the crab, drawn at build
  time from the mark. The image's initramfs is rebuilt with it; ZFS stays
  out of the initramfs.

### Consequences

- Good, because a machine with a wrong network, a lost token or a bad update
  can be fixed at its screen.
- Good, because no secret is stored in plain text, not even for the console.
- Good, because the console's risky actions are in the audit log.
- Bad, because the PIN protects against people at the keyboard, not against
  code running as root, which needs no PIN.
- Bad, because nine digits are weak against an attacker with the hash file;
  the hash file is readable only by root and the API's user.
- Bad, because whoever loses both the token and the PIN has to reinstall.
- Bad, because the initramfs is rebuilt for every version, so every update
  downloads it again.

### Confirmation

Snapshot tests draw every screen at 80×25 and 120×38 and fail on a
character the console font lacks. API tests cover the socket (root only)
and the PIN lock; the boot test checks the socket's mode, a PIN check
through it, that the console runs on tty1 and tty2, and that the initramfs
has the splash but no ZFS, and saves screenshots of tty1 at the first boot
and on the home screen.

## Pros and Cons of the Options

### A console with the admin token stored on the machine

- Good, because the console could use the same API path as every client.
- Bad, because the token would sit in plain text on the disk, and anyone at
  the keyboard would have it.

### A login prompt with a root password

- Good, because it is what Linux does already.
- Bad, because a shell is everything at once, and a password is a secret to
  set, store and forget.

### No console

- Bad, because a wrong network setting or a lost token means a reinstall.

## More Information

- ADR 0015 (admin token), ADR 0016 (controlled updates), ADR 0002 (Molt)
- Design kit: the `lc-*` and `tui-recovery` screens
- Code: `crates/crust-console`, `crates/crust-tui`,
  `crates/crust-api/src/local.rs`, `molt/rootfs/usr/libexec/crustates/crust-firstboot`
