//! Step 2: can this machine run Crustates? Facts are read once
//! ([`Facts::read`]); [`check`] judges them, so tests need no machine.

use std::path::Path;

use crate::answers::{MIN_DISK, SMALL_DISK, human};

/// A disk the installer can use.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Disk {
    /// `/dev/nvme0n1`.
    pub path: String,
    /// Bytes.
    pub size: u64,
    /// The model, from the disk.
    pub model: String,
    /// `NVMe`, `SATA`, `USB`, `virtio`.
    pub transport: String,
    /// What is on it: `empty`, `2 partitions (ntfs)`.
    pub contents: String,
    /// Whether the installer itself was started from it.
    pub install_media: bool,
}

/// What the hardware check reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// `amd64` or `arm64`.
    pub arch: String,
    /// Booted with UEFI.
    pub uefi: bool,
    /// The CPU flags (`vmx`, `svm`) on amd64.
    pub cpu_flags: String,
    /// Whether `/dev/kvm` exists.
    pub kvm: bool,
    /// The CPU model and its core count.
    pub cpu: String,
    /// IOMMU groups.
    pub iommu_groups: usize,
    /// Bytes of memory.
    pub memory: u64,
    /// `2.0`, if there is a TPM.
    pub tpm: Option<String>,
    /// Secure Boot on, if the firmware says.
    pub secure_boot: Option<bool>,
    /// Whether the Crustates module-signing key is enrolled (Secure Boot on).
    pub crustates_key: bool,
    /// The disks.
    pub disks: Vec<Disk>,
    /// Wired ports and how many have a link.
    pub ports: (usize, usize),
}

/// How a check came out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Fine.
    Ok,
    /// Works, with a caveat.
    Warn,
    /// Crustates cannot be installed.
    Fail,
}

/// One line of the hardware check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// What was checked.
    pub name: &'static str,
    /// How it came out.
    pub verdict: Verdict,
    /// What was found.
    pub result: String,
    /// What to do about a warning or failure.
    pub advice: String,
}

fn finding(
    name: &'static str,
    verdict: Verdict,
    result: impl Into<String>,
    advice: &str,
) -> Finding {
    Finding {
        name,
        verdict,
        result: result.into(),
        advice: advice.into(),
    }
}

/// Judges the facts.
#[must_use]
#[allow(clippy::too_many_lines, reason = "one check after the other")]
pub fn check(f: &Facts) -> Vec<Finding> {
    let mut out = Vec::new();
    let virt = if f.arch == "amd64" {
        match (f.cpu_flags.contains("vmx"), f.cpu_flags.contains("svm")) {
            (true, _) => Some("Intel VT-x"),
            (_, true) => Some("AMD-V"),
            _ => None,
        }
    } else {
        f.kvm.then_some("KVM")
    };
    out.push(match virt {
        Some(kind) => finding(
            "CPU virtualization",
            Verdict::Ok,
            format!("{kind} · {}", f.cpu),
            "",
        ),
        None => finding(
            "CPU virtualization",
            Verdict::Warn,
            format!("off · {}", f.cpu),
            "VMs need it: turn on VT-x, AMD-V or SVM in the firmware settings.",
        ),
    });
    out.push(if f.iommu_groups > 0 {
        finding(
            "IOMMU",
            Verdict::Ok,
            format!("on · {} groups", f.iommu_groups),
            "",
        )
    } else {
        finding(
            "IOMMU",
            Verdict::Warn,
            "off",
            "Passing devices to VMs needs it: turn on VT-d or AMD-Vi in the firmware settings.",
        )
    });
    let gib = f.memory.div_ceil(1 << 30);
    out.push(match gib {
        0..=3 => finding(
            "Memory",
            Verdict::Fail,
            format!("{gib} GB"),
            "Crustates needs 4 GB of memory or more.",
        ),
        4..=7 => finding(
            "Memory",
            Verdict::Warn,
            format!("{gib} GB"),
            "Fine for Crustates itself; VMs and apps want 8 GB or more.",
        ),
        _ => finding("Memory", Verdict::Ok, format!("{gib} GB"), ""),
    });
    out.push(match &f.tpm {
        Some(version) => finding("TPM", Verdict::Ok, format!("TPM {version}"), ""),
        None => finding(
            "TPM",
            Verdict::Warn,
            "none",
            "Disk encryption with a TPM comes with Reef; it will need one.",
        ),
    });
    out.push(match (f.secure_boot, f.crustates_key) {
        (Some(true), true) => finding(
            "Secure Boot",
            Verdict::Ok,
            "on · Crustates key enrolled",
            "",
        ),
        (Some(true), false) => finding(
            "Secure Boot",
            Verdict::Warn,
            "on · Crustates key not enrolled",
            "ZFS loads after `crust molt enroll-key` and one reboot.",
        ),
        (Some(false), _) => finding("Secure Boot", Verdict::Ok, "off", ""),
        (None, _) => finding(
            "Secure Boot",
            Verdict::Ok,
            "not supported by the firmware",
            "",
        ),
    });
    out.push(if f.uefi {
        finding("Firmware", Verdict::Ok, "UEFI", "")
    } else {
        finding(
            "Firmware",
            Verdict::Fail,
            "BIOS (legacy)",
            "Crustates needs UEFI: switch the boot mode to UEFI in the firmware settings.",
        )
    });
    let usable: Vec<&Disk> = f.disks.iter().filter(|d| !d.install_media).collect();
    let biggest = usable.iter().map(|d| d.size).max().unwrap_or(0);
    let count = format!("{} found", usable.len());
    out.push(if biggest >= SMALL_DISK {
        finding("Disks", Verdict::Ok, count, "")
    } else if biggest >= MIN_DISK {
        finding(
            "Disks",
            Verdict::Warn,
            format!("{count}, largest {}", human(biggest)),
            "32 GB works; 64 GB or more leaves room for updates and logs.",
        )
    } else {
        finding(
            "Disks",
            Verdict::Fail,
            format!("{count}, largest {}", human(biggest)),
            "The boot disk needs 32 GB or more.",
        )
    });
    let (ports, links) = f.ports;
    out.push(match (ports, links) {
        (0, _) => finding(
            "Network",
            Verdict::Warn,
            "no wired port",
            "Wifi is not supported; install offline and add a wired port later.",
        ),
        (p, 0) => finding(
            "Network",
            Verdict::Warn,
            format!("{p} ports · no cable"),
            "Plug in a cable, or install offline.",
        ),
        (p, l) => finding(
            "Network",
            Verdict::Ok,
            format!("{p} ports · {l} with a link"),
            "",
        ),
    });
    out.push(finding("Architecture", Verdict::Ok, f.arch.clone(), ""));
    out
}

