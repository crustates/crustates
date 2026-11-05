//! What the console shows and does, behind a trait: [`crate::linux::Linux`]
//! on a Molt machine, [`crate::fake::Fake`] in tests and `--demo`.

use std::future::Future;

use crust_tui::widgets::Check;
use crust_types::MoltStatus;
use serde::Deserialize;

/// Everything the main screens show, read every few seconds.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// The local time, `2026-10-30 21:45`.
    pub now: String,
    /// The Crustates version of the booted image.
    pub version: String,
    /// The machine's hostname.
    pub hostname: String,
    /// `amd64` or `arm64`.
    pub arch: String,
    /// The node's name in Crustates.
    pub node: String,
    /// The port of the REST API.
    pub api_port: u16,
    /// Whether the API answers.
    pub api: Service,
    /// Whether the node agent runs.
    pub agent: Service,
    /// The admin token and recovery PIN, until someone confirms they wrote
    /// them down at the first boot.
    pub first_boot: Option<FirstBoot>,
    /// The management network.
    pub network: Network,
    /// The node's Molt status, from the API.
    pub molt: Option<MoltStatus>,
    /// ZFS pools: name and whether it is online.
    pub pools: Vec<(String, bool)>,
    /// The health check of a new image, while it runs or after it failed.
    pub health: Option<Health>,
}

/// A service's state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Service {
    /// It runs.
    Running,
    /// It is starting.
    Starting,
    /// It does not run.
    #[default]
    Stopped,
}

/// The secrets made at the first boot: `/run/crustates/firstboot.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct FirstBoot {
    /// The admin token, `crt_…`.
    pub token: String,
    /// The recovery PIN, nine digits.
    pub pin: String,
}

/// How the management interface gets its address.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Method {
    /// DHCP.
    #[default]
    Dhcp,
    /// A fixed address.
    Static,
}

/// The machine's network.
#[derive(Clone, Debug, Default)]
pub struct Network {
    /// Wired and wireless interfaces.
    pub interfaces: Vec<Interface>,
    /// The interface Crustates is reached on: the one with the default
    /// route, else the first one with a link.
    pub management: Option<String>,
    /// How it gets its address.
    pub method: Method,
    /// The default gateway.
    pub gateway: Option<String>,
    /// DNS servers.
    pub dns: Vec<String>,
    /// DNS search domains.
    pub search: Vec<String>,
    /// The management VLAN tag, if any.
    pub vlan: Option<u16>,
}

impl Network {
    /// The management interface.
    #[must_use]
    pub fn management(&self) -> Option<&Interface> {
        let name = self.management.as_ref()?;
        self.interfaces.iter().find(|i| &i.name == name)
    }

    /// The first IPv4 address of the management interface, without prefix.
    #[must_use]
    pub fn address(&self) -> Option<&str> {
        let address = self.management()?.addresses.first()?;
        Some(address.split('/').next().unwrap_or(address))
    }

    /// Whether the machine can be reached: an address and a link.
    #[must_use]
    pub fn is_up(&self) -> bool {
        self.management()
            .is_some_and(|i| i.link == Link::Up && !i.addresses.is_empty())
    }
}

/// An interface's link.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Link {
    /// Cable in, link up.
    Up,
    /// No link.
    #[default]
    Down,
    /// Not managed by Crustates (wifi).
    Ignored,
}

/// One network interface.
#[derive(Clone, Debug, Default)]
pub struct Interface {
    /// `enp3s0`.
    pub name: String,
    /// Its link.
    pub link: Link,
    /// Link speed in Mb/s.
    pub speed: Option<u32>,
    /// Whether the link is full duplex.
    pub full_duplex: bool,
    /// The MAC address.
    pub mac: String,
    /// IPv4 addresses with prefix, `10.0.10.2/24`.
    pub addresses: Vec<String>,
}

/// The health check of a new image (`/run/crustates/health.json`, written
/// by greenboot's check).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Health {
    /// `running`, `ok` or `failed`.
    pub state: String,
    /// Each check, in order.
    pub checks: Vec<HealthCheck>,
    /// Seconds the check may take.
    #[serde(default)]
    pub deadline: u64,
}

/// One health check.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct HealthCheck {
    /// What is checked.
    pub name: String,
    /// `ok`, `running`, `waiting` or `failed`.
    pub state: String,
    /// What was found.
    #[serde(default)]
    pub detail: String,
}

/// The diagnostics screen's facts; slower to read than the snapshot.
#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    /// Services: name, state, active, and a note such as the uptime.
    pub services: Vec<ServiceRow>,
    /// This boot.
    pub boot: Vec<(String, String)>,
    /// Disks and their SMART health.
    pub disks: Vec<Disk>,
}

