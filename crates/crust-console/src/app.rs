//! The console's state and what each key does. Keys return [`Effect`]s:
//! requests the main loop runs against the [`crate::system::System`],
//! whose results come back through [`App::done`]. Nothing here waits.

use std::time::{Duration, Instant};

use crate::system::{Action, Diagnostics, Method, NetworkChange, Outcome, Snapshot, TestResult};

/// How long a network change waits for confirmation before
/// `NetworkManager` reverts it.
pub const CONFIRM_WITHIN: Duration = Duration::from_secs(60);
/// Digits in the recovery PIN.
pub const PIN_DIGITS: usize = 9;

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
    /// Shift-tab.
    BackTab,
    /// Backspace.
    Backspace,
    /// A function key.
    F(u8),
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
}

/// Something for the main loop to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Check the PIN for an action.
    CheckPin(String, Action),
    /// Run an action whose PIN was checked.
    Run(Action),
    /// Keep a network change.
    Confirm(String),
    /// Ask for a DHCP lease again.
    RetryDhcp,
    /// Read the diagnostics.
    Diagnostics,
    /// Test the connection.
    Test,
    /// Forget the first-boot secrets.
    FinishFirstBoot,
}

/// A result of an [`Effect`].
#[derive(Clone, Debug)]
pub enum Done {
    /// The PIN check of an action.
    Pin(Action, Result<(), String>),
    /// An action ran.
    Ran(Result<Outcome, String>),
    /// A confirmation, a DHCP retry or the end of the first boot.
    Simple(Result<String, String>),
    /// Diagnostics were read.
    Diagnostics(Diagnostics),
    /// The connection was tested.
    Tested(Vec<TestResult>),
}

/// The screen behind any modal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    /// Home, first boot, no network or the post-update boot, chosen by
    /// what the snapshot shows.
    Main,
    /// Show and change the network.
    Network,
    /// Services, this boot, disks, tests.
    Diagnostics,
}

/// A box over the screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Modal {
    /// Asks for the recovery PIN before `action`.
    Pin {
        /// What it is for.
        action: Action,
        /// The digits typed so far.
        digits: String,
        /// Why the last attempt failed.
        error: String,
        /// Waiting for the API.
        checking: bool,
    },
    /// Reboot or shut down: 0 reboot, 1 shut down, 2 cancel.
    Power(usize),
    /// Something happened.
    Message {
        /// The title.
        title: String,
        /// What to say.
        text: String,
        /// Whether it is a failure.
        bad: bool,
    },
    /// A new admin token, shown once.
    Token(String),
    /// A network change that reverts unless confirmed.
    Confirm {
        /// The `NetworkManager` checkpoint.
        checkpoint: String,
        /// What changed.
        message: String,
        /// When it reverts.
        until: Instant,
    },
    /// An action is running.
    Busy(String),
}

/// The network form's fields, in tab order after the method.
pub const FIELDS: [&str; 6] = [
    "IPv4 address",
    "Gateway",
    "DNS servers",
    "Search domain",
    "Hostname",
    "VLAN tag",
];

/// The network screen's form.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Form {
    /// The interface being changed.
    pub interface: usize,
    /// DHCP or static.
    pub method: Method,
    /// The values of [`FIELDS`].
    pub fields: [String; 6],
    /// 0 the method, 1 to 6 the fields, 7 Apply, 8 Test only, 9 Reset to DHCP.
    pub focus: usize,
}

impl Form {
    /// Tab stops.
    const STOPS: usize = 10;

    /// Whether a text field has the focus.
    #[must_use]
    pub fn editing(&self) -> bool {
        (1..=FIELDS.len()).contains(&self.focus)
    }
}

/// The console.
#[derive(Debug)]
pub struct App {
    /// The latest facts.
    pub snapshot: Snapshot,
    /// The screen.
    pub screen: Screen,
    /// A box over it.
    pub modal: Option<Modal>,
    /// The selected menu entry.
    pub selected: usize,
    /// The diagnostics, once read.
    pub diagnostics: Option<Diagnostics>,
    /// The last connection tests.
    pub tests: Vec<TestResult>,
    /// The network form.
    pub form: Form,
    /// Whether the post-update screen was put away.
    pub hide_health: bool,
}

