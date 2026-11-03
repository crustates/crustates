//! The Crustates control plane: the REST API with its `/docs` reference, the
//! gRPC service node agents report to, and the reconcile loops.

pub mod auth;
mod commands;
mod config;
mod grpc;
mod http;
pub mod local;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use crust_proto::agent::v1::node_service_server::NodeServiceServer;
use crust_reconcile::node::NodeHeartbeat;
use crust_store::SqliteStore;
use tokio::net::{TcpListener, UnixListener};
use tokio_stream::wrappers::TcpListenerStream;
use tokio_util::sync::CancellationToken;

pub use auth::Auth;
pub use commands::{CommandError, Commands};
pub use config::Config;
pub use grpc::NodeReports;
pub use http::{AppState, local_router, openapi, router};
pub use local::RecoveryPin;

/// What can stop the control plane.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The REST API server failed.
    #[error("API server: {0}")]
    Http(#[from] std::io::Error),
    /// The agent gRPC server failed.
    #[error("agent server: {0}")]
    Grpc(#[from] tonic::transport::Error),
}

/// Where the control plane listens.
#[derive(Debug)]
pub struct Listeners {
    /// The REST API, for clients with the admin token.
    pub http: TcpListener,
    /// The agent service (gRPC).
    pub agents: TcpListener,
    /// The local socket for root on the machine (ADR 0018), if any.
    pub local: Option<UnixListener>,
}

/// Serves the REST API on `listeners.http`, and to root on
/// `listeners.local` (ADR 0018), and the agent service on
/// `listeners.agents`, and runs the reconcile loops, until `shutdown` fires. Nodes turn `NotReady` after `heartbeat_timeout`
/// without a report. `/v1/` requests on `http` need the admin token that
/// `auth` checks; the local console's risky actions need the PIN `pin`
/// checks.
///
/// # Errors
///
/// Returns the first server error; the other servers stop with it.
pub async fn serve(
    store: SqliteStore,
    heartbeat_timeout: Duration,
    auth: Auth,
    pin: RecoveryPin,
    listeners: Listeners,
    shutdown: CancellationToken,
) -> Result<(), Error> {
    let Listeners {
        http,
        agents,
        local,
    } = listeners;
    let commands = Commands::default();
    let state = AppState {
        store: store.clone(),
        shutdown: shutdown.clone(),
        auth: Arc::new(auth),
        commands: commands.clone(),
        pin: Arc::new(pin),
    };
    let local_api = {
        let (state, shutdown) = (state.clone(), shutdown.clone());
        async move {
            let Some(local) = local else {
                return Ok(());
            };
            let api = local_router(state).into_make_service_with_connect_info::<local::Peer>();
            axum::serve(local, api)
                .with_graceful_shutdown(shutdown.cancelled_owned())
                .await
        }
    };
    let api = router(state);
    // Connection info gives the rate limit for wrong tokens its client address.
    let api = axum::serve(
        http,
        api.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown.clone().cancelled_owned());
    let agent_service = tonic::transport::Server::builder()
        .add_service(NodeServiceServer::new(NodeReports::new(
            store.clone(),
            commands,
            shutdown.clone(),
        )))
        .serve_with_incoming_shutdown(
            TcpListenerStream::new(agents),
            shutdown.clone().cancelled_owned(),
        );
    let reconcile = crust_reconcile::run(
        store.clone(),
        NodeHeartbeat::new(store, heartbeat_timeout),
        shutdown,
    );
    tokio::try_join!(
        async { api.await.map_err(Error::from) },
        async { local_api.await.map_err(Error::from) },
        async { agent_service.await.map_err(Error::from) },
        async {
            reconcile.await;
            Ok(())
        },
    )?;
    Ok(())
}
