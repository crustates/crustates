//! The install itself: write the OS image to the boot disk, configure the
//! new system, make the pool. Each step reports its progress.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::answers::Answers;
use crate::app::{Done, Phase};
use crate::system::run;
use crate::target;

/// The image repository installed machines update from.
pub const REPOSITORY: &str = "ghcr.io/crustates/molt";
/// Where the new system's root is mounted.
const ROOT: &str = "/run/crust-install/root";

/// The install's steps, for the progress list.
#[must_use]
pub fn steps(answers: &Answers) -> Vec<String> {
    let mut steps = vec![
        format!(
            "Write the OS image to {}",
            answers.disk.trim_start_matches("/dev/")
        ),
        "Configure the new system".to_owned(),
    ];
    if let Some(pool) = &answers.pool {
        steps.push(format!("Create pool {} (ZFS {})", pool.name, pool.layout));
    }
    steps
}

/// The `bootc` arguments that write `payload` (the media's OCI layout) to the
/// boot disk. The live system is this image, so bootc runs on it directly:
/// `podman run` would first copy the image into its storage, which on a live
/// system is memory. The new system follows its channel in the registry.
#[must_use]
pub fn bootc_args(answers: &Answers, payload: &str) -> Vec<String> {
    vec![
        "install".into(),
        "to-disk".into(),
        "--wipe".into(),
        "--source-imgref".into(),
        payload.into(),
        "--target-imgref".into(),
        format!("{REPOSITORY}:{}", answers.channel),
        answers.disk.clone(),
    ]
}

/// The `zpool create` arguments for the answers' pool.
#[must_use]
pub fn zpool_args(answers: &Answers) -> Option<Vec<String>> {
    let pool = answers.pool.as_ref()?;
    let mut args: Vec<String> = [
        "create",
        "-f",
        "-o",
        "ashift=12",
        "-O",
        "compression=lz4",
        "-O",
        "atime=off",
        "-O",
        "xattr=sa",
        "-O",
        "acltype=posixacl",
        "-O",
        "mountpoint=none",
    ]
    .map(String::from)
    .to_vec();
    args.push(pool.name.clone());
    if pool.layout != "single" {
        args.push(pool.layout.clone());
    }
    args.extend(pool.disks.iter().cloned());
    Some(args)
}

/// Runs a program, sending its output lines to the log.
async fn logged(program: &str, args: &[String], tx: &mpsc::Sender<Done>) -> Result<(), String> {
    let _ = tx
        .send(Done::Log(format!("$ {program} {}", args.join(" "))))
        .await;
    let mut child = Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    let (Some(out), Some(err)) = (child.stdout.take(), child.stderr.take()) else {
        return Err(format!("{program}: no output"));
    };
    let forward = |stream: Box<dyn tokio::io::AsyncRead + Unpin + Send>| {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx.send(Done::Log(line)).await;
            }
        })
    };
    let (a, b) = (forward(Box::new(out)), forward(Box::new(err)));
    let status = child.wait().await.map_err(|e| format!("{program}: {e}"))?;
    let _ = tokio::join!(a, b);
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} failed ({status})"))
    }
}

/// The new system's root partition: the XFS partition on the boot disk.
async fn root_partition(disk: &str) -> Result<String, String> {
    let list = run("lsblk", &["-n", "-p", "-r", "-o", "PATH,FSTYPE", disk]).await?;
    list.lines()
        .find_map(|l| {
            l.split_once(' ')
                .filter(|(_, fs)| *fs == "xfs")
                .map(|(p, _)| p.to_owned())
        })
        .ok_or_else(|| format!("no root partition on {disk}"))
}

fn write(path: &Path, text: &str, mode: u32) -> Result<(), String> {
    let error = |e: std::io::Error| format!("{}: {e}", path.display());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(error)?;
    }
    std::fs::write(path, text).map_err(error)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(error)
}

