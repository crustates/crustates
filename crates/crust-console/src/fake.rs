//! A machine in memory, for the screen tests and `crust-console --demo`.
//! The values are examples, like the design kit's.

use std::sync::Mutex;

use crust_tui::widgets::Check;
use crust_types::{MoltImage, MoltStatus, UpdatePolicy};

use crate::system::{
    Action, Diagnostics, Disk, FirstBoot, Interface, Link, Method, Network, Outcome, Service,
    ServiceRow, Snapshot, System, TestResult,
};

/// A healthy node, `node-1`, on 10.0.10.2.
#[must_use]
pub fn snapshot() -> Snapshot {
    let image = |version: &str, digest: &str| MoltImage {
        image: format!("ghcr.io/crustates/molt@sha256:{digest}"),
        digest: format!("sha256:{digest}"),
        version: version.into(),
        verified: true,
    };
    Snapshot {
        now: "2026-11-03 21:45".into(),
        version: "0.2.0".into(),
        hostname: "node-1".into(),
        arch: "amd64".into(),
        node: "node-1".into(),
        api_port: 8080,
        api: Service::Running,
        agent: Service::Running,
        first_boot: None,
        network: Network {
            interfaces: vec![
                Interface {
                    name: "enp3s0".into(),
                    link: Link::Up,
                    speed: Some(2500),
                    full_duplex: true,
                    mac: "a8:a1:59:3c:10:02".into(),
                    addresses: vec!["10.0.10.2/24".into()],
                },
                Interface {
                    name: "enp4s0".into(),
                    mac: "a8:a1:59:3c:10:03".into(),
                    ..Interface::default()
                },
                Interface {
                    name: "wlp2s0".into(),
                    link: Link::Ignored,
                    mac: "f4:26:79:00:aa:01".into(),
                    ..Interface::default()
                },
            ],
            management: Some("enp3s0".into()),
            method: Method::Dhcp,
            gateway: Some("10.0.10.1".into()),
            dns: vec!["10.0.10.53".into()],
            search: vec!["home.arpa".into()],
            vlan: None,
        },
        molt: Some(MoltStatus {
            booted: Some(image("0.2.0", "4f2a9c01d3e5b7a90c1d2e3f4a5b6c7d")),
            rollback: Some(image("0.1.0", "9b1e77aa0c2d4e6f8a0b1c2d3e4f5a6b")),
            available: Some(image("0.2.1", "c3d4e5f60718293a4b5c6d7e8f901a2b")),
            repository: "ghcr.io/crustates/molt".into(),
            channel: "stable".into(),
            policy: UpdatePolicy::Auto,
            window: "03:00-05:00".into(),
            automatic_rollback: true,
            ..MoltStatus::default()
        }),
        pools: vec![("tank".into(), true), ("fast".into(), true)],
        health: None,
    }
}

/// The secrets of a first boot.
#[must_use]
pub fn first_boot() -> FirstBoot {
    FirstBoot {
        token: "crt_K7Q4M-2MRXV-9DPAZ-H3T8W".into(),
        pin: "482913075".into(),
    }
}

/// The health check of a new image in `state`.
#[cfg(test)]
#[must_use]
pub fn health(state: &str) -> crate::system::Health {
    use crate::system::{Health, HealthCheck};
    let check = |name: &str, state: &str, detail: &str| HealthCheck {
        name: name.into(),
        state: state.into(),
        detail: detail.into(),
    };
    let last = if state == "failed" {
        "failed"
    } else {
        "running"
    };
    Health {
        state: state.into(),
        checks: vec![
            check("crust-api", "ok", "ready"),
            check(
                "crust-agent",
                last,
                if state == "failed" {
                    "did not become ready in 300 s"
                } else {
                    "starting"
                },
            ),
            check("ZFS pools", "waiting", ""),
            check("network", "waiting", ""),
        ],
        deadline: 300,
    }
}

/// A network without link or address.
#[cfg(test)]
#[must_use]
pub fn offline() -> Network {
    let mut network = snapshot().network;
    for interface in &mut network.interfaces {
        if interface.link == Link::Up {
            interface.link = Link::Down;
        }
        interface.addresses.clear();
        interface.speed = None;
    }
    network.gateway = None;
    network
}

