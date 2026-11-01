//! Runs the store contract against SQLite, in memory and on disk.

#![allow(clippy::unwrap_used, reason = "test helpers panic on failure")]

use crust_store::{SqliteStore, contract};

#[tokio::test]
async fn in_memory_store_passes_the_contract() {
    contract::run(SqliteStore::memory().await.unwrap()).await;
}

#[tokio::test]
async fn file_store_passes_the_contract() {
    let dir = std::env::temp_dir().join(format!("crust-store-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("contract.db");
    let _ = std::fs::remove_file(&path);
    contract::run(SqliteStore::open(&path).await.unwrap()).await;
    std::fs::remove_dir_all(&dir).unwrap();
}
