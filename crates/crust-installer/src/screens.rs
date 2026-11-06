//! Drawing the installer's steps (design kit: `lc-install-*`,
//! `tui-installer`), from the [`App`]'s state alone.

#![allow(clippy::too_many_lines, reason = "a screen is drawn line by line")]

use crust_tui::palette::Role;
use crust_tui::widgets::{background, centered, field, fit, key_bar, pane, progress, top_bar};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};

use crate::answers::{MIN_DISK, SMALL_DISK, human};
use crate::app::{App, KEYMAPS, LAYOUTS, MODES, NET_FIELDS, Phase, Step};
use crate::hardware::Verdict;

/// The steps in the bar at the top.
const STEPS: [&str; 6] = [
    "Language",
    "Hardware check",
    "Disks",
    "Network",
    "Admin",
    "Install",
];

fn plain(text: impl Into<String>, role: Role) -> Line<'static> {
    Line::from(Span::styled(text.into(), role.fg()))
}

fn blank() -> Line<'static> {
    Line::default()
}

fn selected() -> Style {
    Style::new().fg(Role::Bar.color()).bg(Role::Focus.color())
}

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

fn step_index(step: &Step) -> usize {
    match step {
        Step::Welcome | Step::Rescue => 0,
        Step::Hardware => 1,
        Step::Disks => 2,
        Step::Network => 3,
        Step::Admin | Step::Review => 4,
        Step::Install | Step::Done => 5,
    }
}

/// Draws the installer into `buf`.
pub fn draw(app: &App, buf: &mut Buffer) {
    let full = buf.area;
    background(buf, full);
    let area = centered(full, 240, 67);
    let firmware = if app.facts.uefi { "UEFI" } else { "BIOS" };
    let secure = match app.facts.secure_boot {
        Some(true) => "Secure Boot on",
        _ => "Secure Boot off",
    };
    let index = step_index(&app.step);
    let right = match app.step {
        Step::Review => "review before install".to_owned(),
        Step::Rescue => "rescue".to_owned(),
        _ => format!("step {} of 6", index + 1),
    };
    let facts = [
        app.version.as_str(),
        app.facts.arch.as_str(),
        firmware,
        secure,
    ];
    top_bar(
        buf,
        Rect::new(area.x, area.y, area.width, 1),
        "Crustates installer",
        &facts,
        &right,
    );
    key_bar(
        buf,
        Rect::new(area.x, area.bottom() - 1, area.width, 1),
        &keys(app),
    );
    let gap = u16::from(area.height >= 30);
    let body = Rect::new(
        area.x + 1,
        area.y + 1 + gap,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2 + 2 * gap),
    );

    // The steps.
    let mut spans = vec![Span::raw(" ")];
    for (i, name) in STEPS.iter().enumerate() {
        let span = match i.cmp(&index) {
            std::cmp::Ordering::Less => {
                Span::styled(format!("[x] {} {name}", i + 1), Role::Ok.fg())
            }
            std::cmp::Ordering::Equal => Span::styled(format!(" > {} {name} ", i + 1), selected()),
            std::cmp::Ordering::Greater => {
                Span::styled(format!("[ ] {} {name}", i + 1), Role::Dim.fg())
            }
        };
        spans.extend([span, Span::raw("   ")]);
    }
    let steps = Rect::new(body.x, body.y, body.width, 3);
    let block = pane("Steps", None);
    let inner = block.inner(steps);
    block.render(steps, buf);
    buf.set_line(inner.x, inner.y, &Line::from(spans), inner.width);
    let rest = Rect::new(
        body.x,
        body.y + 3,
        body.width,
        body.height.saturating_sub(3),
    );
    match app.step {
        Step::Welcome => welcome(app, buf, rest),
        Step::Hardware => hardware(app, buf, rest),
        Step::Disks => disks(app, buf, rest),
        Step::Network => network(app, buf, rest),
        Step::Admin => admin(app, buf, rest),
        Step::Review => review(app, buf, rest),
        Step::Install | Step::Done => install(app, buf, rest),
        Step::Rescue => rescue(app, buf, rest),
    }
}

