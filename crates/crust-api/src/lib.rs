//! The Crustates control plane: the REST API with its `/docs` reference, the
//! gRPC service node agents report to, and the reconcile loops.

pub mod auth;
mod commands;
mod config;
mod grpc;
mod http;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use crust_proto::agent::v1::node_service_server::NodeServiceServer;
use crust_reconcile::node::NodeHeartbeat;
use crust_store::SqliteStore;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tokio_util::sync::CancellationToken;

pub use auth::Auth;
pub use commands::{CommandError, Commands};
pub use config::Config;
pub use grpc::NodeReports;
pub use http::{AppState, openapi, router};

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

/// Serves the REST API on `http` and the agent service on `agents`, and runs
/// the reconcile loops, until `shutdown` fires. Nodes turn `NotReady` after
/// `heartbeat_timeout` without a report. `/v1/` requests need the admin
/// token that `auth` checks.
///
/// # Errors
///
/// Returns the first server error; the other servers stop with it.
pub async fn serve(
    store: SqliteStore,
    heartbeat_timeout: Duration,
    auth: Auth,
    http: TcpListener,
    agents: TcpListener,
    shutdown: CancellationToken,
) -> Result<(), Error> {
    let commands = Commands::default();
    let api = router(AppState {
        store: store.clone(),
        shutdown: shutdown.clone(),
        auth: Arc::new(auth),
        commands: commands.clone(),
    });
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
        async { agent_service.await.map_err(Error::from) },
        async {
            reconcile.await;
            Ok(())
        },
    )?;
    Ok(())
}
