//! Drawing: one function per screen of the design kit, from the [`App`]'s
//! state alone, so tests can snapshot every screen.

#![allow(clippy::too_many_lines, reason = "a screen is drawn line by line")]

use std::fmt::Write as _;

use std::time::Instant;

use crust_tui::mark;
use crust_tui::palette::Role;
use crust_tui::widgets::{
    self, Check, Item, State, background, button, centered, check, field, key_bar, modal, pane,
    progress, status, top_bar,
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};

use crate::app::{App, Entry, FIELDS, Main, Modal, Screen};
use crate::system::{Action, Link, Method, Service, Snapshot};

/// Widest and tallest layout; larger screens show it centred.
pub const MAX: (u16, u16) = (240, 67);
/// Narrower than this, the left pane drops.
const TWO_PANES: u16 = 120;
/// Width of status labels.
const LABEL: usize = 13;

/// Draws the whole console into `buf`.
pub fn draw(app: &App, buf: &mut Buffer, now: Instant) {
    let full = buf.area;
    background(buf, full);
    let area = centered(full, MAX.0, MAX.1);
    let snap = &app.snapshot;
    let version = format!("Crustates {}", snap.version);
    let (title, facts): (&str, [&str; 3]) = match (app.screen, app.main()) {
        (Screen::Network, _) => (
            "Configure network",
            [&version, "tty1", "changes need the recovery PIN"],
        ),
        (Screen::Diagnostics, _) => ("Diagnostics", [&version, "tty1", "local access only"]),
        (_, Main::UpdateBoot) => ("Molt", [&version, "tty1", "boot"]),
        (_, Main::FirstBoot) => ("Crustates", [&version, "tty1", "first boot"]),
        _ => (
            "Crustates recovery console",
            [&version, "tty1", "local access only"],
        ),
    };
    top_bar(buf, row(area, 0), title, &facts, &snap.now);
    key_bar(buf, row(area, area.height - 1), &keys(app));
    let body = body(area);
    match app.screen {
        Screen::Network => network(app, buf, body),
        Screen::Diagnostics => diagnostics(app, buf, body),
        Screen::Main => match app.main() {
            Main::UpdateBoot => update_boot(snap, buf, body),
            Main::FirstBoot => split(
                buf,
                body,
                |b, a| qr_pane(snap, b, a),
                |b, a| first_boot(snap, b, a),
            ),
            Main::NoNetwork => split(
                buf,
                body,
                |b, a| crab(snap, b, a),
                |b, a| no_network(app, b, a),
            ),
            Main::Home => split(buf, body, |b, a| crab(snap, b, a), |b, a| home(app, b, a)),
        },
    }
    if let Some(m) = &app.modal {
        draw_modal(app, m, buf, area, now);
    }
}

fn row(area: Rect, y: u16) -> Rect {
    Rect::new(area.x, area.y + y, area.width, 1)
}

/// The space between the bars, with a blank row on each side when tall
/// enough.
fn body(area: Rect) -> Rect {
    let gap = u16::from(area.height >= 30);
    Rect::new(
        area.x + 1,
        area.y + 1 + gap,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2 + 2 * gap),
    )
}

/// Two panes side by side, or only the right one on a narrow screen.
fn split(
    buf: &mut Buffer,
    area: Rect,
    left: impl FnOnce(&mut Buffer, Rect),
    right: impl FnOnce(&mut Buffer, Rect),
) {
    if area.width < TWO_PANES - 2 {
        right(buf, area);
        return;
    }
    let left_width = area.width * 2 / 5;
    left(buf, Rect::new(area.x, area.y, left_width, area.height));
    right(
        buf,
        Rect::new(
            area.x + left_width + 1,
            area.y,
            area.width - left_width - 1,
            area.height,
        ),
    );
}

/// Draws a pane and its lines, wrapped, one blank row and column inside.
fn panel(
    buf: &mut Buffer,
    area: Rect,
    title: &str,
    accent: Option<Role>,
    lines: Vec<Line<'static>>,
) {
    let block = pane(title, accent);
    let inner = block.inner(area);
    block.render(area, buf);
    let text = Rect::new(
        inner.x + 1,
        inner.y + 1,
        inner.width.saturating_sub(2),
        inner.height.saturating_sub(1),
    );
    Paragraph::new(fit(lines, text.width, text.height))
        .wrap(Wrap { trim: false })
        .render(text, buf);
}