fn keys(app: &App) -> Vec<(&'static str, &'static str)> {
    match app.step {
        Step::Welcome => vec![("tab", "next list"), ("up/down", "move"), ("enter", "next")],
        Step::Hardware if app.blocked() => vec![
            ("enter", "reboot into the firmware settings"),
            ("b", "back"),
        ],
        Step::Hardware => vec![("enter", "next"), ("b", "back")],
        Step::Disks => vec![
            ("up/down", "move"),
            ("space", "select disk"),
            ("m", "pool layout"),
            ("enter", "next"),
            ("b", "back"),
        ],
        Step::Network => vec![
            ("tab", "next field"),
            ("space", "DHCP or static"),
            ("t", "test"),
            ("enter", "next"),
            ("esc", "back"),
        ],
        Step::Admin => vec![("space", "tick"), ("enter", "next"), ("esc", "back")],
        Step::Review => vec![
            ("type", "ERASE to confirm"),
            ("enter", "install"),
            ("esc", "back"),
        ],
        Step::Install => vec![("alt+f2", "system log")],
        Step::Done => vec![("enter", "reboot")],
        Step::Rescue => vec![
            ("p", "previous image"),
            ("n", "network to DHCP"),
            ("z", "pools"),
            ("r", "look again"),
            ("enter", "reboot"),
            ("esc", "back"),
        ],
    }
}

fn welcome(app: &App, buf: &mut Buffer, area: Rect) {
    let top = Rect::new(area.x, area.y, area.width, 9.min(area.height));
    let half = top.width / 2;
    panel(
        buf,
        Rect::new(top.x, top.y, half, top.height),
        "Language",
        None,
        vec![
            Line::from(Span::styled(" > English ", selected())),
            plain("   More languages come in a later release.", Role::Dim),
        ],
    );
    let mut layouts: Vec<Line<'static>> = Vec::new();
    let first = app.keymap.saturating_sub(4);
    for (i, (_, label)) in KEYMAPS.iter().enumerate().skip(first).take(5) {
        layouts.push(if i == app.keymap {
            let style = if app.focus == 0 {
                selected()
            } else {
                Role::bold(Role::Text)
            };
            Line::from(Span::styled(format!(" > {label} "), style))
        } else {
            plain(format!("   {label}"), Role::Text)
        });
    }
    layouts.push(Line::from(vec![
        Span::styled("   Type here to test: ", Role::Dim.fg()),
        Span::styled(format!("{}_", app.keymap_test), Role::Focus.fg()),
    ]));
    panel(
        buf,
        Rect::new(top.x + half + 1, top.y, top.width - half - 1, top.height),
        "Keyboard layout",
        (app.focus == 0).then_some(Role::Focus),
        layouts,
    );
    let mut lines = Vec::new();
    for (i, (title, text)) in MODES.iter().enumerate() {
        let mark = if i == app.mode { "(o)" } else { "( )" };
        let disabled = i == 1;
        lines.push(if i == app.mode && app.focus == 1 {
            Line::from(Span::styled(format!("{mark} {title} "), selected()))
        } else {
            plain(
                format!("{mark} {title}"),
                if disabled { Role::Dim } else { Role::Text },
            )
        });
        lines.extend([plain(format!("    {text}"), Role::Dim), blank()]);
    }
    let rest = Rect::new(
        area.x,
        area.y + top.height,
        area.width,
        area.height.saturating_sub(top.height),
    );
    panel(
        buf,
        rest,
        "What do you want to do?",
        (app.focus == 1).then_some(Role::Focus),
        lines,
    );
}

fn hardware(app: &App, buf: &mut Buffer, area: Rect) {
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{:<6}{:<20}", "", "Check"), Role::bold(Role::Text)),
        Span::styled("Result", Role::bold(Role::Text)),
    ])];
    for f in &app.findings {
        let (word, color) = match f.verdict {
            Verdict::Ok => ("ok", Role::Ok),
            Verdict::Warn => ("warn", Role::Focus),
            Verdict::Fail => ("fail", Role::Bad),
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{word:<6}"), color.fg()),
            Span::styled(format!("{:<20}", f.name), Role::Text.fg()),
            Span::styled(
                f.result.clone(),
                if f.verdict == Verdict::Fail {
                    Role::Bad.fg()
                } else {
                    Role::Text.fg()
                },
            ),
        ]));
    }
    lines.extend([blank(), plain("─".repeat(60), Role::Dim)]);
    let fails: Vec<_> = app
        .findings
        .iter()
        .filter(|f| f.verdict == Verdict::Fail)
        .collect();
    let warns: Vec<_> = app
        .findings
        .iter()
        .filter(|f| f.verdict == Verdict::Warn)
        .collect();
    if fails.is_empty() {
        lines.push(plain("Ready to install.", Role::Ok));
    } else {
        lines.push(plain(
            "Crustates cannot be installed on this machine yet:",
            Role::Bad,
        ));
        for f in fails {
            lines.push(plain(format!("✗ {}", f.advice), Role::Bad));
        }
    }
    for f in warns {
        lines.push(Line::from(vec![
            Span::styled("! ", Role::Focus.fg()),
            Span::styled(f.advice.clone(), Role::Dim.fg()),
        ]));
    }
    let accent = if app.blocked() {
        Role::Bad
    } else {
        Role::Focus
    };
    panel(buf, area, "Hardware check", Some(accent), lines);
}

