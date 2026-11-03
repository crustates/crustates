# Install Crustates

> **Status:** milestone M0. Releases are beta: fine for trying out, not yet
> for data you can't lose.

Crustates runs on **Molt**, its own read-only operating system. You install
Molt once; after that it updates itself from `ghcr.io/crustates/molt`.

## What you need

| Part       | Minimum (it runs)             | Recommended (typical homelab)                                |
| ---------- | ----------------------------- | ------------------------------------------------------------ |
| CPU        | 64-bit, 2 cores, VT-x / AMD-V | 6–8 cores, VT-d / AMD-Vi (IOMMU); Intel Quick Sync for media |
| Memory     | 8 GB                          | 32 GB, ECC if the board supports it                          |
| Boot disk  | 64 GB SSD                     | 256 GB NVMe, separate from data                              |
| Data disks | Optional                      | 2+ matching disks in a mirror                                |
| Network    | 1 GbE, wired                  | 2.5 GbE, a second port for VLANs                             |
| Firmware   | UEFI                          | UEFI with Secure Boot and TPM 2.0                            |
| GPU        | None                          | iGPU for transcoding                                         |
| Example    | N100/N150 mini PC, 16 GB      | Ryzen 5/7 or 12th-gen+ Core i5, 32 GB, two disks             |

- The firmware must boot in **UEFI** mode; legacy BIOS boot is not supported.
- Without VT-x or AMD-V there are no VMs or microVMs; apps fall back to shared
  containers.
- **ARM:** `arm64` images exist, but a Raspberry Pi has not been verified to
  boot them yet.

## 1. Download

