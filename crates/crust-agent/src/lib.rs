//! The Crustates node agent: reports this machine's facts to the control
//! plane every few seconds. Each report is also the node's heartbeat.

use std::time::Duration;

use crust_proto::agent::v1::ReportRequest;
use crust_proto::agent::v1::node_service_client::NodeServiceClient;
use crust_types::MAX_NAME_LEN;
use figment::Figment;
use figment::providers::{Env, Serialized};
use serde::{Deserialize, Serialize};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
use tokio::time::{MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;
use tonic::transport::Endpoint;
use tracing::{debug, info, warn};

/// Agent settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// The control plane's agent endpoint.
    pub api: String,
    /// The node name; defaults to the host name made into a DNS label.
    pub name: Option<String>,
    /// Milliseconds between reports.
    pub report_interval_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api: "http://127.0.0.1:8081".into(),
            name: None,
            report_interval_ms: 5000,
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

/// Reports to the control plane every `report_interval_ms` until `shutdown`
/// fires. Failed reports are logged and retried at the next tick.
///
/// # Errors
///
/// Fails if `config.api` is not a valid endpoint or the system facts cannot
/// be read.
pub async fn run(config: Config, shutdown: CancellationToken) -> Result<(), Error> {
    // TODO(mTLS): plain TCP for now; agents will authenticate with mTLS, see
    // the security model in docs/plan.md#security-model.
    let channel = Endpoint::from_shared(config.api.clone())?
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(5))
        .connect_lazy();
    let mut client = NodeServiceClient::new(channel);
    let report = facts(config.name)?;
    info!(node = report.name, api = config.api, "agent started");

    let mut ticks = interval(Duration::from_millis(config.report_interval_ms));
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = shutdown.cancelled() => return Ok(()),
            _ = ticks.tick() => match client.report(report.clone()).await {
                Ok(_) => debug!(node = report.name, "reported"),
                Err(status) => warn!(node = report.name, %status, "report failed; retrying"),
            },
        }
    }
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
        assert!(!report.arch.is_empty());
    }
}