fn disk_line(app: &App, i: usize, pool: bool) -> Line<'static> {
    let d = &app.facts.disks[i];
    let mark = if pool {
        if app.pool[i] { "[x]" } else { "[ ]" }
    } else if app.boot == Some(i) {
        "(o)"
    } else {
        "( )"
    };
    let name = d.path.trim_start_matches("/dev/");
    let note = if d.install_media {
        "the install stick".to_owned()
    } else if pool && app.boot == Some(i) {
        "used as boot disk".to_owned()
    } else if !pool && d.size < MIN_DISK {
        "too small (32 GB or more)".to_owned()
    } else {
        d.contents.clone()
    };
    let text = format!(
        "{mark} {name:<10} {:<22} {:>8}  {:<6} ",
        d.model.chars().take(22).collect::<String>(),
        human(d.size),
        d.transport
    );
    if app.cursor == i && app.in_pool == pool {
        return Line::from(Span::styled(format!("{text}{note} "), selected()));
    }
    let note_color = if note.starts_with("has data") {
        Role::Focus
    } else {
        Role::Dim
    };
    let text_color = if d.install_media || (!pool && d.size < MIN_DISK) {
        Role::Dim
    } else {
        Role::Text
    };
    Line::from(vec![
        Span::styled(text, text_color.fg()),
        Span::styled(note, note_color.fg()),
    ])
}

fn disks(app: &App, buf: &mut Buffer, area: Rect) {
    let mut lines = vec![
        Line::from(vec![
            Span::styled("Boot disk ", Role::bold(Role::Text)),
            Span::styled(
                "(holds the read-only OS image, at least 32 GB)",
                Role::Dim.fg(),
            ),
        ]),
        blank(),
    ];
    for i in 0..app.facts.disks.len() {
        lines.push(disk_line(app, i, false));
    }
    lines.extend([
        blank(),
        Line::from(vec![
            Span::styled("Storage pool disks ", Role::bold(Role::Text)),
            Span::styled(
                "(optional: a ZFS pool for your data; more can be added later)",
                Role::Dim.fg(),
            ),
        ]),
        blank(),
    ]);
    for i in 0..app.facts.disks.len() {
        lines.push(disk_line(app, i, true));
    }
    let pool = app.pool_disks();
    let (layout, needed) = LAYOUTS[app.layout];
    let raw: u64 = app
        .facts
        .disks
        .iter()
        .zip(&app.pool)
        .filter(|(_, on)| **on)
        .map(|(d, _)| d.size)
        .sum();
    let smallest = app
        .facts
        .disks
        .iter()
        .zip(&app.pool)
        .filter(|(_, on)| **on)
        .map(|(d, _)| d.size)
        .min()
        .unwrap_or(0);
    let count = u64::try_from(pool.len()).unwrap_or(0);
    let usable = match layout {
        "mirror" => smallest,
        "raidz" => smallest * count.saturating_sub(1),
        _ => raw,
    };
    lines.push(blank());
    if pool.is_empty() {
        lines.push(plain(
            "No pool: the disks stay as they are. Pools can be made later.",
            Role::Dim,
        ));
    } else {
        lines.extend([
            Line::from(vec![
                Span::styled(format!("{:<15}", "Pool layout"), Role::Dim.fg()),
                Span::styled(format!("ZFS {layout}"), Role::Text.fg()),
                Span::styled("  (m changes it)", Role::Dim.fg()),
            ]),
            Line::from(vec![
                Span::styled(format!("{:<15}", "Pool name"), Role::Dim.fg()),
                Span::styled(app.pool_name.clone(), Role::Text.fg()),
            ]),
            Line::from(vec![
                Span::styled(format!("{:<15}", "Usable space"), Role::Dim.fg()),
                Span::styled(
                    format!("about {} of {} raw", human(usable), human(raw)),
                    Role::Text.fg(),
                ),
            ]),
            Line::from(vec![
                Span::styled(format!("{:<15}", "Encryption"), Role::Dim.fg()),
                Span::styled("comes with Reef", Role::Dim.fg()),
            ]),
        ]);
        if pool.len() < needed {
            lines.push(plain(
                format!("A {layout} pool needs {needed} disks or more."),
                Role::Bad,
            ));
        }
    }
    let mut erased: Vec<String> = app
        .boot
        .and_then(|b| app.facts.disks.get(b))
        .map(|d| d.path.trim_start_matches("/dev/").to_owned())
        .into_iter()
        .collect();
    erased.extend(
        pool.iter()
            .map(|p| p.trim_start_matches("/dev/").to_owned()),
    );
    if !erased.is_empty() {
        lines.extend([
            blank(),
            Line::from(vec![
                Span::styled("!  ", Role::Focus.fg()),
                Span::styled(
                    format!(
                        "Everything on {} will be erased; the other disks are not touched.",
                        erased.join(", ")
                    ),
                    Role::Focus.fg(),
                ),
            ]),
        ]);
    }
    if app
        .boot
        .and_then(|b| app.facts.disks.get(b))
        .is_some_and(|d| d.size < SMALL_DISK)
    {
        lines.push(plain(
            "   The boot disk is under 64 GB: it works, with little room for updates and logs.",
            Role::Dim,
        ));
    }
    panel(buf, area, "Choose disks", Some(Role::Focus), lines);
}

