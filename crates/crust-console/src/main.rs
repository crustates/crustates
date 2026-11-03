//! `crust-console`: the console on the machine's own screen (ADR 0018). On
//! tty1 it shows the status, the first-boot secrets and the recovery menu;
//! `--logs` shows the system log (tty2); `--demo` runs in any terminal with
//! example data.

mod api;
mod app;
mod fake;
mod linux;
mod logs;
mod screens;
mod system;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::bail;
use crust_tui::glyphs::Glyphs;
use crust_tui::palette;
use crust_tui::psf::Psf;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;

use crate::app::{App, Done, Effect, Key};
use crate::system::{LogLine, Network, Snapshot, System};

/// The API's local socket.
const SOCKET: &str = "/run/crustates/api.sock";
/// Where the `crustates-console` fonts are.
const FONTS: &str = "/usr/lib/kbd/consolefonts";

/// A key, or the request to quit (`--demo` only).
enum Input {
    Key(Key),
    Quit,
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args
        .iter()
        .any(|a| !matches!(a.as_str(), "--logs" | "--demo"))
    {
        bail!("usage: crust-console [--logs] [--demo]");
    }
    let (logs, demo) = (
        args.iter().any(|a| a == "--logs"),
        args.iter().any(|a| a == "--demo"),
    );
    let vt = std::env::var("TERM").is_ok_and(|t| t == "linux");
    let glyphs = if vt { set_up_vt() } else { Glyphs::any() };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let mut terminal = ratatui::try_init()?;
    let result = runtime.block_on(async {
        match (logs, demo) {
            (true, _) => run_logs(&mut terminal, &glyphs, vt, demo).await,
            (false, true) => {
                run_console(
                    &mut terminal,
                    Arc::new(fake::Fake::default()),
                    &glyphs,
                    vt,
                    true,
                )
                .await
            }
            (false, false) => {
                run_console(
                    &mut terminal,
                    Arc::new(linux::Linux::new(SOCKET.into())),
                    &glyphs,
                    vt,
                    false,
                )
                .await
            }
        }
    });
    ratatui::restore();
    result
}

/// On the Linux console: loads our palette and font, and returns the
/// font's characters (ASCII if the font did not load).
fn set_up_vt() -> Glyphs {
    use std::io::Write;
    print!("{}", palette::vt_palette());
    let _ = std::io::stdout().flush();
    // Past 240 columns at 8×16, the doubled font keeps text readable.
    let width: u16 = std::fs::read_to_string("/sys/class/graphics/fb0/virtual_size")
        .ok()
        .and_then(|s| s.split(',').next()?.trim().parse().ok())
        .unwrap_or(0);
    let name = if width / 8 > screens::MAX.0 {
        "crustates-console-32.psfu"
    } else {
        "crustates-console-16.psfu"
    };
    let path = Path::new(FONTS).join(name);
    let loaded = std::process::Command::new("setfont")
        .arg(&path)
        .status()
        .is_ok_and(|s| s.success());
    let font = std::fs::read(&path)
        .ok()
        .and_then(|bytes| Psf::parse(&bytes).ok());
    match font {
        Some(font) if loaded => Glyphs::of(&font),
        _ => Glyphs::ascii(),
    }
}

/// Reads keys on a thread of its own: crossterm's reads block.
fn read_keys() -> mpsc::Receiver<Input> {
    let (tx, rx) = mpsc::channel(32);
    std::thread::spawn(move || {
        while let Ok(event) = event::read() {
            let Event::Key(k) = event else { continue };
            if k.kind != KeyEventKind::Press {
                continue;
            }
            let input = match k.code {
                KeyCode::Char('c' | 'q') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    Input::Quit
                }
                KeyCode::Char(c) => Input::Key(Key::Char(c)),
                KeyCode::Enter => Input::Key(Key::Enter),
                KeyCode::Esc => Input::Key(Key::Esc),
                KeyCode::Up => Input::Key(Key::Up),
                KeyCode::Down => Input::Key(Key::Down),
                KeyCode::Left => Input::Key(Key::Left),
                KeyCode::Right => Input::Key(Key::Right),
                KeyCode::Tab => Input::Key(Key::Tab),
                KeyCode::BackTab => Input::Key(Key::BackTab),
                KeyCode::Backspace => Input::Key(Key::Backspace),
                KeyCode::PageUp => Input::Key(Key::PageUp),
                KeyCode::PageDown => Input::Key(Key::PageDown),
                KeyCode::F(n) => Input::Key(Key::F(n)),
                _ => continue,
            };
            if tx.blocking_send(input).is_err() {
                break;
            }
        }
    });
    rx
}

