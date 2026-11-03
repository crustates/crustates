//! The Crustates node agent: reports this machine's facts to the control
//! plane every few seconds (each report is also the node's heartbeat), runs
//! the commands the control plane sends, and keeps Molt up to date.

pub mod bootc;
pub mod molt;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crust_proto::agent::v1::node_service_client::NodeServiceClient;
use crust_proto::agent::v1::{self as pb, CommandsRequest, ReportRequest, command};
use crust_types::{MAX_NAME_LEN, UpdatePolicy, Window};
use figment::Figment;
use figment::providers::{Env, Serialized};
use serde::{Deserialize, Serialize};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
use tokio::time::{MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;
use tonic::transport::{Channel, Endpoint};
use tracing::{debug, info, warn};

use crate::molt::{Os, Paths, Updater};

/// Agent settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// The control plane's agent endpoint.
    pub api: String,
    /// The node name; defaults to the host name made into a DNS label.
    pub name: Option<String>,
    /// Milliseconds between reports.
    pub report_interval_ms: u64,
    /// The image's update settings.
    pub molt_defaults: PathBuf,
    /// Local update settings, written by `crust molt policy`.
    pub molt_overrides: PathBuf,
    /// Where the agent remembers update state across reboots.
    pub state_dir: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api: "http://127.0.0.1:8081".into(),
            name: None,
            report_interval_ms: 5000,
            molt_defaults: "/usr/lib/crustates/molt.toml".into(),
            molt_overrides: "/etc/crustates/molt.toml".into(),
            state_dir: "/var/lib/crust-agent".into(),
        }
    }
}

impl Config {
    /// Where the update loop keeps its files.
    #[must_use]
    pub fn molt_paths(&self) -> Paths {
        Paths {
            defaults: self.molt_defaults.clone(),
            overrides: self.molt_overrides.clone(),
            state: self.state_dir.join("molt.json"),
        }
    }
}

impl Config {
    /// Loads defaults overridden by `CRUST_AGENT_<FIELD>` variables, for
    /// example `CRUST_AGENT_API`.
    ///
    /// # Errors
    ///
    /// Fails if a variable holds an invalid value.
    pub fn load() -> Result<Self, Box<figment::Error>> {
        Figment::from(Serialized::defaults(Config::default()))
            .merge(Env::prefixed("CRUST_AGENT_"))
            .extract()
            .map_err(Box::new)
    }
}

/// What can stop the agent.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The control plane endpoint is not a valid URI.
    #[error("invalid API endpoint: {0}")]
    Endpoint(#[from] tonic::transport::Error),
    /// This machine's facts could not be read.
    #[error("reading system facts: {0}")]
    Facts(#[from] sysinfo::Error),
}

/// Reports to the control plane every `report_interval_ms` and runs its
/// commands until `shutdown` fires. With `os` (a Molt machine) it also
/// keeps Molt up to date and reports its state. Failed reports are logged
/// and retried at the next tick.
///
/// # Errors
///
/// Fails if `config.api` is not a valid endpoint or the system facts cannot
/// be read.
pub async fn run<O: Os>(
    config: Config,
    os: Option<O>,
    shutdown: CancellationToken,
) -> Result<(), Error> {
    // TODO(mTLS): plain TCP for now; agents will authenticate with mTLS, see
    // the security model in docs/plan.md#security-model.
    let endpoint =
        Endpoint::from_shared(config.api.clone())?.connect_timeout(Duration::from_secs(5));
    let mut client = NodeServiceClient::new(
        endpoint
            .clone()
            .timeout(Duration::from_secs(5))
            .connect_lazy(),
    );
    // No timeout: the command stream stays open.
    let commands = NodeServiceClient::new(endpoint.connect_lazy());
    let mut report = facts(config.name.clone())?;
    let updater = os.map(|os| Arc::new(Updater::new(os, config.molt_paths())));
    info!(
        node = report.name,
        api = config.api,
        molt = updater.is_some(),
        "agent started"
    );

    if let Some(updater) = &updater {
        // The status is in the first report already.
        if let Err(e) = updater.refresh().await {
            warn!(error = %e, "reading the Molt status failed");
        }
        tokio::spawn(molt::run(Arc::clone(updater), shutdown.clone()));
    }
    tokio::spawn(serve_commands(
        commands,
        client.clone(),
        report.name.clone(),
        updater.clone(),
        shutdown.clone(),
    ));

    let mut ticks = interval(Duration::from_millis(config.report_interval_ms));
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = shutdown.cancelled() => return Ok(()),
            _ = ticks.tick() => {
                report.molt = updater.as_ref().map(|u| u.status().into());
                match client.report(report.clone()).await {
                    Ok(_) => debug!(node = report.name, "reported"),
                    Err(status) => warn!(node = report.name, %status, "report failed; retrying"),
                }
            }
        }
    }
}

