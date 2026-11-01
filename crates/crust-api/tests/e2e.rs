//! The control plane and a real agent, in one process: the node appears,
//! turns Ready, and turns NotReady when the agent stops.

#![allow(clippy::unwrap_used, reason = "test helpers panic on failure")]

use std::time::Duration;

use crust_store::{SqliteStore, Store};
use crust_types::{ConditionStatus, NodeSpec};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn agent_heartbeats_drive_node_readiness() {
    let store = SqliteStore::memory().await.unwrap();
    let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let agents = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let agent_addr = agents.local_addr().unwrap();
    let server_shutdown = CancellationToken::new();
    let server = tokio::spawn(crust_api::serve(
        store.clone(),
        Duration::from_millis(500),
        http,
        agents,
        server_shutdown.clone(),
    ));

    let agent_shutdown = CancellationToken::new();
    let config = crust_agent::Config {
        api: format!("http://{agent_addr}"),
        name: Some("e2e".into()),
        report_interval_ms: 100,
    };
    let agent = tokio::spawn(crust_agent::run(config, agent_shutdown.clone()));

    wait_for(&store, ConditionStatus::True).await;
    let node = store.get::<NodeSpec>("e2e").await.unwrap();
    assert!(node.status.cpus > 0);
    assert_eq!(node.status.arch, std::env::consts::ARCH);

    agent_shutdown.cancel();
    agent.await.unwrap().unwrap();
    wait_for(&store, ConditionStatus::False).await;

    server_shutdown.cancel();
    server.await.unwrap().unwrap();
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