fn network(app: &App, buf: &mut Buffer, area: Rect) {
    let choice = |label: &str, on: bool| {
        let text = format!("({}) {label}", if on { "o" } else { " " });
        if on && app.net_focus == 0 {
            Span::styled(text, selected())
        } else {
            Span::styled(text, Role::Text.fg())
        }
    };
    let row = |i: usize, enabled: bool| {
        let mut f = field(&app.net[i], 20, app.net_focus == i + 1);
        if !enabled && app.net_focus != i + 1 {
            f = f.style(Role::Dim.fg());
        }
        Line::from(vec![
            Span::styled(format!("  {:<15}", NET_FIELDS[i]), Role::Text.fg()),
            f,
        ])
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled("Address    ", Role::bold(Role::Text)),
            choice("DHCP", !app.static_ip),
            Span::raw("    "),
            choice("Static", app.static_ip),
        ]),
        blank(),
        row(0, app.static_ip),
        row(1, app.static_ip),
        row(2, app.static_ip),
        row(3, true),
        blank(),
        row(4, true),
        row(5, true),
        blank(),
        plain(
            "Without a network the install works too: updates start once the machine is online.",
            Role::Dim,
        ),
    ];
    if let Some(e) = &app.error {
        lines.extend([blank(), plain(format!("✗ {e}"), Role::Bad)]);
    }
    if !app.net_test.is_empty() {
        lines.extend([
            blank(),
            plain("Test connection", Role::Text).style(Role::bold(Role::Text)),
        ]);
        lines.extend(app.net_test.iter().map(|l| plain(l.clone(), Role::Text)));
    }
    panel(buf, area, "Management network", Some(Role::Focus), lines);
}

fn code(text: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("    "),
        Span::styled(format!(" {text} "), selected()),
    ])
}