/// Diagnostics of a healthy node with one disk that needs a look.
#[must_use]
pub fn diagnostics() -> Diagnostics {
    let service = |name: &str, state: &str, note: &str| ServiceRow {
        name: name.into(),
        ok: state == "running",
        state: state.into(),
        note: note.into(),
    };
    let disk = |name: &str, model: &str, temperature, status: &str, role: &str, note: &str| Disk {
        name: name.into(),
        model: model.into(),
        temperature: Some(temperature),
        status: status.into(),
        role: role.into(),
        note: note.into(),
    };
    Diagnostics {
        services: vec![
            service("crust-api", "running", "up 3d 4h"),
            service("crust-agent", "running", "up 3d 4h"),
            service("NetworkManager", "running", "up 3d 4h"),
            service("chronyd", "running", "up 3d 4h"),
            service("zfs", "running", "2 pools: tank, fast"),
        ],
        boot: vec![
            (
                "Image".into(),
                "0.2.0  sha256:4f2a9c01d3e5  signature verified".into(),
            ),
            ("Previous".into(), "0.1.0  kept for rollback".into()),
            ("Booted".into(), "2026-10-31 17:41  (3d 4h ago)".into()),
            ("Secure Boot".into(), "on".into()),
            ("TPM".into(), "2.0".into()),
            ("Kernel".into(), "6.17.4-300.fc45.x86_64  ZFS 2.3.4".into()),
            ("Load".into(), "0.82 0.74 0.69  RAM 41 / 64 GiB".into()),
        ],
        disks: vec![
            disk("nvme0n1", "Samsung 980 PRO", 41, "ok", "boot", ""),
            disk("sdb", "WD Red Plus 4TB", 35, "ok", "", ""),
            disk(
                "sdc",
                "WD Red Plus 4TB",
                36,
                "warn",
                "",
                "8 reallocated sectors",
            ),
        ],
    }
}

/// Connection tests where everything but NTP is quick.
#[must_use]
pub fn tests() -> Vec<TestResult> {
    let test = |name: &str, target: &str, result, value: &str| TestResult {
        name: name.into(),
        target: target.into(),
        result,
        value: value.into(),
    };
    vec![
        test("link", "enp3s0", Check::Passed, "2.5 Gb/s full duplex"),
        test("gateway", "10.0.10.1", Check::Passed, "0.4 ms"),
        test("DNS", "ghcr.io", Check::Passed, "12 ms"),
        test("registry", "ghcr.io:443", Check::Passed, "reachable"),
        test("NTP", "chrony", Check::Slow, "offset 310 ms"),
    ]
}

/// A [`System`] whose facts tests set, and which records what it was asked.
#[derive(Debug)]
pub struct Fake {
    /// What [`System::snapshot`] returns.
    pub snapshot: Mutex<Snapshot>,
    /// The PIN [`System::check_pin`] accepts.
    pub pin: String,
    /// The actions run, by audit name.
    pub ran: Mutex<Vec<String>>,
}

impl Default for Fake {
    fn default() -> Self {
        Self {
            snapshot: Mutex::new(snapshot()),
            pin: first_boot().pin,
            ran: Mutex::default(),
        }
    }
}

impl Fake {
    fn record(&self, what: &str) {
        if let Ok(mut ran) = self.ran.lock() {
            ran.push(what.to_owned());
        }
    }
}

#[allow(
    clippy::unused_async_trait_impl,
    reason = "the trait is async; the fake answers at once"
)]
impl System for Fake {
    async fn snapshot(&self) -> Snapshot {
        self.snapshot.lock().map(|s| s.clone()).unwrap_or_default()
    }

    async fn diagnostics(&self) -> Diagnostics {
        diagnostics()
    }

    async fn test_connection(&self, _: &Network) -> Vec<TestResult> {
        tests()
    }

    async fn check_pin(&self, pin: &str, _: &Action) -> Result<(), String> {
        if pin == self.pin {
            Ok(())
        } else {
            Err("wrong PIN; 4 attempts left, then locked for 15 minutes".into())
        }
    }

    async fn run(&self, action: &Action, _: &Network) -> Result<Outcome, String> {
        self.record(action.audit_name());
        Ok(match action {
            Action::NewAdminToken => Outcome::Token("crt_N3W7K-0KENQ-R4ND0-M1234".into()),
            Action::Network(_) | Action::ResetToDhcp => Outcome::Applied {
                checkpoint: "/org/freedesktop/NetworkManager/Checkpoint/1".into(),
                message: "enp3s0 uses the new settings".into(),
            },
            other => Outcome::Done(format!("{} (demo: nothing happened)", other.audit_name())),
        })
    }

    async fn confirm(&self, _: &str) -> Result<(), String> {
        self.record("network.confirm");
        Ok(())
    }

    async fn retry_dhcp(&self, _: &Network) -> Result<(), String> {
        self.record("network.retry");
        Ok(())
    }

    async fn finish_first_boot(&self) -> Result<(), String> {
        if let Ok(mut snapshot) = self.snapshot.lock() {
            snapshot.first_boot = None;
        }
        Ok(())
    }
}