async fn run_console<S: System>(
    terminal: &mut DefaultTerminal,
    system: Arc<S>,
    glyphs: &Glyphs,
    vt: bool,
    demo: bool,
) -> anyhow::Result<()> {
    let mut app = App::new(system.snapshot().await);
    let mut keys = read_keys();
    let (done_tx, mut dones) = mpsc::channel::<Done>(32);
    let (snap_tx, mut snapshots) = mpsc::channel::<Snapshot>(4);
    let refresher = {
        let (system, tx) = (Arc::clone(&system), snap_tx.clone());
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(3)).await;
                if tx.send(system.snapshot().await).await.is_err() {
                    break;
                }
            }
        })
    };
    let mut ticks = tokio::time::interval(Duration::from_secs(1));
    loop {
        terminal.draw(|frame| {
            let buf = frame.buffer_mut();
            screens::draw(&app, buf, Instant::now());
            glyphs.apply(buf);
            if !vt {
                palette::to_rgb(buf);
            }
        })?;
        let effects = tokio::select! {
            Some(input) = keys.recv() => match input {
                Input::Quit if demo => break,
                Input::Quit => vec![],
                Input::Key(key) => app.key(key),
            },
            Some(snapshot) = snapshots.recv() => {
                app.update(snapshot);
                vec![]
            }
            Some(done) = dones.recv() => app.done(done),
            _ = ticks.tick() => {
                app.tick(Instant::now());
                vec![]
            }
        };
        for effect in effects {
            perform(
                Arc::clone(&system),
                effect,
                app.snapshot.network.clone(),
                done_tx.clone(),
                snap_tx.clone(),
            );
        }
    }
    refresher.abort();
    Ok(())
}

/// Runs an effect in the background and reports its result.
fn perform<S: System>(
    system: Arc<S>,
    effect: Effect,
    network: Network,
    done: mpsc::Sender<Done>,
    snapshots: mpsc::Sender<Snapshot>,
) {
    tokio::spawn(async move {
        let result = match effect {
            Effect::CheckPin(pin, action) => {
                let checked = system.check_pin(&pin, &action).await;
                Done::Pin(action, checked)
            }
            Effect::Run(action) => Done::Ran(system.run(&action, &network).await),
            Effect::Confirm(checkpoint) => Done::Simple(
                system
                    .confirm(&checkpoint)
                    .await
                    .map(|()| "The new network settings stay.".into()),
            ),
            Effect::RetryDhcp => Done::Simple(
                system
                    .retry_dhcp(&network)
                    .await
                    .map(|()| "Asked the network for an address again.".into()),
            ),
            Effect::Diagnostics => Done::Diagnostics(system.diagnostics().await),
            Effect::Test => Done::Tested(system.test_connection(&network).await),
            Effect::FinishFirstBoot => {
                let finished = system.finish_first_boot().await.map(|()| String::new());
                let _ = snapshots.send(system.snapshot().await).await;
                Done::Simple(finished)
            }
        };
        let _ = done.send(result).await;
    });
}

/// Reads the journal with the filter's arguments; lines carry `generation`
/// so lines of an older filter can be told apart.
fn read_journal(
    args: Vec<String>,
    generation: u64,
    tx: mpsc::Sender<(u64, LogLine)>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let child = tokio::process::Command::new("journalctl")
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn();
        let Ok(mut child) = child else { return };
        let Some(stdout) = child.stdout.take() else {
            return;
        };
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(line) = logs::parse(&line)
                && tx.send((generation, line)).await.is_err()
            {
                break;
            }
        }
        drop(child);
    })
}

async fn run_logs(
    terminal: &mut DefaultTerminal,
    glyphs: &Glyphs,
    vt: bool,
    demo: bool,
) -> anyhow::Result<()> {
    let mut view = logs::Logs::default();
    let mut keys = read_keys();
    let (tx, mut lines) = mpsc::channel::<(u64, LogLine)>(1024);
    let mut generation = 0;
    let mut reader = (!demo).then(|| read_journal(view.args(), generation, tx.clone()));
    if demo {
        for (unit, priority, message) in [
            (
                "systemd",
                6,
                "Started crust-agent.service - Crustates node agent.",
            ),
            ("crust-api", 6, "control plane started"),
            (
                "crust-agent",
                4,
                "checking for a new Molt image failed: the registry is not reachable",
            ),
            ("greenboot", 6, "greenboot health checks passed"),
        ] {
            view.push(LogLine {
                time: "Nov 03 21:45:02".into(),
                unit: unit.into(),
                priority,
                message: message.into(),
            });
        }
    }
    let version = std::fs::read_to_string("/usr/lib/os-release")
        .unwrap_or_default()
        .lines()
        .find_map(|l| {
            l.strip_prefix("IMAGE_VERSION=")
                .map(|v| v.trim_matches('"').to_owned())
        })
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());
    let mut ticks = tokio::time::interval(Duration::from_secs(1));
    loop {
        let now = jiff::Zoned::now().strftime("%Y-%m-%d %H:%M").to_string();
        terminal.draw(|frame| {
            let buf = frame.buffer_mut();
            view.draw(buf, &version, &now);
            glyphs.apply(buf);
            if !vt {
                palette::to_rgb(buf);
            }
        })?;
        tokio::select! {
            Some(input) = keys.recv() => match input {
                Input::Quit if demo => break,
                Input::Quit => {}
                Input::Key(key) => {
                    if view.key(key) && !demo {
                        if let Some(old) = reader.take() {
                            old.abort();
                        }
                        generation += 1;
                        reader = Some(read_journal(view.args(), generation, tx.clone()));
                    }
                }
            },
            Some((g, line)) = lines.recv() => {
                if g == generation {
                    view.push(line);
                }
                // Many lines come at once: take them all before drawing.
                while let Ok((g, line)) = lines.try_recv() {
                    if g == generation {
                        view.push(line);
                    }
                }
            }
            _ = ticks.tick() => {}
        }
    }
    Ok(())
}
