//! `crust`: the Crustates command-line client. It talks to the REST API.

use anyhow::Context;
use clap::{Parser, Subcommand};
use crust_types::{ConditionStatus, Node};
use serde::Deserialize;

/// Command-line client for Crustates.
#[derive(Debug, Parser)]
#[command(name = "crust", version)]
struct Cli {
    /// Base URL of the Crustates API.
    #[arg(
        long,
        global = true,
        env = "CRUST_API",
        default_value = "http://127.0.0.1:8080"
    )]
    api: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print the version of crust.
    Version,
    /// Show whether the API is up and how many nodes are ready.
    Status,
    /// Work with nodes.
    Node {
        #[command(subcommand)]
        command: NodeCommand,
    },
}

#[derive(Debug, Subcommand)]
enum NodeCommand {
    /// List all nodes.
    List,
}

#[derive(Debug, Deserialize)]
struct Health {
    version: String,
}

#[derive(Debug, Deserialize)]
struct NodeList {
    items: Vec<Node>,
}

// shortcut: plain HTTP only; add TLS when the API gets it (see docs/plan.md#security-model).
struct Client {
    http: reqwest::Client,
    api: String,
}

impl Client {
    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let url = format!("{}{path}", self.api.trim_end_matches('/'));
        let response = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("cannot reach {url}"))?;
        let response = response
            .error_for_status()
            .with_context(|| format!("GET {url} failed"))?;
        response
            .json()
            .await
            .with_context(|| format!("unexpected response from {url}"))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let client = Client {
        http: reqwest::Client::new(),
        api: cli.api.clone(),
    };
    match cli.command {
        Command::Version => println!("crust {}", env!("CARGO_PKG_VERSION")),
        Command::Status => {
            let health: Health = client.get("/healthz").await?;
            let nodes: NodeList = client.get("/v1/nodes").await?;
            let ready = nodes
                .items
                .iter()
                .filter(|n| n.status.ready() == ConditionStatus::True)
                .count();
            println!("API     {} (version {})", cli.api, health.version);
            println!("Nodes   {ready} of {} ready", nodes.items.len());
        }
        Command::Node {
            command: NodeCommand::List,
        } => {
            let nodes: NodeList = client.get("/v1/nodes").await?;
            print!("{}", node_table(&nodes.items));
        }
    }
    Ok(())
}

/// Formats nodes as an aligned table with a header row.
fn node_table(nodes: &[Node]) -> String {
    let mut rows = vec![["NAME", "STATUS", "ARCH", "CPUS", "MEMORY", "KERNEL"].map(String::from)];
    for node in nodes {
        let s = &node.status;
        let status = match s.ready() {
            ConditionStatus::True => "Ready",
            ConditionStatus::False => "NotReady",
            ConditionStatus::Unknown => "Unknown",
        };
        rows.push([
            node.metadata.name.clone(),
            status.to_owned(),
            s.arch.clone(),
            s.cpus.to_string(),
            gib(s.memory_bytes),
            s.kernel.clone(),
        ]);
    }
    let widths: Vec<usize> = (0..6)
        .map(|i| rows.iter().map(|r| r[i].len()).max().unwrap_or(0))
        .collect();
    let mut out = String::new();
    for row in rows {
        let cells: Vec<String> = row
            .iter()
            .zip(&widths)
            .map(|(cell, w)| format!("{cell:<w$}"))
            .collect();
        out.push_str(cells.join("  ").trim_end());
        out.push('\n');
    }
    out
}

/// Bytes as GiB with one decimal, without floating point.
fn gib(bytes: u64) -> String {
    const GIB: u64 = 1 << 30;
    format!("{}.{} GiB", bytes / GIB, bytes % GIB * 10 / GIB)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crust_types::{Condition, Metadata, NodeSpec, NodeStatus, READY, Resource};

    fn node(name: &str, ready: Option<ConditionStatus>) -> Node {
        let conditions = ready
            .map(|status| Condition {
                kind: READY.into(),
                status,
                reason: String::new(),
                message: String::new(),
                last_transition: jiff::Timestamp::UNIX_EPOCH,
            })
            .into_iter()
            .collect();
        Resource {
            metadata: Metadata {
                name: name.into(),
                uid: uuid::Uuid::nil(),
                generation: 1,
                labels: std::collections::BTreeMap::default(),
                created: jiff::Timestamp::UNIX_EPOCH,
                updated: jiff::Timestamp::UNIX_EPOCH,
            },
            spec: NodeSpec {},
            status: NodeStatus {
                hostname: name.into(),
                arch: "aarch64".into(),
                kernel: "6.17.1".into(),
                cpus: 8,
                memory_bytes: 16 << 30,
                last_heartbeat: None,
                conditions,
            },
        }
    }

    #[test]
    fn node_table_aligns_columns_and_names_readiness() {
        let table = node_table(&[
            node("attic", Some(ConditionStatus::True)),
            node("garage-nas", Some(ConditionStatus::False)),
            node("new", None),
        ]);
        assert_eq!(
            table,
            "NAME        STATUS    ARCH     CPUS  MEMORY    KERNEL\n\
             attic       Ready     aarch64  8     16.0 GiB  6.17.1\n\
             garage-nas  NotReady  aarch64  8     16.0 GiB  6.17.1\n\
             new         Unknown   aarch64  8     16.0 GiB  6.17.1\n"
        );
    }

    #[test]
    fn gib_rounds_down_to_one_decimal() {
        assert_eq!(gib(0), "0.0 GiB");
        assert_eq!(gib(3 << 29), "1.5 GiB");
        assert_eq!(gib((1 << 30) - 1), "0.9 GiB");
    }
}
