//! [`Os`] on a Molt machine: bootc for deployments, skopeo for the
//! registry, cosign for signatures, mokutil for Secure Boot keys.

use std::fmt::Write as _;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::molt::{Deployment, Deployments, Error, Os, Resolved};

/// The signers Molt images are accepted from: the Crustates workflows that
/// publish (`molt.yaml`) and promote (`promote.yaml`) images, on the
/// development branch or a release tag.
const SIGNER: &str = r"^https://github\.com/crustates/crustates/\.github/workflows/(molt|promote)\.yaml@refs/(heads/development|tags/v[0-9][0-9A-Za-z.+-]*)$";
const ISSUER: &str = "https://token.actions.githubusercontent.com";
/// cosign is not packaged in Fedora; the image ships a pinned release.
const COSIGN: &str = "/usr/libexec/crustates/cosign";
/// The certificate the image's out-of-tree modules are signed with.
const MODULE_CERT: &str = "/usr/share/crustates/keys/crustates-module-signing.der";
/// greenboot's health check, and the boot loader part it needs: the boot
/// counter that bootupd installs into grub.cfg only at install time.
const GREENBOOT: &str = "/usr/libexec/greenboot/greenboot";
const GRUB_CFG: &str = "/boot/grub2/grub.cfg";

/// The machine's own bootc, skopeo, cosign and mokutil.
#[derive(Clone, Copy, Debug, Default)]
pub struct Bootc;

impl Bootc {
    /// Whether this machine is booted from a bootc image.
    #[must_use]
    pub fn is_booted() -> bool {
        Path::new("/run/ostree-booted").exists()
    }
}

/// `bootc status --json`, the parts the agent reads.
#[derive(Debug, Deserialize)]
struct Host {
    status: HostStatus,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HostStatus {
    staged: Option<BootEntry>,
    booted: Option<BootEntry>,
    rollback: Option<BootEntry>,
    #[serde(default)]
    rollback_queued: bool,
}

#[derive(Debug, Deserialize)]
struct BootEntry {
    image: Option<ImageStatus>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImageStatus {
    image: ImageReference,
    version: Option<String>,
    image_digest: String,
}

#[derive(Debug, Deserialize)]
struct ImageReference {
    image: String,
}

fn deployment(entry: Option<BootEntry>) -> Option<Deployment> {
    let image = entry?.image?;
    Some(Deployment {
        image: image.image.image,
        digest: image.image_digest,
        version: image.version.unwrap_or_default(),
    })
}

fn parse_status(json: &[u8]) -> Result<Deployments, Error> {
    let host: Host = serde_json::from_slice(json).map_err(|e| Error::Output {
        program: "bootc",
        message: e.to_string(),
    })?;
    Ok(Deployments {
        booted: deployment(host.status.booted),
        staged: deployment(host.status.staged),
        rollback: deployment(host.status.rollback),
        rollback_queued: host.status.rollback_queued,
    })
}

/// The digest of `manifest` and, for an image index, of this architecture's
/// image in it.
fn parse_manifest(manifest: &[u8], arch: &str) -> Result<(String, String), Error> {
    let digest = Sha256::digest(manifest)
        .iter()
        .fold("sha256:".to_owned(), |mut hex, b| {
            let _ = write!(hex, "{b:02x}");
            hex
        });
    let json: Value = serde_json::from_slice(manifest).map_err(|e| Error::Output {
        program: "skopeo",
        message: e.to_string(),
    })?;
    let Some(manifests) = json["manifests"].as_array() else {
        return Ok((digest.clone(), digest));
    };
    let platform = manifests
        .iter()
        .find(|m| m["platform"]["os"] == "linux" && m["platform"]["architecture"] == arch)
        .and_then(|m| m["digest"].as_str())
        .ok_or_else(|| Error::Output {
            program: "skopeo",
            message: format!("the image has no linux/{arch} variant"),
        })?;
    Ok((digest, platform.to_owned()))
}

/// The architecture as OCI names it.
fn oci_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => other,
    }
}

