//! `cargo xtask molt`: build, boot and test the Molt OS image.
//!
//! - `build [--test]`: builds `localhost/crustates/molt:dev` (or `:dev-test`,
//!   which adds the selftest service)
//! - `run [--test]`: turns the built image into a qcow2 and boots it with the
//!   serial console on this terminal; the API is on `localhost:18080`
//! - `test [--upgrade]`: builds the test image, boots it headless, and waits
//!   for `CRUST-SELFTEST: PASS` on the serial console. With `--upgrade` it
//!   then upgrades the VM to a second image from a local registry, and then
//!   to an image whose health check fails, which greenboot must roll back
//! - `keygen`: makes the release module-signing key pair, once, for the
//!   `release` environment secrets
//! - `images [IMAGE]`: makes the release disk images (qcow2, raw.zst) from
//!   IMAGE, by default the built `molt:dev`, into `target/molt/images/dist`
//! - `iso [IMAGE]`: makes the install ISO for IMAGE (by default the built
//!   `molt:dev`; a registry image by digest keeps the digests the release
//!   signed) into `target/molt/iso`
//! - `test-install`: builds the test image and its ISO, installs it on an
//!   empty VM with an answers file and a mirrored pool, boots the installed
//!   disk and waits for the selftest
//!
//! `MOLT_ACCEL=tcg` boots without KVM (slow); see `molt/tools/boot-vm`.
//! `MOLT_TEST_SECS` lengthens how long a test waits for each selftest.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};

use crate::root;