/// One service on the diagnostics screen.
#[derive(Clone, Debug, Default)]
pub struct ServiceRow {
    /// The unit, without `.service`.
    pub name: String,
    /// Whether it is fine.
    pub ok: bool,
    /// `running`, `failed`, `inactive`.
    pub state: String,
    /// `up 3d 4h`.
    pub note: String,
}

/// A disk and its SMART health.
#[derive(Clone, Debug, Default)]
pub struct Disk {
    /// `nvme0n1`.
    pub name: String,
    /// The model.
    pub model: String,
    /// °C, if the disk reports it.
    pub temperature: Option<i64>,
    /// `ok`, `warn`, `failing` or `unknown`.
    pub status: String,
    /// `boot` for the boot disk.
    pub role: String,
    /// Why the status is not `ok`.
    pub note: String,
}

/// One connection test.
#[derive(Clone, Debug)]
pub struct TestResult {
    /// What was tested, `gateway`.
    pub name: String,
    /// Against what, `10.0.10.1`.
    pub target: String,
    /// How it went.
    pub result: Check,
    /// `0.4 ms`, `reachable`, `timed out`.
    pub value: String,
}

/// A network setting to apply.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkChange {
    /// The interface.
    pub interface: String,
    /// DHCP or static.
    pub method: Method,
    /// `10.0.10.2/24`, for static.
    pub address: String,
    /// For static.
    pub gateway: String,
    /// Comma-separated, for static.
    pub dns: String,
    /// Search domain.
    pub search: String,
    /// The VLAN tag; empty for untagged.
    pub vlan: String,
    /// The hostname; unchanged when it is the current one.
    pub hostname: String,
}

/// A risky action of the recovery menu; each needs the recovery PIN.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// The management interface back to DHCP.
    ResetToDhcp,
    /// Boot the previous Molt image.
    RollBack,
    /// Make a new admin token, replacing the old one.
    NewAdminToken,
    /// Reboot the machine.
    Reboot,
    /// Shut it down.
    PowerOff,
    /// Change the network.
    Network(NetworkChange),
}

impl Action {
    /// The name in the audit log.
    #[must_use]
    pub fn audit_name(&self) -> &'static str {
        match self {
            Action::ResetToDhcp => "network.dhcp",
            Action::RollBack => "molt.rollback",
            Action::NewAdminToken => "admin-token.new",
            Action::Reboot => "power.reboot",
            Action::PowerOff => "power.off",
            Action::Network(_) => "network.apply",
        }
    }
}

/// What an action did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Done; the message says what happened.
    Done(String),
    /// A new admin token, shown once.
    Token(String),
    /// A network change that reverts unless it is confirmed with the
    /// checkpoint within a minute.
    Applied {
        /// The `NetworkManager` checkpoint.
        checkpoint: String,
        /// What changed.
        message: String,
    },
}

/// One line of the system log.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogLine {
    /// `Oct 30 21:44:58`.
    pub time: String,
    /// The unit or program.
    pub unit: String,
    /// syslog priority: 0 emergency to 7 debug.
    pub priority: u8,
    /// The message.
    pub message: String,
}

/// The machine, as the console sees and changes it.
pub trait System: Send + Sync + 'static {
    /// Reads the main screens' facts.
    fn snapshot(&self) -> impl Future<Output = Snapshot> + Send;
    /// Reads the diagnostics screen's facts.
    fn diagnostics(&self) -> impl Future<Output = Diagnostics> + Send;
    /// Tests the connection: link, gateway, DNS, the registry, NTP.
    fn test_connection(&self, network: &Network) -> impl Future<Output = Vec<TestResult>> + Send;
    /// Checks the recovery PIN for `action` with the API.
    fn check_pin(
        &self,
        pin: &str,
        action: &Action,
    ) -> impl Future<Output = Result<(), String>> + Send;
    /// Runs `action`; its PIN was checked.
    fn run(
        &self,
        action: &Action,
        network: &Network,
    ) -> impl Future<Output = Result<Outcome, String>> + Send;
    /// Keeps a network change.
    fn confirm(&self, checkpoint: &str) -> impl Future<Output = Result<(), String>> + Send;
    /// Asks for a DHCP lease again.
    fn retry_dhcp(&self, network: &Network) -> impl Future<Output = Result<(), String>> + Send;
    /// Forgets the first-boot secrets: someone wrote them down.
    fn finish_first_boot(&self) -> impl Future<Output = Result<(), String>> + Send;
}
