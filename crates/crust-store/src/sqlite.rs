//! [`Store`] on SQLite.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crust_types::{Kind, Metadata, Object, Resource};
use futures::{Stream, StreamExt, future};
use jiff::Timestamp;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

use crate::{Error, Event, Store};

/// How many changes a slow watcher may fall behind before its stream ends.
const WATCH_BUFFER: usize = 1024;

/// A change as it leaves the database, before it is decoded for a watcher.
#[derive(Clone, Debug)]
struct Change {
    kind: &'static str,
    created: bool,
    body: Arc<str>,
}

/// A [`Store`] in one SQLite database. Clones share the database and the
/// watchers.
#[derive(Clone, Debug)]
pub struct SqliteStore {
    pool: SqlitePool,
    changes: broadcast::Sender<Change>,
}

impl SqliteStore {
    /// Opens (or creates) the database file at `path` and migrates it.
    ///
    /// # Errors
    ///
    /// Fails if the file cannot be opened or migrated.
    pub async fn open(path: &Path) -> Result<Self, Error> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new().connect_with(options).await?;
        Self::migrate(pool).await
    }

    /// Opens a fresh in-memory database, for tests.
    ///
    /// # Errors
    ///
    /// Fails if SQLite cannot start or migrate.
    pub async fn memory() -> Result<Self, Error> {
        // Every connection to `:memory:` is its own database, so keep exactly
        // one connection open for the life of the pool.
        let pool = SqlitePoolOptions::new()
            .min_connections(1)
            .max_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
            .connect_with(SqliteConnectOptions::new().in_memory(true))
            .await?;
        Self::migrate(pool).await
    }

    async fn migrate(pool: SqlitePool) -> Result<Self, Error> {
        sqlx::migrate!().run(&pool).await?;
        let (changes, _) = broadcast::channel(WATCH_BUFFER);
        Ok(Self { pool, changes })
    }

    fn publish(&self, kind: &'static str, created: bool, body: String) {
        // No receivers is fine: nobody is watching.
        let _ = self.changes.send(Change {
            kind,
            created,
            body: body.into(),
        });
    }
}

fn decode<K: Kind>(body: &str) -> Result<Object<K>, Error> {
    serde_json::from_str(body).map_err(|source| Error::Invalid {
        kind: K::KIND,
        source,
    })
}

fn encode<K: Kind>(object: &Object<K>) -> Result<String, Error> {
    serde_json::to_string(object).map_err(|source| Error::Invalid {
        kind: K::KIND,
        source,
    })
}

impl Store for SqliteStore {
    async fn create<K: Kind>(
        &self,
        name: &str,
        spec: K,
        status: K::Status,
    ) -> Result<Object<K>, Error> {
        let now = Timestamp::now();
        let object = Resource {
            metadata: Metadata {
                name: name.to_owned(),
                uid: Uuid::new_v4(),
                generation: 1,
                labels: BTreeMap::default(),
                created: now,
                updated: now,
            },
            spec,
            status,
        };
        let body = encode(&object)?;
        let result =
            sqlx::query("INSERT INTO resources (kind, name, generation, body) VALUES (?, ?, 1, ?)")
                .bind(K::KIND)
                .bind(name)
                .bind(&body)
                .execute(&self.pool)
                .await;
        match result {
            Ok(_) => {
                self.publish(K::KIND, true, body);
                Ok(object)
            }
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => Err(Error::AlreadyExists {
                kind: K::KIND,
                name: name.to_owned(),
            }),
            Err(e) => Err(e.into()),
        }
    }

    async fn get<K: Kind>(&self, name: &str) -> Result<Object<K>, Error> {
        let body: Option<String> =
            sqlx::query_scalar("SELECT body FROM resources WHERE kind = ? AND name = ?")
                .bind(K::KIND)
                .bind(name)
                .fetch_optional(&self.pool)
                .await?;
        match body {
            Some(body) => decode(&body),
            None => Err(Error::NotFound {
                kind: K::KIND,
                name: name.to_owned(),
            }),
        }
    }

    async fn list<K: Kind>(&self) -> Result<Vec<Object<K>>, Error> {
        let bodies: Vec<String> =
            sqlx::query_scalar("SELECT body FROM resources WHERE kind = ? ORDER BY name")
                .bind(K::KIND)
                .fetch_all(&self.pool)
                .await?;
        bodies.iter().map(|b| decode(b)).collect()
    }

    async fn update<K: Kind>(&self, mut object: Object<K>) -> Result<Object<K>, Error> {
        let expected = object.metadata.generation;
        object.metadata.generation = expected + 1;
        object.metadata.updated = Timestamp::now();
        let body = encode(&object)?;
        let name = &object.metadata.name;
        let updated = sqlx::query(
            "UPDATE resources SET generation = ?, body = ? WHERE kind = ? AND name = ? AND generation = ?",
        )
        .bind(expected + 1)
        .bind(&body)
        .bind(K::KIND)
        .bind(name)
        .bind(expected)
        .execute(&self.pool)
        .await?
        .rows_affected();
        if updated == 1 {
            self.publish(K::KIND, false, body);
            return Ok(object);
        }
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT generation FROM resources WHERE kind = ? AND name = ?")
                .bind(K::KIND)
                .bind(name)
                .fetch_optional(&self.pool)
                .await?;
        Err(match exists {
            Some(_) => Error::Conflict {
                kind: K::KIND,
                name: name.clone(),
                expected,
            },
            None => Error::NotFound {
                kind: K::KIND,
                name: name.clone(),
            },
        })
    }

    fn watch<K: Kind>(&self) -> impl Stream<Item = Event<K>> + Send + use<K> {
        BroadcastStream::new(self.changes.subscribe())
            // A lagged receiver has lost changes: end the stream (see `Store::watch`).
            .take_while(|change| future::ready(change.is_ok()))
            .filter_map(|change| {
                future::ready(change.ok().filter(|c| c.kind == K::KIND).and_then(|c| {
                    let object = decode::<K>(&c.body).ok()?;
                    Some(if c.created {
                        Event::Created(object)
                    } else {
                        Event::Updated(object)
                    })
                }))
            })
    }
}