fn admin(app: &App, buf: &mut Buffer, area: Rect) {
    let pin = format!(
        "{} {} {}",
        app.pin.get(..3).unwrap_or_default(),
        app.pin.get(3..6).unwrap_or_default(),
        app.pin.get(6..).unwrap_or_default()
    );
    let mut lines = vec![
        plain(
            "After the install, log in from a computer on the same network:",
            Role::Text,
        ),
        blank(),
        plain(
            "    crust login --api http://<this machine>:8080",
            Role::Info,
        ),
        blank(),
        plain("with this admin token:", Role::Text),
        blank(),
    ];
    if app.token.is_empty() {
        lines.push(plain("    making the token…", Role::Dim));
    } else {
        lines.push(code(&app.token));
    }
    lines.extend([
        blank(),
        plain("─".repeat(60), Role::Dim),
        Line::from(vec![Span::styled("Recovery PIN   ", Role::bold(Role::Text)), Span::styled("shown only now · write it down", Role::Dim.fg())]),
        blank(),
        Line::from(Span::styled(format!("    {pin}"), Role::bold(Role::Focus))),
        blank(),
        plain("The console on this machine's screen asks for it before risky actions: rolling back the OS, changing the network, a new admin token, rebooting.", Role::Dim),
        blank(),
        Line::from(Span::styled(
            format!("{} I wrote down the admin token and the recovery PIN", if app.wrote_down { "[x]" } else { "[ ]" }),
            if app.wrote_down { Role::Ok.fg() } else { Role::Text.fg() },
        )),
    ]);
    if let Some(e) = &app.error {
        lines.extend([blank(), plain(format!("✗ {e}"), Role::Bad)]);
    }
    panel(
        buf,
        area,
        "Admin token and recovery PIN",
        Some(Role::Focus),
        lines,
    );
}

fn review(app: &App, buf: &mut Buffer, area: Rect) {
    let a = app.answers();
    let half = area.width * 3 / 5;
    let row = |k: &str, v: String| {
        Line::from(vec![
            Span::styled(format!("{k:<14}"), Role::Dim.fg()),
            Span::styled(v, Role::Text.fg()),
        ])
    };
    let network = if a.network.is_static() {
        format!(
            "{} · static · {} · gw {}",
            a.network.interface, a.network.address, a.network.gateway
        )
    } else {
        format!("{} · DHCP", a.network.interface)
    };
    let boot = app.boot.and_then(|b| app.facts.disks.get(b));
    let mut lines = vec![
        row(
            "Language",
            format!("English · keyboard {}", KEYMAPS[app.keymap].0),
        ),
        row("Hostname", a.hostname().to_owned()),
        row("Network", network),
        row(
            "Boot disk",
            boot.map_or("-".into(), |d| {
                format!(
                    "{}  {}  {}",
                    d.path.trim_start_matches("/dev/"),
                    d.model,
                    human(d.size)
                )
            }),
        ),
        row(
            "Pool",
            a.pool.as_ref().map_or("none".into(), |p| {
                format!("{} · ZFS {} of {}", p.name, p.layout, p.disks.join(" + "))
            }),
        ),
        row("Channel", format!("{} · updates checked daily", a.channel)),
        row("Recovery PIN", "written down".into()),
        blank(),
        plain(
            "Press esc to change anything. Nothing has been written yet.",
            Role::Dim,
        ),
    ];
    if let Some(e) = &app.error {
        lines.extend([blank(), plain(format!("✗ {e}"), Role::Bad)]);
    }
    panel(
        buf,
        Rect::new(area.x, area.y, half, area.height),
        "Review your choices",
        Some(Role::Focus),
        lines,
    );
    let mut erased = Vec::new();
    for (i, d) in app.facts.disks.iter().enumerate() {
        let wiped = app.boot == Some(i) || app.pool[i];
        let color = if wiped { Role::Focus } else { Role::Ok };
        erased.push(Line::from(vec![
            Span::styled(if wiped { "!  " } else { "   " }, color.fg()),
            Span::styled(
                format!(
                    "{:<10} {:<18} {}",
                    d.path.trim_start_matches("/dev/"),
                    d.model.chars().take(18).collect::<String>(),
                    human(d.size)
                ),
                color.fg(),
            ),
        ]));
        erased.push(plain(
            format!(
                "   {}",
                if wiped {
                    d.contents.clone()
                } else {
                    "not selected, will not be touched".into()
                }
            ),
            Role::Dim,
        ));
    }
    erased.extend([
        blank(),
        plain("Type ERASE to confirm", Role::Text).style(Role::bold(Role::Text)),
        blank(),
        Line::from(vec![Span::raw("   "), field(&app.erase, 10, true)]),
        blank(),
        plain("The install starts when you press enter.", Role::Dim),
    ]);
    panel(
        buf,
        Rect::new(
            area.x + half + 1,
            area.y,
            area.width - half - 1,
            area.height,
        ),
        "These disks will be erased",
        Some(Role::Focus),
        erased,
    );
}

