//! Marks nodes `Ready` or `NotReady` from agent heartbeats.

use std::time::Duration;

use crust_store::{Error, Store};
use crust_types::{Condition, ConditionStatus, NodeSpec, READY, condition};
use jiff::Timestamp;

use crate::{Action, Reconciler};

/// Sets each node's [`READY`] condition: `True` while its agent reported
/// within `timeout`, `False` after that, `Unknown` before the first report.
#[derive(Clone, Debug)]
pub struct NodeHeartbeat<S> {
    store: S,
    timeout: Duration,
}

impl<S> NodeHeartbeat<S> {
    /// A reconciler that calls a node `NotReady` after `timeout` without a
    /// report.
    pub fn new(store: S, timeout: Duration) -> Self {
        Self { store, timeout }
    }
}

impl<S: Store> Reconciler for NodeHeartbeat<S> {
    type Kind = NodeSpec;
    type Error = Error;

    async fn reconcile(&self, name: &str) -> Result<Action, Error> {
        let mut node = match self.store.get::<NodeSpec>(name).await {
            Err(Error::NotFound { .. }) => return Ok(Action::Done),
            result => result?,
        };
        let now = Timestamp::now();
        let age = node
            .status
            .last_heartbeat
            .map(|at| Duration::try_from(now.duration_since(at)).unwrap_or_default());

        let (status, reason, message, action) = match age {
            Some(age) if age < self.timeout => (
                ConditionStatus::True,
                "HeartbeatReceived",
                "the agent reports regularly".to_owned(),
                // Check again just after the heartbeat would go stale.
                Action::RequeueAfter(self.timeout.saturating_sub(age) + Duration::from_millis(50)),
            ),
            Some(_) => (
                ConditionStatus::False,
                "HeartbeatMissed",
                format!(
                    "no report from the agent for more than {} s",
                    self.timeout.as_secs_f32()
                ),
                Action::Done,
            ),
            None => (
                ConditionStatus::Unknown,
                "NoHeartbeat",
                "the agent has not reported yet".to_owned(),
                Action::Done,
            ),
        };

        let current = condition(&node.status.conditions, READY);
        if current.is_some_and(|c| c.status == status && c.reason == reason) {
            return Ok(action);
        }
        let ready = Condition {
            kind: READY.to_owned(),
            status,
            reason: reason.to_owned(),
            message,
            last_transition: now,
        };
        node.status.conditions.retain(|c| c.kind != READY);
        node.status.conditions.push(ready);
        tracing::info!(node = name, ?status, reason, "node readiness changed");
        // A conflict means the agent reported meanwhile; the loop retries.
        self.store.update(node).await?;
        Ok(action)
    }
}
