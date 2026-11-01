//! The node heartbeat reconciler, alone and inside the reconcile loop.

#![allow(clippy::unwrap_used, reason = "test helpers panic on failure")]

use std::time::Duration;

use crust_reconcile::node::NodeHeartbeat;
use crust_reconcile::{Action, Reconciler, run};
use crust_store::{SqliteStore, Store};
use crust_types::{ConditionStatus, NodeSpec, NodeStatus};
use jiff::{SignedDuration, Timestamp};
use tokio_util::sync::CancellationToken;

async fn node_with_heartbeat(store: &SqliteStore, name: &str, at: Option<Timestamp>) {
    let status = NodeStatus {
        last_heartbeat: at,
        ..NodeStatus::default()
    };
    store.create(name, NodeSpec {}, status).await.unwrap();
}

async fn ready(store: &SqliteStore, name: &str) -> ConditionStatus {
    store.get::<NodeSpec>(name).await.unwrap().status.ready()
}

#[tokio::test]
async fn fresh_heartbeat_is_ready_and_requeued_before_it_goes_stale() {
    let store = SqliteStore::memory().await.unwrap();
    node_with_heartbeat(&store, "n", Some(Timestamp::now())).await;
    let timeout = Duration::from_secs(15);

    let action = NodeHeartbeat::new(store.clone(), timeout)
        .reconcile("n")
        .await
        .unwrap();

    assert_eq!(ready(&store, "n").await, ConditionStatus::True);
    let Action::RequeueAfter(after) = action else {
        panic!("expected a requeue, got {action:?}")
    };
    assert!(
        after <= timeout + Duration::from_millis(50),
        "requeue after {after:?}"
    );
}

#[tokio::test]
async fn stale_heartbeat_is_not_ready() {
    let store = SqliteStore::memory().await.unwrap();
    let old = Timestamp::now() - SignedDuration::from_mins(5);
    node_with_heartbeat(&store, "n", Some(old)).await;

    let action = NodeHeartbeat::new(store.clone(), Duration::from_secs(15))
        .reconcile("n")
        .await
        .unwrap();

    assert_eq!(ready(&store, "n").await, ConditionStatus::False);
    assert_eq!(action, Action::Done);
}

#[tokio::test]
async fn no_heartbeat_is_unknown_and_a_missing_node_is_done() {
    let store = SqliteStore::memory().await.unwrap();
    node_with_heartbeat(&store, "n", None).await;
    let reconciler = NodeHeartbeat::new(store.clone(), Duration::from_secs(15));

    reconciler.reconcile("n").await.unwrap();
    assert_eq!(ready(&store, "n").await, ConditionStatus::Unknown);
    assert_eq!(reconciler.reconcile("gone").await.unwrap(), Action::Done);
}

#[tokio::test]
async fn unchanged_readiness_does_not_write() {
    let store = SqliteStore::memory().await.unwrap();
    node_with_heartbeat(&store, "n", Some(Timestamp::now())).await;
    let reconciler = NodeHeartbeat::new(store.clone(), Duration::from_secs(15));

    reconciler.reconcile("n").await.unwrap();
    let generation = store
        .get::<NodeSpec>("n")
        .await
        .unwrap()
        .metadata
        .generation;
    reconciler.reconcile("n").await.unwrap();
    assert_eq!(
        store
            .get::<NodeSpec>("n")
            .await
            .unwrap()
            .metadata
            .generation,
        generation
    );
}

#[tokio::test]
async fn loop_turns_a_silent_node_not_ready() {
    let store = SqliteStore::memory().await.unwrap();
    let shutdown = CancellationToken::new();
    let reconciler = NodeHeartbeat::new(store.clone(), Duration::from_millis(300));
    let task = tokio::spawn(run(store.clone(), reconciler, shutdown.clone()));

    node_with_heartbeat(&store, "n", Some(Timestamp::now())).await;
    wait_for(&store, "n", ConditionStatus::True).await;
    wait_for(&store, "n", ConditionStatus::False).await;

    shutdown.cancel();
    task.await.unwrap();
}

async fn wait_for(store: &SqliteStore, name: &str, want: ConditionStatus) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while ready(store, name).await != want {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{name} never became {want:?}"));
}
