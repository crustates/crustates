//! The installer's steps and what each key does. Keys return [`Effect`]s
//! for the main loop; results come back through [`App::done`].

use crate::answers::{Answers, MIN_DISK, NetworkAnswers, PoolAnswers};
use crate::hardware::{Facts, Finding, Verdict};

/// A key press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A character.
    Char(char),
    /// Enter.
    Enter,
    /// Escape.
    Esc,
    /// Arrow up.
    Up,
    /// Arrow down.
    Down,
    /// Arrow left.
    Left,
    /// Arrow right.
    Right,
    /// Tab.
    Tab,
    /// Backspace.
    Backspace,
}

/// The keyboard layouts offered: name for `loadkeys`, label.
pub const KEYMAPS: [(&str, &str); 10] = [
    ("us", "English (US)"),
    ("us-intl", "English (US, international)"),
    ("uk", "English (UK)"),
    ("de", "German"),
    ("fr", "French (AZERTY)"),
    ("nl", "Dutch"),
    ("es", "Spanish"),
    ("it", "Italian"),
    ("ch", "Swiss"),
    ("se", "Swedish"),
];

/// What the installer can do, on the welcome screen.
pub const MODES: [(&str, &str); 3] = [
    (
        "Install a new Crustates node",
        "Erase a disk of this machine and install Crustates on it.",
    ),
    (
        "Join an existing Crustates cluster",
        "Comes with Cast: joining nodes is not in this release.",
    ),
    (
        "Rescue",
        "Repair an install: roll back the OS, reset the network, list pools.",
    ),
];

/// Pool layouts: name, disks needed.
pub const LAYOUTS: [(&str, usize); 3] = [("single", 1), ("mirror", 2), ("raidz", 3)];

/// The network form's fields, in tab order.
pub const NET_FIELDS: [&str; 6] = [
    "IPv4 address",
    "Gateway",
    "DNS servers",
    "VLAN tag",
    "Hostname",
    "Interface",
];

/// The installer's steps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Language, keyboard and what to do.
    Welcome,
    /// The hardware check.
    Hardware,
    /// The boot disk and the pool.
    Disks,
    /// The management network.
    Network,
    /// The admin token and recovery PIN.
    Admin,
    /// Everything at a glance; type ERASE.
    Review,
    /// Installing.
    Install,
    /// Installed.
    Done,
    /// Rescue mode.
    Rescue,
}

/// Something for the main loop to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Load a keyboard layout.
    Keymap(String),
    /// Make the token and PIN.
    MakeSecrets,
    /// Test the network.
    TestNetwork,
    /// Install.
    Install(Answers),
    /// Reboot (or into the firmware settings).
    Reboot {
        /// Into the firmware settings.
        firmware: bool,
    },
    /// Look for installs to rescue.
    FindInstalls,
    /// Rescue: make the previous image boot next.
    BootPrevious,
    /// Rescue: back to DHCP.
    ResetNetwork,
    /// Rescue: list the pools.
    ListPools,
}

/// A result of an [`Effect`].
#[derive(Clone, Debug)]
pub enum Done {
    /// The token and PIN.
    Secrets(Result<(String, String), String>),
    /// The network test: one line per check.
    Tested(Vec<String>),
    /// Install progress: the step's index and its state.
    Progress(usize, Phase),
    /// A log line of the install.
    Log(String),
    /// The install ended.
    Installed(Result<(), String>),
    /// Installs found to rescue.
    Installs(Vec<String>),
    /// What a repair did.
    Rescued(Result<String, String>),
}

/// The state of an install step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Not started.
    Next,
    /// Running.
    Now,
    /// Finished.
    Done,
    /// Failed.
    Failed,
}

/// The installer.
#[derive(Debug)]
pub struct App {
    /// The current step.
    pub step: Step,
    /// The installer's version.
    pub version: String,
    /// The hardware.
    pub facts: Facts,
    /// The hardware check.
    pub findings: Vec<Finding>,
    /// Welcome: the focused list, 0 keyboard, 1 mode.
    pub focus: usize,
    /// The keyboard layout.
    pub keymap: usize,
    /// What the user typed to test the layout.
    pub keymap_test: String,
    /// The mode.
    pub mode: usize,
    /// Disks: the cursor and whether it is in the pool list.
    pub cursor: usize,
    /// Disks: whether the cursor is in the pool list.
    pub in_pool: bool,
    /// The boot disk, an index into `facts.disks`.
    pub boot: Option<usize>,
    /// The pool's disks.
    pub pool: Vec<bool>,
    /// An index into [`LAYOUTS`].
    pub layout: usize,
    /// The pool's name.
    pub pool_name: String,
    /// Network: static or DHCP.
    pub static_ip: bool,
    /// The values of [`NET_FIELDS`].
    pub net: [String; 6],
    /// Network: 0 the method, 1 to 6 the fields.
    pub net_focus: usize,
    /// The last network test.
    pub net_test: Vec<String>,
    /// The admin token, once made.
    pub token: String,
    /// The recovery PIN, once made.
    pub pin: String,
    /// The user wrote both down.
    pub wrote_down: bool,
    /// What the user typed on the review screen.
    pub erase: String,
    /// Install steps and their state.
    pub progress: Vec<(String, Phase)>,
    /// The install's log.
    pub log: Vec<String>,
    /// Why the install or a step failed.
    pub error: Option<String>,
    /// Rescue: installs found.
    pub installs: Vec<String>,
    /// Rescue: what the last repair did.
    pub rescued: Option<Result<String, String>>,
    /// The channel to follow.
    pub channel: String,
}