fn install(app: &App, buf: &mut Buffer, area: Rect) {
    let steps = u16::try_from(app.progress.len()).unwrap_or(0);
    let top = Rect::new(area.x, area.y, area.width, (steps + 5).min(area.height));
    let mut lines = Vec::new();
    for (title, phase) in &app.progress {
        let (word, color) = match phase {
            Phase::Done => ("done", Role::Ok),
            Phase::Now => ("now", Role::Focus),
            Phase::Next => ("next", Role::Dim),
            Phase::Failed => ("failed", Role::Bad),
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{word:<8}"), color.fg()),
            Span::styled(
                title.clone(),
                if *phase == Phase::Next {
                    Role::Dim.fg()
                } else {
                    Role::Text.fg()
                },
            ),
        ]));
    }
    let done = app
        .progress
        .iter()
        .filter(|(_, p)| *p == Phase::Done)
        .count();
    lines.extend([blank(), progress(done, app.progress.len(), 40)]);
    let (title, accent) = match (&app.step, &app.error) {
        (_, Some(_)) => ("The install failed", Role::Bad),
        (Step::Done, _) => ("Installed", Role::Ok),
        _ => ("Installing", Role::Focus),
    };
    panel(buf, top, title, Some(accent), lines);
    let rest = Rect::new(
        area.x,
        area.y + top.height,
        area.width,
        area.height.saturating_sub(top.height),
    );
    let mut log: Vec<Line<'static>> = Vec::new();
    if app.step == Step::Done {
        log.extend([
            plain("Crustates is installed.", Role::Ok).style(Role::bold(Role::Ok)),
            blank(),
            plain(
                "Remove the USB stick, then press enter to reboot.",
                Role::Text,
            ),
            plain(
                "The machine's screen then shows its address and the recovery console.",
                Role::Dim,
            ),
        ]);
    } else if let Some(e) = &app.error {
        log.extend([
            plain(format!("✗ {e}"), Role::Bad),
            blank(),
            plain(
                "Nothing on the other disks was touched. The log is on tty2 (alt+f2).",
                Role::Dim,
            ),
            blank(),
        ]);
    }
    let rows = usize::from(rest.height.saturating_sub(3)).saturating_sub(log.len());
    let first = app.log.len().saturating_sub(rows);
    log.extend(
        app.log[first..]
            .iter()
            .map(|l| plain(l.clone(), Role::Text)),
    );
    panel(buf, rest, "Log", None, log);
}

fn rescue(app: &App, buf: &mut Buffer, area: Rect) {
    let mut lines = vec![
        plain(
            "The installer looks for Crustates on this machine's disks. It reads them and changes nothing until you choose a repair.",
            Role::Dim,
        ),
        blank(),
    ];
    if app.installs.is_empty() {
        lines.push(plain(
            "No Crustates install found. Press r to look again.",
            Role::Text,
        ));
    } else {
        lines.push(plain("Found", Role::Text).style(Role::bold(Role::Text)));
        lines.extend(
            app.installs
                .iter()
                .map(|l| plain(format!("  {l}"), Role::Text)),
        );
        lines.extend([
            blank(),
            plain("Repairs", Role::Text).style(Role::bold(Role::Text)),
            plain("  p  boot the previous OS image next", Role::Text),
            plain("  n  reset the network to DHCP on every port", Role::Text),
        ]);
    }
    lines.extend([
        plain(
            "  z  list the ZFS pools (they are not imported)",
            Role::Text,
        ),
        plain("  enter  reboot", Role::Text),
        blank(),
        plain(
            "Repairing the boot loader comes in a later release.",
            Role::Dim,
        ),
    ]);
    match &app.rescued {
        Some(Ok(text)) => lines.extend([blank(), plain(text.clone(), Role::Ok)]),
        Some(Err(text)) => lines.extend([blank(), plain(format!("✗ {text}"), Role::Bad)]),
        None => {}
    }
    panel(
        buf,
        area,
        "Rescue a broken install",
        Some(Role::Focus),
        lines,
    );
}

#[cfg(test)]
mod tests {
    use crust_tui::glyphs::Glyphs;

    use super::*;
    use crate::app::{Done, Key};
    use crate::hardware::{Disk, Facts};