/// The local registry `molt test --upgrade` serves update images from,
/// pinned by digest; Renovate bumps it.
const REGISTRY_IMAGE: &str = "ghcr.io/distribution/distribution:3.1.2@sha256:d106962e6fe3fa69c178cec77eeeee5edb626b8df6535b7bd6b95e658a0a5ca9";
/// Host port of that registry; the guest reaches it as 10.0.2.2:5000.
const REGISTRY_PORT: u16 = 15000;
/// How long an upgrade (download and stage) may take.
const UPGRADE_TIMEOUT_SECS: u32 = 1800;
/// image-builder, for the install ISO (its `generic-iso` type, under the
/// name every version knows), pinned by digest; Renovate bumps it.
const ISO_BUILDER: &str = "ghcr.io/osbuild/image-builder:v89.0.0@sha256:27d349fee2944029a9e47f5b8db5f4bf68392bc7878909d0c53ad8ddadb1a412";
/// The install media image `molt iso` builds.
const INSTALLER_IMAGE: &str = "localhost/crustates/molt-installer:dev";
/// How long installing from the ISO may take.
const INSTALL_TIMEOUT: Duration = Duration::from_mins(45);
/// bootc-image-builder, pinned by digest; Renovate bumps it.
const IMAGE_BUILDER: &str = "quay.io/centos-bootc/bootc-image-builder@sha256:b858892139e0779d48869a4dd8cbbc4e257da49f5a4b502824a55d221ed65a55";
const TOOLS_IMAGE: &str = "localhost/crustates/molt-tools:dev";
/// Host port the guest's API (port 8080) is forwarded to.
const API_PORT: u16 = 18080;
/// How long `molt test` waits for the selftest result.
const TEST_TIMEOUT: Duration = Duration::from_mins(15);
/// [`TEST_TIMEOUT`], or `MOLT_TEST_SECS`.
fn test_timeout() -> Duration {
    std::env::var("MOLT_TEST_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .map_or(TEST_TIMEOUT, Duration::from_secs)
}

/// How long the serial console may stay silent before `molt test` calls the
/// VM stalled. Hosted runners occasionally freeze a nested VM in early boot.
const STALL_TIMEOUT: Duration = Duration::from_mins(5);

pub fn main(args: &[String]) -> anyhow::Result<()> {
    let test = args.iter().any(|a| a == "--test");
    let upgrade = args.iter().any(|a| a == "--upgrade");
    match args.first().map(String::as_str) {
        Some("build") => build(test),
        Some("run") => run(test),
        Some("test") => self_test(upgrade),
        Some("keygen") => keygen(),
        Some("images") => images(args.get(1).map(String::as_str)),
        Some("iso") => iso(args.get(1).map(String::as_str)).map(|_| ()),
        Some("test-install") => test_install(),
        _ => bail!(
            "usage: cargo xtask molt <build|run|test|keygen|images|iso|test-install> [--test] [--upgrade]"
        ),
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
    build_image(target, image(test), env!("CARGO_PKG_VERSION"))
}

/// Builds `target` of the Containerfile as `tag`, with `version` in its
/// os-release and version label.
fn build_image(target: &str, tag: &str, version: &str) -> anyhow::Result<()> {
    let version = format!("VERSION={version}");
    let (key, cert) = signing_key()?;
    let key = format!("id=module-signing-key,src={}", key.display());
    let cert = format!("id=module-signing-cert,src={}", cert.display());
    podman(&[
        "build",
        "-f",
        "molt/Containerfile",
        "--target",
        target,
        "--build-arg",
        &version,
        "--secret",
        &key,
        "--secret",
        &cert,
        "-t",
        tag,
        ".",
    ])
}

/// The module-signing key and certificate: `MOLT_SIGNING_KEY` (PEM) and
/// `MOLT_SIGNING_CERT` (DER) when set, as in release builds. Otherwise a
/// throwaway pair in `target/molt/throwaway-key`, made once and reused.
fn signing_key() -> anyhow::Result<(PathBuf, PathBuf)> {
    if let (Some(key), Some(cert)) = (
        std::env::var_os("MOLT_SIGNING_KEY"),
        std::env::var_os("MOLT_SIGNING_CERT"),
    ) {
        return Ok((key.into(), cert.into()));
    }
    let dir = root().join("target/molt/throwaway-key");
    let (key, cert) = (dir.join("key.pem"), dir.join("cert.der"));
    if !key.exists() || !cert.exists() {
        make_key(
            &dir,
            "Crustates module signing (throwaway, not for release)",
        )?;
    }
    println!(
        "THROWAWAY module-signing key {}: the modules load on machines without Secure Boot only. \
         Set MOLT_SIGNING_KEY and MOLT_SIGNING_CERT to sign with the real key.",
        key.display()
    );
    Ok((key, cert))
}

/// Makes the release key pair in `target/molt/release-key` and prints how to
/// store it as secrets. Refuses to overwrite an existing pair.
fn keygen() -> anyhow::Result<()> {
    let dir = root().join("target/molt/release-key");
    if dir.join("key.pem").exists() {
        bail!("{} already holds a key; move it away first", dir.display());
    }
    let (key, cert) = make_key(&dir, "Crustates module signing")?;
    let fingerprint = Command::new("openssl")
        .args([
            "x509",
            "-inform",
            "DER",
            "-noout",
            "-fingerprint",
            "-sha256",
            "-in",
        ])
        .arg(&cert)
        .output()
        .context("reading the certificate fingerprint")?;
    print!("{}", String::from_utf8_lossy(&fingerprint.stdout));
    println!(
        "Store the pair in the release environment, then delete {} or keep it offline:\n  \
         gh secret set MOLT_SIGNING_KEY --env release < {}\n  \
         base64 < {} | gh secret set MOLT_SIGNING_CERT --env release",
        dir.display(),
        key.display(),
        cert.display()
    );
    Ok(())
}

/// Makes a 4096-bit RSA key (`key.pem`) and a self-signed code-signing
/// certificate (`cert.der`) in `dir`, readable only by this user.
fn make_key(dir: &Path, subject: &str) -> anyhow::Result<(PathBuf, PathBuf)> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let (key, cert) = (dir.join("key.pem"), dir.join("cert.der"));
    let status = Command::new("sh")
        .arg("-c")
        // umask: the private key must not be readable by other users.
        .arg("umask 077 && exec openssl \"$@\"")
        .arg("openssl")
        .args([
            "req", "-new", "-x509", "-newkey", "rsa:4096", "-nodes", "-days", "36500",
        ])
        .args(["-outform", "DER", "-subj", &format!("/CN={subject}/")])
        .args(["-addext", "extendedKeyUsage=codeSigning"])
        .arg("-keyout")
        .arg(&key)
        .arg("-out")
        .arg(&cert)
        .status()
        .context("running openssl")?;
    if !status.success() {
        bail!("openssl could not make the signing key");
    }
    Ok((key, cert))
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

fn self_test(upgrade: bool) -> anyhow::Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    let (b, bad) = (format!("{version}+b"), format!("{version}+bad"));
    build(true)?;
    if upgrade {
        // Small on top of the first: only the version layers differ.
        build_image("molt-test", "localhost/crustates/molt:dev-test-b", &b)?;
        build_image(
            "molt-test-bad",
            "localhost/crustates/molt:dev-test-bad",
            &bad,
        )?;
    }
    let out = disk(true)?;
    build_tools()?;
    let log = out.join("serial.log");
    std::fs::write(&log, "").context("creating the serial log")?;
    // One pod: the VM's user network reaches the registry on the pod's
    // localhost, which the guest sees as 10.0.2.2.
    let pod = format!("molt-test-{}", std::process::id());
    podman(&[
        "pod",
        "create",
        "--name",
        &pod,
        "-p",
        &format!("{API_PORT}:8080"),
        "-p",
        &format!("{REGISTRY_PORT}:5000"),
    ])?;
    let result = (|| {
        if upgrade {
            podman(&[
                "run",
                "-d",
                "--pod",
                &pod,
                "--name",
                &format!("{pod}-registry"),
                REGISTRY_IMAGE,
            ])?;
        }
        podman(&[
            "run",
            "-d",
            "--pod",
            &pod,
            "--name",
            &format!("{pod}-vm"),
            "--device",
            "/dev/kvm",
            "--security-opt",
            "label=disable",
            "-e",
            "MOLT_ACCEL",
            "-v",
            &format!("{}:/vm", out.display()),
            TOOLS_IMAGE,
            "/vm/qcow2/disk.qcow2",
            "/vm/serial.log",
        ])?;
        let mut serial = Serial { log: &log, from: 0 };
        let token = serial.wait_for_pass(version, false)?;
        screenshots(&format!("{pod}-vm"), &out)?;
        let api = Api::new(token);
        api.check()?;
        if upgrade {
            upgrade_test(&api, &mut serial, version, &b, &bad)?;
        }
        Ok(())
    })();
    // Keep the VM's console output even when the test fails.
    let _ = podman(&["pod", "rm", "-f", &pod]);
    println!("serial log: {}", log.display());
    result
}

/// Screenshots of tty1 as PNGs next to the serial log: the first-boot
/// screen, then, after enter, the home screen of the console.
fn screenshots(vm: &str, out: &Path) -> anyhow::Result<()> {
    let shot = |name: &str| -> anyhow::Result<()> {
        let ppm = out.join(format!("{name}.ppm"));
        let _ = std::fs::remove_file(&ppm);
        monitor(vm, &format!("screendump /vm/{name}.ppm"))?;
        let image = std::fs::read(&ppm).with_context(|| format!("reading {}", ppm.display()))?;
        let png = out.join(format!("{name}.png"));
        std::fs::write(&png, crate::console::ppm_to_png(&image)?)
            .with_context(|| format!("writing {}", png.display()))?;
        let _ = std::fs::remove_file(&ppm);
        println!("screenshot: {}", png.display());
        Ok(())
    };
    shot("tty1-first-boot")?;
    monitor(vm, "sendkey ret")?;
    // The console reads the machine again at once, then draws the home screen.
    std::thread::sleep(Duration::from_secs(5));
    shot("tty1-home")
}

/// Sends a command to the QEMU monitor of the VM container `vm`.
fn monitor(vm: &str, command: &str) -> anyhow::Result<()> {
    // The tools image has no netcat; bash talks TCP. The pause lets QEMU
    // finish the command before the connection closes.
    let script = "exec 3<>/dev/tcp/127.0.0.1/4444 && printf '%s\\n' \"$1\" >&3 && sleep 2";
    podman(&["exec", vm, "bash", "-c", script, "monitor", command])
}

/// A → B (healthy) → B-bad (fails its health check), which greenboot must
/// roll back to B and the agent must never stage again.
fn upgrade_test(api: &Api, serial: &mut Serial, a: &str, b: &str, bad: &str) -> anyhow::Result<()> {
    let node = api.node()?;
    push("localhost/crustates/molt:dev-test-b")?;
    serial.skip_to_end();
    api.upgrade(&node, b)?;
    serial.wait_for_pass(b, false)?;
    api.audited_upgrade()?;
    let molt = api.molt(&node)?;
    expect(&molt, "/booted/version", b)?;
    expect(&molt, "/rollback/version", a)?;
    // Picked by the first check: the test registry has no :stable.
    expect(&molt, "/channel", "beta")?;
    if molt["automatic_rollback"] != true {
        bail!("automatic rollback is off after the upgrade: {molt}");
    }

    push("localhost/crustates/molt:dev-test-bad")?;
    serial.skip_to_end();
    api.upgrade(&node, bad)?;
    // B-bad fails its health check and greenboot rolls back: B boots again.
    serial.wait_for_pass(b, true)?;
    api.audited_upgrade()?;
    let molt = api.molt(&node)?;
    expect(&molt, "/booted/version", b)?;
    if molt["bad_digests"].as_array().is_none_or(Vec::is_empty) {
        bail!("the failed image was not marked bad: {molt}");
    }
    let refused = api.call(
        "POST",
        &format!("/v1/nodes/{node}/molt/upgrade"),
        r#"{"reboot":false}"#,
    )?;
    if !refused.contains("failed its health check") {
        bail!("the agent did not refuse the bad image again: {refused}");
    }
    println!("rolled back from {bad} to {b}; {bad} is marked bad and refused");
    Ok(())
}

/// Pushes `image` to the test registry as its `beta` channel.
fn push(image: &str) -> anyhow::Result<()> {
    podman(&[
        "push",
        "--tls-verify=false",
        image,
        &format!("docker://localhost:{REGISTRY_PORT}/crustates/molt:beta"),
    ])
}

fn expect(json: &serde_json::Value, pointer: &str, want: &str) -> anyhow::Result<()> {
    match json.pointer(pointer).and_then(serde_json::Value::as_str) {
        Some(got) if got == want => Ok(()),
        got => bail!("{pointer} is {got:?}, want {want:?}: {json}"),
    }
}

/// The serial log of the VM, read from `from` on.
struct Serial<'a> {
    log: &'a Path,
    from: usize,
}