/// Drops blank lines, last first, until `lines` fit `width`×`height`: on
/// small screens the content moves together instead of being cut off.
fn fit(mut lines: Vec<Line<'static>>, width: u16, height: u16) -> Vec<Line<'static>> {
    let rows = |lines: &[Line<'_>]| -> usize {
        lines
            .iter()
            .map(|l| l.width().div_ceil(usize::from(width.max(1))).max(1))
            .sum()
    };
    while rows(&lines) > usize::from(height) {
        let Some(blank) = lines.iter().rposition(|l| l.width() == 0) else {
            break;
        };
        lines.remove(blank);
    }
    lines
}

fn blank() -> Line<'static> {
    Line::default()
}

fn plain(text: impl Into<String>, role: Role) -> Line<'static> {
    Line::from(Span::styled(text.into(), role.fg()))
}

fn rule(width: u16) -> Line<'static> {
    plain(
        "─".repeat(usize::from(width.saturating_sub(4)).min(60)),
        Role::Dim,
    )
}

fn api_url(snap: &Snapshot) -> Option<String> {
    snap.network
        .address()
        .map(|a| format!("http://{a}:{}", snap.api_port))
}

fn network_line(snap: &Snapshot) -> Line<'static> {
    let n = &snap.network;
    let Some(i) = n.management() else {
        return status(State::None, "Network", LABEL, "none", "");
    };
    let mut text = format!(
        "{}  {}",
        i.name,
        i.addresses.first().map_or("no address", String::as_str)
    );
    if let Some(gw) = &n.gateway {
        let _ = write!(text, "  gw {gw}");
    }
    if let Some(dns) = n.dns.first() {
        let _ = write!(text, "  dns {dns}");
    }
    status(State::None, "Network", LABEL, &text, "")
}

fn service_line(name: &str, service: Service) -> Line<'static> {
    match service {
        Service::Running => status(State::Ok, name, LABEL, "running", ""),
        Service::Starting => status(State::Busy, name, LABEL, "starting", ""),
        Service::Stopped => status(
            State::Bad,
            name,
            LABEL,
            "not running",
            "see the system log, alt+f2",
        ),
    }
}

fn pools_line(snap: &Snapshot) -> Line<'static> {
    if snap.pools.is_empty() {
        return status(State::None, "Pools", LABEL, "none yet", "");
    }
    let online = snap.pools.iter().filter(|(_, ok)| *ok).count();
    let names: Vec<&str> = snap.pools.iter().map(|(n, _)| n.as_str()).collect();
    let state = if online == snap.pools.len() {
        State::Ok
    } else {
        State::Bad
    };
    status(
        state,
        "Pools",
        LABEL,
        &format!("{online} of {} online", snap.pools.len()),
        &format!("({})", names.join(", ")),
    )
}

fn update_line(snap: &Snapshot) -> Line<'static> {
    let Some(molt) = &snap.molt else {
        return status(State::Waiting, "Update", LABEL, "not known yet", "");
    };
    if let Some(staged) = &molt.staged {
        return status(
            State::Busy,
            "Update",
            LABEL,
            &format!("{} staged", staged.version),
            "boots at the next reboot",
        );
    }
    if let Some(available) = &molt.available {
        return status(
            State::Warn,
            "Update",
            LABEL,
            &format!("{} available", available.version),
            &format!("on {}", molt.channel),
        );
    }
    let booted = molt.booted.as_ref().map_or("", |b| b.version.as_str());
    status(
        State::Ok,
        "Update",
        LABEL,
        "up to date",
        &format!("({booted} on {})", molt.channel),
    )
}

fn crab(snap: &Snapshot, buf: &mut Buffer, area: Rect) {
    let mut lines = mark::lines(24.min(area.width.saturating_sub(4)));
    lines.push(blank());
    lines.push(plain("crustates", Role::Text).style(Role::bold(Role::Text)));
    lines.push(blank());
    lines.push(plain("Your own cloud, on your own hardware.", Role::Dim));
    lines.push(blank());
    lines.push(plain(
        format!("Crustates {} · {} · {}", snap.version, snap.node, snap.arch),
        Role::Dim,
    ));
    centered_pane(buf, area, &snap.hostname, lines);
}

/// A dim pane with `lines` centred in it.
fn centered_pane(buf: &mut Buffer, area: Rect, title: &str, lines: Vec<Line<'static>>) {
    let block = pane(title, None);
    let inner = block.inner(area);
    block.render(area, buf);
    let height = u16::try_from(lines.len())
        .unwrap_or(inner.height)
        .min(inner.height);
    let at = Rect::new(
        inner.x,
        inner.y + (inner.height - height) / 2,
        inner.width,
        height,
    );
    Paragraph::new(lines).centered().render(at, buf);
}

