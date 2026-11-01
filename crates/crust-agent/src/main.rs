//! `crust-agent`: runs the node agent on this machine.

use anyhow::Context;
use crust_agent::Config;
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
    let shutdown = CancellationToken::new();
    tokio::spawn(cancel_on_signal(shutdown.clone()));
    crust_agent::run(config, shutdown).await?;
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