impl Serial<'_> {
    /// Ignores everything logged so far.
    fn skip_to_end(&mut self) {
        self.from =
            std::fs::metadata(self.log).map_or(0, |m| usize::try_from(m.len()).unwrap_or(0));
    }

    /// Waits for `CRUST-SELFTEST: PASS <version>` and returns the admin
    /// token printed before it. A failed selftest ends the wait unless
    /// `tolerate_fail` (a boot that is meant to fail). Gives up early when
    /// the console stays silent for `STALL_TIMEOUT`: the VM has stalled.
    fn wait_for_pass(&mut self, version: &str, tolerate_fail: bool) -> anyhow::Result<String> {
        let start = Instant::now();
        let (mut seen, mut last_output) = (0, Instant::now());
        while start.elapsed() < test_timeout() {
            let serial = std::fs::read(self.log).unwrap_or_default();
            let new = serial.get(self.from..).unwrap_or_default();
            match selftest_result(new, version) {
                Some(Ok((token, end))) => {
                    println!("CRUST-SELFTEST: PASS {version}");
                    self.from += end;
                    return Ok(token);
                }
                Some(Err(line)) if !tolerate_fail => bail!("selftest failed: {line}"),
                _ => {}
            }
            if serial.len() > seen {
                (seen, last_output) = (serial.len(), Instant::now());
            } else if last_output.elapsed() > STALL_TIMEOUT {
                bail!(
                    "the VM stalled: no serial output for {} s (after {} s of waiting for {version})",
                    STALL_TIMEOUT.as_secs(),
                    start.elapsed().as_secs()
                );
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        bail!(
            "no selftest PASS for {version} within {} s",
            test_timeout().as_secs()
        )
    }
}

/// The selftest's result in `serial`: `Ok` with the admin token and the end
/// of the PASS line for `version`, `Err` with a FAIL line, `None` if neither
/// is there yet.
fn selftest_result(serial: &[u8], version: &str) -> Option<Result<(String, usize), String>> {
    // Lossy: firmware and early boot write bytes that are not UTF-8.
    let text = String::from_utf8_lossy(serial);
    let (mut token, mut end) = (String::new(), 0);
    for line in text.split_inclusive('\n') {
        end += line.len();
        let line = line.trim_end();
        if let Some(t) = line.split("CRUST-SELFTEST-TOKEN: ").nth(1) {
            t.trim().clone_into(&mut token);
        } else if let Some(result) = line.split("CRUST-SELFTEST: ").nth(1) {
            if result.strip_prefix("PASS ") == Some(version) {
                // Lossy decoding may have changed lengths before this point;
                // the end only needs to fall after the line.
                return Some(Ok((token, end.min(serial.len()))));
            }
            if result.starts_with("FAIL") {
                return Some(Err(line.to_owned()));
            }
        }
    }
    None
}

/// The API of the VM, through the forwarded port, with the admin token.
struct Api {
    base: String,
    token: String,
}

impl Api {
    fn new(token: String) -> Self {
        Self {
            base: format!("http://localhost:{API_PORT}"),
            token,
        }
    }

    /// Checks the API from outside the VM: open `/healthz`, closed `/v1/`
    /// without the token, and a Ready node with it.
    fn check(&self) -> anyhow::Result<()> {
        let health = curl(&format!("{}/healthz", self.base))?;
        if !health.contains("\"status\":\"ok\"") {
            bail!("unexpected /healthz answer: {health}");
        }
        let anonymous = Command::new("curl")
            .args(["-s", "-o", "/dev/null", "-w", "%{http_code}"])
            .arg(format!("{}/v1/nodes", self.base))
            .output()?;
        if anonymous.stdout != b"401" {
            bail!(
                "/v1/nodes without the admin token answered {}, want 401",
                String::from_utf8_lossy(&anonymous.stdout)
            );
        }
        let nodes = self.call("GET", "/v1/nodes", "")?;
        if !nodes.contains("\"type\":\"Ready\",\"status\":\"True\"") {
            bail!("no Ready node in /v1/nodes: {nodes}");
        }
        println!(
            "/healthz and /v1/nodes answer through port {API_PORT}, /v1 only with the token; the node is Ready"
        );
        Ok(())
    }

    /// Calls the API; returns the body of a success or of an error. Only
    /// GET is retried: a POST may have run although its answer was lost.
    fn call(&self, method: &str, path: &str, body: &str) -> anyhow::Result<String> {
        let mut args = vec!["-sS".to_owned()];
        if method == "GET" {
            args.extend(["--retry".into(), "5".into(), "--retry-all-errors".into()]);
        }
        args.extend([
            "--max-time".into(),
            UPGRADE_TIMEOUT_SECS.to_string(),
            "-X".into(),
            method.into(),
            "-H".into(),
            format!("Authorization: Bearer {}", self.token),
        ]);
        if !body.is_empty() {
            args.extend([
                "-H".into(),
                "Content-Type: application/json".into(),
                "-d".into(),
                body.into(),
            ]);
        }
        args.push(format!("{}{path}", self.base));
        let output = Command::new("curl").args(&args).output()?;
        if !output.status.success() {
            bail!(
                "{method} {path} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(String::from_utf8(output.stdout)?)
    }

    fn json(&self, path: &str) -> anyhow::Result<serde_json::Value> {
        Ok(serde_json::from_str(&self.call("GET", path, "")?)?)
    }

    /// The name of the VM's node.
    fn node(&self) -> anyhow::Result<String> {
        let nodes = self.json("/v1/nodes")?;
        nodes
            .pointer("/items/0/metadata/name")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .with_context(|| format!("no node: {nodes}"))
    }

    fn molt(&self, node: &str) -> anyhow::Result<serde_json::Value> {
        let molt = self.json(&format!("/v1/nodes/{node}"))?["status"]["molt"].take();
        if molt.is_null() {
            bail!("node {node} reports no Molt status");
        }
        Ok(molt)
    }

    /// Asks the node to stage the channel's image and reboot into it.
    fn upgrade(&self, node: &str, version: &str) -> anyhow::Result<()> {
        let path = format!("/v1/nodes/{node}/molt/upgrade");
        let answer = match self.call("POST", &path, r#"{"reboot":true}"#) {
            Ok(answer) => answer,
            // The agent stages and reboots whether or not the answer gets
            // back; on macOS the podman machine's port forward has dropped
            // such long requests. The audit log shows the outcome later.
            Err(e) => {
                println!(
                    "warning: the upgrade request lost its answer ({e}); waiting for the reboot"
                );
                return Ok(());
            }
        };
        let answer: serde_json::Value = serde_json::from_str(&answer)?;
        match answer["message"].as_str() {
            Some(message) if message.starts_with("staged") => {
                println!("upgrade to {version}: {message}");
                Ok(())
            }
            _ => bail!("the upgrade did not stage an image: {answer}"),
        }
    }

    /// Checks that the last upgrade request worked, by the audit log.
    fn audited_upgrade(&self) -> anyhow::Result<()> {
        let log = self.json("/v1/audit?limit=20")?;
        let last = log["items"]
            .as_array()
            .and_then(|items| items.iter().find(|e| e["action"] == "molt.upgrade"));
        match last {
            Some(entry) if entry["outcome"] == "ok" => Ok(()),
            _ => bail!("the audit log has no successful molt.upgrade: {log}"),
        }
    }
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
    image_builder(image(test), &out, &["qcow2"])?;
    Ok(out)
}

/// Makes the release disk images of `image` for this machine's architecture.
fn images(image: Option<&str>) -> anyhow::Result<()> {
    let image = image.unwrap_or(self::image(false));
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => bail!("no Molt images for {other}"),
    };
    let out = root().join("target/molt/images");
    build_tools()?;
    // No installer ISO yet: anaconda-iso cannot find Fedora 45's repos and is
    // deprecated upstream; the ISO comes with Crustates' own installer.
    image_builder(image, &out, &["qcow2", "raw"])?;
    podman(&[
        "run",
        "--rm",
        "--security-opt",
        "label=disable",
        "-v",
        &format!("{}:/out", out.display()),
        "--entrypoint",
        "/usr/local/bin/package-images",
        TOOLS_IMAGE,
        env!("CARGO_PKG_VERSION"),
        arch,
    ])?;
    println!("release files in {}", out.join("dist").display());
    Ok(())
}

/// Runs bootc-image-builder on `image`, writing each of `types` under `out`.
fn image_builder(image: &str, out: &Path, types: &[&str]) -> anyhow::Result<()> {
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let volume = format!("{}:/output", out.display());
    let mut args = vec![
        "run",
        "--rm",
        "--privileged",
        "--pull=missing",
        "--security-opt",
        "label=type:unconfined_t",
        "-v",
        "/var/lib/containers/storage:/var/lib/containers/storage",
        "-v",
        &volume,
        IMAGE_BUILDER,
    ];
    for kind in types {
        args.extend(["--type", kind]);
    }
    args.push(image);
    podman(&args)
}

/// `amd64` or `arm64`.
fn arch() -> anyhow::Result<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("amd64"),
        "aarch64" => Ok("arm64"),
        other => bail!("no Molt images for {other}"),
    }
}