fn home(app: &App, buf: &mut Buffer, area: Rect) {
    let snap = &app.snapshot;
    let mut lines = vec![
        plain("Open the API reference in a browser:", Role::Text),
        blank(),
    ];
    if let Some(url) = api_url(snap) {
        lines.push(plain(format!("  {url}/docs"), Role::Info));
        lines.push(Line::from(vec![
            Span::styled("  or log in with the CLI: ", Role::Dim.fg()),
            Span::styled(format!("crust login --api {url}"), Role::Text.fg()),
        ]));
    }
    lines.extend([
        blank(),
        service_line("API", snap.api),
        service_line("Agent", snap.agent),
        pools_line(snap),
        update_line(snap),
        network_line(snap),
        blank(),
        rule(area.width),
        plain("Recovery menu", Role::Text).style(Role::bold(Role::Text)),
        blank(),
    ]);
    lines.extend(menu(app, area.width));
    lines.extend([
        blank(),
        plain(
            "Actions 2, 3, 4, 6 and 7 ask for the recovery PIN shown at the first boot.",
            Role::Dim,
        ),
    ]);
    panel(buf, area, "Status", Some(Role::Focus), lines);
}

fn menu(app: &App, width: u16) -> Vec<Line<'static>> {
    let rollback = app.snapshot.molt.as_ref().and_then(|m| {
        Some((
            m.booted.as_ref()?.version.clone(),
            m.rollback.as_ref()?.version.clone(),
        ))
    });
    let items: Vec<Item> = app
        .menu()
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let (text, note) = match entry {
                Entry::Network if app.main() == Main::NoNetwork => {
                    ("Configure network", String::new())
                }
                Entry::Network => ("Show network settings", String::new()),
                Entry::ResetToDhcp => ("Reset network to DHCP", String::new()),
                Entry::RollBack => (
                    "Roll back to previous OS",
                    rollback
                        .as_ref()
                        .map_or(String::new(), |(_, to)| format!("(Molt: {to})")),
                ),
                Entry::NewAdminToken => ("Make a new admin token", String::new()),
                Entry::Diagnostics => ("Show diagnostics", String::new()),
                Entry::Reboot => ("Reboot", String::new()),
                Entry::PowerOff => ("Shut down", String::new()),
                Entry::RetryDhcp => ("Retry DHCP now", String::new()),
            };
            Item {
                key: (i + 1).to_string(),
                text: text.into(),
                note,
            }
        })
        .collect();
    widgets::menu(&items, app.selected, width.saturating_sub(4).min(60))
}

fn no_network(app: &App, buf: &mut Buffer, area: Rect) {
    let snap = &app.snapshot;
    let n = &snap.network;
    let mut lines = vec![
        Line::from(vec![
            Span::styled("■", Role::Bad.fg()),
            Span::styled("  No network connection", Role::Bad.fg()),
        ]),
        blank(),
    ];
    for i in n.interfaces.iter().filter(|i| i.link != Link::Ignored) {
        let managed = n.management.as_ref() == Some(&i.name) || n.management.is_none();
        lines.push(match (i.link, managed) {
            (Link::Up, _) => status(State::Ok, &i.name, LABEL, "link up", "no address yet"),
            (_, true) => status(State::Bad, &i.name, LABEL, "no link: cable unplugged?", ""),
            _ => status(State::None, &i.name, LABEL, "no link", ""),
        });
    }
    lines.extend([
        blank(),
        plain(
            "The API cannot be reached until there is an address.",
            Role::Dim,
        ),
        plain("NetworkManager keeps trying on its own.", Role::Dim),
        blank(),
        pools_line(snap),
        blank(),
        rule(area.width),
        blank(),
    ]);
    lines.extend(menu(app, area.width));
    panel(buf, area, "No network connection", Some(Role::Bad), lines);
}

fn qr_pane(snap: &Snapshot, buf: &mut Buffer, area: Rect) {
    let mut lines = Vec::new();
    match api_url(snap) {
        Some(url) => {
            let docs = format!("{url}/docs");
            if let Some(qr) = widgets::qr(&docs) {
                lines.extend(qr);
            }
            lines.extend([
                blank(),
                plain(docs, Role::Info),
                plain("Opens the API reference", Role::Dim),
            ]);
        }
        None => lines.push(plain("No network yet: press f2 to set it up.", Role::Bad)),
    }
    centered_pane(buf, area, "Scan with your phone", lines);
}

fn code(text: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("     "),
        Span::styled(
            format!(" {text} "),
            Style::new().fg(Role::Bar.color()).bg(Role::Focus.color()),
        ),
    ])
}

