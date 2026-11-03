//! What to install: chosen on the screens, or read from an answers file
//! for an install without anyone at the machine.
//!
//! ```toml
//! disk = "/dev/nvme0n1"   # the boot disk; everything on it is erased
//! erase = true            # required: the answers-file form of typing ERASE
//! hostname = "node-1"
//! channel = "stable"      # stable or beta
//! policy = "auto"         # auto or manual
//! after = "reboot"        # reboot or poweroff
//!
//! [network]
//! method = "static"       # dhcp (the default) or static
//! address = "10.0.10.2/24"
//! gateway = "10.0.10.1"
//! dns = ["10.0.10.53"]
//! vlan = 10               # optional
//!
//! [pool]                  # optional
//! name = "tank"
//! layout = "mirror"       # single, mirror or raidz
//! disks = ["/dev/sda", "/dev/sdb"]
//! ```

use std::net::Ipv4Addr;

use serde::Deserialize;

/// The smallest boot disk, in bytes.
pub const MIN_DISK: u64 = 32 * 1000 * 1000 * 1000;
/// Below this the installer warns: images and logs fill the disk.
pub const SMALL_DISK: u64 = 64 * 1000 * 1000 * 1000;

/// An install.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Answers {
    /// The boot disk, `/dev/…`.
    pub disk: String,
    /// Whether the boot disk may be erased.
    #[serde(default)]
    pub erase: bool,
    /// The hostname; `node` if empty.
    #[serde(default)]
    pub hostname: String,
    /// The Molt channel.
    #[serde(default = "stable")]
    pub channel: String,
    /// `auto` or `manual`.
    #[serde(default = "auto")]
    pub policy: String,
    /// `reboot` or `poweroff` when done.
    #[serde(default = "reboot")]
    pub after: String,
    /// The management network.
    #[serde(default)]
    pub network: NetworkAnswers,
    /// A ZFS pool to make.
    pub pool: Option<PoolAnswers>,
}

fn stable() -> String {
    "stable".into()
}

fn auto() -> String {
    "auto".into()
}

fn reboot() -> String {
    "reboot".into()
}

/// The management network.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkAnswers {
    /// `dhcp` or `static`.
    #[serde(default)]
    pub method: String,
    /// The interface; the first one with a link if empty.
    #[serde(default)]
    pub interface: String,
    /// `10.0.10.2/24`, for static.
    #[serde(default)]
    pub address: String,
    /// For static.
    #[serde(default)]
    pub gateway: String,
    /// DNS servers.
    #[serde(default)]
    pub dns: Vec<String>,
    /// A VLAN tag.
    pub vlan: Option<u16>,
}

impl NetworkAnswers {
    /// Whether the address is fixed.
    #[must_use]
    pub fn is_static(&self) -> bool {
        self.method == "static"
    }
}

/// A ZFS pool.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoolAnswers {
    /// Its name.
    pub name: String,
    /// `single`, `mirror` or `raidz`.
    pub layout: String,
    /// Its disks.
    pub disks: Vec<String>,
}

impl Answers {
    /// Reads an answers file.
    ///
    /// # Errors
    ///
    /// Fails if it is not valid TOML of this shape.
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// The hostname to set.
    #[must_use]
    pub fn hostname(&self) -> &str {
        if self.hostname.is_empty() {
            "node"
        } else {
            &self.hostname
        }
    }

