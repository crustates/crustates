//! Rescue mode: find Crustates on this machine's disks and repair it from
//! the install media. Disks are mounted read-only to look; a repair
//! remounts what it changes. ZFS pools are listed, never imported.

use std::path::{Path, PathBuf};

use crate::system::run;

/// Where installs are mounted.
const MOUNTS: &str = "/run/crust-rescue";

/// A Crustates install found on a disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Install {
    /// The root partition.
    pub root: String,
    /// The boot partition, if found.
    pub boot: Option<String>,
    /// Its deployments, newest boot first: version and whether it boots next.
    pub deployments: Vec<(String, bool)>,
}

/// Parses `ostree admin status`: one deployment per line that names a
/// stateroot and checksum, `*` marking the default.
fn deployments(status: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for line in status.lines() {
        let trimmed = line.trim_start();
        let default = trimmed.starts_with('*');
        let rest = trimmed.trim_start_matches(['*', ' ']);
        if let Some((stateroot, checksum)) = rest.split_once(' ')
            && !stateroot.is_empty()
            && checksum.len() >= 12
            && checksum.chars().take(12).all(|c| c.is_ascii_hexdigit())
        {
            out.push((checksum.chars().take(12).collect(), default));
        } else if let Some(version) = trimmed.strip_prefix("Version: ")
            && let Some(last) = out.last_mut()
        {
            last.0 = format!(
                "{} ({})",
                version.split_whitespace().next().unwrap_or(version),
                last.0
            );
        }
    }
    out
}

async fn mount(device: &str, at: &Path, write: bool) -> Result<(), String> {
    std::fs::create_dir_all(at).map_err(|e| format!("{}: {e}", at.display()))?;
    let options = if write { "rw" } else { "ro" };
    let target = at.to_string_lossy();
    run("mount", &["-o", options, device, &target])
        .await
        .map(|_| ())
}

async fn unmount(at: &Path) {
    let _ = run("umount", &["-R", &at.to_string_lossy()]).await;
}

/// Looks at every XFS partition for an ostree root (`/ostree/deploy`).
pub async fn find() -> Vec<Install> {
    let list = run("lsblk", &["-n", "-p", "-r", "-o", "PATH,FSTYPE,LABEL"])
        .await
        .unwrap_or_default();
    let parts: Vec<(String, String, String)> = list
        .lines()
        .filter_map(|l| {
            let mut f = l.split(' ');
            Some((
                f.next()?.to_owned(),
                f.next().unwrap_or_default().to_owned(),
                f.next().unwrap_or_default().to_owned(),
            ))
        })
        .collect();
    let boot = parts
        .iter()
        .find(|(_, fs, label)| label == "boot" && fs != "vfat")
        .map(|(p, _, _)| p.clone());
    let mut found = Vec::new();
    for (path, fs, _) in parts.iter().filter(|(_, fs, _)| fs == "xfs") {
        let at = Path::new(MOUNTS).join(path.trim_start_matches("/dev/").replace('/', "-"));
        if mount(path, &at, false).await.is_err() {
            let _ = fs;
            continue;
        }
        if at.join("ostree/deploy").is_dir() {
            if let Some(boot) = &boot {
                let _ = mount(boot, &at.join("boot"), false).await;
            }
            let status = run(
                "ostree",
                &["admin", &format!("--sysroot={}", at.display()), "status"],
            )
            .await
            .unwrap_or_default();
            found.push(Install {
                root: path.clone(),
                boot: boot.clone(),
                deployments: deployments(&status),
            });
        }
        unmount(&at).await;
    }
    found
}

/// Mounts `install` for a repair, runs `f` on its mount point, unmounts.
async fn with_install<T>(
    install: &Install,
    f: impl AsyncFnOnce(&Path) -> Result<T, String>,
) -> Result<T, String> {
    let at = PathBuf::from(MOUNTS).join("repair");
    mount(&install.root, &at, true).await?;
    if let Some(boot) = &install.boot {
        mount(boot, &at.join("boot"), true).await?;
    }
    let result = f(&at).await;
    unmount(&at).await;
    result
}

/// Makes the previous deployment boot next.
pub async fn boot_previous(install: &Install) -> Result<String, String> {
    with_install(install, async |at: &Path| {
        run(
            "ostree",
            &[
                "admin",
                &format!("--sysroot={}", at.display()),
                "set-default",
                "1",
            ],
        )
        .await?;
        Ok("The previous OS image boots next. Remove the stick and reboot.".to_owned())
    })
    .await
}

/// Removes the management network settings the installer or the console
/// wrote, in every deployment: the machine falls back to DHCP.
pub async fn reset_network(install: &Install) -> Result<String, String> {
    with_install(install, async |at: &Path| {
        let deploy = at.join("ostree/deploy/default/deploy");
        let mut removed = 0;
        for entry in std::fs::read_dir(&deploy).map_err(|e| format!("{}: {e}", deploy.display()))?.filter_map(Result::ok) {
            let connections = entry.path().join("etc/NetworkManager/system-connections");
            for file in std::fs::read_dir(&connections).into_iter().flatten().filter_map(Result::ok) {
                if file.path().extension().is_some_and(|e| e == "nmconnection") && std::fs::remove_file(file.path()).is_ok() {
                    removed += 1;
                }
            }
        }
        Ok(format!("Removed {removed} network settings: the machine uses DHCP on every port at its next boot."))
    })
    .await
}

/// Lists the ZFS pools that could be imported, without importing them.
pub async fn pools() -> String {
    match run("zpool", &["import"]).await {
        Ok(out) if out.trim().is_empty() => "No ZFS pools found.".into(),
        Ok(out) => out,
        Err(e) if e.contains("no pools available") => "No ZFS pools found.".into(),
        Err(e) => e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ostree_status_lists_deployments_default_first() {
        let status = "* default 4f2a9c01d3e5b7a90c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b.0\n    Version: 0.2.0 (2026-11-03T10:00:00Z)\n    origin refspec: ...\n  default 9b1e77aa0c2d4e6f8a0b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f.0 (rollback)\n    Version: 0.1.0\n";
        assert_eq!(
            deployments(status),
            [
                ("0.2.0 (4f2a9c01d3e5)".to_owned(), true),
                ("0.1.0 (9b1e77aa0c2d)".to_owned(), false)
            ]
        );
    }
}