/// Keeps a command stream open, runs each command and returns its result.
/// Reconnects after five seconds when the stream breaks.
async fn serve_commands<O: Os>(
    mut stream_client: NodeServiceClient<Channel>,
    mut client: NodeServiceClient<Channel>,
    name: String,
    updater: Option<Arc<Updater<O>>>,
    shutdown: CancellationToken,
) {
    loop {
        let session = async {
            let mut stream = stream_client
                .commands(CommandsRequest { name: name.clone() })
                .await?
                .into_inner();
            debug!(node = name, "waiting for commands");
            while let Some(command) = stream.message().await? {
                let id = command.id;
                let result = execute(command, updater.as_ref()).await;
                if !result.error.is_empty() {
                    warn!(command = id, error = result.error, "command failed");
                }
                client.complete_command(result).await?;
            }
            Ok::<_, tonic::Status>(())
        };
        tokio::select! {
            () = shutdown.cancelled() => return,
            result = session => if let Err(status) = result {
                debug!(%status, "command stream ended; reconnecting");
            },
        }
        tokio::select! {
            () = shutdown.cancelled() => return,
            () = tokio::time::sleep(Duration::from_secs(5)) => {}
        }
    }
}

/// Runs one command. Molt commands need a Molt machine.
async fn execute<O: Os>(
    command: pb::Command,
    updater: Option<&Arc<Updater<O>>>,
) -> pb::CommandResult {
    let mut result = pb::CommandResult {
        id: command.id,
        ..pb::CommandResult::default()
    };
    let Some(kind) = command.kind else {
        result.error = "this agent does not know the command; update the agent".into();
        return result;
    };
    let Some(updater) = updater else {
        result.error = "this node does not run Molt".into();
        return result;
    };
    let name = match &kind {
        command::Kind::MoltUpgrade(_) => "molt upgrade",
        command::Kind::MoltRollback(_) => "molt rollback",
        command::Kind::MoltPolicy(_) => "molt policy",
        command::Kind::MoltEnrollKey(_) => "molt enroll-key",
        command::Kind::MoltChannel(_) => "molt channel",
    };
    info!(command = command.id, name, "running a command");
    let outcome = match kind {
        command::Kind::MoltUpgrade(c) => updater.upgrade(c.reboot).await,
        command::Kind::MoltRollback(c) => updater.rollback(c.reboot).await,
        command::Kind::MoltPolicy(c) => {
            let policy = match c.policy() {
                pb::UpdatePolicy::Manual => UpdatePolicy::Manual,
                pb::UpdatePolicy::Auto | pb::UpdatePolicy::Unspecified => UpdatePolicy::Auto,
            };
            match (!c.window.is_empty())
                .then(|| c.window.parse::<Window>())
                .transpose()
            {
                Ok(window) => updater.set_policy(policy, window).await,
                Err(e) => Err(molt::Error::Settings(e)),
            }
        }
        command::Kind::MoltChannel(c) => updater.set_channel(&c.channel, c.allow_downgrade).await,
        command::Kind::MoltEnrollKey(_) => updater.enroll_key().await.map(|(message, password)| {
            result.secret = password;
            message
        }),
    };
    match outcome {
        Ok(message) => result.message = message,
        Err(e) => result.error = e.to_string(),
    }
    result
}

/// This machine's facts. They only change with a reboot, so they are read once.
fn facts(name: Option<String>) -> Result<ReportRequest, sysinfo::Error> {
    let system = System::new_with_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::nothing())
            .with_memory(MemoryRefreshKind::nothing().with_ram()),
    )?;
    let hostname = System::host_name().unwrap_or_default();
    Ok(ReportRequest {
        name: name.unwrap_or_else(|| node_name(&hostname)),
        hostname,
        arch: std::env::consts::ARCH.to_owned(),
        kernel: System::kernel_version().unwrap_or_default(),
        cpus: u32::try_from(system.cpus().len()).unwrap_or(u32::MAX),
        memory_bytes: system.total_memory(),
        molt: None,
    })
}

/// Turns a host name into a node name: the first label, lower case, with
/// anything outside `a-z`, `0-9` and `-` replaced by `-`.
fn node_name(hostname: &str) -> String {
    let label = hostname
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let label: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                c
            } else {
                '-'
            }
        })
        .take(MAX_NAME_LEN)
        .collect();
    let label = label.trim_matches('-');
    if label.is_empty() {
        "node".to_owned()
    } else {
        label.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_names_become_valid_node_names() {
        for (host, want) in [
            ("Miguels-MacBook-Pro.local", "miguels-macbook-pro"),
            ("nas", "nas"),
            ("my_box.lan", "my-box"),
            ("-edge-", "edge"),
            ("", "node"),
            ("...", "node"),
        ] {
            let name = node_name(host);
            assert_eq!(name, want, "for {host:?}");
            crust_types::validate_name(&name).unwrap();
        }
        let long = node_name(&"a".repeat(100));
        assert_eq!(long.len(), MAX_NAME_LEN);
    }

    #[test]
    fn facts_describe_this_machine() {
        let report = facts(Some("test".into())).unwrap();
        assert_eq!(report.name, "test");
        assert!(report.cpus > 0);
        assert!(report.memory_bytes > 0);
        assert_ne!(report.arch, "");
    }
}
