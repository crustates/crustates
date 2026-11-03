//! `crust`: the Crustates command-line client. It talks to the REST API.

use std::fmt::Write as _;
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use crust_types::{CHANNELS, ConditionStatus, MoltImage, MoltStatus, Node, UpdatePolicy};
use serde::{Deserialize, Serialize};

/// The API when neither `--api` nor a login names one.
const DEFAULT_API: &str = "http://127.0.0.1:8080";

/// Command-line client for Crustates.
#[derive(Debug, Parser)]
#[command(name = "crust", version)]
struct Cli {
    /// Base URL of the Crustates API. Defaults to the one you last logged in
    /// to, else <http://127.0.0.1:8080>.
    #[arg(long, global = true, env = "CRUST_API")]
    api: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print the version of crust.
    Version,
    /// Show whether the API is up and how many nodes are ready.
    Status,
    /// Save the admin token for this API, after checking it.
    ///
    /// The token (crt_…) is shown on the machine's screen at its first boot.
    /// It is saved in ~/.config/crust/config.toml, readable only by you, and
    /// sent only to this API. `CRUST_TOKEN` overrides it.
    Login,
    /// Work with nodes.
    Node {
        #[command(subcommand)]
        command: NodeCommand,
    },
    /// The Molt OS image of a node and its updates.
    Molt {
        /// The node; can be left out when there is only one.
        #[arg(long, global = true)]
        node: Option<String>,
        #[command(subcommand)]
        command: MoltCommand,
    },
}

#[derive(Debug, Subcommand)]
enum NodeCommand {
    /// List all nodes.
    List,
}

#[derive(Debug, Subcommand)]
enum MoltCommand {
    /// Show the booted, staged and previous image, and the update settings.
    Status,
    /// Check the channel and stage a newer image (its signature is checked).
    Upgrade {
        /// Reboot into it right away.
        #[arg(long)]
        now: bool,
    },
    /// Make the previous image boot next.
    Rollback {
        /// Reboot into it right away.
        #[arg(long)]
        now: bool,
    },
    /// Change how updates are applied.
    Policy {
        #[command(subcommand)]
        command: PolicyCommand,
    },
    /// Change the channel the node follows.
    Channel {
        #[command(subcommand)]
        command: ChannelCommand,
    },
    /// Queue the Crustates module-signing key for Secure Boot (MOK).
    ///
    /// Someone at the machine confirms it at the next boot with the one-time
    /// password this prints.
    EnrollKey,
}

#[derive(Debug, Subcommand)]
enum PolicyCommand {
    /// auto: stage new images and reboot in the window. manual: only report
    /// them; stage with `crust molt upgrade`.
    Set {
        /// auto or manual.
        policy: UpdatePolicy,
        /// When auto may reboot, HH:MM-HH:MM in the node's local time.
        #[arg(long)]
        window: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum ChannelCommand {
    /// stable: releases after a week on beta. beta: every release.
    /// development: every merge. The node checks the channel at once; with
    /// the auto policy it stages a newer image there right away.
    Set {
        /// The channel.
        #[arg(value_parser = CHANNELS)]
        channel: String,
        /// Switch even if the channel holds an older version than the node
        /// runs. An older version may not read what a newer one wrote.
        #[arg(long)]
        allow_downgrade: bool,
    },
}

#[derive(Debug, Deserialize)]
struct Health {
    version: String,
}

#[derive(Debug, Deserialize)]
struct NodeList {
    items: Vec<Node>,
}

#[derive(Debug, Deserialize)]
struct Outcome {
    message: String,
}

#[derive(Debug, Deserialize)]
struct Enrolment {
    message: String,
    password: String,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    error: String,
}

/// `~/.config/crust/config.toml`: the API last logged in to and its token.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Saved {
    api: Option<String>,
    token: Option<String>,
}

fn config_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("crust/config.toml"))
}

fn load_saved() -> anyhow::Result<Saved> {
    let Some(file) = config_file() else {
        return Ok(Saved::default());
    };
    match std::fs::read_to_string(&file) {
        Ok(text) => toml::from_str(&text).with_context(|| format!("reading {}", file.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Saved::default()),
        Err(e) => Err(e).with_context(|| format!("reading {}", file.display())),
    }
}

/// Writes the config file, readable only by this user.
fn save(saved: &Saved) -> anyhow::Result<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let file = config_file().context("cannot tell where the config goes: HOME is not set")?;
    if let Some(dir) = file.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .with_context(|| format!("creating {}", dir.display()))?;
    }
    let text = toml::to_string(saved)?;
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&file)
        .with_context(|| format!("writing {}", file.display()))?;
    out.write_all(text.as_bytes())?;
    Ok(file)
}

// shortcut: plain HTTP only; add TLS when the API gets it (see docs/plan.md#security-model).
struct Client {
    http: reqwest::Client,
    api: String,
    token: Option<String>,
}