fn first_boot(snap: &Snapshot, buf: &mut Buffer, area: Rect) {
    let Some(first) = &snap.first_boot else {
        return;
    };
    let url = api_url(snap).unwrap_or_else(|| format!("http://<address>:{}", snap.api_port));
    let pin = format!(
        "{} {} {}",
        first.pin.get(..3).unwrap_or_default(),
        first.pin.get(3..6).unwrap_or_default(),
        first.pin.get(6..).unwrap_or_default()
    );
    let step = |n: &str, text: &str| {
        Line::from(vec![
            Span::styled(format!("{n}  "), Role::bold(Role::Text)),
            Span::styled(text.to_owned(), Role::Text.fg()),
        ])
    };
    let mut lines = vec![
        step("1", "From a computer on the same network, log in:"),
        blank(),
        plain(format!("     crust login --api {url}"), Role::Info),
        blank(),
    ];
    // A machine set up before the console keeps its token; only the PIN is new.
    if first.token.is_empty() {
        lines.extend([step("2", "Your admin token stays as it was."), blank()]);
    } else {
        lines.extend([
            step("2", "Enter this admin token when it asks:"),
            blank(),
            code(&first.token),
            blank(),
        ]);
    }
    lines.extend([
        step("3", "Keep the recovery PIN for this screen:"),
        blank(),
        code(&pin),
        blank(),
        plain(
            "   Rollback, network changes and reboots here ask for it.",
            Role::Dim,
        ),
        blank(),
        Line::from(vec![
            Span::styled("!  ", Role::Focus.fg()),
            Span::styled(
                "Write them down. They are not shown again after you press enter or the machine reboots.",
                Role::Focus.fg(),
            ),
        ]),
        blank(),
        rule(area.width),
        service_line("API", snap.api),
        service_line("Agent", snap.agent),
        network_line(snap),
    ]);
    panel(buf, area, "Finish setup", Some(Role::Focus), lines);
}

fn update_boot(snap: &Snapshot, buf: &mut Buffer, area: Rect) {
    let Some(health) = &snap.health else {
        return;
    };
    let failed = health.state == "failed";
    let booted = snap
        .molt
        .as_ref()
        .and_then(|m| m.booted.as_ref())
        .map_or(snap.version.clone(), |b| b.version.clone());
    let previous = snap
        .molt
        .as_ref()
        .and_then(|m| m.rollback.as_ref())
        .map_or("the previous image".to_owned(), |r| r.version.clone());
    let done = health.checks.iter().filter(|c| c.state == "ok").count();
    let mut left = vec![
        plain(format!("The update installed {booted}."), Role::Dim),
        plain(format!("{previous} stays for rollback."), Role::Dim),
        blank(),
        plain("Health checks", Role::Text),
    ];
    for c in &health.checks {
        let result = match c.state.as_str() {
            "ok" => Check::Passed,
            "failed" => Check::Failed,
            "running" => Check::Running,
            _ => Check::Pending,
        };
        let detail = if c.detail.is_empty() {
            c.state.clone()
        } else {
            c.detail.clone()
        };
        left.push(check(result, &c.name, 14, &detail, ""));
    }
    left.extend([
        blank(),
        progress(done, health.checks.len(), 24),
        blank(),
        plain(format!("If a check fails within {} minutes, Molt rolls back to {previous} by itself. Nothing for you to do.", health.deadline.div_ceil(60).max(1)), Role::Dim),
    ]);
    let after = [
        plain(
            "After the rollback:",
            if failed { Role::Text } else { Role::Dim },
        ),
        plain("  · your pools and their data are untouched", Role::Dim),
        plain(
            format!("  · {booted} is marked bad and not installed again"),
            Role::Dim,
        ),
        plain("  · the system log keeps what happened (alt+f2)", Role::Dim),
    ];
    let right = if failed {
        let mut lines = vec![
            Line::from(Span::styled("✗  Health checks failed", Role::Bad.fg())),
            blank(),
            Line::from(vec![
                Span::styled("   Rolling back to ", Role::Text.fg()),
                Span::styled(previous.clone(), Role::bold(Role::Text)),
                Span::styled(" automatically.", Role::Text.fg()),
            ]),
            blank(),
            plain("Reason", Role::Text),
        ];
        for c in health.checks.iter().filter(|c| c.state == "failed") {
            lines.push(check(Check::Failed, &c.name, 14, &c.detail, ""));
        }
        lines.extend([
            blank(),
            plain(
                format!("The machine reboots into {previous} now."),
                Role::Dim,
            ),
            blank(),
        ]);
        lines.extend(after);
        lines
    } else {
        let mut lines = vec![
            plain(
                format!("If a check fails, Molt rolls back to {previous} on its own."),
                Role::Dim,
            ),
            blank(),
        ];
        lines.extend(after);
        lines
    };
    let accent = if failed { Some(Role::Bad) } else { None };
    let title = format!("Booting Crustates {booted}");
    split(
        buf,
        area,
        |b, a| panel(b, a, &title, Some(Role::Focus), left),
        |b, a| panel(b, a, "If checks fail", accent, right),
    );
}

