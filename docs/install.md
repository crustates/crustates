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

| File                           | Use it to                                        |
| ------------------------------ | ------------------------------------------------ |
| `molt-VERSION-ARCH.raw.zst`    | Write Molt onto the machine's boot disk (step 3) |
| `molt-VERSION-ARCH.qcow2`      | Run Molt as a virtual machine (step 3b)          |
| `molt-VERSION-ARCH.SHA256SUMS` | Check the downloads (step 2)                     |

Each file has a `.sigstore.json` signature next to it. There is no installer
ISO yet: Crustates' own installer, with a first-run wizard and a hardware
check, comes later in M0.

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

## 3. Write Molt onto the boot disk

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

### 3b. Or: run Molt as a virtual machine

Import `molt-VERSION-ARCH.qcow2` as the VM's disk, with UEFI firmware
(OVMF), 2 CPUs and 2 GB of memory or more. For example with libvirt:

```sh
virt-install --name molt --memory 4096 --vcpus 2 --boot uefi \
  --disk molt-VERSION-amd64.qcow2 --import --os-variant fedora-unknown
```

## 4. Write down the admin token

Molt has no SSH: you manage it through its API, and the API needs the
machine's **admin token**. Molt makes it on the first boot and shows it on
the machine's screen, above the login prompt:

```text
Crustates admin token: crt_XXXXX-XXXXX-XXXXX-XXXXX
Write it down now: it is shown only until the next reboot.
On your computer: crust --api http://192.168.1.23:8080 login
```

Write it down before you reboot. Without it you cannot use the API, and
until the local recovery console arrives (later in M0) the only way to get a
new one is to reinstall.

## 5. Log in

The screen also shows the API address. Or look in your router's list of
connected devices. Open `http://ADDRESS:8080/docs` in a browser for the API
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

## Secure Boot

Molt boots with Secure Boot on. To use ZFS on such a machine, enrol the
Crustates module-signing key once: see [crust molt enroll-key](molt/enroll-key.md).

## Updates

Molt follows the `beta` channel of `ghcr.io/crustates/molt`. The machine
checks it once a day, and installs only images that the Crustates release
workflow signed.

```sh
crust molt status                  # what runs, what is staged, what is new
crust molt policy set auto --window 03:00-05:00
crust molt policy set manual       # only report new images
crust molt upgrade [--now]         # stage the newest image (and reboot)
crust molt rollback [--now]        # go back to the previous image
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
