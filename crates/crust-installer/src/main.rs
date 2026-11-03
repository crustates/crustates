//! `crust-installer`: installs Crustates from the install media. On tty1 it
//! walks through the steps of the design kit; with an answers file
//! (`--answers FILE`, `answers.toml` on a stick labelled `CRUSTANS`, or
//! QEMU's `-fw_cfg name=opt/crustates/answers`) it installs without
//! questions and writes its progress to the console;
//! `--rescue` (or `crust.rescue` on the kernel command line) repairs an
//! install; `--demo` shows the screens anywhere and installs nothing.

mod answers;
mod app;
mod disks;
mod hardware;
mod install;
mod rescue;
mod screens;
mod system;
mod target;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use crust_tui::glyphs::Glyphs;
use crust_tui::palette;
use crust_tui::psf::Psf;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use tokio::sync::mpsc;

use crate::answers::Answers;
use crate::app::{App, Done, Effect, Key, Step};
use crate::hardware::{Disk, Facts};

/// The image this media installs, written when the media is built.
const PAYLOAD: &str = "/usr/lib/crustates/installer-payload";
/// Where the token and PIN hashes wait for the install.
const SECRETS: &str = "/run/crust-installer/secrets";
/// The label of a stick with `answers.toml`.
const ANSWERS_LABEL: &str = "/dev/disk/by-label/CRUSTANS";
/// Answers a virtual machine gets from QEMU (`-fw_cfg`).
const ANSWERS_FW_CFG: &str = "/sys/firmware/qemu_fw_cfg/by_name/opt/crustates/answers/raw";

enum Input {
    Key(Key),
    Quit,
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let answers = args
        .iter()
        .position(|a| a == "--answers")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);
    let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
    let (demo, rescue) = (
        flag("--demo"),
        flag("--rescue") || cmdline.split_whitespace().any(|w| w == "crust.rescue"),
    );
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let version = std::fs::read_to_string("/usr/lib/os-release")
        .unwrap_or_default()
        .lines()
        .find_map(|l| {
            l.strip_prefix("IMAGE_VERSION=")
                .map(|v| v.trim_matches('"').to_owned())
        })
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());
    if !demo {
        let answers = match answers {
            Some(path) => Some(path),
            None => runtime.block_on(answers_stick()),
        };
        if let Some(path) = answers {
            return runtime.block_on(unattended(&path));
        }
    }
    let vt = std::env::var("TERM").is_ok_and(|t| t == "linux");
    let glyphs = if vt { set_up_vt() } else { Glyphs::any() };
    let mut terminal = ratatui::try_init()?;
    let result = runtime.block_on(interactive(
        &mut terminal,
        &version,
        &glyphs,
        vt,
        demo,
        rescue,
    ));
    ratatui::restore();
    result
}

/// Loads the console palette and font, as the console does.
fn set_up_vt() -> Glyphs {
    print!("{}", palette::vt_palette());
    let _ = std::io::stdout().flush();
    let path = Path::new("/usr/lib/kbd/consolefonts/crustates-console-16.psfu");
    let loaded = std::process::Command::new("setfont")
        .arg(path)
        .status()
        .is_ok_and(|s| s.success());
    match std::fs::read(path).ok().and_then(|b| Psf::parse(&b).ok()) {
        Some(font) if loaded => Glyphs::of(&font),
        _ => Glyphs::ascii(),
    }
}

/// `answers.toml` from a stick labelled `CRUSTANS` or from QEMU, copied out.
async fn answers_stick() -> Option<PathBuf> {
    let _ = system::run("modprobe", &["qemu_fw_cfg"]).await;
    if Path::new(ANSWERS_FW_CFG).exists() {
        let copy = PathBuf::from("/run/crust-installer/answers.toml");
        std::fs::create_dir_all("/run/crust-installer").ok()?;
        return std::fs::copy(ANSWERS_FW_CFG, &copy).is_ok().then_some(copy);
    }
    if !Path::new(ANSWERS_LABEL).exists() {
        return None;
    }
    let at = "/run/crust-installer/answers";
    std::fs::create_dir_all(at).ok()?;
    system::run("mount", &["-o", "ro", ANSWERS_LABEL, at])
        .await
        .ok()?;
    let copy = PathBuf::from("/run/crust-installer/answers.toml");
    let copied = std::fs::copy(Path::new(at).join("answers.toml"), &copy).is_ok();
    let _ = system::run("umount", &[at]).await;
    copied.then_some(copy)
}