fn network(app: &App, buf: &mut Buffer, area: Rect) {
    let snap = &app.snapshot;
    let n = &snap.network;
    let form = &app.form;
    // Narrow screens show only the form, like the crab pane drops elsewhere.
    let wide = area.width >= TWO_PANES - 2;
    let left_width = if wide { area.width * 2 / 5 } else { 0 };
    let left = Rect::new(area.x, area.y, left_width, area.height);
    let right = if wide {
        Rect::new(
            area.x + left_width + 1,
            area.y,
            area.width - left_width - 1,
            area.height,
        )
    } else {
        area
    };

    let selected = Style::new().fg(Role::Bar.color()).bg(Role::Focus.color());
    let mut lines = vec![plain(
        format!("{:<8} {:<7} {:<8} MAC", "NAME", "LINK", "SPEED"),
        Role::Dim,
    )];
    for (i, iface) in n.interfaces.iter().enumerate() {
        let link = match iface.link {
            Link::Up => "up",
            Link::Down => "down",
            Link::Ignored => "ignored",
        };
        let speed = iface.speed.map_or("-".to_owned(), speed);
        let mac = if iface.link == Link::Ignored {
            "wifi, unused".into()
        } else {
            iface.mac.clone()
        };
        let text = format!("{:<8} {:<7} {:<8} {mac}", iface.name, link, speed);
        let pad = usize::from(left_width.saturating_sub(4)).saturating_sub(text.chars().count());
        let text = format!("{text}{}", " ".repeat(pad));
        lines.push(if i == form.interface {
            Line::from(Span::styled(text, selected))
        } else if iface.link == Link::Up {
            plain(text, Role::Text)
        } else {
            plain(text, Role::Dim)
        });
    }
    lines.extend([
        blank(),
        plain(
            format!(
                "The API is reached on {}.",
                n.management.as_deref().unwrap_or("no interface yet")
            ),
            Role::Dim,
        ),
        plain(
            "Bridges, bonds and VLANs for workloads come with Current.",
            Role::Dim,
        ),
        blank(),
        rule(left.width),
        plain("Test connection", Role::Text).style(Role::bold(Role::Text)),
    ]);
    if app.tests.is_empty() {
        lines.push(plain(
            "Press t to test the link, gateway, DNS, registry and NTP.",
            Role::Dim,
        ));
    }
    for t in &app.tests {
        lines.push(check(t.result, &t.name, 9, &t.target, &t.value));
    }
    if wide {
        panel(buf, left, "Interfaces", None, lines);
    }

    let name = n
        .interfaces
        .get(form.interface)
        .map_or("interface", |i| i.name.as_str());
    let choice = |label: &str, on: bool, focused: bool| {
        let text = format!("({}) {label}", if on { "o" } else { " " });
        if focused && on {
            Span::styled(text, selected)
        } else {
            Span::styled(text, Role::Text.fg())
        }
    };
    let static_on = form.method == Method::Static;
    let row = |i: usize, label: &str, enabled: bool| {
        let value = &form.fields[i];
        let mut field = field(value, 18, form.focus == i + 1);
        if !enabled && form.focus != i + 1 {
            field = field.style(Role::Dim.fg());
        }
        Line::from(vec![
            Span::styled(format!("  {label:<15}"), Role::Text.fg()),
            field,
        ])
    };
    let lines = vec![
        Line::from(vec![
            Span::styled("Address    ", Role::bold(Role::Text)),
            choice("DHCP", !static_on, form.focus == 0),
            Span::raw("    "),
            choice("Static", static_on, form.focus == 0),
        ]),
        blank(),
        row(0, FIELDS[0], static_on),
        row(1, FIELDS[1], static_on),
        Line::from(vec![
            Span::styled(format!("  {:<15}", "IPv6"), Role::Text.fg()),
            Span::styled("automatic (SLAAC)", Role::Dim.fg()),
        ]),
        blank(),
        plain("DNS", Role::Text).style(Role::bold(Role::Text)),
        row(2, "Servers", static_on),
        row(3, FIELDS[3], true),
        blank(),
        plain("Host", Role::Text).style(Role::bold(Role::Text)),
        row(4, FIELDS[4], true),
        row(5, FIELDS[5], true),
        blank(),
        rule(right.width),
        Line::from(vec![
            Span::styled("!  ", Role::Focus.fg()),
            Span::styled(
                "Applying may disconnect the CLI and other API clients.",
                Role::Focus.fg(),
            ),
        ]),
        Line::from(vec![
            Span::styled("   It reverts in ", Role::Text.fg()),
            Span::styled("60 s", Role::bold(Role::Text)),
            Span::styled(" unless you press ", Role::Text.fg()),
            Span::styled(
                " c ",
                Style::new().fg(Role::Bar.color()).bg(Role::Text.color()),
            ),
            Span::styled(" here to keep it.", Role::Text.fg()),
        ]),
        blank(),
        Line::from(vec![
            Span::raw("   "),
            button("Apply", form.focus == 7),
            Span::raw("   "),
            button("Test only", form.focus == 8),
            Span::raw("   "),
            button("Reset to DHCP", form.focus == 9),
        ]),
    ];
    panel(buf, right, name, Some(Role::Focus), lines);
}

