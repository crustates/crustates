//! The Crustates state store.
//!
//! [`Store`] is the interface the control plane uses for all resources;
//! [`SqliteStore`] implements it (ADR 0010). With the `contract` feature,
//! [`contract::run`] checks any implementation against the shared contract.

#[cfg(feature = "contract")]
pub mod contract;
mod sqlite;

use std::future::Future;

use crust_types::{Kind, Object};
use futures::Stream;

pub use sqlite::SqliteStore;

/// What can go wrong in a [`Store`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No resource of this kind has this name.
    #[error("{kind} {name:?} not found")]
    NotFound {
        /// The resource kind.
        kind: &'static str,
        /// The resource name.
        name: String,
    },
    /// A resource of this kind already has this name.
    #[error("{kind} {name:?} already exists")]
    AlreadyExists {
        /// The resource kind.
        kind: &'static str,
        /// The resource name.
        name: String,
    },
    /// Someone else wrote the resource since it was read.
    #[error("{kind} {name:?} changed since generation {expected}; read it again")]
    Conflict {
        /// The resource kind.
        kind: &'static str,
        /// The resource name.
        name: String,
        /// The generation the caller read.
        expected: i64,
    },
    /// A stored document does not match its kind.
    #[error("stored {kind} is invalid: {source}")]
    Invalid {
        /// The resource kind.
        kind: &'static str,
        /// Why decoding failed.
        source: serde_json::Error,
    },
    /// The database failed.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    /// A schema migration failed.
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

/// A change to a resource, as seen by [`Store::watch`].
#[derive(Clone, Debug, PartialEq)]
pub enum Event<K: Kind> {
    /// The resource was created.
    Created(Object<K>),
    /// The resource was updated.
    Updated(Object<K>),
}

impl<K: Kind> Event<K> {
    /// The resource after the change.
    #[must_use]
    pub fn object(&self) -> &Object<K> {
        match self {
            Event::Created(o) | Event::Updated(o) => o,
        }
    }
}

/// Stores resources of every [`Kind`], with optimistic concurrency on
/// `metadata.generation`.
pub trait Store: Clone + Send + Sync + 'static {
    /// Creates a resource with generation 1 and empty labels.
    fn create<K: Kind>(
        &self,
        name: &str,
        spec: K,
        status: K::Status,
    ) -> impl Future<Output = Result<Object<K>, Error>> + Send;

    /// Reads one resource.
    fn get<K: Kind>(&self, name: &str) -> impl Future<Output = Result<Object<K>, Error>> + Send;

    /// Reads every resource of kind `K`, sorted by name.
    fn list<K: Kind>(&self) -> impl Future<Output = Result<Vec<Object<K>>, Error>> + Send;

    /// Writes `object` if its `metadata.generation` is still the stored one,
    /// and returns it with the next generation. Otherwise returns
    /// [`Error::Conflict`].
    fn update<K: Kind>(
        &self,
        object: Object<K>,
    ) -> impl Future<Output = Result<Object<K>, Error>> + Send;

    /// Streams every change to resources of kind `K` made after this call.
    ///
    /// The stream ends if the watcher falls too far behind. Callers then
    /// list again and start a new watch.
    fn watch<K: Kind>(&self) -> impl Stream<Item = Event<K>> + Send + use<Self, K>;
}