    fn app() -> App {
        let disk = |path: &str, size, model: &str, transport: &str, contents: &str| Disk {
            path: path.into(),
            size,
            model: model.into(),
            transport: transport.into(),
            contents: contents.into(),
            install_media: false,
        };
        let mut stick = disk(
            "/dev/sde",
            32_000_000_000,
            "SanDisk Ultra",
            "USB",
            "has data: 1 partition (iso9660)",
        );
        stick.install_media = true;
        let facts = Facts {
            arch: "amd64".into(),
            uefi: true,
            cpu_flags: "vmx".into(),
            cpu: "8 cores (i7-12700T)".into(),
            iommu_groups: 24,
            memory: 64 << 30,
            tpm: Some("2.0".into()),
            secure_boot: Some(true),
            disks: vec![
                disk(
                    "/dev/nvme0n1",
                    500_107_862_016,
                    "Samsung 980 PRO",
                    "NVMe",
                    "empty",
                ),
                disk(
                    "/dev/sdb",
                    4_000_787_030_016,
                    "WD Red Plus",
                    "SATA",
                    "empty",
                ),
                disk(
                    "/dev/sdc",
                    4_000_787_030_016,
                    "WD Red Plus",
                    "SATA",
                    "empty",
                ),
                stick,
            ],
            ports: (2, 1),
            ..Facts::default()
        };
        App::new("0.2.0", facts, "enp3s0")
    }

    fn render(app: &App, width: u16, height: u16) -> String {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        draw(app, &mut buf);
        let glyphs = Glyphs::console();
        let mut text = String::new();
        for y in 0..height {
            for x in 0..width {
                let symbol = buf[(x, y)].symbol();
                for c in symbol.chars() {
                    assert!(glyphs.has(c), "the console font has no {c:?}");
                }
                text.push_str(symbol);
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn every_step_at_80x25_and_120x38() {
        let mut app = app();
        let mut cases = vec![("welcome", render(&app, 120, 38), render(&app, 80, 25))];
        app.key(Key::Enter);
        cases.push(("hardware", render(&app, 120, 38), render(&app, 80, 25)));
        app.key(Key::Enter);
        for key in [
            Key::Down,
            Key::Down,
            Key::Down,
            Key::Down,
            Key::Down,
            Key::Char(' '),
            Key::Down,
            Key::Char(' '),
        ] {
            app.key(key);
        }
        cases.push(("disks", render(&app, 120, 38), render(&app, 80, 25)));
        app.key(Key::Enter);
        cases.push(("network", render(&app, 120, 38), render(&app, 80, 25)));
        app.key(Key::Enter);
        app.done(Done::Secrets(Ok((
            "crt_K7Q4M-2MRXV-9DPAZ-H3T8W".into(),
            "482913075".into(),
        ))));
        app.key(Key::Char(' '));
        cases.push(("admin", render(&app, 120, 38), render(&app, 80, 25)));
        app.key(Key::Enter);
        for c in "ERA".chars() {
            app.key(Key::Char(c));
        }
        cases.push(("review", render(&app, 120, 38), render(&app, 80, 25)));
        app.key(Key::Char('S'));
        app.key(Key::Char('E'));
        app.key(Key::Enter);
        app.progress = vec![
            ("Write the OS image to nvme0n1".into(), Phase::Done),
            ("Configure the new system".into(), Phase::Now),
            ("Create pool tank (ZFS mirror)".into(), Phase::Next),
        ];
        app.log = vec![
            "$ bootc install to-disk --wipe … /dev/nvme0n1".into(),
            "Installation complete!".into(),
        ];
        cases.push(("install", render(&app, 120, 38), render(&app, 80, 25)));
        app.step = Step::Done;
        cases.push(("done", render(&app, 120, 38), render(&app, 80, 25)));
        let mut rescue = self::app();
        rescue.step = Step::Rescue;
        rescue.done(Done::Installs(vec![
            "/dev/nvme0n1p3: 0.2.0 (4f2a9c01d3e5), boots next · 0.1.0 (9b1e77aa0c2d)".into(),
        ]));
        cases.push(("rescue", render(&rescue, 120, 38), render(&rescue, 80, 25)));
        for (name, big, small) in cases {
            insta::assert_snapshot!(format!("{name}_120x38"), big);
            insta::assert_snapshot!(format!("{name}_80x25"), small);
        }
    }
}