fn speed(mbps: u32) -> String {
    if mbps >= 1000 {
        let tenths = mbps / 100;
        if tenths.is_multiple_of(10) {
            format!("{} Gb/s", tenths / 10)
        } else {
            format!("{}.{} Gb/s", tenths / 10, tenths % 10)
        }
    } else {
        format!("{mbps} Mb/s")
    }
}

fn diagnostics(app: &App, buf: &mut Buffer, area: Rect) {
    let half = area.width / 2;
    let top_height = 11.min(area.height / 2);
    let top_left = Rect::new(area.x, area.y, half, top_height);
    let top_right = Rect::new(area.x + half + 1, area.y, area.width - half - 1, top_height);
    let bottom_left = Rect::new(area.x, area.y + top_height, half, area.height - top_height);
    let bottom_right = Rect::new(
        area.x + half + 1,
        area.y + top_height,
        area.width - half - 1,
        area.height - top_height,
    );
    let Some(d) = &app.diagnostics else {
        panel(
            buf,
            area,
            "Diagnostics",
            Some(Role::Focus),
            vec![plain("Reading services, disks and this boot…", Role::Dim)],
        );
        return;
    };
    let services = d
        .services
        .iter()
        .map(|s| {
            status(
                if s.ok { State::Ok } else { State::Bad },
                &s.name,
                15,
                &format!("{:<9}", s.state),
                &s.note,
            )
        })
        .collect();
    panel(buf, top_left, "Services", None, services);
    let boot = d
        .boot
        .iter()
        .map(|(k, v)| {
            Line::from(vec![
                Span::styled(format!("{k:<12}"), Role::Text.fg()),
                Span::styled(v.clone(), Role::Dim.fg()),
            ])
        })
        .collect();
    panel(buf, top_right, "This boot", None, boot);
    let mut disks = vec![plain(
        format!(
            "{:<9} {:<18} {:<5} {:<7}",
            "DISK", "MODEL", "TEMP", "STATUS"
        ),
        Role::Dim,
    )];
    for disk in &d.disks {
        let temp = disk
            .temperature
            .map_or("-".to_owned(), |t| format!("{t}°C"));
        let color = match disk.status.as_str() {
            "ok" => Role::Ok,
            "warn" => Role::Focus,
            "failing" => Role::Bad,
            _ => Role::Dim,
        };
        disks.push(Line::from(vec![
            Span::styled(
                format!(
                    "{:<9} {:<18} {temp:<5} ",
                    disk.name,
                    disk.model.chars().take(18).collect::<String>()
                ),
                Role::Text.fg(),
            ),
            Span::styled(format!("{:<7} ", disk.status), color.fg()),
            Span::styled(disk.role.clone(), Role::Dim.fg()),
        ]));
        if !disk.note.is_empty() {
            disks.push(plain(format!("{:<29}{}", "", disk.note), color));
        }
    }
    if d.disks.is_empty() {
        disks.push(plain("No disk reports SMART data.", Role::Dim));
    }
    panel(buf, bottom_left, "Disks (SMART)", None, disks);
    let mut tests: Vec<Line<'static>> = app
        .tests
        .iter()
        .map(|t| {
            check(
                t.result,
                &format!("{} {}", t.name, t.target),
                24,
                &t.value,
                "",
            )
        })
        .collect();
    if tests.is_empty() {
        tests.push(check(Check::Running, "testing the connection", 24, "", ""));
    }
    tests.extend([
        blank(),
        plain(
            "Support bundles to a USB stick come in a later release.",
            Role::Dim,
        ),
    ]);
    panel(
        buf,
        bottom_right,
        "Tests and support",
        Some(Role::Focus),
        tests,
    );
}

