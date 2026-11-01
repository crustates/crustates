//! The gRPC protocol between the node agent and the control plane.
//!
//! The control plane serves [`agent::v1::node_service_server::NodeService`];
//! agents call it with [`agent::v1::node_service_client::NodeServiceClient`].

/// The `crustates.agent` protobuf packages.
pub mod agent {
    /// Version 1 of the agent protocol.
    #[allow(missing_docs, clippy::pedantic, reason = "generated code")]
    pub mod v1 {
        tonic::include_proto!("crustates.agent.v1");
    }
}