/// Runs `path` with `args` and returns its standard output.
async fn run(program: &'static str, path: &str, args: &[&str]) -> Result<Vec<u8>, Error> {
    let output = tokio::process::Command::new(path)
        .args(args)
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| Error::Command {
            program,
            message: e.to_string(),
        })?;
    // skopeo exits with 2 only when the image does not exist.
    if program == "skopeo" && output.status.code() == Some(2) {
        let reference = args
            .last()
            .map_or("", |a| a.trim_start_matches("docker://"));
        return Err(Error::NotFound(reference.to_owned()));
    }
    if !output.status.success() {
        return Err(Error::Command {
            program,
            message: last_lines(&output.stderr),
        });
    }
    Ok(output.stdout)
}

/// The end of a program's error output: where the reason usually is.
fn last_lines(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(3)..].join(" ");
    let mut tail = tail.trim().to_owned();
    if tail.len() > 500 {
        let mut end = 500;
        while !tail.is_char_boundary(end) {
            end -= 1;
        }
        tail.truncate(end);
    }
    if tail.is_empty() {
        "exited with an error".into()
    } else {
        tail
    }
}

impl Os for Bootc {
    async fn deployments(&self) -> Result<Deployments, Error> {
        parse_status(&run("bootc", "bootc", &["status", "--json"]).await?)
    }