impl Facts {
    /// Reads this machine.
    #[must_use]
    pub fn read(disks: Vec<Disk>) -> Self {
        let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
        let cpuinfo = read("/proc/cpuinfo");
        let flags = cpuinfo
            .lines()
            .find(|l| l.starts_with("flags"))
            .unwrap_or_default()
            .to_owned();
        let model = cpuinfo
            .lines()
            .find_map(|l| {
                l.strip_prefix("model name")
                    .map(|m| m.trim_start_matches([' ', '\t', ':']).to_owned())
            })
            .unwrap_or_default();
        let cores = cpuinfo
            .lines()
            .filter(|l| l.starts_with("processor"))
            .count();
        let memory = read("/proc/meminfo")
            .lines()
            .find_map(|l| {
                l.strip_prefix("MemTotal:")?
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()
            })
            .unwrap_or(0)
            * 1024;
        let sb = std::fs::read_dir("/sys/firmware/efi/efivars")
            .ok()
            .and_then(|d| {
                let var = d
                    .filter_map(Result::ok)
                    .find(|e| e.file_name().to_string_lossy().starts_with("SecureBoot-"))?;
                std::fs::read(var.path()).ok()?.last().map(|b| *b == 1)
            });
        let mut ports = (0, 0);
        for entry in std::fs::read_dir("/sys/class/net")
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
        {
            let sys = entry.path();
            if sys.join("device").exists() && !sys.join("wireless").exists() {
                ports.0 += 1;
                if std::fs::read_to_string(sys.join("carrier")).is_ok_and(|c| c.trim() == "1") {
                    ports.1 += 1;
                }
            }
        }
        Self {
            arch: match std::env::consts::ARCH {
                "x86_64" => "amd64".into(),
                "aarch64" => "arm64".into(),
                other => other.into(),
            },
            uefi: Path::new("/sys/firmware/efi").exists(),
            cpu_flags: flags,
            kvm: Path::new("/dev/kvm").exists(),
            cpu: if model.is_empty() {
                format!("{cores} cores")
            } else {
                format!("{cores} cores ({model})")
            },
            iommu_groups: std::fs::read_dir("/sys/kernel/iommu_groups").map_or(0, Iterator::count),
            memory,
            tpm: std::fs::read_to_string("/sys/class/tpm/tpm0/tpm_version_major")
                .ok()
                .map(|v| format!("{}.0", v.trim())),
            secure_boot: sb,
            crustates_key: std::process::Command::new("mokutil")
                .args([
                    "--test-key",
                    "/usr/share/crustates/keys/crustates-module-signing.der",
                ])
                .output()
                .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("already enrolled")),
            disks,
            ports,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine() -> Facts {
        Facts {
            arch: "amd64".into(),
            uefi: true,
            cpu_flags: "flags : fpu vmx sse".into(),
            cpu: "8 cores".into(),
            iommu_groups: 24,
            memory: 64 << 30,
            tpm: Some("2.0".into()),
            secure_boot: Some(true),
            crustates_key: true,
            disks: vec![Disk {
                path: "/dev/nvme0n1".into(),
                size: 500_000_000_000,
                ..Disk::default()
            }],
            ports: (2, 1),
            ..Facts::default()
        }
    }

    #[test]
    fn a_good_machine_passes_and_a_bad_one_says_why() {
        assert!(check(&machine()).iter().all(|f| f.verdict == Verdict::Ok));
        let mut old = machine();
        old.uefi = false;
        old.memory = 2 << 30;
        old.cpu_flags.clear();
        old.disks[0].size = 16_000_000_000;
        old.ports = (1, 0);
        let findings = check(&old);
        let verdict = |name: &str| findings.iter().find(|f| f.name == name).unwrap().verdict;
        assert_eq!(verdict("Firmware"), Verdict::Fail);
        assert_eq!(verdict("Memory"), Verdict::Fail);
        assert_eq!(verdict("Disks"), Verdict::Fail);
        assert_eq!(verdict("CPU virtualization"), Verdict::Warn);
        assert_eq!(verdict("Network"), Verdict::Warn);
        assert!(
            findings
                .iter()
                .filter(|f| f.verdict != Verdict::Ok)
                .all(|f| !f.advice.is_empty())
        );
    }

    #[test]
    fn the_install_stick_is_not_a_boot_disk() {
        let mut f = machine();
        f.disks = vec![Disk {
            path: "/dev/sda".into(),
            size: 64_000_000_000,
            install_media: true,
            ..Disk::default()
        }];
        let disks = check(&f).into_iter().find(|f| f.name == "Disks").unwrap();
        assert_eq!(disks.verdict, Verdict::Fail);
    }
}