impl Client {
    async fn send<T: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> anyhow::Result<T> {
        let url = format!("{}{path}", self.api.trim_end_matches('/'));
        let mut request = self.http.request(method.clone(), &url);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .with_context(|| format!("cannot reach {url}"))?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            bail!(
                "{} rejected the request: log in with `crust --api {} login`",
                self.api,
                self.api
            );
        }
        if !status.is_success() {
            let error = response
                .json::<ErrorBody>()
                .await
                .map_or_else(|_| status.to_string(), |e| e.error);
            bail!("{method} {url} failed: {error}");
        }
        response
            .json()
            .await
            .with_context(|| format!("unexpected response from {url}"))
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        self.send(reqwest::Method::GET, path, None).await
    }

    /// The node named by `--node`, or the only node.
    async fn node(&self, name: Option<String>) -> anyhow::Result<String> {
        if let Some(name) = name {
            return Ok(name);
        }
        let nodes: NodeList = self.get("/v1/nodes").await?;
        match nodes.items.as_slice() {
            [only] => Ok(only.metadata.name.clone()),
            [] => bail!("there are no nodes yet"),
            many => {
                let names: Vec<&str> = many.iter().map(|n| n.metadata.name.as_str()).collect();
                bail!(
                    "there are {} nodes; pick one with --node: {}",
                    names.len(),
                    names.join(", ")
                )
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let saved = load_saved()?;
    let api = cli
        .api
        .clone()
        .or_else(|| saved.api.clone())
        .unwrap_or_else(|| DEFAULT_API.to_owned());
    let token = token_for(&saved, &api, std::env::var("CRUST_TOKEN").ok());
    let client = Client {
        http: reqwest::Client::new(),
        api: api.clone(),
        token,
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
            println!("API     {api} (version {})", health.version);
            println!("Nodes   {ready} of {} ready", nodes.items.len());
        }
        Command::Login => login(client).await?,
        Command::Node {
            command: NodeCommand::List,
        } => {
            let nodes: NodeList = client.get("/v1/nodes").await?;
            print!("{}", node_table(&nodes.items));
        }
        Command::Molt { node, command } => molt(&client, node, command).await?,
    }
    Ok(())
}

/// `CRUST_TOKEN` if set, else the saved token, but only for the API it was
/// saved for: a token never goes to another server.
fn token_for(saved: &Saved, api: &str, env: Option<String>) -> Option<String> {
    env.filter(|t| !t.is_empty()).or_else(|| {
        saved
            .token
            .clone()
            .filter(|_| saved.api.as_deref() == Some(api))
    })
}

async fn login(mut client: Client) -> anyhow::Result<()> {
    let token = if std::io::stdin().is_terminal() {
        rpassword::prompt_password("Admin token (crt_…): ")?
    } else {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        line
    };
    let token = token.trim().to_owned();
    if token.is_empty() {
        bail!("no token given");
    }
    client.token = Some(token.clone());
    let _: NodeList = client.get("/v1/nodes").await?;
    let file = save(&Saved {
        api: Some(client.api.clone()),
        token: Some(token),
    })?;
    println!(
        "Logged in to {}; the token is saved in {}",
        client.api,
        file.display()
    );
    Ok(())
}

async fn molt(client: &Client, node: Option<String>, command: MoltCommand) -> anyhow::Result<()> {
    let node = client.node(node).await?;
    let path = |action: &str| format!("/v1/nodes/{node}/molt/{action}");
    let post = reqwest::Method::POST;
    match command {
        MoltCommand::Status => {
            let node: Node = client.get(&format!("/v1/nodes/{node}")).await?;
            match &node.status.molt {
                Some(molt) => print!("{}", molt_table(&node.metadata.name, molt)),
                None => println!("{} does not run Molt", node.metadata.name),
            }
        }
        MoltCommand::Upgrade { now } => {
            eprintln!("Checking the channel of {node}; downloading a new image can take minutes…");
            let body = serde_json::json!({ "reboot": now });
            let outcome: Outcome = client.send(post, &path("upgrade"), Some(body)).await?;
            println!("{}", outcome.message);
        }
        MoltCommand::Rollback { now } => {
            let body = serde_json::json!({ "reboot": now });
            let outcome: Outcome = client.send(post, &path("rollback"), Some(body)).await?;
            println!("{}", outcome.message);
        }
        MoltCommand::Policy {
            command: PolicyCommand::Set { policy, window },
        } => {
            let body = serde_json::json!({ "policy": policy, "window": window });
            let outcome: Outcome = client
                .send(reqwest::Method::PUT, &path("policy"), Some(body))
                .await?;
            println!("{}", outcome.message);
        }
        MoltCommand::Channel {
            command:
                ChannelCommand::Set {
                    channel,
                    allow_downgrade,
                },
        } => {
            eprintln!("Checking {channel} for {node}; downloading a new image can take minutes…");
            let body =
                serde_json::json!({ "channel": channel, "allow_downgrade": allow_downgrade });
            let outcome: Outcome = client
                .send(reqwest::Method::PUT, &path("channel"), Some(body))
                .await?;
            println!("{}", outcome.message);
        }
        MoltCommand::EnrollKey => {
            let enrolment: Enrolment = client.send(post, &path("enroll-key"), None).await?;
            println!("{}.", enrolment.message);
            println!("One-time password: {}", enrolment.password);
            println!("Check the fingerprint against the release notes before you confirm.");
        }
    }
    Ok(())
}

/// The Molt status of `node` as aligned lines.
fn molt_table(node: &str, m: &MoltStatus) -> String {
    let image = |i: &MoltImage| {
        let short = i.digest.get(..19).unwrap_or(&i.digest);
        let version = if i.version.is_empty() {
            "-"
        } else {
            &i.version
        };
        let signature = if i.verified {
            "  signature verified"
        } else {
            ""
        };
        format!("{version}  {short}{signature}")
    };
    let mut rows = vec![("Node", node.to_owned())];
    rows.push((
        "Channel",
        if m.channel.is_empty() {
            "not picked yet: the first check picks stable or beta".into()
        } else {
            format!("{} ({}:{})", m.channel, m.repository, m.channel)
        },
    ));
    rows.push((
        "Policy",
        match m.policy {
            UpdatePolicy::Auto => format!(
                "auto, reboots between {} (local time)",
                m.window.replacen('-', " and ", 1)
            ),
            UpdatePolicy::Manual => "manual, new images are only reported".into(),
        },
    ));
    let next = if m.rollback_queued {
        "  (boots next)"
    } else {
        ""
    };
    rows.push(("Booted", m.booted.as_ref().map_or("-".into(), image)));
    rows.push((
        "Staged",
        m.staged
            .as_ref()
            .map_or("-".into(), |i| format!("{}  (boots next)", image(i))),
    ));
    rows.push((
        "Rollback",
        m.rollback
            .as_ref()
            .map_or("-".into(), |i| format!("{}{next}", image(i))),
    ));
    let checked = m.last_check.map_or("not checked yet".into(), |t| {
        format!("checked {}", t.strftime("%Y-%m-%d %H:%M UTC"))
    });
    rows.push((
        "Available",
        m.available
            .as_ref()
            .map_or(format!("none ({checked})"), |i| {
                format!("{}  ({checked})", image(i))
            }),
    ));
    if !m.last_error.is_empty() {
        rows.push(("Problem", m.last_error.clone()));
    }
    if !m.bad_digests.is_empty() {
        rows.push(("Marked bad", m.bad_digests.join(", ")));
    }
    rows.push((
        "Rollback on failure",
        if m.automatic_rollback {
            "on: an image that fails to boot or its health check is rolled back".into()
        } else {
            "off: the boot loader has no boot counter (reinstall to get it)".into()
        },
    ));
    let width = rows.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    rows.iter().fold(String::new(), |mut out, (k, v)| {
        let _ = writeln!(out, "{k:<width$}  {v}");
        out
    })
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
                molt: None,
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

    #[test]
    fn molt_table_says_what_boots_next() {
        let image = |version: &str, digest: &str, verified| MoltImage {
            image: format!("ghcr.io/crustates/molt@{digest}"),
            digest: digest.into(),
            version: version.into(),
            verified,
        };
        let status = MoltStatus {
            booted: Some(image("0.1.0", "sha256:0123456789abcdef", false)),
            staged: Some(image("0.2.0", "sha256:fedcba9876543210", true)),
            repository: "ghcr.io/crustates/molt".into(),
            channel: "beta".into(),
            window: "03:00-05:00".into(),
            automatic_rollback: true,
            ..MoltStatus::default()
        };
        let table = molt_table("attic", &status);
        assert!(
            table.contains(
                "Policy               auto, reboots between 03:00 and 05:00 (local time)\n"
            ),
            "{table}"
        );
        assert!(
            table.contains("Booted               0.1.0  sha256:0123456789ab\n"),
            "{table}"
        );
        assert!(table.contains("Staged               0.2.0  sha256:fedcba987654  signature verified  (boots next)\n"), "{table}");
        assert!(
            table.contains("Available            none (not checked yet)\n"),
            "{table}"
        );
        assert!(table.contains("Rollback on failure  on:"), "{table}");
    }

    #[test]
    fn saved_tokens_go_only_to_their_api() {
        let saved: Saved = toml::from_str("api = \"http://a:8080\"\ntoken = \"crt_x\"\n").unwrap();
        assert_eq!(
            token_for(&saved, "http://a:8080", None).as_deref(),
            Some("crt_x")
        );
        assert_eq!(token_for(&saved, "http://b:8080", None), None);
        let env = Some("crt_env".to_owned());
        assert_eq!(
            token_for(&saved, "http://b:8080", env).as_deref(),
            Some("crt_env")
        );
    }
}
