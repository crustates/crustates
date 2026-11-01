//! `crust-api`: runs the control plane.

use std::time::Duration;

use anyhow::Context;
use crust_api::Config;
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
        http,
        agents,
        shutdown,
    )
    .await?;
    info!("control plane stopped");
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