/// What the main screen shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Main {
    /// A new image is being checked, or failed its check.
    UpdateBoot,
    /// The token and PIN of the first boot.
    FirstBoot,
    /// No link or no address.
    NoNetwork,
    /// The recovery console.
    Home,
}

/// A menu entry of the main screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    /// The network screen.
    Network,
    /// Back to DHCP.
    ResetToDhcp,
    /// Roll back the OS.
    RollBack,
    /// A new admin token.
    NewAdminToken,
    /// The diagnostics screen.
    Diagnostics,
    /// Reboot.
    Reboot,
    /// Shut down.
    PowerOff,
    /// Ask for a DHCP lease again.
    RetryDhcp,
}

impl App {
    /// A console showing `snapshot`.
    #[must_use]
    pub fn new(snapshot: Snapshot) -> Self {
        Self {
            snapshot,
            screen: Screen::Main,
            modal: None,
            selected: 0,
            diagnostics: None,
            tests: Vec::new(),
            form: Form::default(),
            hide_health: false,
        }
    }

    /// What the main screen shows now.
    #[must_use]
    pub fn main(&self) -> Main {
        let health = self.snapshot.health.as_ref();
        if !self.hide_health && health.is_some_and(|h| h.state != "ok") {
            Main::UpdateBoot
        } else if self.snapshot.first_boot.is_some() {
            Main::FirstBoot
        } else if !self.snapshot.network.is_up() {
            Main::NoNetwork
        } else {
            Main::Home
        }
    }

