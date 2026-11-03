//! `crust-api`: runs the control plane. `crust-api new-admin-token` makes a
//! new admin token instead: it prints the token and stores only its hash.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, bail};
use crust_api::{Auth, Config, auth};
use crust_store::SqliteStore;
use tokio::net::TcpListener;
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
        Some("new-admin-token") => return new_admin_token(&config.admin_token_file),
        Some(other) => bail!("unknown argument {other:?}; the only command is new-admin-token"),
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
    info!(api = %config.listen, agents = %config.agent_listen, docs = %format!("http://{}/docs", config.listen), "control plane started");

    let shutdown = CancellationToken::new();
    tokio::spawn(cancel_on_signal(shutdown.clone()));
    crust_api::serve(
        store,
        Duration::from_secs(config.heartbeat_timeout_secs),
        Auth::new(config.admin_token_file),
        http,
        agents,
        shutdown,
    )
    .await?;
    info!("control plane stopped");
    Ok(())
}

/// Writes the hash of a new admin token to `file`, replacing the old one,
/// and prints the token: the only time it is shown.
fn new_admin_token(file: &Path) -> anyhow::Result<()> {
    let token = auth::new_token().context("making a random token")?;
    let hash = auth::hash(&token).map_err(|e| anyhow::anyhow!("hashing the token: {e}"))?;
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
    println!("{token}");
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