impl App {
    /// The installer on a machine with `facts`; `interface` is the first
    /// wired port with a link.
    #[must_use]
    pub fn new(version: &str, facts: Facts, interface: &str) -> Self {
        let findings = crate::hardware::check(&facts);
        let disks = facts.disks.len();
        let boot = facts
            .disks
            .iter()
            .position(|d| !d.install_media && d.size >= MIN_DISK);
        Self {
            step: Step::Welcome,
            version: version.into(),
            findings,
            focus: 1,
            keymap: 0,
            keymap_test: String::new(),
            mode: 0,
            cursor: boot.unwrap_or(0),
            in_pool: false,
            boot,
            pool: vec![false; disks],
            layout: 1,
            pool_name: "tank".into(),
            static_ip: false,
            net: [
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                "node-1".into(),
                interface.into(),
            ],
            net_focus: 0,
            net_test: Vec::new(),
            token: String::new(),
            pin: String::new(),
            wrote_down: false,
            erase: String::new(),
            progress: Vec::new(),
            log: Vec::new(),
            error: None,
            installs: Vec::new(),
            rescued: None,
            channel: "stable".into(),
            facts,
        }
    }

    /// Whether the hardware check found something that stops the install.
    #[must_use]
    pub fn blocked(&self) -> bool {
        self.findings.iter().any(|f| f.verdict == Verdict::Fail)
    }

    /// The pool's disks, by path.
    #[must_use]
    pub fn pool_disks(&self) -> Vec<String> {
        self.facts
            .disks
            .iter()
            .zip(&self.pool)
            .filter(|(_, on)| **on)
            .map(|(d, _)| d.path.clone())
            .collect()
    }

    /// The choices as answers.
    #[must_use]
    pub fn answers(&self) -> Answers {
        let disks = self.pool_disks();
        let [address, gateway, dns, vlan, hostname, interface] = self.net.clone();
        Answers {
            disk: self
                .boot
                .and_then(|b| self.facts.disks.get(b))
                .map(|d| d.path.clone())
                .unwrap_or_default(),
            erase: self.erase == "ERASE",
            hostname,
            channel: self.channel.clone(),
            policy: "auto".into(),
            after: "reboot".into(),
            network: NetworkAnswers {
                method: if self.static_ip {
                    "static".into()
                } else {
                    "dhcp".into()
                },
                interface,
                address,
                gateway,
                dns: dns
                    .split([',', ' '])
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect(),
                vlan: vlan.trim().parse().ok(),
            },
            pool: (!disks.is_empty()).then(|| PoolAnswers {
                name: self.pool_name.clone(),
                layout: LAYOUTS[self.layout].0.into(),
                disks,
            }),
        }
    }

    fn disk_sizes(&self) -> Vec<(String, u64)> {
        self.facts
            .disks
            .iter()
            .map(|d| (d.path.clone(), d.size))
            .collect()
    }

