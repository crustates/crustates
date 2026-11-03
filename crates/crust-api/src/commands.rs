//! Commands from the API to node agents. Agents dial out and keep a stream
//! open (`NodeService.Commands`); a command waits here until the agent
//! returns its result (`NodeService.CompleteCommand`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crust_proto::agent::v1 as pb;
use tokio::sync::{mpsc, oneshot};

/// Agents that are connected, and commands waiting for their result.
#[derive(Clone, Debug, Default)]
pub struct Commands {
    inner: Arc<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    agents: Mutex<HashMap<String, mpsc::Sender<pb::Command>>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<pb::CommandResult>>>,
    next_id: AtomicU64,
}

/// Why a command did not complete.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    /// The node's agent has no command stream open.
    #[error("the agent on node {0:?} is not connected")]
    NotConnected(String),
    /// The agent did not answer in time; the command may still run.
    #[error("the agent did not answer within {} s; the command may still finish", .0.as_secs())]
    Timeout(Duration),
    /// The agent ran the command and it failed.
    #[error("{0}")]
    Failed(String),
}

/// Removes a pending command when its caller stops waiting.
struct Pending<'a>(&'a Inner, u64);

impl Drop for Pending<'_> {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.0.pending.lock() {
            pending.remove(&self.1);
        }
    }
}

impl Commands {
    /// Registers the agent of `node` and returns the commands for it. A
    /// second stream for the same node replaces the first.
    #[must_use]
    pub fn connect(&self, node: &str) -> mpsc::Receiver<pb::Command> {
        let (tx, rx) = mpsc::channel(8);
        if let Ok(mut agents) = self.inner.agents.lock() {
            agents.insert(node.to_owned(), tx);
        }
        rx
    }

    /// Sends `kind` to the agent of `node` and waits up to `timeout` for the
    /// result.
    ///
    /// # Errors
    ///
    /// Fails if the agent is not connected, does not answer in time, or
    /// reports that the command failed.
    pub async fn run(
        &self,
        node: &str,
        kind: pb::command::Kind,
        timeout: Duration,
    ) -> Result<pb::CommandResult, CommandError> {
        let not_connected = || CommandError::NotConnected(node.to_owned());
        let agent = self
            .inner
            .agents
            .lock()
            .ok()
            .and_then(|agents| agents.get(node).cloned())
            .ok_or_else(not_connected)?;
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        if let Ok(mut pending) = self.inner.pending.lock() {
            pending.insert(id, tx);
        }
        let _pending = Pending(&self.inner, id);
        let command = pb::Command {
            id,
            kind: Some(kind),
        };
        if agent.send(command).await.is_err() {
            return Err(not_connected());
        }
        match tokio::time::timeout(timeout, rx).await {
            Err(_) => Err(CommandError::Timeout(timeout)),
            // The stream ended before the agent answered.
            Ok(Err(_)) => Err(not_connected()),
            Ok(Ok(result)) if !result.error.is_empty() => Err(CommandError::Failed(result.error)),
            Ok(Ok(result)) => Ok(result),
        }
    }

    /// Hands an agent's result to the command waiting for it. Returns false
    /// if nothing waits for it any more.
    #[must_use]
    pub fn complete(&self, result: pb::CommandResult) -> bool {
        let waiting = self
            .inner
            .pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.remove(&result.id));
        waiting.is_some_and(|tx| tx.send(result).is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upgrade() -> pb::command::Kind {
        pb::command::Kind::MoltUpgrade(pb::MoltUpgrade { reboot: false })
    }

    #[tokio::test]
    async fn results_reach_the_waiting_command() {
        let commands = Commands::default();
        let mut rx = commands.connect("a");
        let agent = commands.clone();
        tokio::spawn(async move {
            while let Some(command) = rx.recv().await {
                assert!(agent.complete(pb::CommandResult {
                    id: command.id,
                    message: "staged".into(),
                    ..Default::default()
                }));
            }
        });
        let result = commands
            .run("a", upgrade(), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(result.message, "staged");
    }

    #[tokio::test]
    async fn unknown_silent_and_failing_agents_are_errors() {
        let commands = Commands::default();
        let err = commands
            .run("nobody", upgrade(), Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::NotConnected(_)), "{err:?}");

        let _silent = commands.connect("silent");
        let err = commands
            .run("silent", upgrade(), Duration::from_millis(50))
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Timeout(_)), "{err:?}");
        assert!(commands.inner.pending.lock().unwrap().is_empty());

        let mut rx = commands.connect("broken");
        let agent = commands.clone();
        tokio::spawn(async move {
            let command = rx.recv().await.unwrap();
            assert!(agent.complete(pb::CommandResult {
                id: command.id,
                error: "signature check failed".into(),
                ..Default::default()
            }));
        });
        let err = commands
            .run("broken", upgrade(), Duration::from_secs(5))
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "signature check failed");
    }
}
