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

## 4. Find the API address

Molt has no SSH: you manage it through its API.

- The machine's own screen shows the address above the login prompt:
  `Crustates API: http://192.168.1.23:8080/docs`.
- Or look in your router's list of connected devices.

Open that address in a browser for the API reference, or use the CLI:

```sh
crust --api http://192.168.1.23:8080 status
crust --api http://192.168.1.23:8080 node list
```

The node shows as `Ready` within a few seconds.

> **The API has no login yet** (accounts come in M4). Only connect Molt to a
> network you trust.

## Secure Boot

Molt boots with Secure Boot on. To use ZFS on such a machine, enrol the
Crustates module-signing key once: see [crust molt enroll-key](molt/enroll-key.md).

## Updates

Molt follows the `beta` channel of `ghcr.io/crustates/molt`, checks for new
images by itself and reboots into them. `crust molt` commands to control
upgrades and roll back come later.
