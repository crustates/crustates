//! [`System`] on a Molt machine. The console runs as root; it reads the
//! machine directly (sysfs, `ip`, `nmcli`, `systemctl`, `smartctl`), and
//! goes through the API for what the API owns: the PIN, the Molt status
//! and rollbacks. Network changes go through `NetworkManager` with a
//! checkpoint that reverts them after a minute unless confirmed.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crust_tui::widgets::Check;
use serde_json::Value;
use tokio::process::Command;

use crate::api::Api;
use crate::system::{
    Action, Diagnostics, Disk, FirstBoot, Health, Interface, Link, Method, Network, NetworkChange,
    Outcome, Service, ServiceRow, Snapshot, System, TestResult,
};

/// The first-boot secrets, written by `crust-firstboot`.
const FIRST_BOOT: &str = "/run/crustates/firstboot.json";
/// The health check's progress, written by greenboot's check.
const HEALTH: &str = "/run/crustates/health.json";
/// The node name the agent registered with.
const NODE_NAME: &str = "/var/lib/crust-agent/node-name";
/// How long `NetworkManager` waits for a confirmation.
const CHECKPOINT_SECS: u32 = 60;
/// Checkpoint flags: destroy older checkpoints, delete connections made
/// after it, disconnect devices that appear after it.
const CHECKPOINT_FLAGS: u32 = 0x1 | 0x2 | 0x4;

/// The machine the console runs on.
#[derive(Debug)]
pub struct Linux {
    api: Api,
    api_port: u16,
    admin_token_file: PathBuf,
}

impl Linux {
    /// The machine, its API at the local socket `socket`; `crust-api.env`
    /// gives the port and the token file.
    #[must_use]
    pub fn new(socket: PathBuf) -> Self {
        let port = std::env::var("CRUST_LISTEN")
            .ok()
            .and_then(|l| l.rsplit(':').next().and_then(|p| p.parse().ok()))
            .unwrap_or(8080);
        let token = std::env::var_os("CRUST_ADMIN_TOKEN_FILE").map_or_else(
            || PathBuf::from("/var/lib/crustates/secrets/admin-token"),
            PathBuf::from,
        );
        Self {
            api: Api::new(socket),
            api_port: port,
            admin_token_file: token,
        }
    }
}

/// Runs a program and returns its standard output.
async fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let output = tokio::time::timeout(
        Duration::from_secs(20),
        Command::new(program).args(args).kill_on_drop(true).output(),
    )
    .await
    .map_err(|_| format!("{program} did not finish in time"))?
    .map_err(|e| format!("{program}: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "{program}: {}",
            stderr.trim().lines().last().unwrap_or("failed")
        ))
    }
}

fn read(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_owned())
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

async fn service(unit: &str) -> Service {
    match run("systemctl", &["is-active", unit])
        .await
        .as_deref()
        .map(str::trim)
    {
        Ok("active") => Service::Running,
        Err(e) if e.contains("activating") => Service::Starting,
        _ => match run("systemctl", &["show", "-p", "ActiveState", "--value", unit])
            .await
            .as_deref()
            .map(str::trim)
        {
            Ok("activating" | "reloading") => Service::Starting,
            _ => Service::Stopped,
        },
    }
}

/// `nmcli -t -g` values, which separate several values with ` | `.
async fn nm_values(args: &[&str]) -> Vec<String> {
    run("nmcli", args)
        .await
        .unwrap_or_default()
        .lines()
        .flat_map(|l| l.split(" | "))
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty() && v != "--")
        .collect()
}

