//! `crust-agent`: runs the node agent on this machine.
//! `crust-agent molt-mark-bad` marks the booted Molt image as bad instead;
//! greenboot runs it when the image fails its health check.

use std::sync::Arc;

use anyhow::{Context, bail};
use crust_agent::Config;
use crust_agent::bootc::Bootc;
use crust_agent::molt::Updater;
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
        Some("molt-mark-bad") => {
            let updater = Arc::new(Updater::new(Bootc, config.molt_paths()));
            let digest = updater.mark_booted_bad().await?;
            info!(digest, "marked the booted Molt image as bad");
            return Ok(());
        }
        Some(other) => bail!("unknown argument {other:?}; the only command is molt-mark-bad"),
    }
    let os = Bootc::is_booted().then_some(Bootc);
    let shutdown = CancellationToken::new();
    tokio::spawn(cancel_on_signal(shutdown.clone()));
    crust_agent::run(config, os, shutdown).await?;
    info!("agent stopped");
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