fn keys(app: &App) -> Vec<(&'static str, &'static str)> {
    match &app.modal {
        Some(Modal::Pin { .. }) => {
            return vec![
                ("0-9", "enter PIN"),
                ("enter", "confirm"),
                ("esc", "cancel"),
            ];
        }
        Some(Modal::Power(_)) => {
            return vec![
                ("left/right", "choose"),
                ("enter", "confirm with PIN"),
                ("esc", "cancel"),
            ];
        }
        Some(Modal::Confirm { .. }) => return vec![("c", "keep the new settings")],
        Some(Modal::Message { .. } | Modal::Token(_)) => return vec![("enter", "close")],
        Some(Modal::Busy(_)) => return vec![],
        None => {}
    }
    match (app.screen, app.main()) {
        (Screen::Network, _) => vec![
            ("tab", "next field"),
            ("up/down", "interface"),
            ("t", "test"),
            ("a", "apply"),
            ("esc", "back"),
        ],
        (Screen::Diagnostics, _) => vec![
            ("r", "run tests again"),
            ("alt+f2", "system log"),
            ("esc", "back"),
        ],
        (_, Main::UpdateBoot) => vec![("alt+f2", "system log"), ("esc", "hide")],
        (_, Main::FirstBoot) => vec![
            ("enter", "I wrote them down"),
            ("f2", "network"),
            ("f3", "diagnostics"),
            ("alt+f2", "system log"),
            ("f12", "reboot or shut down"),
        ],
        (_, Main::NoNetwork) => vec![
            ("1-4", "choose"),
            ("up/down", "move"),
            ("enter", "run"),
            ("alt+f2", "system log"),
        ],
        (_, Main::Home) => vec![
            ("1-7", "choose"),
            ("up/down", "move"),
            ("enter", "run"),
            ("alt+f2", "system log"),
        ],
    }
}

fn describe(action: &Action, snap: &Snapshot) -> String {
    let interface = snap
        .network
        .management
        .clone()
        .unwrap_or_else(|| "the network".into());
    match action {
        Action::ResetToDhcp => format!("Reset {interface} to DHCP"),
        Action::RollBack => {
            let versions = snap.molt.as_ref().and_then(|m| {
                Some((
                    m.booted.as_ref()?.version.clone(),
                    m.rollback.as_ref()?.version.clone(),
                ))
            });
            match versions {
                Some((from, to)) => format!("Roll back to the previous OS (Molt: {from} → {to})"),
                None => "Roll back to the previous OS".into(),
            }
        }
        Action::NewAdminToken => "Make a new admin token (the old one stops working)".into(),
        Action::Reboot => format!("Reboot {}", snap.hostname),
        Action::PowerOff => format!("Shut down {}", snap.hostname),
        Action::Network(change) => format!("Apply the network settings of {}", change.interface),
    }
}

fn draw_modal(app: &App, m: &Modal, buf: &mut Buffer, area: Rect, now: Instant) {
    let snap = &app.snapshot;
    let width = 64.min(area.width.saturating_sub(4));
    let (accent, title, lines): (Role, String, Vec<Line<'static>>) = match m {
        Modal::Pin {
            action,
            digits,
            error,
            checking,
        } => {
            let typed = "●".repeat(digits.len());
            let left = "○".repeat(crate::app::PIN_DIGITS - digits.len());
            let mut lines = vec![
                plain(describe(action, snap), Role::Text),
                plain(
                    "needs the recovery PIN shown at the first boot.",
                    Role::Text,
                ),
                blank(),
                Line::from(vec![
                    Span::styled("   PIN    ", Role::Text.fg()),
                    Span::styled(
                        typed,
                        Style::new().fg(Role::Bar.color()).bg(Role::Focus.color()),
                    ),
                    Span::styled(left, Role::Dim.fg()),
                ]),
                blank(),
            ];
            lines.push(if *checking {
                plain("Checking…", Role::Dim)
            } else if error.is_empty() {
                plain("Five wrong PINs lock it for 15 minutes.", Role::Dim)
            } else {
                plain(error.clone(), Role::Bad)
            });
            lines.extend([
                blank(),
                plain("Forgot the PIN? Reinstalling Molt on the boot disk leaves the other disks and their pools alone.", Role::Dim),
            ]);
            (Role::Focus, "Recovery PIN needed".into(), lines)
        }
        Modal::Power(choice) => (
            Role::Focus,
            format!("Reboot or shut down {}?", snap.hostname),
            vec![
                plain(
                    "Nothing else runs on this node yet: apps and VMs come with Tide.",
                    Role::Text,
                ),
                blank(),
                plain(
                    "The API and the agent stop and start again after a reboot.",
                    Role::Dim,
                ),
                blank(),
                Line::from(vec![
                    Span::raw("   "),
                    button("Reboot", *choice == 0),
                    Span::raw("   "),
                    button("Shut down", *choice == 1),
                    Span::raw("   "),
                    button("Cancel", *choice == 2),
                ]),
            ],
        ),
        Modal::Message { title, text, bad } => (
            if *bad { Role::Bad } else { Role::Focus },
            title.clone(),
            vec![plain(text.clone(), Role::Text)],
        ),
        Modal::Token(token) => (
            Role::Focus,
            "New admin token".into(),
            vec![
                code(token),
                blank(),
                plain(
                    "The old token no longer works. Log in again with crust login.",
                    Role::Text,
                ),
                plain("This is the only time the token is shown.", Role::Focus),
            ],
        ),
        Modal::Confirm { message, until, .. } => {
            let left = until.saturating_duration_since(now).as_secs();
            (
                Role::Focus,
                "Keep the new network settings?".into(),
                vec![
                    plain(message.clone(), Role::Text),
                    blank(),
                    Line::from(vec![
                        Span::styled("Press ", Role::Text.fg()),
                        Span::styled(
                            " c ",
                            Style::new().fg(Role::Bar.color()).bg(Role::Text.color()),
                        ),
                        Span::styled(" within ", Role::Text.fg()),
                        Span::styled(format!("{left} s"), Role::bold(Role::Focus)),
                        Span::styled(" to keep them; otherwise they revert.", Role::Text.fg()),
                    ]),
                ],
            )
        }
        Modal::Busy(text) => (
            Role::Focus,
            "Please wait".into(),
            vec![plain(text.clone(), Role::Text)],
        ),
    };
    let text_width = width.saturating_sub(6).max(1);
    let rows: u16 = lines
        .iter()
        .map(|l| u16::try_from(l.width().div_ceil(usize::from(text_width)).max(1)).unwrap_or(1))
        .sum();
    let inner = modal(buf, area, width, rows + 5, accent);
    let mut all = vec![
        Line::from(Span::styled(
            title,
            accent.fg().add_modifier(Modifier::BOLD),
        )),
        blank(),
    ];
    all.extend(lines);
    Paragraph::new(all)
        .wrap(Wrap { trim: false })
        .render(inner, buf);
}