/// Makes the install ISO for `image` (ADR 0019): the image goes in as an
/// OCI layout, the installer image is built FROM it, and image-builder turns
/// that into `target/molt/iso/crustates-VERSION-ARCH.iso`.
fn iso(image: Option<&str>) -> anyhow::Result<PathBuf> {
    let image = image.unwrap_or(self::image(false));
    let out = root().join("target/molt/iso");
    let payload = out.join("payload");
    std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    // Root may own an older payload (sudo podman); the container removes it.
    let (source, preserve) = if image.starts_with("localhost/") {
        (format!("containers-storage:{image}"), None)
    } else {
        (format!("docker://{image}"), Some("--preserve-digests"))
    };
    let mount = format!("{}:/out", out.display());
    let copy = format!(
        "rm -rf /out/payload && skopeo copy {} {source} oci:/out/payload",
        preserve.unwrap_or_default()
    );
    podman(&[
        "run",
        "--rm",
        "--privileged",
        "--security-opt",
        "label=type:unconfined_t",
        "-v",
        "/var/lib/containers/storage:/var/lib/containers/storage",
        "-v",
        &mount,
        "--entrypoint",
        "bash",
        image,
        "-c",
        &copy,
    ])?;
    let molt = format!("MOLT={image}");
    let context = format!("payload={}", payload.display());
    podman(&[
        "build",
        "-f",
        "molt/installer/Containerfile",
        "--build-arg",
        &molt,
        "--build-context",
        &context,
        "-t",
        INSTALLER_IMAGE,
        ".",
    ])?;
    let name = format!("crustates-{}-{}", env!("CARGO_PKG_VERSION"), arch()?);
    let output = format!("{}:/output", out.display());
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
        &output,
        ISO_BUILDER,
        "build",
        "--output-dir",
        "/output",
        "--output-name",
        &name,
        "--bootc-ref",
        INSTALLER_IMAGE,
        "--bootc-default-fs",
        "xfs",
        "bootc-generic-iso",
    ])?;
    let iso = std::fs::read_dir(&out)
        .with_context(|| format!("reading {}", out.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .chain(
            std::fs::read_dir(out.join("bootiso"))
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|e| e.path()),
        )
        .find(|p| p.extension().is_some_and(|e| e == "iso"))
        .context("image-builder made no .iso")?;
    println!("install ISO: {}", iso.display());
    Ok(iso)
}

