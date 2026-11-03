//! The live system the installer runs on: its disks, keyboard and network,
//! and the secrets it makes.

use std::path::Path;
use std::time::Duration;

use tokio::process::Command;

use crate::disks;
use crate::hardware::Disk;

/// Runs a program and returns its standard output.
pub async fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let output = tokio::time::timeout(
        Duration::from_secs(60),
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

/// The device the live system was started from, if it is a disk (a USB
/// stick rather than a CD).
async fn install_media() -> Option<String> {
    let source = run("findmnt", &["-n", "-o", "SOURCE", "/run/initramfs/live"])
        .await
        .ok()?;
    let source = source.trim();
    (!source.is_empty()).then(|| source.to_owned())
}

/// The machine's disks.
pub async fn disks() -> Vec<Disk> {
    let json = run("lsblk", &disks::LSBLK).await.unwrap_or_default();
    disks::parse(&json, install_media().await.as_deref())
}

/// The first wired port with a link, else the first wired port.
#[must_use]
pub fn first_port() -> String {
    let mut ports: Vec<(String, bool)> = std::fs::read_dir("/sys/class/net")
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.path().join("device").exists() && !e.path().join("wireless").exists())
        .map(|e| {
            let carrier =
                std::fs::read_to_string(e.path().join("carrier")).is_ok_and(|c| c.trim() == "1");
            (e.file_name().to_string_lossy().into_owned(), carrier)
        })
        .collect();
    ports.sort();
    ports
        .iter()
        .find(|(_, up)| *up)
        .or(ports.first())
        .map(|(n, _)| n.clone())
        .unwrap_or_default()
}

/// Makes the admin token and the recovery PIN; their hashes go into `dir`
/// for the install to copy, the secrets come back to be shown once.
pub async fn make_secrets(dir: &Path) -> Result<(String, String), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let token_file = dir.join("admin-token");
    let pin_file = dir.join("recovery-pin");
    let output = |var: &str, file: &Path, command: &str| {
        let mut c = Command::new("crust-api");
        c.arg(command).env(var, file).kill_on_drop(true);
        c
    };
    let token = output("CRUST_ADMIN_TOKEN_FILE", &token_file, "new-admin-token")
        .output()
        .await
        .map_err(|e| format!("crust-api: {e}"))?;
    let pin = output("CRUST_RECOVERY_PIN_FILE", &pin_file, "new-recovery-pin")
        .output()
        .await
        .map_err(|e| format!("crust-api: {e}"))?;
    if !token.status.success() || !pin.status.success() {
        return Err("crust-api could not make the token and PIN".into());
    }
    Ok((
        String::from_utf8_lossy(&token.stdout).trim().to_owned(),
        String::from_utf8_lossy(&pin.stdout).trim().to_owned(),
    ))
}

/// Loads a keyboard layout on the console.
pub async fn keymap(name: &str) -> Result<(), String> {
    run("loadkeys", &[name]).await.map(|_| ())
}

/// Tests the network: link, address, gateway, DNS, the registry.
pub async fn test_network(interface: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let carrier = std::fs::read_to_string(format!("/sys/class/net/{interface}/carrier"))
        .is_ok_and(|c| c.trim() == "1");
    lines.push(if carrier {
        format!("✓ link      {interface} up")
    } else {
        format!("✗ link      {interface}: no cable?")
    });
    let route = run("ip", &["-4", "route", "show", "default"])
        .await
        .unwrap_or_default();
    let gateway = route
        .split_whitespace()
        .skip_while(|w| *w != "via")
        .nth(1)
        .map(str::to_owned);
    lines.push(match &gateway {
        Some(gw) => match run("ping", &["-c", "1", "-W", "2", gw]).await {
            Ok(_) => format!("✓ gateway   {gw} answers"),
            Err(_) => format!("✗ gateway   {gw} does not answer"),
        },
        None => "✗ gateway   none: no address yet?".into(),
    });
    let dns = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host("ghcr.io:443"),
    )
    .await;
    lines.push(if matches!(dns, Ok(Ok(_))) {
        "✓ DNS       ghcr.io resolves".into()
    } else {
        "✗ DNS       ghcr.io does not resolve".into()
    });
    let tcp = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::TcpStream::connect("ghcr.io:443"),
    )
    .await;
    lines.push(if matches!(tcp, Ok(Ok(_))) {
        "✓ registry  ghcr.io:443 reachable".into()
    } else {
        "! registry  ghcr.io:443 not reachable: updates wait until it is".into()
    });
    lines
}

/// Reboots, or into the firmware settings.
pub async fn reboot(firmware: bool) -> Result<(), String> {
    let args: &[&str] = if firmware {
        &["reboot", "--firmware-setup"]
    } else {
        &["reboot"]
    };
    run("systemctl", args).await.map(|_| ())
}