/// Writes the answers into the new system's `/etc` and `/var`.
async fn configure(
    answers: &Answers,
    secrets: &Path,
    interface: &str,
) -> Result<Vec<String>, String> {
    let root = PathBuf::from(ROOT);
    std::fs::create_dir_all(&root).map_err(|e| format!("{ROOT}: {e}"))?;
    let partition = root_partition(&answers.disk).await?;
    run("mount", &[&partition, ROOT]).await?;
    // bootc finishes with `remount,ro`, and that state can outlive its unmount.
    run("mount", &["-o", "remount,rw", ROOT]).await?;
    let options = run("findmnt", &["-no", "OPTIONS", ROOT]).await?;
    let result = (|| {
        let deploy = root.join("ostree/deploy/default/deploy");
        let deployment = std::fs::read_dir(&deploy)
            .map_err(|e| format!("{}: {e}", deploy.display()))?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| p.is_dir())
            .ok_or("the new system has no deployment")?;
        let etc = deployment.join("etc");
        let var = root.join("ostree/deploy/default/var");
        let mut wrote = Vec::new();
        write(
            &etc.join("hostname"),
            &format!("{}\n", answers.hostname()),
            0o644,
        )?;
        wrote.push(format!("hostname {}", answers.hostname()));
        write(
            &etc.join("crustates/molt.toml"),
            &target::molt_toml(answers),
            0o644,
        )?;
        wrote.push(format!(
            "channel {}, policy {}",
            answers.channel, answers.policy
        ));
        if let Some(keyfile) = target::keyfile(answers, interface) {
            write(
                &etc.join("NetworkManager/system-connections/crust-mgmt.nmconnection"),
                &keyfile,
                0o600,
            )?;
            wrote.push("network settings".into());
        }
        // crust-firstboot gives them to the crustates group at the first boot.
        for name in ["admin-token", "recovery-pin"] {
            let hash =
                std::fs::read_to_string(secrets.join(name)).map_err(|e| format!("{name}: {e}"))?;
            write(&var.join("lib/crustates/secrets").join(name), &hash, 0o600)?;
        }
        wrote.push("admin token and recovery PIN hashes".into());
        Ok::<_, String>(wrote)
    })();
    let _ = run("umount", &[ROOT]).await;
    result.map_err(|e| format!("{e} (mounted {})", options.trim()))
}

/// Installs, reporting each step's progress and log lines on `tx`.
pub async fn install(answers: Answers, payload: String, secrets: PathBuf, tx: mpsc::Sender<Done>) {
    let result = async {
        let step = async |i: usize, phase: Phase| {
            let _ = tx.send(Done::Progress(i, phase)).await;
        };
        let interface = if answers.network.interface.is_empty() { crate::system::first_port() } else { answers.network.interface.clone() };
        step(0, Phase::Now).await;
        if let Err(e) = logged("bootc", &bootc_args(&answers, &payload), &tx).await {
            step(0, Phase::Failed).await;
            return Err(e);
        }
        step(0, Phase::Done).await;
        step(1, Phase::Now).await;
        match configure(&answers, &secrets, &interface).await {
            Ok(wrote) => {
                for line in wrote {
                    let _ = tx.send(Done::Log(format!("wrote {line}"))).await;
                }
                step(1, Phase::Done).await;
            }
            Err(e) => {
                step(1, Phase::Failed).await;
                return Err(e);
            }
        }
        if let Some(args) = zpool_args(&answers) {
            step(2, Phase::Now).await;
            let made = async {
                logged("modprobe", &["zfs".into()], &tx)
                    .await
                    .map_err(|_| "ZFS does not load here: with Secure Boot on, enrol the Crustates key first (crust molt enroll-key), then make the pool".to_owned())?;
                logged("zpool", &args, &tx).await?;
                // Exported, so the new system imports it as its own.
                let name = answers.pool.as_ref().map(|p| p.name.clone()).unwrap_or_default();
                logged("zpool", &["export".into(), name], &tx).await
            }
            .await;
            if let Err(e) = made {
                step(2, Phase::Failed).await;
                return Err(e);
            }
            step(2, Phase::Done).await;
        }
        let _ = run("sync", &[]).await;
        Ok(())
    }
    .await;
    let _ = tx.send(Done::Installed(result)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::answers::PoolAnswers;

    #[test]
    fn the_install_writes_the_payload_and_follows_the_channel() {
        let answers = Answers {
            disk: "/dev/vda".into(),
            channel: "beta".into(),
            pool: Some(PoolAnswers {
                name: "tank".into(),
                layout: "mirror".into(),
                disks: vec!["/dev/vdb".into(), "/dev/vdc".into()],
            }),
            ..Answers::default()
        };
        let args = bootc_args(&answers, "oci:/payload:molt").join(" ");
        assert_eq!(
            args,
            "install to-disk --wipe --source-imgref oci:/payload:molt --target-imgref ghcr.io/crustates/molt:beta /dev/vda"
        );
        assert!(
            zpool_args(&answers)
                .unwrap()
                .join(" ")
                .ends_with("mountpoint=none tank mirror /dev/vdb /dev/vdc")
        );
        assert_eq!(steps(&answers).len(), 3);
        let single = Answers {
            pool: Some(PoolAnswers {
                name: "t".into(),
                layout: "single".into(),
                disks: vec!["/dev/vdb".into()],
            }),
            ..answers
        };
        assert!(
            zpool_args(&single)
                .unwrap()
                .join(" ")
                .ends_with("mountpoint=none t /dev/vdb")
        );
    }
}
