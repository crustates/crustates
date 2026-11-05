//! The control plane and a real agent, in one process: the node appears,
//! turns Ready, runs a Molt upgrade and a channel switch sent through the
//! REST API, and turns NotReady when the agent stops.

#![allow(clippy::unwrap_used, reason = "test helpers panic on failure")]

use std::time::Duration;

use crust_agent::molt::{Deployment, FakeOs, Resolved};
use crust_api::{Auth, auth};
use crust_store::{SqliteStore, Store};
use crust_types::{ConditionStatus, NodeSpec};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn agent_heartbeats_drive_node_readiness_and_commands_reach_the_agent() {
    let dir = tempfile::tempdir().unwrap();
    let token = auth::new_token().unwrap();
    let token_file = dir.path().join("admin-token");
    std::fs::write(&token_file, auth::hash(&token).unwrap()).unwrap();

    let store = SqliteStore::memory().await.unwrap();
    let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api = format!("http://{}", http.local_addr().unwrap());
    let agents = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let agent_addr = agents.local_addr().unwrap();
    let server_shutdown = CancellationToken::new();
    let server = tokio::spawn(crust_api::serve(
        store.clone(),
        Duration::from_millis(500),
        Auth::new(token_file),
        crust_api::RecoveryPin::new(dir.path().join("recovery-pin")),
        crust_api::Listeners {
            http,
            agents,
            local: Some(tokio::net::UnixListener::bind(dir.path().join("api.sock")).unwrap()),
        },
        server_shutdown.clone(),
    ));

    let (os, molt_defaults) = molt_node(dir.path());
    let agent_shutdown = CancellationToken::new();
    let config = crust_agent::Config {
        api: format!("http://{agent_addr}"),
        name: Some("e2e".into()),
        report_interval_ms: 100,
        molt_defaults,
        molt_overrides: dir.path().join("etc/molt.toml"),
        state_dir: dir.path().join("state"),
    };
    let agent = tokio::spawn(crust_agent::run(config, Some(os), agent_shutdown.clone()));

    wait_for(&store, ConditionStatus::True).await;
    if !is_root() {
        assert_eq!(
            local_status(&dir.path().join("api.sock")).await,
            "HTTP/1.1 403 Forbidden"
        );
    }
    let node = store.get::<NodeSpec>("e2e").await.unwrap();
    assert!(node.status.cpus > 0);
    assert_eq!(node.status.arch, std::env::consts::ARCH);
    assert_eq!(node.status.molt.unwrap().booted.unwrap().version, "1.0");

    let client = reqwest::Client::new();
    let upgrade = format!("{api}/v1/nodes/e2e/molt/upgrade");
    let response = client
        .post(&upgrade)
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    // The agent's command stream may still be opening: retry while it is not connected.
    let body: Value = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let response = client
                .post(&upgrade)
                .bearer_auth(&token)
                .json(&serde_json::json!({}))
                .send()
                .await
                .unwrap();
            if response.status() != 503 {
                assert_eq!(response.status(), 200);
                return response.json().await.unwrap();
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        body["message"],
        "staged 1.1 (sha256:11); it boots at the next reboot"
    );

    // The next report carries the staged image.
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let node = store.get::<NodeSpec>("e2e").await.unwrap();
            if node.status.molt.and_then(|m| m.staged).is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    let audit = store.audit_log(1, None).await.unwrap();
    assert_eq!(
        (audit[0].action.as_str(), audit[0].outcome.as_str()),
        ("molt.upgrade", "ok")
    );

    switch_channel(&client, &api, &token).await;

    agent_shutdown.cancel();
    agent.await.unwrap().unwrap();
    wait_for(&store, ConditionStatus::False).await;

    server_shutdown.cancel();
    server.await.unwrap().unwrap();
}

/// Switches the node's channel through the API: unknown channels are a 400,
/// downgrades need `allow_downgrade`.
async fn switch_channel(client: &reqwest::Client, api: &str, token: &str) {
    let channel = format!("{api}/v1/nodes/e2e/molt/channel");
    let put = |body: Value| client.put(&channel).bearer_auth(token).json(&body).send();
    let response = put(serde_json::json!({ "channel": "nightly" }))
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    // "1.0" is not SemVer, so the agent cannot tell an upgrade from a downgrade.
    let response = put(serde_json::json!({ "channel": "development" }))
        .await
        .unwrap();
    assert_eq!(response.status(), 422);
    let body: Value = response.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("--allow-downgrade"),
        "{body}"
    );
    let body = serde_json::json!({ "channel": "development", "allow_downgrade": true });
    let response = put(body).await.unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(
        body["message"],
        "following development; available: 1.2 (sha256:12); crust molt upgrade stages it"
    );
}

/// The status line of a request to the local socket.
async fn local_status(socket: &std::path::Path) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::UnixStream::connect(socket).await.unwrap();
    stream
        .write_all(b"GET /v1/nodes HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    response.lines().next().unwrap_or_default().to_owned()
}

fn is_root() -> bool {
    std::fs::metadata("/proc/self").is_ok_and(|m| std::os::unix::fs::MetadataExt::uid(&m) == 0)
}

async fn wait_for(store: &SqliteStore, want: ConditionStatus) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(node) = store.get::<NodeSpec>("e2e").await
                && node.status.ready() == want
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("node never became {want:?}"));
}

/// A Molt node that boots 1.0 and follows beta, where 1.1 waits (1.2 on
/// development), and its update settings file.
fn molt_node(dir: &std::path::Path) -> (FakeOs, std::path::PathBuf) {
    let molt_defaults = dir.join("molt.toml");
    std::fs::write(
        &molt_defaults,
        "repository = \"example.org/molt\"\nchannel = \"beta\"\npolicy = \"manual\"\nwindow = \"03:00-05:00\"\n",
    )
    .unwrap();
    let os = FakeOs::default();
    {
        let mut state = os.state.lock().unwrap();
        state.deployments.booted = Some(Deployment {
            image: "example.org/molt:beta".into(),
            digest: "sha256:10".into(),
            version: "1.0".into(),
        });
        for (tag, version, digest) in [
            ("beta", "1.1", "sha256:11"),
            ("development", "1.2", "sha256:12"),
        ] {
            state.tags.insert(
                format!("example.org/molt:{tag}"),
                Resolved {
                    digest: digest.into(),
                    platform_digest: digest.into(),
                    version: version.into(),
                },
            );
        }
    }
    (os, molt_defaults)
}