#[allow(clippy::too_many_lines, reason = "one fact after the other")]
async fn network() -> Network {
    let mut interfaces = Vec::new();
    let mut names: Vec<String> = std::fs::read_dir("/sys/class/net")
        .map(|d| {
            d.filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    let addresses = json(
        &run("ip", &["-j", "-4", "addr", "show"])
            .await
            .unwrap_or_default(),
    );
    let address_of = |dev: &str| -> Vec<String> {
        addresses
            .as_array()
            .into_iter()
            .flatten()
            .filter(|a| a["ifname"] == dev)
            .flat_map(|a| a["addr_info"].as_array().cloned().unwrap_or_default())
            .filter_map(|i| {
                Some(format!(
                    "{}/{}",
                    i["local"].as_str()?,
                    i["prefixlen"].as_u64()?
                ))
            })
            .collect()
    };
    let route = json(
        &run("ip", &["-j", "-4", "route", "show", "default"])
            .await
            .unwrap_or_default(),
    );
    let default_dev = route[0]["dev"].as_str().map(str::to_owned);
    let gateway = route[0]["gateway"].as_str().map(str::to_owned);
    // A VLAN device is `parent.id`: show its address on the parent.
    let (parent, vlan) = match default_dev.as_deref().and_then(|d| d.split_once('.')) {
        Some((parent, id)) => (Some(parent.to_owned()), id.parse().ok()),
        None => (default_dev.clone(), None),
    };
    for name in names {
        let sys = Path::new("/sys/class/net").join(&name);
        if !sys.join("device").exists() {
            continue;
        }
        let wireless = sys.join("wireless").exists() || sys.join("phy80211").exists();
        let mut addresses = address_of(&name);
        if parent.as_deref() == Some(name.as_str())
            && let Some(dev) = &default_dev
            && dev != &name
        {
            addresses.extend(address_of(dev));
        }
        interfaces.push(Interface {
            link: if wireless {
                Link::Ignored
            } else if read(sys.join("carrier")).as_deref() == Some("1") {
                Link::Up
            } else {
                Link::Down
            },
            speed: read(sys.join("speed"))
                .and_then(|s| s.parse().ok())
                .filter(|s: &u32| *s > 0),
            full_duplex: read(sys.join("duplex")).as_deref() == Some("full"),
            mac: read(sys.join("address")).unwrap_or_default(),
            addresses,
            name,
        });
    }
    let management = parent.or_else(|| {
        interfaces
            .iter()
            .find(|i| i.link == Link::Up)
            .or_else(|| interfaces.iter().find(|i| i.link != Link::Ignored))
            .map(|i| i.name.clone())
    });
    let device = default_dev
        .clone()
        .or_else(|| management.clone())
        .unwrap_or_default();
    let connection =
        nm_values(&["-t", "-g", "GENERAL.CONNECTION", "device", "show", &device]).await;
    let method = match connection.first() {
        Some(c) => nm_values(&["-t", "-g", "ipv4.method", "connection", "show", c]).await,
        None => Vec::new(),
    };
    Network {
        interfaces,
        management,
        method: if method.first().is_some_and(|m| m == "manual") {
            Method::Static
        } else {
            Method::Dhcp
        },
        gateway,
        dns: nm_values(&["-t", "-g", "IP4.DNS", "device", "show", &device]).await,
        search: nm_values(&["-t", "-g", "IP4.DOMAIN", "device", "show", &device]).await,
        vlan,
    }
}

fn version() -> String {
    read("/usr/lib/os-release")
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix("IMAGE_VERSION="))
        .map_or_else(
            || env!("CARGO_PKG_VERSION").to_owned(),
            |v| v.trim_matches('"').to_owned(),
        )
}

fn arch() -> String {
    match std::env::consts::ARCH {
        "x86_64" => "amd64".into(),
        "aarch64" => "arm64".into(),
        other => other.into(),
    }
}

impl System for Linux {
    async fn snapshot(&self) -> Snapshot {
        let hostname = read("/proc/sys/kernel/hostname").unwrap_or_default();
        let api_up = self.api.healthy().await;
        let nodes = if api_up {
            self.api.nodes().await.unwrap_or_default()
        } else {
            Vec::new()
        };
        let node = read(NODE_NAME)
            .or_else(|| (nodes.len() == 1).then(|| nodes[0].metadata.name.clone()))
            .unwrap_or_else(|| hostname.clone());
        let molt = nodes
            .iter()
            .find(|n| n.metadata.name == node)
            .and_then(|n| n.status.molt.clone());
        let pools = run("zpool", &["list", "-H", "-o", "name,health"])
            .await
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.split_once('\t'))
            .map(|(name, health)| (name.to_owned(), health == "ONLINE"))
            .collect();
        Snapshot {
            now: jiff::Zoned::now().strftime("%Y-%m-%d %H:%M").to_string(),
            version: version(),
            arch: arch(),
            node,
            api_port: self.api_port,
            api: if api_up {
                Service::Running
            } else {
                service("crust-api").await
            },
            agent: service("crust-agent").await,
            first_boot: read(FIRST_BOOT).and_then(|t| serde_json::from_str::<FirstBoot>(&t).ok()),
            network: network().await,
            molt,
            pools,
            health: read(HEALTH).and_then(|t| serde_json::from_str::<Health>(&t).ok()),
            hostname,
        }
    }

    #[allow(clippy::too_many_lines, reason = "one fact after the other")]
    async fn diagnostics(&self) -> Diagnostics {
        let snapshot = self.snapshot().await;
        let mut services = Vec::new();
        for unit in [
            "crust-api",
            "crust-agent",
            "crust-console",
            "NetworkManager",
            "chronyd",
        ] {
            let show = run(
                "systemctl",
                &[
                    "show",
                    "--timestamp=unix",
                    "-p",
                    "ActiveState",
                    "-p",
                    "SubState",
                    "-p",
                    "ActiveEnterTimestamp",
                    unit,
                ],
            )
            .await
            .unwrap_or_default();
            let value = |key: &str| {
                show.lines()
                    .find_map(|l| l.strip_prefix(&format!("{key}=")))
                    .unwrap_or_default()
                    .to_owned()
            };
            let since = value("ActiveEnterTimestamp")
                .trim_start_matches('@')
                .parse::<i64>()
                .ok();
            let up = since
                .map(|t| jiff::Timestamp::now().as_second() - t)
                .filter(|s| *s > 0);
            services.push(ServiceRow {
                name: unit.into(),
                ok: value("ActiveState") == "active",
                state: value("SubState"),
                note: up.map_or_else(String::new, |s| format!("up {}", ago(s))),
            });
        }
        let names: Vec<&str> = snapshot.pools.iter().map(|(n, _)| n.as_str()).collect();
        services.push(ServiceRow {
            name: "zfs".into(),
            ok: snapshot.pools.iter().all(|(_, ok)| *ok),
            state: format!("{} pools", names.len()),
            note: names.join(", "),
        });

        let mut boot = Vec::new();
        if let Some(molt) = &snapshot.molt {
            if let Some(b) = &molt.booted {
                let signed = if b.verified {
                    "signature verified"
                } else {
                    "signature not checked here"
                };
                boot.push((
                    "Image".into(),
                    format!(
                        "{}  {}  {signed}",
                        b.version,
                        b.digest.get(..19).unwrap_or(&b.digest)
                    ),
                ));
            }
            if let Some(r) = &molt.rollback {
                boot.push((
                    "Previous".into(),
                    format!("{}  kept for rollback", r.version),
                ));
            }
        }
        if let Some(up) =
            read("/proc/uptime").and_then(|u| u.split('.').next()?.parse::<i64>().ok())
        {
            let booted = jiff::Timestamp::now() - jiff::SignedDuration::from_secs(up);
            let local = booted
                .to_zoned(jiff::tz::TimeZone::system())
                .strftime("%Y-%m-%d %H:%M")
                .to_string();
            boot.push(("Booted".into(), format!("{local}  ({} ago)", ago(up))));
        }
        let secure = run("mokutil", &["--sb-state"]).await.unwrap_or_default();
        boot.push((
            "Secure Boot".into(),
            if secure.contains("enabled") {
                "on".into()
            } else {
                "off".into()
            },
        ));
        boot.push((
            "TPM".into(),
            read("/sys/class/tpm/tpm0/tpm_version_major")
                .map_or("none".into(), |v| format!("{v}.0")),
        ));
        let zfs = read("/sys/module/zfs/version").map_or_else(String::new, |v| {
            format!("  ZFS {}", v.split('-').next().unwrap_or(&v))
        });
        boot.push((
            "Kernel".into(),
            format!(
                "{}{zfs}",
                read("/proc/sys/kernel/osrelease").unwrap_or_default()
            ),
        ));
        let load = read("/proc/loadavg")
            .unwrap_or_default()
            .split_whitespace()
            .take(3)
            .collect::<Vec<_>>()
            .join(" ");
        let meminfo = read("/proc/meminfo").unwrap_or_default();
        let kib = |key: &str| {
            meminfo
                .lines()
                .find_map(|l| {
                    l.strip_prefix(key)?
                        .split_whitespace()
                        .next()?
                        .parse::<u64>()
                        .ok()
                })
                .unwrap_or(0)
        };
        let (total, available) = (kib("MemTotal:"), kib("MemAvailable:"));
        boot.push((
            "Load".into(),
            format!(
                "{load}  RAM {} / {} GiB",
                (total - available).div_ceil(1 << 20),
                total.div_ceil(1 << 20)
            ),
        ));

        Diagnostics {
            services,
            boot,
            disks: disks().await,
        }
    }

    async fn test_connection(&self, network: &Network) -> Vec<TestResult> {
        let mut tests = Vec::new();
        let test = |name: &str, target: &str, result, value: String| TestResult {
            name: name.into(),
            target: target.into(),
            result,
            value,
        };
        if let Some(i) = network.management() {
            let speed = i.speed.map_or_else(String::new, |s| format!("{s} Mb/s "));
            let duplex = if i.full_duplex { "full duplex" } else { "" };
            tests.push(match i.link {
                Link::Up => test(
                    "link",
                    &i.name,
                    Check::Passed,
                    format!("{speed}{duplex}").trim().to_owned(),
                ),
                _ => test("link", &i.name, Check::Failed, "no link".into()),
            });
        }
        if let Some(gw) = &network.gateway {
            tests.push(match run("ping", &["-c", "1", "-W", "2", gw]).await {
                Ok(out) => {
                    let time = out
                        .split("time=")
                        .nth(1)
                        .and_then(|t| t.split_whitespace().next())
                        .unwrap_or("?");
                    test("gateway", gw, Check::Passed, format!("{time} ms"))
                }
                Err(_) => test("gateway", gw, Check::Failed, "no answer".into()),
            });
        }
        let started = Instant::now();
        let resolved = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::lookup_host("ghcr.io:443"),
        )
        .await;
        tests.push(match resolved {
            Ok(Ok(_)) => test(
                "DNS",
                "ghcr.io",
                Check::Passed,
                format!("{} ms", started.elapsed().as_millis()),
            ),
            _ => test("DNS", "ghcr.io", Check::Failed, "cannot resolve".into()),
        });
        let connected = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::TcpStream::connect("ghcr.io:443"),
        )
        .await;
        tests.push(match connected {
            Ok(Ok(_)) => test("registry", "ghcr.io:443", Check::Passed, "reachable".into()),
            _ => test(
                "registry",
                "ghcr.io:443",
                Check::Failed,
                "not reachable".into(),
            ),
        });
        let tracking = run("chronyc", &["-c", "tracking"])
            .await
            .unwrap_or_default();
        let fields: Vec<&str> = tracking.trim().split(',').collect();
        let offset = fields
            .get(4)
            .and_then(|o| o.parse::<f64>().ok())
            .map(|o| (o.abs() * 1000.0).round());
        tests.push(match (fields.get(13).map(|s| s.trim()), offset) {
            (Some("Normal"), Some(ms)) if ms < 100.0 => {
                test("NTP", "chrony", Check::Passed, format!("offset {ms} ms"))
            }
            (Some("Normal"), Some(ms)) => {
                test("NTP", "chrony", Check::Slow, format!("offset {ms} ms"))
            }
            _ => test("NTP", "chrony", Check::Failed, "not synchronised".into()),
        });
        tests
    }

    async fn check_pin(&self, pin: &str, action: &Action) -> Result<(), String> {
        self.api.check_pin(pin, action.audit_name()).await
    }

    async fn run(&self, action: &Action, network: &Network) -> Result<Outcome, String> {
        match action {
            Action::RollBack => {
                let node = self.snapshot().await.node;
                self.api.roll_back(&node).await.map(Outcome::Done)
            }
            Action::NewAdminToken => {
                let token = run("crust-api", &["new-admin-token"])
                    .await?
                    .trim()
                    .to_owned();
                let file = self.admin_token_file.to_string_lossy().into_owned();
                run("chgrp", &["crustates", &file]).await?;
                run("chmod", &["0640", &file]).await?;
                Ok(Outcome::Token(token))
            }
            Action::Reboot => run("systemctl", &["reboot"])
                .await
                .map(|_| Outcome::Done("Rebooting now.".into())),
            Action::PowerOff => run("systemctl", &["poweroff"])
                .await
                .map(|_| Outcome::Done("Shutting down now.".into())),
            Action::ResetToDhcp => {
                let change = NetworkChange {
                    interface: network.management.clone().unwrap_or_default(),
                    method: Method::Dhcp,
                    vlan: network.vlan.map(|v| v.to_string()).unwrap_or_default(),
                    ..NetworkChange::default()
                };
                apply(&change).await
            }
            Action::Network(change) => apply(change).await,
        }
    }

    async fn confirm(&self, checkpoint: &str) -> Result<(), String> {
        checkpoint_call("CheckpointDestroy", checkpoint).await
    }

    async fn retry_dhcp(&self, network: &Network) -> Result<(), String> {
        let device = network.management.clone().ok_or("no network interface")?;
        run("nmcli", &["device", "connect", &device])
            .await
            .map(|_| ())
    }

    async fn finish_first_boot(&self) -> Result<(), String> {
        match tokio::fs::remove_file(FIRST_BOOT).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("{FIRST_BOOT}: {e}")),
        }
    }
}