    /// The main screen's menu.
    #[must_use]
    pub fn menu(&self) -> &'static [Entry] {
        match self.main() {
            Main::NoNetwork => &[
                Entry::Network,
                Entry::RetryDhcp,
                Entry::Diagnostics,
                Entry::Reboot,
            ],
            _ => &[
                Entry::Network,
                Entry::ResetToDhcp,
                Entry::RollBack,
                Entry::NewAdminToken,
                Entry::Diagnostics,
                Entry::Reboot,
                Entry::PowerOff,
            ],
        }
    }

    /// New facts.
    pub fn update(&mut self, snapshot: Snapshot) {
        self.snapshot = snapshot;
        self.selected = self.selected.min(self.menu().len() - 1);
    }

    /// Time passes: a network change that was not confirmed has reverted.
    pub fn tick(&mut self, now: Instant) {
        if let Some(Modal::Confirm { until, .. }) = &self.modal
            && now >= *until
        {
            self.modal = Some(Modal::Message {
                title: "Network change reverted".into(),
                text: "It was not confirmed within 60 s, so NetworkManager restored the previous settings.".into(),
                bad: true,
            });
        }
    }

    /// Handles a key.
    pub fn key(&mut self, key: Key) -> Vec<Effect> {
        if self.modal.is_some() {
            return self.modal_key(key);
        }
        match self.screen {
            Screen::Main => self.main_key(key),
            Screen::Network => self.network_key(key),
            Screen::Diagnostics => match key {
                Key::Esc => {
                    self.screen = Screen::Main;
                    vec![]
                }
                Key::Char('r') => vec![Effect::Test],
                _ => vec![],
            },
        }
    }

    fn main_key(&mut self, key: Key) -> Vec<Effect> {
        match (self.main(), key) {
            (Main::UpdateBoot, Key::Esc) => {
                self.hide_health = true;
                vec![]
            }
            (Main::FirstBoot, Key::Enter) => vec![Effect::FinishFirstBoot],
            (Main::FirstBoot, Key::F(2)) => self.choose(Entry::Network),
            (Main::FirstBoot, Key::F(3)) => self.choose(Entry::Diagnostics),
            (Main::FirstBoot, Key::F(12)) => self.choose(Entry::Reboot),
            (Main::UpdateBoot | Main::FirstBoot, _) => vec![],
            (_, Key::Up) => {
                self.selected = self.selected.saturating_sub(1);
                vec![]
            }
            (_, Key::Down) => {
                self.selected = (self.selected + 1).min(self.menu().len() - 1);
                vec![]
            }
            (_, Key::Enter) => self.choose(self.menu()[self.selected]),
            (_, Key::Char(c)) => match c.to_digit(10).and_then(|d| usize::try_from(d).ok()) {
                Some(n) if (1..=self.menu().len()).contains(&n) => {
                    self.selected = n - 1;
                    self.choose(self.menu()[n - 1])
                }
                _ => vec![],
            },
            _ => vec![],
        }
    }

    fn choose(&mut self, entry: Entry) -> Vec<Effect> {
        match entry {
            Entry::Network => {
                self.open_network();
                vec![]
            }
            Entry::Diagnostics => {
                self.screen = Screen::Diagnostics;
                vec![Effect::Diagnostics, Effect::Test]
            }
            Entry::RetryDhcp => vec![Effect::RetryDhcp],
            Entry::Reboot => {
                self.modal = Some(Modal::Power(0));
                vec![]
            }
            Entry::PowerOff => {
                self.modal = Some(Modal::Power(1));
                vec![]
            }
            Entry::RollBack
                if self
                    .snapshot
                    .molt
                    .as_ref()
                    .and_then(|m| m.rollback.as_ref())
                    .is_none() =>
            {
                self.modal = Some(Modal::Message {
                    title: "Nothing to roll back to".into(),
                    text: "There is no previous OS image on this machine yet: it comes with the first update.".into(),
                    bad: false,
                });
                vec![]
            }
            Entry::ResetToDhcp => self.ask_pin(Action::ResetToDhcp),
            Entry::RollBack => self.ask_pin(Action::RollBack),
            Entry::NewAdminToken => self.ask_pin(Action::NewAdminToken),
        }
    }

    fn ask_pin(&mut self, action: Action) -> Vec<Effect> {
        self.modal = Some(Modal::Pin {
            action,
            digits: String::new(),
            error: String::new(),
            checking: false,
        });
        vec![]
    }

    fn open_network(&mut self) {
        let network = &self.snapshot.network;
        let interface = network
            .management
            .as_ref()
            .and_then(|m| network.interfaces.iter().position(|i| &i.name == m))
            .unwrap_or(0);
        self.form = Form {
            interface,
            method: network.method,
            fields: [
                network
                    .interfaces
                    .get(interface)
                    .and_then(|i| i.addresses.first().cloned())
                    .unwrap_or_default(),
                network.gateway.clone().unwrap_or_default(),
                network.dns.join(", "),
                network.search.join(", "),
                self.snapshot.hostname.clone(),
                network.vlan.map(|v| v.to_string()).unwrap_or_default(),
            ],
            focus: 0,
        };
        self.tests.clear();
        self.screen = Screen::Network;
    }

    fn change(&self) -> NetworkChange {
        let [address, gateway, dns, search, hostname, vlan] = self.form.fields.clone();
        NetworkChange {
            interface: self
                .snapshot
                .network
                .interfaces
                .get(self.form.interface)
                .map(|i| i.name.clone())
                .unwrap_or_default(),
            method: self.form.method,
            address,
            gateway,
            dns,
            search,
            vlan,
            hostname,
        }
    }

    fn network_key(&mut self, key: Key) -> Vec<Effect> {
        let form = &mut self.form;
        match key {
            Key::Esc => self.screen = Screen::Main,
            Key::Tab => form.focus = (form.focus + 1) % Form::STOPS,
            Key::BackTab => form.focus = (form.focus + Form::STOPS - 1) % Form::STOPS,
            Key::Up | Key::Down if !form.editing() => {
                let count = self.snapshot.network.interfaces.len().max(1);
                form.interface = if key == Key::Up {
                    form.interface.saturating_sub(1)
                } else {
                    (form.interface + 1).min(count - 1)
                };
            }
            Key::Left | Key::Right | Key::Char(' ') if form.focus == 0 => {
                form.method = match form.method {
                    Method::Dhcp => Method::Static,
                    Method::Static => Method::Dhcp,
                };
            }
            Key::Backspace if form.editing() => {
                form.fields[form.focus - 1].pop();
            }
            Key::Char(c) if form.editing() => form.fields[form.focus - 1].push(c),
            Key::Char('t') => return vec![Effect::Test],
            Key::Char('a') => return self.ask_pin(Action::Network(self.change())),
            Key::Enter => match form.focus {
                8 => return vec![Effect::Test],
                9 => return self.ask_pin(Action::ResetToDhcp),
                _ => return self.ask_pin(Action::Network(self.change())),
            },
            _ => {}
        }
        vec![]
    }

    fn modal_key(&mut self, key: Key) -> Vec<Effect> {
        let Some(modal) = self.modal.as_mut() else {
            return vec![];
        };
        match modal {
            Modal::Pin { checking: true, .. } | Modal::Busy(_) => {}
            Modal::Pin {
                action,
                digits,
                error,
                checking,
            } => match key {
                Key::Esc => self.modal = None,
                Key::Backspace => {
                    digits.pop();
                }
                Key::Char(c) if c.is_ascii_digit() && digits.len() < PIN_DIGITS => {
                    digits.push(c);
                    error.clear();
                }
                Key::Enter if digits.len() == PIN_DIGITS => {
                    *checking = true;
                    return vec![Effect::CheckPin(digits.clone(), action.clone())];
                }
                _ => {}
            },
            Modal::Power(choice) => match key {
                Key::Esc => self.modal = None,
                Key::Left => *choice = choice.saturating_sub(1),
                Key::Right | Key::Tab => *choice = (*choice + 1).min(2),
                Key::Enter => {
                    return match *choice {
                        0 => self.ask_pin(Action::Reboot),
                        1 => self.ask_pin(Action::PowerOff),
                        _ => {
                            self.modal = None;
                            vec![]
                        }
                    };
                }
                _ => {}
            },
            Modal::Confirm { checkpoint, .. } => {
                if key == Key::Char('c') {
                    let checkpoint = checkpoint.clone();
                    self.modal = Some(Modal::Busy("Keeping the new settings…".into()));
                    return vec![Effect::Confirm(checkpoint)];
                }
            }
            Modal::Message { .. } | Modal::Token(_) => {
                if matches!(key, Key::Enter | Key::Esc) {
                    self.modal = None;
                }
            }
        }
        vec![]
    }

    /// Takes the result of an [`Effect`].
    pub fn done(&mut self, done: Done) -> Vec<Effect> {
        match done {
            Done::Pin(action, Ok(())) => {
                self.modal = Some(Modal::Busy(busy_text(&action).into()));
                return vec![Effect::Run(action)];
            }
            Done::Pin(action, Err(error)) => {
                self.modal = Some(Modal::Pin {
                    action,
                    digits: String::new(),
                    error,
                    checking: false,
                });
            }
            Done::Ran(Ok(Outcome::Done(text))) => {
                self.modal = Some(Modal::Message {
                    title: "Done".into(),
                    text,
                    bad: false,
                });
            }
            Done::Ran(Ok(Outcome::Token(token))) => self.modal = Some(Modal::Token(token)),
            Done::Ran(Ok(Outcome::Applied {
                checkpoint,
                message,
            })) => {
                self.modal = Some(Modal::Confirm {
                    checkpoint,
                    message,
                    until: Instant::now() + CONFIRM_WITHIN,
                });
            }
            Done::Ran(Err(text)) | Done::Simple(Err(text)) => {
                self.modal = Some(Modal::Message {
                    title: "That did not work".into(),
                    text,
                    bad: true,
                });
            }
            Done::Simple(Ok(text)) => {
                self.modal = (!text.is_empty()).then(|| Modal::Message {
                    title: "Done".into(),
                    text,
                    bad: false,
                });
            }
            Done::Diagnostics(diagnostics) => self.diagnostics = Some(diagnostics),
            Done::Tested(tests) => self.tests = tests,
        }
        vec![]
    }
}