fn payload() -> anyhow::Result<String> {
    if let Ok(p) = std::env::var("CRUST_PAYLOAD") {
        return Ok(p);
    }
    let text = std::fs::read_to_string(PAYLOAD)
        .with_context(|| format!("reading {PAYLOAD}: this is not install media"))?;
    Ok(text.trim().to_owned())
}

/// Prints a line where a person (tty1) and a test (the serial port) see it.
fn say(line: &str) {
    println!("{line}");
    if let Ok(mut kmsg) = std::fs::OpenOptions::new().write(true).open("/dev/kmsg") {
        let _ = writeln!(kmsg, "<2>{line}");
    }
}

/// Installs from an answers file, without questions.
async fn unattended(path: &Path) -> anyhow::Result<()> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let answers = Answers::parse(&text)
        .map_err(anyhow::Error::msg)
        .context("the answers file")?;
    let disks: Vec<Disk> = system::disks().await;
    let sizes: Vec<(String, u64)> = disks
        .iter()
        .filter(|d| !d.install_media)
        .map(|d| (d.path.clone(), d.size))
        .collect();
    if let Err(e) = answers.check(&sizes) {
        say(&format!("CRUST-INSTALL: FAIL {e}"));
        bail!(e);
    }
    let secrets = PathBuf::from(SECRETS);
    let (token, pin) = system::make_secrets(&secrets)
        .await
        .map_err(anyhow::Error::msg)?;
    // Nobody is at the screen: these lines are the only place they appear.
    say(&format!("CRUST-INSTALL-TOKEN: {token}"));
    say(&format!("CRUST-INSTALL-PIN: {pin}"));
    let steps = install::steps(&answers);
    let (tx, mut rx) = mpsc::channel(256);
    let task = tokio::spawn(install::install(answers.clone(), payload()?, secrets, tx));
    let mut outcome = Err("the install stopped".to_owned());
    while let Some(done) = rx.recv().await {
        match done {
            Done::Progress(i, phase) => say(&format!(
                "crust-installer: {:?} {}",
                phase,
                steps.get(i).map_or("", String::as_str)
            )),
            // On the serial console too: a long write would look like a stall.
            Done::Log(line) => say(&format!("crust-installer: {line}")),
            Done::Installed(result) => {
                outcome = result;
                break;
            }
            _ => {}
        }
    }
    let _ = task.await;
    match outcome {
        Ok(()) => {
            say("CRUST-INSTALL: DONE");
            let action = if answers.after == "poweroff" {
                "poweroff"
            } else {
                "reboot"
            };
            system::run("systemctl", &[action])
                .await
                .map_err(anyhow::Error::msg)?;
            Ok(())
        }
        Err(e) => {
            say(&format!("CRUST-INSTALL: FAIL {e}"));
            bail!(e)
        }
    }
}

fn read_keys() -> mpsc::Receiver<Input> {
    let (tx, rx) = mpsc::channel(32);
    std::thread::spawn(move || {
        while let Ok(event) = event::read() {
            let Event::Key(k) = event else { continue };
            if k.kind != KeyEventKind::Press {
                continue;
            }
            let input = match k.code {
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => Input::Quit,
                KeyCode::Char(c) => Input::Key(Key::Char(c)),
                KeyCode::Enter => Input::Key(Key::Enter),
                KeyCode::Esc => Input::Key(Key::Esc),
                KeyCode::Up => Input::Key(Key::Up),
                KeyCode::Down => Input::Key(Key::Down),
                KeyCode::Left => Input::Key(Key::Left),
                KeyCode::Right => Input::Key(Key::Right),
                KeyCode::Tab => Input::Key(Key::Tab),
                KeyCode::Backspace => Input::Key(Key::Backspace),
                _ => continue,
            };
            if tx.blocking_send(input).is_err() {
                break;
            }
        }
    });
    rx
}

/// The facts of the demo: a machine like the design kit's.
fn demo_facts() -> Facts {
    let disk = |path: &str, size, model: &str, transport: &str, contents: &str| Disk {
        path: path.into(),
        size,
        model: model.into(),
        transport: transport.into(),
        contents: contents.into(),
        install_media: false,
    };
    Facts {
        arch: "amd64".into(),
        uefi: true,
        cpu_flags: "vmx".into(),
        cpu: "8 cores (i7-12700T)".into(),
        iommu_groups: 24,
        memory: 64 << 30,
        tpm: Some("2.0".into()),
        secure_boot: Some(true),
        crustates_key: false,
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
            disk(
                "/dev/sdd",
                8_001_563_222_016,
                "Seagate IronWolf",
                "SATA",
                "has data: 2 partitions (ntfs)",
            ),
        ],
        ports: (2, 1),
        ..Facts::default()
    }
}