/// `3d 4h`, `5h 12m`, `7m`.
fn ago(secs: i64) -> String {
    let (d, h, m) = (secs / 86_400, secs % 86_400 / 3600, secs % 3600 / 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

async fn disks() -> Vec<Disk> {
    let scan = json(&run("smartctl", &["--scan", "-j"]).await.unwrap_or_default());
    let boot_source = run("findmnt", &["-n", "-o", "SOURCE", "/sysroot"])
        .await
        .unwrap_or_default();
    let boot = run("lsblk", &["-n", "-o", "PKNAME", boot_source.trim()])
        .await
        .unwrap_or_default();
    let mut disks = Vec::new();
    for device in scan["devices"].as_array().into_iter().flatten() {
        let Some(path) = device["name"].as_str() else {
            continue;
        };
        // smartctl's exit code is a bit mask of findings; read the JSON anyway.
        let output = Command::new("smartctl")
            .args(["-j", "-i", "-H", "-A", path])
            .output()
            .await;
        let info = output.map_or(Value::Null, |o| json(&String::from_utf8_lossy(&o.stdout)));
        let name = path.trim_start_matches("/dev/").to_owned();
        let reallocated = info["ata_smart_attributes"]["table"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|a| a["id"] == 5)
            .and_then(|a| a["raw"]["value"].as_u64())
            .unwrap_or(0);
        let media_errors = info["nvme_smart_health_information_log"]["media_errors"]
            .as_u64()
            .unwrap_or(0);
        let (status, note) = match info["smart_status"]["passed"].as_bool() {
            Some(false) => ("failing", "SMART says the disk is failing".to_owned()),
            Some(true) if reallocated > 0 => ("warn", format!("{reallocated} reallocated sectors")),
            Some(true) if media_errors > 0 => ("warn", format!("{media_errors} media errors")),
            Some(true) => ("ok", String::new()),
            None => ("unknown", String::new()),
        };
        disks.push(Disk {
            role: if boot.trim() == name {
                "boot".into()
            } else {
                String::new()
            },
            model: info["model_name"].as_str().unwrap_or("-").to_owned(),
            temperature: info["temperature"]["current"].as_i64(),
            status: status.into(),
            note,
            name,
        });
    }
    disks
}

async fn checkpoint_call(method: &str, checkpoint: &str) -> Result<(), String> {
    let connection = zbus::Connection::system()
        .await
        .map_err(|e| e.to_string())?;
    let path = zbus::ObjectPath::try_from(checkpoint).map_err(|e| e.to_string())?;
    connection
        .call_method(
            Some("org.freedesktop.NetworkManager"),
            "/org/freedesktop/NetworkManager",
            Some("org.freedesktop.NetworkManager"),
            method,
            &(path,),
        )
        .await
        .map_err(|e| format!("NetworkManager: {e}"))?;
    Ok(())
}

/// Checks a change before anything is touched.
fn validate(change: &NetworkChange) -> Result<(), String> {
    if change.interface.is_empty() {
        return Err("there is no network interface to change".into());
    }
    if change.method == Method::Static {
        let (ip, prefix) = change
            .address
            .split_once('/')
            .ok_or("the address needs a prefix, for example 10.0.10.2/24")?;
        ip.parse::<std::net::Ipv4Addr>()
            .map_err(|_| format!("{ip} is not an IPv4 address"))?;
        prefix
            .parse::<u8>()
            .ok()
            .filter(|p| (1..=32).contains(p))
            .ok_or("the prefix must be 1 to 32")?;
        if !change.gateway.is_empty() {
            change
                .gateway
                .parse::<std::net::Ipv4Addr>()
                .map_err(|_| format!("{} is not an IPv4 address", change.gateway))?;
        }
    }
    if !change.vlan.is_empty() {
        change
            .vlan
            .parse::<u16>()
            .ok()
            .filter(|v| (1..=4094).contains(v))
            .ok_or("the VLAN tag must be 1 to 4094")?;
    }
    let host = &change.hostname;
    if !host.is_empty()
        && (host.len() > 63
            || !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            || host.starts_with('-'))
    {
        return Err("the hostname may hold letters, digits and dashes, up to 63".into());
    }
    Ok(())
}

/// Applies `change` under a `NetworkManager` checkpoint: it reverts after a
/// minute unless [`System::confirm`] keeps it.
async fn apply(change: &NetworkChange) -> Result<Outcome, String> {
    validate(change)?;
    let connection = zbus::Connection::system()
        .await
        .map_err(|e| e.to_string())?;
    let reply = connection
        .call_method(
            Some("org.freedesktop.NetworkManager"),
            "/org/freedesktop/NetworkManager",
            Some("org.freedesktop.NetworkManager"),
            "CheckpointCreate",
            &(
                Vec::<zbus::ObjectPath<'_>>::new(),
                CHECKPOINT_SECS,
                CHECKPOINT_FLAGS,
            ),
        )
        .await
        .map_err(|e| format!("NetworkManager could not make a checkpoint: {e}"))?;
    let checkpoint: zbus::OwnedObjectPath =
        reply.body().deserialize().map_err(|e| e.to_string())?;
    let checkpoint = checkpoint.as_str().to_owned();
    match configure(change).await {
        Ok(message) => Ok(Outcome::Applied {
            checkpoint,
            message,
        }),
        Err(e) => {
            let _ = checkpoint_call("CheckpointRollback", &checkpoint).await;
            Err(format!("{e}; the previous settings are back"))
        }
    }
}

async fn configure(change: &NetworkChange) -> Result<String, String> {
    let (device, connection) = if change.vlan.is_empty() {
        let device = change.interface.clone();
        let active =
            nm_values(&["-t", "-g", "GENERAL.CONNECTION", "device", "show", &device]).await;
        let name = if let Some(name) = active.first() {
            name.clone()
        } else {
            run(
                "nmcli",
                &[
                    "connection",
                    "add",
                    "type",
                    "ethernet",
                    "con-name",
                    &device,
                    "ifname",
                    &device,
                ],
            )
            .await?;
            device.clone()
        };
        (device, name)
    } else {
        let device = format!("{}.{}", change.interface, change.vlan);
        let name = format!("crust-vlan{}", change.vlan);
        if run(
            "nmcli",
            &["-t", "-g", "connection.id", "connection", "show", &name],
        )
        .await
        .is_err()
        {
            run(
                "nmcli",
                &[
                    "connection",
                    "add",
                    "type",
                    "vlan",
                    "con-name",
                    &name,
                    "ifname",
                    &device,
                    "dev",
                    &change.interface,
                    "id",
                    &change.vlan,
                ],
            )
            .await?;
        }
        (device, name)
    };
    let mut args: Vec<&str> = vec!["connection", "modify", &connection];
    let dns = change.dns.replace(',', " ");
    match change.method {
        Method::Dhcp => args.extend([
            "ipv4.method",
            "auto",
            "ipv4.addresses",
            "",
            "ipv4.gateway",
            "",
            "ipv4.dns",
            "",
        ]),
        Method::Static => args.extend([
            "ipv4.method",
            "manual",
            "ipv4.addresses",
            &change.address,
            "ipv4.gateway",
            &change.gateway,
            "ipv4.dns",
            dns.trim(),
        ]),
    }
    args.extend(["ipv4.dns-search", change.search.trim()]);
    run("nmcli", &args).await?;
    run("nmcli", &["connection", "up", &connection]).await?;
    let current = read("/proc/sys/kernel/hostname").unwrap_or_default();
    let mut message = match change.method {
        Method::Dhcp => format!("{device} uses DHCP now."),
        Method::Static => format!("{device} uses {} now.", change.address),
    };
    if !change.hostname.is_empty() && change.hostname != current {
        run("hostnamectl", &["set-hostname", &change.hostname]).await?;
        let _ = write!(
            message,
            " The hostname is {} (a revert keeps it).",
            change.hostname
        );
    }
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_are_checked_before_anything_is_touched() {
        let good = NetworkChange {
            interface: "enp3s0".into(),
            method: Method::Static,
            address: "10.0.10.2/24".into(),
            gateway: "10.0.10.1".into(),
            ..NetworkChange::default()
        };
        assert_eq!(validate(&good), Ok(()));
        for (field, value) in [
            ("address", "10.0.10.2"),
            ("address", "10.0.10.300/24"),
            ("gateway", "router"),
            ("vlan", "5000"),
            ("hostname", "node 1"),
        ] {
            let mut bad = good.clone();
            match field {
                "address" => bad.address = value.into(),
                "gateway" => bad.gateway = value.into(),
                "vlan" => bad.vlan = value.into(),
                _ => bad.hostname = value.into(),
            }
            assert!(validate(&bad).is_err(), "{field} {value}");
        }
        assert_eq!(ago(3 * 86_400 + 4 * 3600), "3d 4h");
        assert_eq!(ago(420), "7m");
    }
}