    /// Checks the answers against the machine's disks: `(path, bytes)`.
    ///
    /// # Errors
    ///
    /// Says what is wrong, in words for the screen.
    pub fn check(&self, disks: &[(String, u64)]) -> Result<(), String> {
        if !self.erase {
            return Err("erase = true is missing: the install erases the boot disk".into());
        }
        let size = |path: &str| disks.iter().find(|(p, _)| p == path).map(|(_, s)| *s);
        match size(&self.disk) {
            None => return Err(format!("there is no disk {}", self.disk)),
            Some(bytes) if bytes < MIN_DISK => {
                return Err(format!(
                    "{} is {}; the boot disk needs 32 GB or more",
                    self.disk,
                    human(bytes)
                ));
            }
            Some(_) => {}
        }
        if !matches!(self.channel.as_str(), "stable" | "beta") {
            return Err(format!("channel {:?}: stable or beta", self.channel));
        }
        if !matches!(self.policy.as_str(), "auto" | "manual") {
            return Err(format!("policy {:?}: auto or manual", self.policy));
        }
        if !matches!(self.after.as_str(), "reboot" | "poweroff") {
            return Err(format!("after {:?}: reboot or poweroff", self.after));
        }
        let host = self.hostname();
        if host.len() > 63
            || host.starts_with('-')
            || !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return Err(format!(
                "hostname {host:?}: letters, digits and dashes, up to 63"
            ));
        }
        let net = &self.network;
        if !matches!(net.method.as_str(), "" | "dhcp" | "static") {
            return Err(format!("network method {:?}: dhcp or static", net.method));
        }
        if net.is_static() {
            let (ip, prefix) = net
                .address
                .split_once('/')
                .ok_or("network address needs a prefix, for example 10.0.10.2/24")?;
            ip.parse::<Ipv4Addr>()
                .map_err(|_| format!("{ip} is not an IPv4 address"))?;
            prefix
                .parse::<u8>()
                .ok()
                .filter(|p| (1..=32).contains(p))
                .ok_or("the prefix must be 1 to 32")?;
            if !net.gateway.is_empty() {
                net.gateway
                    .parse::<Ipv4Addr>()
                    .map_err(|_| format!("{} is not an IPv4 address", net.gateway))?;
            }
        }
        for dns in &net.dns {
            dns.parse::<Ipv4Addr>()
                .map_err(|_| format!("DNS server {dns} is not an IPv4 address"))?;
        }
        if net.vlan.is_some_and(|v| !(1..=4094).contains(&v)) {
            return Err("the VLAN tag must be 1 to 4094".into());
        }
        if let Some(pool) = &self.pool {
            let valid = !pool.name.is_empty()
                && pool
                    .name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic())
                && pool
                    .name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
            if !valid {
                return Err(format!(
                    "pool name {:?}: a letter, then letters, digits, - _ .",
                    pool.name
                ));
            }
            let needed = match pool.layout.as_str() {
                "single" => 1,
                "mirror" => 2,
                "raidz" => 3,
                other => return Err(format!("pool layout {other:?}: single, mirror or raidz")),
            };
            if pool.disks.len() < needed {
                return Err(format!(
                    "a {} pool needs {needed} disks or more",
                    pool.layout
                ));
            }
            for disk in &pool.disks {
                if disk == &self.disk {
                    return Err(format!(
                        "{disk} is the boot disk; the pool needs other disks"
                    ));
                }
                if size(disk).is_none() {
                    return Err(format!("there is no disk {disk}"));
                }
            }
        }
        Ok(())
    }
}

/// `512 GB`, `31.9 GB`.
#[must_use]
pub fn human(bytes: u64) -> String {
    let gb = bytes / 100_000_000;
    if gb >= 10_000 {
        format!("{}.{} TB", gb / 10_000, gb % 10_000 / 1000)
    } else if gb >= 1000 {
        format!("{} GB", gb / 10)
    } else {
        format!("{}.{} GB", gb / 10, gb % 10)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DISKS: [(&str, u64); 3] = [
        ("/dev/vda", 40_000_000_000),
        ("/dev/vdb", 8_000_000_000),
        ("/dev/vdc", 8_000_000_000),
    ];

    fn disks() -> Vec<(String, u64)> {
        DISKS.iter().map(|(p, s)| ((*p).to_owned(), *s)).collect()
    }

    #[test]
    fn a_minimal_file_installs_with_defaults() {
        let answers = Answers::parse("disk = \"/dev/vda\"\nerase = true\n").unwrap();
        assert_eq!(answers.check(&disks()), Ok(()));
        assert_eq!(
            (
                answers.hostname(),
                answers.channel.as_str(),
                answers.policy.as_str()
            ),
            ("node", "stable", "auto")
        );
        assert!(!answers.network.is_static());
    }

    #[test]
    fn mistakes_are_caught_before_anything_is_erased() {
        let full = "disk = \"/dev/vda\"\nerase = true\n[network]\nmethod = \"static\"\naddress = \"10.0.10.2/24\"\ngateway = \"10.0.10.1\"\n[pool]\nname = \"tank\"\nlayout = \"mirror\"\ndisks = [\"/dev/vdb\", \"/dev/vdc\"]\n";
        assert_eq!(Answers::parse(full).unwrap().check(&disks()), Ok(()));
        for (from, to, why) in [
            ("erase = true", "erase = false", "erase"),
            ("/dev/vda\"\nerase", "/dev/vdb\"\nerase", "32 GB"),
            ("10.0.10.2/24", "10.0.10.2", "prefix"),
            ("\"/dev/vdb\", \"/dev/vdc\"", "\"/dev/vdb\"", "2 disks"),
            (
                "\"/dev/vdb\", \"/dev/vdc\"",
                "\"/dev/vda\", \"/dev/vdc\"",
                "boot disk",
            ),
            ("layout = \"mirror\"", "layout = \"raid5\"", "layout"),
        ] {
            let text = full.replace(from, to);
            let err = Answers::parse(&text).unwrap().check(&disks()).unwrap_err();
            assert!(err.contains(why), "{why}: {err}");
        }
        assert!(
            Answers::parse("disk = \"/dev/vda\"\ncolour = \"red\"\n").is_err(),
            "unknown keys"
        );
        assert_eq!(human(512_110_190_592), "512 GB");
        assert_eq!(human(31_900_000_000), "31.9 GB");
        assert_eq!(human(4_000_787_030_016), "4.0 TB");
    }
}