Open the [latest release](https://github.com/crustates/crustates/releases)
and take the files for your CPU: `amd64` for Intel and AMD, `arm64` for ARM.

| File                           | Use it to                                                |
| ------------------------------ | -------------------------------------------------------- |
| `molt-VERSION-ARCH.iso`        | Install from a USB stick (step 3)                        |
| `molt-VERSION-ARCH.raw.zst`    | Or write Molt straight onto the machine's disk (step 3b) |
| `molt-VERSION-ARCH.qcow2`      | Or run Molt as a virtual machine (step 3c)               |
| `molt-VERSION-ARCH.SHA256SUMS` | Check the downloads (step 2)                             |

Each file has a `.sigstore.json` signature next to it. The ISO for `arm64`
is built like the `amd64` one, but only the `amd64` one is installed in
CI.

## 2. Check the download

Check that the file arrived intact:

```sh
sha256sum --check --ignore-missing molt-VERSION-ARCH.SHA256SUMS
```

Check that the Crustates release workflow made it, with
[cosign](https://docs.sigstore.dev/cosign/system_config/installation/):

```sh
cosign verify-blob molt-VERSION-ARCH.raw.zst \
  --bundle molt-VERSION-ARCH.raw.zst.sigstore.json \
  --certificate-identity "https://github.com/crustates/crustates/.github/workflows/molt.yaml@refs/tags/vVERSION" \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

It prints `Verified OK`.

## 3. Install from a USB stick

Write the ISO onto a USB stick of 4 GB or more (everything on the stick is
erased), for example with [Fedora Media Writer](https://fedoraproject.org/workstation/download),
or with `dd`:

```sh
sudo dd if=molt-VERSION-ARCH.iso of=/dev/sdX bs=4M status=progress conv=fsync
```

Boot the machine from the stick (the firmware's boot menu is usually F11,
F12 or Esc). The boot menu offers:

- **Install Crustates**: the installer, below.
- **Install Crustates (basic graphics)**: the same, for screens that stay
  black.
- **Rescue a broken install**: boot the previous OS image, reset the network
  to DHCP, list the ZFS pools.
- **Check install media**: verifies the stick against its checksum first.

The installer checks the hardware, then asks for the boot disk (everything on
it is erased; 32 GB or more), an optional ZFS pool on other disks (single,
mirror or raidz), and the network. It shows the admin token and the recovery
PIN (step 4), and installs once you type `ERASE`. Remove the stick and
reboot.

### Install without questions

Put an `answers.toml` on a second stick labelled `CRUSTANS`, and the
installer uses it instead of asking. It prints the admin token and PIN on
the screen and the serial port.

```toml
disk = "/dev/nvme0n1"   # the boot disk; everything on it is erased
erase = true            # required: you agree to erase it
hostname = "node-1"
channel = "stable"      # or beta
after = "reboot"        # or poweroff

[network]               # leave it out for DHCP
method = "static"
address = "192.168.1.23/24"
gateway = "192.168.1.1"
dns = ["192.168.1.1"]

[pool]                  # optional
name = "tank"
layout = "mirror"       # single, mirror or raidz
disks = ["/dev/sda", "/dev/sdb"]
```

## 3b. Or: write Molt onto the boot disk

You write the disk image straight onto the disk the machine will boot from.
**Everything on that disk is erased.** Two ways:

- **From another computer:** put the target SSD in a USB enclosure or adapter
  and connect it to a Linux or macOS computer.
- **On the machine itself:** start it from any live Linux USB stick (for
  example Fedora Workstation Live), download the file there, and write it to
  the internal disk.

Find the target disk. On Linux run `lsblk` (for example `/dev/nvme0n1` or
`/dev/sdb`); on macOS run `diskutil list` (for example `/dev/disk4`) and
`diskutil unmountDisk /dev/diskN` first. Check the size and model twice: the
wrong disk loses its data. Then:

```sh
# Linux
zstd -dc molt-VERSION-ARCH.raw.zst | sudo dd of=/dev/sdX bs=4M status=progress conv=fsync
# macOS
zstd -dc molt-VERSION-ARCH.raw.zst | sudo dd of=/dev/rdiskN bs=4m
```

Boot the machine from that disk. On first boot Molt grows its root filesystem
to fill the disk.

### 3c. Or: run Molt as a virtual machine

Import `molt-VERSION-ARCH.qcow2` as the VM's disk, with UEFI firmware
(OVMF), 2 CPUs and 2 GB of memory or more. For example with libvirt:

```sh
virt-install --name molt --memory 4096 --vcpus 2 --boot uefi \
  --disk molt-VERSION-amd64.qcow2 --import --os-variant fedora-unknown
```

## 4. Write down the admin token and the recovery PIN

Molt has no SSH: you manage it through its API, and the API needs the
machine's **admin token**. With it comes a nine-digit **recovery PIN** for
the console on the machine's screen. The installer shows both before it
installs. Installed from a disk image (3b, 3c), Molt makes them on its first
boot and its screen shows them, with a QR code of the API reference:

```text
1  From a computer on the same network, log in:
     crust login --api http://192.168.1.23:8080
2  Enter this admin token when it asks:
     crt_XXXXX-XXXXX-XXXXX-XXXXX
3  Keep the recovery PIN for this screen:
     123 456 789
```

Write both down, then press enter. They are not shown again, after enter or
after a reboot. The screen's risky actions ask for the PIN; with it you can
also make a new admin token there. Without either, reinstall.

## 5. Log in

The screen shows the API address; so does your router's list of connected
devices. Open `http://ADDRESS:8080/docs` in a browser for the API
reference, or use the CLI:

```sh
crust --api http://192.168.1.23:8080 login    # paste the admin token
crust status
crust node list
```

`crust login` checks the token and saves it, readable only by you, in
`~/.config/crust/config.toml`; later commands use that API and token. The node
shows as `Ready` within a few seconds.

> **The API speaks plain HTTP for now** (TLS comes with ingress in M2), so the
> token crosses the network unencrypted. Only connect Molt to a network you
> trust.

## The console on the machine's screen

After the first boot the screen shows the **recovery console**: whether the
API and the agent run, the address, the update state, and a menu.

| Key      | What                                                                      |
| -------- | ------------------------------------------------------------------------- |
| `1`      | Show and change the network: DHCP or a fixed address, DNS, hostname, VLAN |
| `2`      | Reset the network to DHCP                                                 |
| `3`      | Roll back to the previous OS image                                        |
| `4`      | Make a new admin token; the old one stops working                         |
| `5`      | Diagnostics: services, this boot, disk health (SMART), connection tests   |
| `6`, `7` | Reboot, shut down                                                         |
| `alt+f2` | The system log (`alt+f1` goes back)                                       |

Actions 2, 3, 4, 6 and 7, and network changes, ask for the recovery PIN.
Five wrong PINs lock it for 15 minutes; every attempt is in the audit log
(`GET /v1/audit`). A network change reverts by itself after 60 seconds
unless you press `c` to keep it, so a typo cannot lock you out.

After an update, the first boot of the new image shows its health checks.
If one fails, Molt goes back to the previous image by itself.

## Secure Boot

Molt boots with Secure Boot on. To use ZFS on such a machine, enrol the
Crustates module-signing key once: see [crust molt enroll-key](molt/enroll-key.md).

## Updates

Molt follows one channel of `ghcr.io/crustates/molt`. The machine checks it
once a day, and installs only images that the Crustates release workflows
signed.

```sh
crust molt status                  # what runs, what is staged, what is new
crust molt policy set auto --window 03:00-05:00
crust molt policy set manual       # only report new images
crust molt upgrade [--now]         # stage the newest image (and reboot)
crust molt rollback [--now]        # go back to the previous image
crust molt channel set stable      # follow another channel
```

- **auto** (the default) stages a new image when it appears and reboots into
  it inside the window, in the machine's local time.
- **manual** only reports new images; `crust molt upgrade` stages one, and
  `--now` reboots into it right away.

After an update the machine checks itself: the API answers, the agent runs,
ZFS pools imported, the network is up. If the new image fails that check, or
does not boot at all, the machine goes back to the previous image by itself
and never installs that image again. `crust molt status` shows it under
"Marked bad".

### Channels

| Channel       | Gets                                                       |
| ------------- | ---------------------------------------------------------- |
| `stable`      | Releases that ran on `beta` for at least a week, unchanged |
| `beta`        | Every release, pre-releases too                            |
| `development` | Every merge; for testing Crustates itself                  |

A new machine picks its channel at its first check: `stable` if it holds a
version at least as new as the installed one, else `beta`. It keeps that
choice; `crust molt status` shows it.

`crust molt channel set <channel>` switches. A switch to a channel with an
older version is refused, because an older version may not read what a newer
one wrote; `--allow-downgrade` switches anyway. With the `auto` policy the
machine stages the new channel's image right away; with `manual`, run
`crust molt upgrade`.