fn busy_text(action: &Action) -> &'static str {
    match action {
        Action::ResetToDhcp => "Switching to DHCP…",
        Action::RollBack => "Rolling back…",
        Action::NewAdminToken => "Making a new admin token…",
        Action::Reboot => "Rebooting…",
        Action::PowerOff => "Shutting down…",
        Action::Network(_) => "Applying the network settings…",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake;

    fn app() -> App {
        App::new(fake::snapshot())
    }

    #[test]
    fn risky_entries_ask_for_the_pin_then_run() {
        let mut app = app();
        assert_eq!(app.main(), Main::Home);
        assert_eq!(app.key(Key::Char('3')), []);
        assert!(matches!(
            app.modal,
            Some(Modal::Pin {
                action: Action::RollBack,
                ..
            })
        ));
        for c in "12345678".chars() {
            app.key(Key::Char(c));
        }
        assert_eq!(app.key(Key::Enter), [], "eight digits are not a PIN");
        app.key(Key::Char('9'));
        let effects = app.key(Key::Enter);
        assert_eq!(
            effects,
            [Effect::CheckPin("123456789".into(), Action::RollBack)]
        );
        assert_eq!(app.key(Key::Esc), [], "no escape while the PIN is checked");

        app.done(Done::Pin(
            Action::RollBack,
            Err("wrong PIN; 4 attempts left".into()),
        ));
        let Some(Modal::Pin { digits, error, .. }) = &app.modal else {
            panic!("{:?}", app.modal)
        };
        assert_eq!(digits, "");
        assert!(error.contains("4 attempts"));

        let effects = app.done(Done::Pin(Action::RollBack, Ok(())));
        assert_eq!(effects, [Effect::Run(Action::RollBack)]);
        app.done(Done::Ran(Ok(Outcome::Done("rolling back".into()))));
        assert!(matches!(app.modal, Some(Modal::Message { bad: false, .. })));
    }

    #[test]
    fn reboot_and_shut_down_choose_first() {
        let mut app = app();
        app.key(Key::Char('7'));
        assert_eq!(app.modal, Some(Modal::Power(1)));
        app.key(Key::Left);
        app.key(Key::Enter);
        assert!(matches!(
            app.modal,
            Some(Modal::Pin {
                action: Action::Reboot,
                ..
            })
        ));
    }

    #[test]
    fn network_changes_revert_unless_confirmed() {
        let mut app = app();
        app.key(Key::Char('1'));
        assert_eq!(app.screen, Screen::Network);
        assert_eq!(app.form.fields[0], "10.0.10.2/24");
        app.key(Key::Right);
        assert_eq!(app.form.method, Method::Static);
        app.key(Key::Tab);
        app.key(Key::Backspace);
        app.key(Key::Char('5'));
        assert_eq!(app.form.fields[0], "10.0.10.2/25");
        app.key(Key::Char('a'));
        assert_eq!(
            app.form.fields[0], "10.0.10.2/25a",
            "typing goes into the field"
        );
        app.form.focus = 7;
        app.key(Key::Enter);
        let Some(Modal::Pin {
            action: Action::Network(change),
            ..
        }) = &app.modal
        else {
            panic!()
        };
        assert_eq!(change.interface, "enp3s0");
        assert_eq!(change.method, Method::Static);

        app.done(Done::Ran(Ok(Outcome::Applied {
            checkpoint: "/cp/1".into(),
            message: "applied".into(),
        })));
        assert_eq!(app.key(Key::Char('c')), [Effect::Confirm("/cp/1".into())]);
        app.modal = Some(Modal::Confirm {
            checkpoint: "/cp/2".into(),
            message: String::new(),
            until: Instant::now(),
        });
        app.tick(Instant::now());
        assert!(matches!(app.modal, Some(Modal::Message { bad: true, .. })));
    }

    #[test]
    fn the_main_screen_follows_the_facts() {
        let mut app = app();
        let mut snapshot = fake::snapshot();
        snapshot.first_boot = Some(fake::first_boot());
        app.update(snapshot.clone());
        assert_eq!(app.main(), Main::FirstBoot);
        assert_eq!(app.key(Key::Enter), [Effect::FinishFirstBoot]);

        snapshot.health = Some(fake::health("running"));
        app.update(snapshot.clone());
        assert_eq!(app.main(), Main::UpdateBoot);
        app.key(Key::Esc);
        assert_eq!(app.main(), Main::FirstBoot, "put away");

        let mut offline = fake::snapshot();
        offline.network = fake::offline();
        app.update(offline);
        assert_eq!(app.main(), Main::NoNetwork);
        assert_eq!(app.key(Key::Char('2')), [Effect::RetryDhcp]);
        assert_eq!(app.key(Key::Char('5')), [], "four entries only");
    }
}
