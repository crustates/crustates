//! `crust-api`: runs the control plane. `crust-api new-admin-token` and
//! `crust-api new-recovery-pin` make a new admin token or recovery PIN
//! instead: they print it and store only its hash.

use std::io::Write;
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, bail};
use crust_api::{Auth, Config, Listeners, RecoveryPin, auth, local};
use crust_store::SqliteStore;
use tokio::net::{TcpListener, UnixListener};
use tokio_util::sync::CancellationToken;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::load().context("loading the configuration")?;
    match std::env::args().nth(1).as_deref() {
        None => {}
        Some("new-admin-token") => {
            let token = auth::new_token().context("making a random token")?;
            return new_secret(&config.admin_token_file, &token);
        }
        Some("new-recovery-pin") => {
            let pin = local::new_pin().context("making a random PIN")?;
            return new_secret(&config.recovery_pin_file, &pin);
        }
        Some(other) => bail!(
            "unknown argument {other:?}; the commands are new-admin-token and new-recovery-pin"
        ),
    }
    let store = SqliteStore::open(&config.database)
        .await
        .with_context(|| format!("opening {}", config.database.display()))?;
    let http = TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("listening on {}", config.listen))?;
    let agents = TcpListener::bind(config.agent_listen)
        .await
        .with_context(|| format!("listening on {}", config.agent_listen))?;
    let local = match &config.local_socket {
        Some(path) => Some(local_socket(path)?),
        None => None,
    };
    info!(api = %config.listen, agents = %config.agent_listen, docs = %format!("http://{}/docs", config.listen), "control plane started");

    let shutdown = CancellationToken::new();
    tokio::spawn(cancel_on_signal(shutdown.clone()));
    crust_api::serve(
        store,
        Duration::from_secs(config.heartbeat_timeout_secs),
        Auth::new(config.admin_token_file),
        RecoveryPin::new(config.recovery_pin_file),
        Listeners {
            http,
            agents,
            local,
        },
        shutdown,
    )
    .await?;
    info!("control plane stopped");
    Ok(())
}

/// Listens on the local socket at `path`, mode 0660: the console runs as
/// root, and only root's requests are served (ADR 0018).
fn local_socket(path: &Path) -> anyhow::Result<UnixListener> {
    // A socket left by the last run; anything else is not ours to remove.
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_socket()) {
        std::fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
    }
    let listener =
        UnixListener::bind(path).with_context(|| format!("listening on {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660))
        .with_context(|| format!("setting the mode of {}", path.display()))?;
    Ok(listener)
}

/// Writes the hash of a new `secret` (an admin token or a recovery PIN) to
/// `file`, replacing the old one, and prints the secret: the only time it
/// is shown.
fn new_secret(file: &Path, secret: &str) -> anyhow::Result<()> {
    let hash = auth::hash(secret).map_err(|e| anyhow::anyhow!("hashing: {e}"))?;
    if let Some(dir) = file.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    // Written next to the file and renamed over it, so a reader never sees
    // half a hash.
    let partial = file.with_extension("new");
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o640)
        .open(&partial)
        .with_context(|| format!("writing {}", partial.display()))?;
    writeln!(out, "{hash}").and_then(|()| out.sync_all())?;
    std::fs::rename(&partial, file).with_context(|| format!("replacing {}", file.display()))?;
    println!("{secret}");
    Ok(())
}

/// Cancels `token` on Ctrl-C or SIGTERM.
async fn cancel_on_signal(token: CancellationToken) {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
    info!("shutting down");
    token.cancel();
}