#[allow(clippy::too_many_lines, reason = "one arm per effect")]
async fn interactive(
    terminal: &mut ratatui::DefaultTerminal,
    version: &str,
    glyphs: &Glyphs,
    vt: bool,
    demo: bool,
    rescue: bool,
) -> anyhow::Result<()> {
    let facts = if demo {
        demo_facts()
    } else {
        Facts::read(system::disks().await)
    };
    let port = if demo {
        "enp3s0".to_owned()
    } else {
        system::first_port()
    };
    let mut app = App::new(version, facts, &port);
    let mut keys = read_keys();
    let (tx, mut dones) = mpsc::channel::<Done>(256);
    let mut effects = Vec::new();
    if rescue {
        app.step = Step::Rescue;
        effects.push(Effect::FindInstalls);
    }
    let mut ticks = tokio::time::interval(Duration::from_secs(1));
    let mut found: Vec<rescue::Install> = Vec::new();
    loop {
        for effect in effects.drain(..) {
            let tx = tx.clone();
            match effect {
                Effect::Keymap(name) if !demo => {
                    tokio::spawn(async move { system::keymap(&name).await });
                }
                Effect::MakeSecrets if demo => {
                    let _ = tx
                        .send(Done::Secrets(Ok((
                            "crt_K7Q4M-2MRXV-9DPAZ-H3T8W".into(),
                            "482913075".into(),
                        ))))
                        .await;
                }
                Effect::MakeSecrets => {
                    tokio::spawn(async move {
                        tx.send(Done::Secrets(
                            system::make_secrets(Path::new(SECRETS)).await,
                        ))
                        .await
                    });
                }
                Effect::TestNetwork => {
                    let port = app.net[5].clone();
                    tokio::spawn(async move {
                        tx.send(Done::Tested(system::test_network(&port).await))
                            .await
                    });
                }
                Effect::Install(answers) => {
                    let answers = *answers;
                    app.progress = install::steps(&answers)
                        .into_iter()
                        .map(|s| (s, app::Phase::Next))
                        .collect();
                    if demo {
                        let _ = tx.send(Done::Log("demo: nothing is written".into())).await;
                        let _ = tx.send(Done::Installed(Ok(()))).await;
                    } else {
                        match payload() {
                            Ok(payload) => {
                                tokio::spawn(install::install(
                                    answers,
                                    payload,
                                    PathBuf::from(SECRETS),
                                    tx,
                                ));
                            }
                            Err(e) => app.error = Some(e.to_string()),
                        }
                    }
                }
                Effect::Reboot { firmware } if !demo => {
                    tokio::spawn(async move { system::reboot(firmware).await });
                }
                Effect::Reboot { .. } => return Ok(()),
                Effect::FindInstalls if !demo => {
                    found = rescue::find().await;
                    let lines = found
                        .iter()
                        .map(|i| {
                            let versions: Vec<String> = i
                                .deployments
                                .iter()
                                .map(|(v, default)| {
                                    if *default {
                                        format!("{v}, boots next")
                                    } else {
                                        v.clone()
                                    }
                                })
                                .collect();
                            format!("{}: {}", i.root, versions.join(" · "))
                        })
                        .collect();
                    app.done(Done::Installs(lines));
                }
                Effect::BootPrevious | Effect::ResetNetwork if !demo => {
                    if let Some(install) = found.first().cloned() {
                        let reset = matches!(effect, Effect::ResetNetwork);
                        tokio::spawn(async move {
                            let result = if reset {
                                rescue::reset_network(&install).await
                            } else {
                                rescue::boot_previous(&install).await
                            };
                            tx.send(Done::Rescued(result)).await
                        });
                    }
                }
                Effect::ListPools if !demo => {
                    tokio::spawn(
                        async move { tx.send(Done::Rescued(Ok(rescue::pools().await))).await },
                    );
                }
                _ => {}
            }
        }
        terminal.draw(|frame| {
            let buf = frame.buffer_mut();
            screens::draw(&app, buf);
            glyphs.apply(buf);
            if !vt {
                palette::to_rgb(buf);
            }
        })?;
        tokio::select! {
            Some(input) = keys.recv() => match input {
                Input::Quit if demo => return Ok(()),
                Input::Quit => {}
                Input::Key(key) => effects = app.key(key),
            },
            Some(done) = dones.recv() => app.done(done),
            _ = ticks.tick() => {}
        }
    }
}