/// Makes a sparse raw disk of `bytes`.
fn blank_disk(path: &Path, bytes: u64) -> anyhow::Result<()> {
    let file =
        std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    file.set_len(bytes)
        .with_context(|| format!("sizing {}", path.display()))
}

/// Waits for `CRUST-INSTALL: DONE` in `log` and returns the token and PIN
/// the installer printed.
fn wait_for_install(log: &Path) -> anyhow::Result<(String, String)> {
    let start = Instant::now();
    let (mut seen, mut last_output) = (0, Instant::now());
    while start.elapsed() < INSTALL_TIMEOUT {
        let serial = std::fs::read(log).unwrap_or_default();
        let text = String::from_utf8_lossy(&serial);
        let value = |key: &str| {
            text.lines()
                .rev()
                .find_map(|l| l.split(key).nth(1))
                .map(|v| v.trim().to_owned())
        };
        if let Some(fail) = value("CRUST-INSTALL: FAIL") {
            bail!("the install failed: {fail}");
        }
        if text.contains("CRUST-INSTALL: DONE") {
            let token = value("CRUST-INSTALL-TOKEN: ").context("the installer printed no token")?;
            let pin = value("CRUST-INSTALL-PIN: ").context("the installer printed no PIN")?;
            println!("CRUST-INSTALL: DONE after {} s", start.elapsed().as_secs());
            return Ok((token, pin));
        }
        if serial.len() > seen {
            (seen, last_output) = (serial.len(), Instant::now());
        } else if last_output.elapsed() > STALL_TIMEOUT {
            bail!(
                "the installer VM stalled: no serial output for {} s",
                STALL_TIMEOUT.as_secs()
            );
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    bail!(
        "no CRUST-INSTALL: DONE within {} s",
        INSTALL_TIMEOUT.as_secs()
    )
}

/// Installs the test image from its ISO onto an empty VM, with a mirrored
/// pool on two more disks, then boots the installed disk: the selftest must
/// pass, the pool must be there, and the API must take the installer's token.
#[allow(clippy::too_many_lines, reason = "two VM runs, one after the other")]
fn test_install() -> anyhow::Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    build(true)?;
    let iso = iso(Some(image(true)))?;
    build_tools()?;
    let out = root().join("target/molt/install-test");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    blank_disk(&out.join("disk.raw"), 40_000_000_000)?;
    blank_disk(&out.join("pool-1.raw"), 4_000_000_000)?;
    blank_disk(&out.join("pool-2.raw"), 4_000_000_000)?;
    std::fs::write(
        out.join("answers.toml"),
        "disk = \"/dev/vda\"\nerase = true\nhostname = \"install-test\"\nafter = \"poweroff\"\n\n[pool]\nname = \"tank\"\nlayout = \"mirror\"\ndisks = [\"/dev/vdb\", \"/dev/vdc\"]\n",
    )?;
    let iso_dir = iso.parent().context("the ISO has no directory")?;
    let iso_name = iso
        .file_name()
        .context("the ISO has no name")?
        .to_string_lossy()
        .into_owned();
    let vm_mount = format!("{}:/vm", out.display());
    let iso_mount = format!("{}:/iso:ro", iso_dir.display());
    let cdrom = format!("MOLT_CDROM=/iso/{iso_name}");
    let extra = "MOLT_EXTRA_DISKS=/vm/pool-1.raw /vm/pool-2.raw";
    let install_log = out.join("install.log");
    std::fs::write(&install_log, "")?;
    let name = format!("molt-install-{}", std::process::id());
    let installed = (|| {
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
            "-e",
            &cdrom,
            "-e",
            "MOLT_DISK_RW=1",
            "-e",
            extra,
            "-e",
            "MOLT_FW_CFG=name=opt/crustates/answers,file=/vm/answers.toml",
            "-v",
            &vm_mount,
            "-v",
            &iso_mount,
            TOOLS_IMAGE,
            "/vm/disk.raw",
            "/vm/install.log",
        ])?;
        let secrets = wait_for_install(&install_log)?;
        // The installer powers the VM off when it is done.
        podman(&["wait", &name])?;
        Ok::<_, anyhow::Error>(secrets)
    })();
    let _ = podman(&["rm", "-f", &name]);
    println!("install log: {}", install_log.display());
    let (token, pin) = installed?;
    std::fs::write(out.join("secrets"), format!("{token} {pin}\n"))?;

    let log = out.join("serial.log");
    std::fs::write(&log, "")?;
    let pod = format!("molt-installed-{}", std::process::id());
    podman(&[
        "pod",
        "create",
        "--name",
        &pod,
        "-p",
        &format!("{API_PORT}:8080"),
    ])?;
    let result = (|| {
        podman(&[
            "run",
            "-d",
            "--pod",
            &pod,
            "--name",
            &format!("{pod}-vm"),
            "--device",
            "/dev/kvm",
            "--security-opt",
            "label=disable",
            "-e",
            "MOLT_ACCEL",
            "-e",
            extra,
            "-e",
            "MOLT_FW_CFG=name=opt/crustates/secrets,file=/vm/secrets",
            "-v",
            &vm_mount,
            TOOLS_IMAGE,
            "/vm/disk.raw",
            "/vm/serial.log",
        ])?;
        let mut serial = Serial { log: &log, from: 0 };
        let printed = serial.wait_for_pass(version, false)?;
        if printed != token {
            bail!("the installed system has another token than the installer printed");
        }
        Api::new(token).check()
    })();
    let _ = podman(&["pod", "rm", "-f", &pod]);
    println!("serial log of the installed system: {}", log.display());
    result
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

#[cfg(test)]
mod tests {
    use super::selftest_result;

    #[test]
    fn finds_the_result_after_bytes_that_are_not_utf8() {
        let mut serial = b"UEFI \xff\xfe garbage\r\n[   10.0] booting\r\n".to_vec();
        assert_eq!(selftest_result(&serial, "1.0"), None);
        serial
            .extend_from_slice(b"[   12.8] CRUST-SELFTEST-TOKEN: crt_AAAAA-BBBBB-CCCCC-DDDDD\r\n");
        serial.extend_from_slice(b"[   12.9] CRUST-SELFTEST: PASS 1.0\r\n");
        let (token, end) = selftest_result(&serial, "1.0").unwrap().unwrap();
        assert_eq!(token, "crt_AAAAA-BBBBB-CCCCC-DDDDD");
        assert!(end <= serial.len());
        // Another version's PASS does not count; a FAIL does.
        assert_eq!(selftest_result(&serial, "1.1"), None);
        serial.extend_from_slice(b"CRUST-SELFTEST: FAIL /usr is writable\n");
        assert_eq!(
            selftest_result(&serial, "1.1"),
            Some(Err("CRUST-SELFTEST: FAIL /usr is writable".into()))
        );
    }
}
