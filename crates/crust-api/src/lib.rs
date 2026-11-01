//! The Crustates control plane: the REST API with its `/docs` reference, the
//! gRPC service node agents report to, and the reconcile loops.

mod config;
mod grpc;
mod http;

use std::time::Duration;

use crust_proto::agent::v1::node_service_server::NodeServiceServer;
use crust_reconcile::node::NodeHeartbeat;
use crust_store::SqliteStore;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tokio_util::sync::CancellationToken;

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
/// `heartbeat_timeout` without a report.
///
/// # Errors
///
/// Returns the first server error; the other servers stop with it.
pub async fn serve(
    store: SqliteStore,
    heartbeat_timeout: Duration,
    http: TcpListener,
    agents: TcpListener,
    shutdown: CancellationToken,
) -> Result<(), Error> {
    let api = router(AppState {
        store: store.clone(),
        shutdown: shutdown.clone(),
    });
    let api = axum::serve(http, api).with_graceful_shutdown(shutdown.clone().cancelled_owned());
    let agent_service = tonic::transport::Server::builder()
        .add_service(NodeServiceServer::new(NodeReports::new(store.clone())))
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