    async fn resolve(&self, repository: &str, tag: &str) -> Result<Resolved, Error> {
        let manifest = run(
            "skopeo",
            "skopeo",
            &["inspect", "--raw", &format!("docker://{repository}:{tag}")],
        )
        .await?;
        let (digest, platform_digest) = parse_manifest(&manifest, oci_arch())?;
        // Labels from exactly the image the digest names.
        let config = run(
            "skopeo",
            "skopeo",
            &[
                "inspect",
                &format!("docker://{repository}@{platform_digest}"),
            ],
        )
        .await?;
        let config: Value = serde_json::from_slice(&config).map_err(|e| Error::Output {
            program: "skopeo",
            message: e.to_string(),
        })?;
        let version = config["Labels"]["org.opencontainers.image.version"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        Ok(Resolved {
            digest,
            platform_digest,
            version,
        })
    }

    async fn verify(&self, image: &str) -> Result<(), Error> {
        run(
            "cosign",
            COSIGN,
            &[
                "verify",
                "--certificate-identity-regexp",
                SIGNER,
                "--certificate-oidc-issuer",
                ISSUER,
                image,
            ],
        )
        .await
        .map(drop)
    }

    async fn stage(&self, image: &str) -> Result<(), Error> {
        run("bootc", "bootc", &["switch", "--quiet", image])
            .await
            .map(drop)
    }

    async fn rollback(&self) -> Result<(), Error> {
        run("bootc", "bootc", &["rollback"]).await.map(drop)
    }

    async fn reboot(&self) -> Result<(), Error> {
        run("systemctl", "systemctl", &["reboot"]).await.map(drop)
    }

    async fn enroll_key(&self, password: &str) -> Result<String, Error> {
        let state = run("mokutil", "mokutil", &["--sb-state"])
            .await
            .map_err(|_| {
                Error::Refused("this machine has no UEFI Secure Boot to enrol a key in".into())
            })?;
        if !String::from_utf8_lossy(&state).contains("SecureBoot enabled") {
            return Err(Error::Refused(
                "Secure Boot is off, so the modules load without enrolling a key".into(),
            ));
        }
        let cert = run(
            "openssl",
            "openssl",
            &[
                "x509",
                "-inform",
                "DER",
                "-noout",
                "-subject",
                "-fingerprint",
                "-sha256",
                "-in",
                MODULE_CERT,
            ],
        )
        .await?;
        let cert = String::from_utf8_lossy(&cert);
        if cert.contains("throwaway") {
            return Err(Error::Refused(
                "this image's modules are signed with a throwaway key from a local or pull request build; never enrol it".into(),
            ));
        }
        let fingerprint = cert
            .lines()
            .find_map(|l| {
                l.split_once("Fingerprint=")
                    .map(|(_, f)| f.trim().to_owned())
            })
            .unwrap_or_default();
        // --test-key exits non-zero either way; its words say which.
        let tested = tokio::process::Command::new("mokutil")
            .args(["--test-key", MODULE_CERT])
            .output()
            .await
            .map_err(|e| Error::Command {
                program: "mokutil",
                message: e.to_string(),
            })?;
        let said = [tested.stdout, tested.stderr].concat();
        if String::from_utf8_lossy(&said).contains("already enrolled") {
            return Err(Error::Refused(
                "the Crustates key is already enrolled".into(),
            ));
        }
        // shortcut: the password is briefly on mokutil's command line; Molt
        // has no other local users. Use a pipe if that changes.
        let hash = run(
            "mokutil",
            "mokutil",
            &[&format!("--generate-hash={password}")],
        )
        .await?;
        let dir = tempfile::tempdir().map_err(|source| Error::File {
            path: std::env::temp_dir(),
            source,
        })?;
        let hash_file = dir.path().join("mok-password");
        std::fs::write(&hash_file, hash).map_err(|source| Error::File {
            path: hash_file.clone(),
            source,
        })?;
        let hash_path = hash_file.to_string_lossy();
        run(
            "mokutil",
            "mokutil",
            &["--import", MODULE_CERT, "--hash-file", &hash_path],
        )
        .await?;
        Ok(fingerprint)
    }

    fn automatic_rollback(&self) -> bool {
        Path::new(GREENBOOT).exists()
            && std::fs::read_to_string(GRUB_CFG).is_ok_and(|cfg| cfg.contains("boot_counter"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_bootc_status() {
        let json = br#"{
          "apiVersion": "org.containers.bootc/v1", "kind": "BootcHost",
          "spec": {"image": {"image": "ghcr.io/crustates/molt:beta", "transport": "registry"}},
          "status": {
            "staged": {"image": {"image": {"image": "ghcr.io/crustates/molt@sha256:bb", "transport": "registry"},
                       "version": "0.2.0", "timestamp": null, "imageDigest": "sha256:b2"}, "pinned": false},
            "booted": {"image": {"image": {"image": "ghcr.io/crustates/molt:beta", "transport": "registry"},
                       "version": null, "timestamp": null, "imageDigest": "sha256:a1"}},
            "rollback": null, "rollbackQueued": false, "type": "bootcHost"
          }
        }"#;
        let d = parse_status(json).unwrap();
        assert_eq!(d.booted.unwrap().digest, "sha256:a1");
        let staged = d.staged.unwrap();
        assert_eq!(staged.image, "ghcr.io/crustates/molt@sha256:bb");
        assert_eq!(staged.version, "0.2.0");
        assert_eq!(d.rollback, None);
    }

    #[test]
    fn finds_this_architecture_in_an_index() {
        let index = br#"{"schemaVersion":2,"mediaType":"application/vnd.oci.image.index.v1+json","manifests":[
          {"digest":"sha256:aaa","platform":{"architecture":"amd64","os":"linux"}},
          {"digest":"sha256:bbb","platform":{"architecture":"arm64","os":"linux"}}]}"#;
        let (digest, arm) = parse_manifest(index, "arm64").unwrap();
        assert!(digest.starts_with("sha256:") && digest.len() == 71);
        assert_eq!(arm, "sha256:bbb");
        assert!(parse_manifest(index, "riscv64").is_err());

        let single = br#"{"schemaVersion":2,"config":{}}"#;
        let (digest, platform) = parse_manifest(single, "amd64").unwrap();
        assert_eq!(digest, platform);
    }

    #[test]
    fn error_output_is_cut_to_the_reason() {
        assert_eq!(last_lines(b"\n"), "exited with an error");
        assert_eq!(last_lines(b"a\nb\n\nc\nd\n"), "b c d");
        assert!(last_lines("é".repeat(400).as_bytes()).len() <= 500);
    }
}
