//! `cargo xtask molt`: build, boot and test the Molt OS image.
//!
//! - `build [--test]`: builds `localhost/crustates/molt:dev` (or `:dev-test`,
//!   which adds the selftest service)
//! - `run [--test]`: turns the built image into a qcow2 and boots it with the
//!   serial console on this terminal; the API is on `localhost:18080`
//! - `test`: builds the test image, boots it headless, and waits for
//!   `CRUST-SELFTEST: PASS` on the serial console
//!
//! `MOLT_ACCEL=tcg` boots without KVM (slow); see `molt/tools/boot-vm`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};

use crate::root;

/// bootc-image-builder, pinned by digest; Renovate bumps it.
const IMAGE_BUILDER: &str = "quay.io/centos-bootc/bootc-image-builder@sha256:b858892139e0779d48869a4dd8cbbc4e257da49f5a4b502824a55d221ed65a55";
const TOOLS_IMAGE: &str = "localhost/crustates/molt-tools:dev";
/// Host port the guest's API (port 8080) is forwarded to.
const API_PORT: u16 = 18080;
/// How long `molt test` waits for the selftest result.
const TEST_TIMEOUT: Duration = Duration::from_mins(15);

pub fn main(args: &[String]) -> anyhow::Result<()> {
    let test = args.iter().any(|a| a == "--test");
    match args.first().map(String::as_str) {
        Some("build") => build(test),
        Some("run") => run(test),
        Some("test") => self_test(),
        _ => bail!("usage: cargo xtask molt <build|run|test> [--test]"),
    }
}

fn image(test: bool) -> &'static str {
    if test {
        "localhost/crustates/molt:dev-test"
    } else {
        "localhost/crustates/molt:dev"
    }
}

fn build(test: bool) -> anyhow::Result<()> {
    let target = if test { "molt-test" } else { "molt" };
    let version = format!("VERSION={}", env!("CARGO_PKG_VERSION"));
    podman(&[
        "build",
        "-f",
        "molt/Containerfile",
        "--target",
        target,
        "--build-arg",
        &version,
        "-t",
        image(test),
        ".",
    ])
}

fn run(test: bool) -> anyhow::Result<()> {
    let out = disk(test)?;
    build_tools()?;
    println!("API on http://localhost:{API_PORT} once the VM is up; Ctrl-A X stops the VM");
    podman(&[
        "run",
        "--rm",
        "-it",
        "--device",
        "/dev/kvm",
        "--security-opt",
        "label=disable",
        "-e",
        "MOLT_ACCEL",
        "-p",
        &format!("{API_PORT}:8080"),
        "-v",
        &format!("{}:/vm", out.display()),
        TOOLS_IMAGE,
        "/vm/qcow2/disk.qcow2",
    ])
}

fn self_test() -> anyhow::Result<()> {
    build(true)?;
    let out = disk(true)?;
    build_tools()?;
    let log = out.join("serial.log");
    std::fs::write(&log, "").context("creating the serial log")?;
    let name = format!("molt-test-{}", std::process::id());
    podman(&[
        "run",
        "-d",
        "--name",
        &name,
        "--device",
        "/dev/kvm",
        "--security-opt",
        "label=disable",
        "-e",
        "MOLT_ACCEL",
        "-p",
        &format!("{API_PORT}:8080"),
        "-v",
        &format!("{}:/vm", out.display()),
        TOOLS_IMAGE,
        "/vm/qcow2/disk.qcow2",
        "/vm/serial.log",
    ])?;
    let result = wait_for_selftest(&log).and_then(|()| check_api());
    // Keep the VM's console output even when the test fails.
    let _ = podman(&["rm", "-f", &name]);
    println!("serial log: {}", log.display());
    result
}

/// Waits for the selftest's result line on the serial console.
fn wait_for_selftest(log: &Path) -> anyhow::Result<()> {
    let start = Instant::now();
    while start.elapsed() < TEST_TIMEOUT {
        let serial = std::fs::read_to_string(log).unwrap_or_default();
        if let Some(line) = serial.lines().find(|l| l.contains("CRUST-SELFTEST: ")) {
            if line.contains("CRUST-SELFTEST: PASS") {
                println!("{line}");
                return Ok(());
            }
            bail!("selftest failed: {line}");
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    bail!("no selftest result within {} s", TEST_TIMEOUT.as_secs())
}

/// Checks the API from outside the VM, through the forwarded port.
fn check_api() -> anyhow::Result<()> {
    let base = format!("http://localhost:{API_PORT}");
    let health = curl(&format!("{base}/healthz"))?;
    if !health.contains("\"status\":\"ok\"") {
        bail!("unexpected /healthz answer: {health}");
    }
    let nodes = curl(&format!("{base}/v1/nodes"))?;
    if !nodes.contains("\"type\":\"Ready\",\"status\":\"True\"") {
        bail!("no Ready node in /v1/nodes: {nodes}");
    }
    println!("/healthz and /v1/nodes answer through port {API_PORT}; the node is Ready");
    Ok(())
}

fn curl(url: &str) -> anyhow::Result<String> {
    let output = Command::new("curl")
        .args(["-fsS", "--retry", "5", "--retry-all-errors", url])
        .output()?;
    if !output.status.success() {
        bail!(
            "GET {url} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8(output.stdout)?)
}

/// Makes `target/molt/<image>/qcow2/disk.qcow2` from the built image.
fn disk(test: bool) -> anyhow::Result<PathBuf> {
    let out = root()
        .join("target/molt")
        .join(if test { "dev-test" } else { "dev" });
    std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    podman(&[
        "run",
        "--rm",
        "--privileged",
        "--pull=missing",
        "--security-opt",
        "label=type:unconfined_t",
        "-v",
        "/var/lib/containers/storage:/var/lib/containers/storage",
        "-v",
        &format!("{}:/output", out.display()),
        IMAGE_BUILDER,
        "--type",
        "qcow2",
        image(test),
    ])?;
    Ok(out)
}

fn build_tools() -> anyhow::Result<()> {
    podman(&["build", "-q", "-t", TOOLS_IMAGE, "molt/tools"])
}

/// Runs podman from the repository root. bootc-image-builder needs the image
/// in root's container storage, so on Linux a non-root user goes through sudo;
/// on macOS the podman machine must be rootful (`podman machine set --rootful`).
fn podman(args: &[&str]) -> anyhow::Result<()> {
    let mut command = if cfg!(target_os = "linux") && !is_root() {
        let mut sudo = Command::new("sudo");
        sudo.arg("podman");
        sudo
    } else {
        Command::new("podman")
    };
    println!("> podman {}", args.join(" "));
    let status = command
        .args(args)
        .current_dir(root())
        .status()
        .context("running podman")?;
    if !status.success() {
        bail!("podman {} failed", args.first().unwrap_or(&""));
    }
    Ok(())
}

fn is_root() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .is_ok_and(|o| o.stdout.starts_with(b"0\n"))
}
