//! The contract every [`Store`] implementation must pass.
//!
//! Call [`run`] from the implementation's tests with a fresh, empty store.

#![allow(
    clippy::missing_panics_doc,
    reason = "contract checks panic on failure, like tests"
)]

use std::time::Duration;

use futures::StreamExt;
use serde::{Deserialize, Serialize};

use crust_types::Kind;

use crate::{Error, Event, Store};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Widget {
    size: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct WidgetStatus {
    seen: bool,
}

impl Kind for Widget {
    const KIND: &'static str = "ContractWidget";
    type Status = WidgetStatus;
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Gadget {}

impl Kind for Gadget {
    const KIND: &'static str = "ContractGadget";
    type Status = WidgetStatus;
}

/// Runs every contract check against `store`, which must start empty.
pub async fn run<S: Store>(store: S) {
    create_then_get(&store).await;
    create_twice_fails(&store).await;
    get_missing_fails(&store).await;
    update_bumps_generation(&store).await;
    stale_update_conflicts(&store).await;
    update_missing_fails(&store).await;
    list_is_per_kind_and_sorted(&store).await;
    watch_sees_changes_in_order(&store).await;
}

async fn create_then_get<S: Store>(store: &S) {
    let created = store
        .create("a", Widget { size: 1 }, WidgetStatus::default())
        .await
        .expect("create");
    assert_eq!(created.metadata.name, "a");
    assert_eq!(created.metadata.generation, 1);
    assert!(created.metadata.labels.is_empty());
    assert_eq!(created.metadata.created, created.metadata.updated);

    let read = store.get::<Widget>("a").await.expect("get");
    assert_eq!(read, created);
}

async fn create_twice_fails<S: Store>(store: &S) {
    store
        .create("twice", Widget::default(), WidgetStatus::default())
        .await
        .expect("first create");
    let err = store
        .create("twice", Widget::default(), WidgetStatus::default())
        .await
        .expect_err("second create");
    assert!(matches!(err, Error::AlreadyExists { .. }), "got {err:?}");
}

async fn get_missing_fails<S: Store>(store: &S) {
    let err = store
        .get::<Widget>("missing")
        .await
        .expect_err("get missing");
    assert!(matches!(err, Error::NotFound { .. }), "got {err:?}");
}

async fn update_bumps_generation<S: Store>(store: &S) {
    let mut object = store
        .create("upd", Widget { size: 1 }, WidgetStatus::default())
        .await
        .expect("create");
    object.spec.size = 2;
    object.status.seen = true;
    object.metadata.labels.insert("zone".into(), "attic".into());

    let updated = store.update(object.clone()).await.expect("update");
    assert_eq!(updated.metadata.generation, 2);
    assert_eq!(updated.metadata.uid, object.metadata.uid);
    assert_eq!(updated.metadata.created, object.metadata.created);
    assert!(updated.metadata.updated >= object.metadata.updated);

    let read = store.get::<Widget>("upd").await.expect("get");
    assert_eq!(read, updated);
    assert_eq!(read.spec.size, 2);
    assert!(read.status.seen);
    assert_eq!(
        read.metadata.labels.get("zone").map(String::as_str),
        Some("attic")
    );
}

async fn stale_update_conflicts<S: Store>(store: &S) {
    let first = store
        .create("race", Widget::default(), WidgetStatus::default())
        .await
        .expect("create");
    let second = first.clone();
    store.update(first).await.expect("first writer wins");

    let err = store
        .update(second)
        .await
        .expect_err("second writer must conflict");
    assert!(
        matches!(err, Error::Conflict { expected: 1, .. }),
        "got {err:?}"
    );
    assert_eq!(
        store
            .get::<Widget>("race")
            .await
            .expect("get")
            .metadata
            .generation,
        2
    );
}

async fn update_missing_fails<S: Store>(store: &S) {
    let mut ghost = store
        .create("ghost", Widget::default(), WidgetStatus::default())
        .await
        .expect("create");
    ghost.metadata.name = "never-created".into();
    let err = store.update(ghost).await.expect_err("update missing");
    assert!(matches!(err, Error::NotFound { .. }), "got {err:?}");
}

async fn list_is_per_kind_and_sorted<S: Store>(store: &S) {
    store
        .create("same-name", Gadget {}, WidgetStatus::default())
        .await
        .expect("gadget");
    store
        .create("zz", Widget::default(), WidgetStatus::default())
        .await
        .expect("widget zz");

    let names: Vec<String> = store
        .list::<Widget>()
        .await
        .expect("list")
        .into_iter()
        .map(|w| w.metadata.name)
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
    assert!(names.contains(&"zz".to_owned()));
    assert!(
        !names.contains(&"same-name".to_owned()),
        "list must not mix kinds"
    );

    // The same name may exist once per kind.
    store
        .create("same-name", Widget::default(), WidgetStatus::default())
        .await
        .expect("same name, other kind");
    assert_eq!(store.list::<Gadget>().await.expect("list gadgets").len(), 1);
}

async fn watch_sees_changes_in_order<S: Store>(store: &S) {
    let mut events = Box::pin(store.watch::<Widget>());
    let created = store
        .create("watched", Widget::default(), WidgetStatus::default())
        .await
        .expect("create");
    store
        .create("other-kind", Gadget {}, WidgetStatus::default())
        .await
        .expect("gadget");
    let updated = store.update(created.clone()).await.expect("update");

    assert_eq!(next(&mut events).await, Event::Created(created));
    // The Gadget change is filtered out; the next Widget event is the update.
    assert_eq!(next(&mut events).await, Event::Updated(updated));
}

async fn next<K: Kind>(events: &mut (impl futures::Stream<Item = Event<K>> + Unpin)) -> Event<K> {
    tokio::time::timeout(Duration::from_secs(5), events.next())
        .await
        .expect("event within 5 s")
        .expect("stream open")
}
