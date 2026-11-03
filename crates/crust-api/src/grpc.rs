//! The agent-facing gRPC service.

use std::pin::Pin;

use crust_proto::agent::v1::node_service_server::NodeService;
use crust_proto::agent::v1::{
    Command, CommandResult, CommandsRequest, CompleteCommandResponse, ReportRequest, ReportResponse,
};
use crust_store::{Error, SqliteStore, Store};
use crust_types::{NodeSpec, NodeStatus, validate_name};
use futures::{Stream, StreamExt};
use jiff::Timestamp;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tonic::{Request, Response, Status};
use tracing::{info, warn};

use crate::commands::Commands;

/// The longest text field an agent may send.
const MAX_FIELD_LEN: usize = 256;
/// How often a report is retried when the reconciler wrote the node meanwhile.
const ATTEMPTS: usize = 3;

/// Records agent reports as node status and heartbeats, and hands agents
/// their commands.
// TODO(mTLS): agents connect over plain TCP and are not authenticated yet.
// See the security model in docs/plan.md#security-model.
#[derive(Clone, Debug)]
pub struct NodeReports {
    store: SqliteStore,
    commands: Commands,
    shutdown: CancellationToken,
}

impl NodeReports {
    /// A service that writes reports into `store` and streams `commands` to
    /// agents until `shutdown` fires.
    #[must_use]
    pub fn new(store: SqliteStore, commands: Commands, shutdown: CancellationToken) -> Self {
        Self {
            store,
            commands,
            shutdown,
        }
    }

    async fn record(&self, report: &ReportRequest) -> Result<(), Error> {
        let now = Timestamp::now();
        match self.store.get::<NodeSpec>(&report.name).await {
            Ok(mut node) => {
                apply(&mut node.status, report, now);
                self.store.update(node).await?;
            }
            Err(Error::NotFound { .. }) => {
                let mut status = NodeStatus::default();
                apply(&mut status, report, now);
                self.store.create(&report.name, NodeSpec {}, status).await?;
                info!(node = report.name, "node registered");
            }
            Err(e) => return Err(e),
        }
        Ok(())
    }
}

/// Copies the agent's facts into `status`; conditions stay as they are.
fn apply(status: &mut NodeStatus, report: &ReportRequest, now: Timestamp) {
    status.hostname.clone_from(&report.hostname);
    status.arch.clone_from(&report.arch);
    status.kernel.clone_from(&report.kernel);
    status.cpus = report.cpus;
    status.memory_bytes = report.memory_bytes;
    status.molt = report.molt.clone().map(Into::into);
    status.last_heartbeat = Some(now);
}

fn validate(report: &ReportRequest) -> Result<(), Status> {
    validate_name(&report.name).map_err(Status::invalid_argument)?;
    for (field, value) in [
        ("hostname", &report.hostname),
        ("arch", &report.arch),
        ("kernel", &report.kernel),
    ] {
        if value.len() > MAX_FIELD_LEN {
            return Err(Status::invalid_argument(format!(
                "{field} is longer than {MAX_FIELD_LEN} bytes"
            )));
        }
    }
    Ok(())
}

#[tonic::async_trait]
impl NodeService for NodeReports {
    type CommandsStream = Pin<Box<dyn Stream<Item = Result<Command, Status>> + Send>>;

    async fn report(
        &self,
        request: Request<ReportRequest>,
    ) -> Result<Response<ReportResponse>, Status> {
        let report = request.into_inner();
        validate(&report)?;
        for _ in 0..ATTEMPTS {
            match self.record(&report).await {
                Ok(()) => return Ok(Response::new(ReportResponse {})),
                // Someone wrote the node between our read and write: read again.
                Err(Error::Conflict { .. } | Error::AlreadyExists { .. }) => {}
                Err(e) => {
                    tracing::error!(node = report.name, error = %e, "recording a report failed");
                    return Err(Status::internal("could not record the report"));
                }
            }
        }
        Err(Status::aborted("the node kept changing; report again"))
    }

    async fn commands(
        &self,
        request: Request<CommandsRequest>,
    ) -> Result<Response<Self::CommandsStream>, Status> {
        let name = request.into_inner().name;
        validate_name(&name).map_err(Status::invalid_argument)?;
        let commands = self.commands.connect(&name);
        info!(node = name, "agent waits for commands");
        // Ends with the server, so a graceful shutdown does not wait for it.
        let stream = ReceiverStream::new(commands)
            .map(Ok)
            .take_until(self.shutdown.clone().cancelled_owned());
        Ok(Response::new(Box::pin(stream)))
    }

    async fn complete_command(
        &self,
        request: Request<CommandResult>,
    ) -> Result<Response<CompleteCommandResponse>, Status> {
        let result = request.into_inner();
        let id = result.id;
        if !self.commands.complete(result) {
            warn!(
                command = id,
                "a result arrived for a command nobody waits for"
            );
        }
        Ok(Response::new(CompleteCommandResponse {}))
    }
}
