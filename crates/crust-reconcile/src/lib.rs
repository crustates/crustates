//! Reconcile loops: watch a kind, queue changed names, call a [`Reconciler`]
//! for each, and retry failures with backoff.

pub mod node;
mod queue;

use std::future::{self, Future};
use std::pin::pin;
use std::time::Duration;

use crust_store::Store;
use crust_types::Kind;
use futures::StreamExt;
use tokio::time::{Instant, sleep, sleep_until};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use queue::Queue;

/// What the loop does with a resource after a successful reconcile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Nothing more to do until the resource changes.
    Done,
    /// Reconcile again after this long, even if nothing changes.
    RequeueAfter(Duration),
}

/// Drives resources of one kind toward their spec.
pub trait Reconciler: Send + Sync + 'static {
    /// The kind this reconciler handles.
    type Kind: Kind;
    /// What a failed reconcile returns; the loop logs it and retries.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Brings the resource named `name` closer to its spec. Must be safe to
    /// call at any time, any number of times: the loop calls it on every
    /// change, on [`Action::RequeueAfter`], and with backoff after an error.
    fn reconcile(&self, name: &str) -> impl Future<Output = Result<Action, Self::Error>> + Send;
}

/// Runs `reconciler` for every resource of its kind until `shutdown` fires.
pub async fn run<S: Store, R: Reconciler>(store: S, reconciler: R, shutdown: CancellationToken) {
    let kind = <R::Kind as Kind>::KIND;
    let mut queue = Queue::default();
    // Each pass of this loop starts a fresh watch and lists everything, so
    // changes missed while the watch was down are picked up.
    loop {
        let mut events = pin!(store.watch::<R::Kind>());
        match store.list::<R::Kind>().await {
            Ok(all) => {
                let now = Instant::now();
                for object in all {
                    queue.add(object.metadata.name, now);
                }
            }
            Err(error) => {
                warn!(kind, %error, "listing failed; retrying");
                tokio::select! {
                    () = shutdown.cancelled() => return,
                    () = sleep(Duration::from_secs(1)) => continue,
                }
            }
        }
        loop {
            let deadline = queue.next_deadline();
            let due = async {
                match deadline {
                    Some(at) => sleep_until(at).await,
                    None => future::pending().await,
                }
            };
            tokio::select! {
                () = shutdown.cancelled() => return,
                event = events.next() => {
                    let Some(event) = event else {
                        debug!(kind, "watch fell behind; listing again");
                        break;
                    };
                    queue.add(event.object().metadata.name.clone(), Instant::now());
                }
                () = due => {
                    while let Some(name) = queue.pop_due(Instant::now()) {
                        reconcile_one(&reconciler, &mut queue, name).await;
                    }
                }
            }
        }
    }
}

async fn reconcile_one<R: Reconciler>(reconciler: &R, queue: &mut Queue, name: String) {
    match reconciler.reconcile(&name).await {
        Ok(Action::Done) => queue.succeeded(&name),
        Ok(Action::RequeueAfter(after)) => {
            queue.succeeded(&name);
            queue.add(name, Instant::now() + after);
        }
        Err(error) => {
            let retry = queue.failed(&name);
            warn!(kind = <R::Kind as Kind>::KIND, name, %error, ?retry, "reconcile failed");
            queue.add(name, Instant::now() + retry);
        }
    }
}
