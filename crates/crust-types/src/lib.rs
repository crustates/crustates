//! The Crustates resource model.
//!
//! Every resource has `metadata`, a `spec` (what the user wants) and a
//! `status` (what the platform observed), as described in ADR 0003.

use std::borrow::Cow;
use std::collections::BTreeMap;

use jiff::Timestamp;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use utoipa::openapi::schema::{ObjectBuilder, Schema};
use utoipa::openapi::{Ref, RefOr};
use utoipa::{PartialSchema, ToSchema};
use uuid::Uuid;

/// A resource kind: ties a spec type to its status type and its name.
pub trait Kind:
    Serialize + DeserializeOwned + std::fmt::Debug + PartialEq + Clone + Send + Sync + 'static
{
    /// The kind's name, for example `Node`.
    const KIND: &'static str;

    /// The status that belongs to this spec.
    type Status: Serialize
        + DeserializeOwned
        + std::fmt::Debug
        + PartialEq
        + Default
        + Clone
        + Send
        + Sync
        + 'static;
}

/// A resource of kind `K`.
pub type Object<K> = Resource<K, <K as Kind>::Status>;

/// A stored resource: metadata, the desired `spec` and the observed `status`.
///
/// In the OpenAPI description it is named after its kind, for example `Node`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Resource<Spec, Status> {
    /// Identity and bookkeeping.
    pub metadata: Metadata,
    /// What the user wants.
    pub spec: Spec,
    /// What the platform observed.
    pub status: Status,
}

// Written by hand because the derive names every `Resource<..>` "Resource"
// and leaves out the spec and status schemas.
impl<Spec: Kind + ToSchema, Status: ToSchema> PartialSchema for Resource<Spec, Status> {
    fn schema() -> RefOr<Schema> {
        ObjectBuilder::new()
            .description(Some(format!(
                "A {} resource: metadata, the desired spec and the observed status.",
                Spec::KIND
            )))
            .property("metadata", Ref::from_schema_name(Metadata::name()))
            .required("metadata")
            .property("spec", Ref::from_schema_name(Spec::name()))
            .required("spec")
            .property("status", Ref::from_schema_name(Status::name()))
            .required("status")
            .into()
    }
}

impl<Spec: Kind + ToSchema, Status: ToSchema> ToSchema for Resource<Spec, Status> {
    fn name() -> Cow<'static, str> {
        Cow::Borrowed(Spec::KIND)
    }

    fn schemas(schemas: &mut Vec<(String, RefOr<Schema>)>) {
        schemas.push((Metadata::name().into_owned(), Metadata::schema()));
        schemas.push((Spec::name().into_owned(), Spec::schema()));
        schemas.push((Status::name().into_owned(), Status::schema()));
        Metadata::schemas(schemas);
        Spec::schemas(schemas);
        Status::schemas(schemas);
    }
}

/// Identity and bookkeeping shared by every resource.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Metadata {
    /// Unique per kind. See [`validate_name`].
    pub name: String,
    /// Unique forever, also across deletes and re-creates.
    pub uid: Uuid,
    /// Increases by one on every write; writes must name the generation they
    /// read, so concurrent writers cannot overwrite each other.
    pub generation: i64,
    /// Free-form key/value pairs for grouping and selection.
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    /// When the resource was created.
    pub created: Timestamp,
    /// When the resource was last written.
    pub updated: Timestamp,
}

/// The longest allowed resource name, as for a DNS label.
pub const MAX_NAME_LEN: usize = 63;

/// Checks that `name` is a DNS label: 1 to 63 characters of `a-z`, `0-9` and
/// `-`, not starting or ending with `-`.
///
/// # Errors
///
/// Returns a message that says what is wrong with the name.
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        return Err(format!("name must be 1 to {MAX_NAME_LEN} characters"));
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(format!("name {name:?} may only contain a-z, 0-9 and -"));
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err(format!("name {name:?} must not start or end with -"));
    }
    Ok(())
}

/// The state of one aspect of a resource, as in Kubernetes conditions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Condition {
    /// What this condition is about, for example [`READY`].
    #[serde(rename = "type")]
    pub kind: String,
    /// Whether the condition holds.
    pub status: ConditionStatus,
    /// A short machine-readable reason, for example `HeartbeatMissed`.
    pub reason: String,
    /// A human-readable explanation.
    pub message: String,
    /// When `status` last changed.
    pub last_transition: Timestamp,
}

/// Whether a [`Condition`] holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum ConditionStatus {
    /// The condition holds.
    True,
    /// The condition does not hold.
    False,
    /// Not known yet.
    Unknown,
}

/// The condition type that says whether a resource is ready.
pub const READY: &str = "Ready";

/// Finds the condition of type `kind` in `conditions`.
#[must_use]
pub fn condition<'a>(conditions: &'a [Condition], kind: &str) -> Option<&'a Condition> {
    conditions.iter().find(|c| c.kind == kind)
}

/// A machine that runs the node agent.
pub type Node = Resource<NodeSpec, NodeStatus>;

/// What the user wants from a node. Node labels live in
/// [`Metadata::labels`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct NodeSpec {}

impl Kind for NodeSpec {
    const KIND: &'static str = "Node";
    type Status = NodeStatus;
}

/// What the agent reports about a node, plus its conditions.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct NodeStatus {
    /// The host name the machine reports.
    pub hostname: String,
    /// CPU architecture, for example `x86_64` or `aarch64`.
    pub arch: String,
    /// Kernel version.
    pub kernel: String,
    /// Number of logical CPUs.
    pub cpus: u32,
    /// Total memory in bytes.
    pub memory_bytes: u64,
    /// When the agent last reported.
    pub last_heartbeat: Option<Timestamp>,
    /// Conditions, including [`READY`].
    #[serde(default)]
    pub conditions: Vec<Condition>,
}

impl NodeStatus {
    /// The status of the [`READY`] condition, or `Unknown` if it is not set.
    #[must_use]
    pub fn ready(&self) -> ConditionStatus {
        condition(&self.conditions, READY).map_or(ConditionStatus::Unknown, |c| c.status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_dns_label_rules() {
        assert!(validate_name("node-1").is_ok());
        assert!(validate_name(&"a".repeat(MAX_NAME_LEN)).is_ok());
        for bad in ["", "Node", "-a", "a-", "a.b", "a_b", &"a".repeat(64)] {
            assert!(validate_name(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn condition_type_serialises_as_type() {
        let c = Condition {
            kind: READY.into(),
            status: ConditionStatus::True,
            reason: "HeartbeatReceived".into(),
            message: String::new(),
            last_transition: Timestamp::UNIX_EPOCH,
        };
        let json = serde_json::to_value(&c).unwrap();
        assert_eq!(json["type"], "Ready");
        assert_eq!(json["status"], "True");
    }

    #[test]
    fn node_ready_defaults_to_unknown() {
        assert_eq!(NodeStatus::default().ready(), ConditionStatus::Unknown);
    }
}