    /// Why the current choices cannot be installed, if they cannot.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        let mut answers = self.answers();
        answers.erase = true;
        answers.check(&self.disk_sizes()).err()
    }

    /// Handles a key.
    pub fn key(&mut self, key: Key) -> Vec<Effect> {
        match self.step {
            Step::Welcome => self.welcome_key(key),
            Step::Hardware => match key {
                Key::Enter if self.blocked() => vec![Effect::Reboot { firmware: true }],
                Key::Enter => {
                    self.step = Step::Disks;
                    vec![]
                }
                Key::Char('b') | Key::Esc => {
                    self.step = Step::Welcome;
                    vec![]
                }
                _ => vec![],
            },
            Step::Disks => self.disks_key(key),
            Step::Network => self.network_key(key),
            Step::Admin => match key {
                Key::Char(' ') => {
                    self.wrote_down = !self.wrote_down;
                    vec![]
                }
                Key::Enter if self.wrote_down => {
                    self.step = Step::Review;
                    vec![]
                }
                Key::Esc => {
                    self.step = Step::Network;
                    vec![]
                }
                _ => vec![],
            },
            Step::Review => match key {
                Key::Esc => {
                    self.step = Step::Admin;
                    vec![]
                }
                Key::Backspace => {
                    self.erase.pop();
                    vec![]
                }
                Key::Char(c) if self.erase.len() < 5 => {
                    self.erase.push(c);
                    vec![]
                }
                Key::Enter if self.erase == "ERASE" => {
                    self.step = Step::Install;
                    vec![Effect::Install(self.answers())]
                }
                _ => vec![],
            },
            Step::Install => vec![],
            Step::Done => match key {
                Key::Enter => vec![Effect::Reboot { firmware: false }],
                _ => vec![],
            },
            Step::Rescue => match key {
                Key::Esc => {
                    self.step = Step::Welcome;
                    vec![]
                }
                Key::Char('r') => vec![Effect::FindInstalls],
                Key::Char('p') if !self.installs.is_empty() => vec![Effect::BootPrevious],
                Key::Char('n') if !self.installs.is_empty() => vec![Effect::ResetNetwork],
                Key::Char('z') => vec![Effect::ListPools],
                Key::Enter => vec![Effect::Reboot { firmware: false }],
                _ => vec![],
            },
        }
    }

    fn welcome_key(&mut self, key: Key) -> Vec<Effect> {
        match key {
            Key::Tab => self.focus = (self.focus + 1) % 2,
            Key::Up if self.focus == 0 => {
                self.keymap = self.keymap.saturating_sub(1);
                return vec![Effect::Keymap(KEYMAPS[self.keymap].0.into())];
            }
            Key::Down if self.focus == 0 => {
                self.keymap = (self.keymap + 1).min(KEYMAPS.len() - 1);
                return vec![Effect::Keymap(KEYMAPS[self.keymap].0.into())];
            }
            Key::Backspace if self.focus == 0 => {
                self.keymap_test.pop();
            }
            Key::Char(c) if self.focus == 0 && self.keymap_test.len() < 24 => {
                self.keymap_test.push(c)
            }
            Key::Up => self.mode = self.mode.saturating_sub(1),
            Key::Down => self.mode = (self.mode + 1).min(MODES.len() - 1),
            Key::Enter => match self.mode {
                0 => self.step = Step::Hardware,
                2 => {
                    self.step = Step::Rescue;
                    return vec![Effect::FindInstalls];
                }
                _ => {}
            },
            _ => {}
        }
        vec![]
    }

    fn disks_key(&mut self, key: Key) -> Vec<Effect> {
        let count = self.facts.disks.len().max(1);
        match key {
            Key::Up => {
                if self.cursor == 0 && self.in_pool {
                    self.in_pool = false;
                    self.cursor = count - 1;
                } else {
                    self.cursor = self.cursor.saturating_sub(1);
                }
            }
            Key::Down => {
                if self.cursor + 1 == count && !self.in_pool {
                    self.in_pool = true;
                    self.cursor = 0;
                } else {
                    self.cursor = (self.cursor + 1).min(count - 1);
                }
            }
            Key::Char(' ') => {
                let disk = self.facts.disks.get(self.cursor);
                if self.in_pool {
                    if self.boot != Some(self.cursor) && disk.is_some_and(|d| !d.install_media) {
                        self.pool[self.cursor] = !self.pool[self.cursor];
                    }
                } else if disk.is_some_and(|d| !d.install_media && d.size >= MIN_DISK) {
                    self.boot = Some(self.cursor);
                    self.pool[self.cursor] = false;
                }
            }
            Key::Char('m') => self.layout = (self.layout + 1) % LAYOUTS.len(),
            Key::Enter
                if self.boot.is_some()
                    && self
                        .problem()
                        .is_none_or(|p| !p.contains("pool") && !p.contains("disk")) =>
            {
                self.step = Step::Network;
            }
            Key::Char('b') | Key::Esc => self.step = Step::Hardware,
            _ => {}
        }
        vec![]
    }

    fn network_key(&mut self, key: Key) -> Vec<Effect> {
        let editing = self.net_focus > 0;
        match key {
            Key::Tab => self.net_focus = (self.net_focus + 1) % (NET_FIELDS.len() + 1),
            Key::Left | Key::Right | Key::Char(' ') if !editing => self.static_ip = !self.static_ip,
            Key::Backspace if editing => {
                self.net[self.net_focus - 1].pop();
            }
            Key::Char(c) if editing => self.net[self.net_focus - 1].push(c),
            Key::Char('t') => return vec![Effect::TestNetwork],
            Key::Esc => self.step = Step::Disks,
            Key::Enter => match self.problem() {
                Some(problem) => self.error = Some(problem),
                None => {
                    self.error = None;
                    self.step = Step::Admin;
                    if self.token.is_empty() {
                        return vec![Effect::MakeSecrets];
                    }
                }
            },
            _ => {}
        }
        vec![]
    }

    /// Takes the result of an [`Effect`].
    pub fn done(&mut self, done: Done) {
        match done {
            Done::Secrets(Ok((token, pin))) => (self.token, self.pin) = (token, pin),
            Done::Secrets(Err(e)) => self.error = Some(e),
            Done::Tested(lines) => self.net_test = lines,
            Done::Progress(i, phase) => {
                if let Some(step) = self.progress.get_mut(i) {
                    step.1 = phase;
                }
            }
            Done::Log(line) => {
                self.log.push(line);
                let excess = self.log.len().saturating_sub(500);
                self.log.drain(..excess);
            }
            Done::Installed(Ok(())) => self.step = Step::Done,
            Done::Installed(Err(e)) => self.error = Some(e),
            Done::Installs(found) => self.installs = found,
            Done::Rescued(result) => self.rescued = Some(result),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::Disk;

    fn app() -> App {
        let disks = vec![
            Disk {
                path: "/dev/vda".into(),
                size: 40_000_000_000,
                ..Disk::default()
            },
            Disk {
                path: "/dev/vdb".into(),
                size: 8_000_000_000,
                ..Disk::default()
            },
            Disk {
                path: "/dev/vdc".into(),
                size: 8_000_000_000,
                ..Disk::default()
            },
            Disk {
                path: "/dev/sdd".into(),
                size: 32_000_000_000,
                install_media: true,
                ..Disk::default()
            },
        ];
        let facts = Facts {
            arch: "amd64".into(),
            uefi: true,
            cpu_flags: "vmx".into(),
            memory: 8 << 30,
            disks,
            ports: (1, 1),
            ..Facts::default()
        };
        App::new("0.2.0", facts, "enp1s0")
    }

    #[test]
    fn a_whole_install_needs_erase_typed() {
        let mut app = app();
        app.key(Key::Enter);
        assert_eq!(app.step, Step::Hardware);
        app.key(Key::Enter);
        assert_eq!((app.step.clone(), app.boot), (Step::Disks, Some(0)));
        // The pool: vdb and vdc, mirrored.
        app.key(Key::Down);
        app.key(Key::Down);
        app.key(Key::Down);
        app.key(Key::Down);
        assert!(app.in_pool);
        app.key(Key::Char(' '));
        assert_eq!(
            app.pool,
            [false, false, false, false],
            "the boot disk is not a pool disk"
        );
        app.key(Key::Down);
        app.key(Key::Char(' '));
        app.key(Key::Down);
        app.key(Key::Char(' '));
        app.key(Key::Down);
        app.key(Key::Char(' '));
        assert_eq!(
            app.pool_disks(),
            ["/dev/vdb", "/dev/vdc"],
            "not the install stick"
        );
        app.key(Key::Enter);
        assert_eq!(app.step, Step::Network);
        assert_eq!(app.key(Key::Enter), [Effect::MakeSecrets]);
        app.done(Done::Secrets(Ok(("crt_X".into(), "123456789".into()))));
        assert_eq!(
            app.key(Key::Enter),
            [],
            "the PIN must be written down first"
        );
        app.key(Key::Char(' '));
        app.key(Key::Enter);
        assert_eq!(app.step, Step::Review);
        for c in "ERAS".chars() {
            app.key(Key::Char(c));
        }
        assert_eq!(app.key(Key::Enter), []);
        app.key(Key::Char('E'));
        let effects = app.key(Key::Enter);
        let [Effect::Install(answers)] = &effects[..] else {
            panic!("{effects:?}")
        };
        assert_eq!(answers.disk, "/dev/vda");
        assert_eq!(answers.pool.as_ref().unwrap().layout, "mirror");
        assert_eq!(answers.check(&app.disk_sizes()), Ok(()));
    }

    #[test]
    fn a_bad_static_address_stops_at_the_network_step() {
        let mut app = app();
        app.step = Step::Network;
        app.static_ip = true;
        app.net[0] = "10.0.10.2".into();
        assert_eq!(app.key(Key::Enter), []);
        assert!(
            app.error.as_deref().is_some_and(|e| e.contains("prefix")),
            "{:?}",
            app.error
        );
        assert_eq!(app.step, Step::Network);
    }

    #[test]
    fn a_blocked_machine_offers_the_firmware_settings() {
        let mut app = app();
        app.facts.uefi = false;
        app.findings = crate::hardware::check(&app.facts);
        app.step = Step::Hardware;
        assert_eq!(app.key(Key::Enter), [Effect::Reboot { firmware: true }]);
    }
}
