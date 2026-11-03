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

use agent::v1 as pb;
use jiff::Timestamp;

impl From<crust_types::MoltStatus> for pb::MoltStatus {
    fn from(s: crust_types::MoltStatus) -> Self {
        let policy = match s.policy {
            crust_types::UpdatePolicy::Auto => pb::UpdatePolicy::Auto,
            crust_types::UpdatePolicy::Manual => pb::UpdatePolicy::Manual,
        };
        Self {
            booted: s.booted.map(Into::into),
            staged: s.staged.map(Into::into),
            rollback: s.rollback.map(Into::into),
            rollback_queued: s.rollback_queued,
            available: s.available.map(Into::into),
            repository: s.repository,
            channel: s.channel,
            policy: policy.into(),
            window: s.window,
            last_check: s.last_check.map_or(0, Timestamp::as_second),
            last_error: s.last_error,
            bad_digests: s.bad_digests,
            automatic_rollback: s.automatic_rollback,
        }
    }
}

impl From<pb::MoltStatus> for crust_types::MoltStatus {
    fn from(s: pb::MoltStatus) -> Self {
        let policy = match s.policy() {
            pb::UpdatePolicy::Manual => crust_types::UpdatePolicy::Manual,
            pb::UpdatePolicy::Auto | pb::UpdatePolicy::Unspecified => {
                crust_types::UpdatePolicy::Auto
            }
        };
        Self {
            booted: s.booted.map(Into::into),
            staged: s.staged.map(Into::into),
            rollback: s.rollback.map(Into::into),
            rollback_queued: s.rollback_queued,
            available: s.available.map(Into::into),
            repository: s.repository,
            channel: s.channel,
            policy,
            window: s.window,
            last_check: (s.last_check != 0)
                .then(|| Timestamp::from_second(s.last_check).ok())
                .flatten(),
            last_error: s.last_error,
            bad_digests: s.bad_digests,
            automatic_rollback: s.automatic_rollback,
        }
    }
}

impl From<crust_types::MoltImage> for pb::MoltImage {
    fn from(i: crust_types::MoltImage) -> Self {
        Self {
            image: i.image,
            digest: i.digest,
            version: i.version,
            verified: i.verified,
        }
    }
}

impl From<pb::MoltImage> for crust_types::MoltImage {
    fn from(i: pb::MoltImage) -> Self {
        Self {
            image: i.image,
            digest: i.digest,
            version: i.version,
            verified: i.verified,
        }
    }
}

#[cfg(test)]
mod tests {
    use crust_types::{MoltImage, MoltStatus, UpdatePolicy};

    #[test]
    fn molt_status_survives_the_round_trip() {
        let status = MoltStatus {
            booted: Some(MoltImage {
                image: "ghcr.io/crustates/molt@sha256:aa".into(),
                digest: "sha256:aa".into(),
                version: "0.1.0".into(),
                verified: true,
            }),
            staged: None,
            rollback: None,
            rollback_queued: true,
            available: None,
            repository: "ghcr.io/crustates/molt".into(),
            channel: "beta".into(),
            policy: UpdatePolicy::Manual,
            window: "03:00-05:00".into(),
            last_check: Some(jiff::Timestamp::from_second(1_800_000_000).unwrap()),
            last_error: String::new(),
            bad_digests: vec!["sha256:bb".into()],
            automatic_rollback: true,
        };
        let wire: super::pb::MoltStatus = status.clone().into();
        assert_eq!(MoltStatus::from(wire), status);
    }
}