#[cfg(test)]
mod tests {
    use crust_tui::glyphs::Glyphs;

    use super::*;
    use crate::app::Key;
    use crate::fake;

    fn render(app: &App, width: u16, height: u16) -> String {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        draw(app, &mut buf, Instant::now());
        let glyphs = Glyphs::console();
        let mut text = String::new();
        for y in 0..height {
            for x in 0..width {
                let symbol = buf[(x, y)].symbol();
                for c in symbol.chars() {
                    assert!(
                        glyphs.has(c),
                        "the console font has no {c:?} ({width}×{height}, row {y})"
                    );
                }
                text.push_str(symbol);
            }
            text.push('\n');
        }
        text
    }

    fn cases() -> Vec<(&'static str, App)> {
        let home = App::new(fake::snapshot());
        let mut first = fake::snapshot();
        first.first_boot = Some(fake::first_boot());
        let mut offline = fake::snapshot();
        offline.network = fake::offline();
        let mut boot = fake::snapshot();
        boot.health = Some(fake::health("running"));
        let mut failed = fake::snapshot();
        failed.health = Some(fake::health("failed"));
        let mut pin = App::new(fake::snapshot());
        pin.key(Key::Char('3'));
        pin.key(Key::Char('4'));
        pin.key(Key::Char('8'));
        let mut power = App::new(fake::snapshot());
        power.key(Key::Char('6'));
        let mut network = App::new(fake::snapshot());
        network.key(Key::Char('1'));
        network.tests = fake::tests();
        let mut diagnostics = App::new(fake::snapshot());
        diagnostics.key(Key::Char('5'));
        diagnostics.diagnostics = Some(fake::diagnostics());
        diagnostics.tests = fake::tests();
        let mut token = App::new(fake::snapshot());
        token.modal = Some(Modal::Token("crt_N3W7K-0KENQ-R4ND0-M1234".into()));
        vec![
            ("home", home),
            ("first_boot", App::new(first)),
            ("no_network", App::new(offline)),
            ("update_boot", App::new(boot)),
            ("update_failed", App::new(failed)),
            ("pin", pin),
            ("power", power),
            ("network", network),
            ("diagnostics", diagnostics),
            ("new_token", token),
        ]
    }

    #[test]
    fn every_screen_at_80x25_and_120x38() {
        for (name, app) in cases() {
            for (width, height) in [(80, 25), (120, 38)] {
                insta::assert_snapshot!(
                    format!("{name}_{width}x{height}"),
                    render(&app, width, height)
                );
            }
        }
    }

    #[test]
    fn large_screens_show_the_layout_centred() {
        let text = render(&App::new(fake::snapshot()), 260, 70);
        let first = text.lines().nth(1).unwrap();
        assert!(first.starts_with("          "), "{first:?}");
    }
}
