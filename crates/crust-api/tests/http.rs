//! The REST API, called in-process through the router.

#![allow(clippy::unwrap_used, reason = "test helpers panic on failure")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use crust_api::{AppState, router};
use crust_store::{SqliteStore, Store};
use crust_types::{NodeSpec, NodeStatus};
use http_body_util::BodyExt;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

async fn state() -> AppState {
    AppState {
        store: SqliteStore::memory().await.unwrap(),
        shutdown: CancellationToken::new(),
    }
}

async fn get(state: &AppState, uri: &str) -> (StatusCode, String) {
    let response = router(state.clone())
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

async fn get_json(state: &AppState, uri: &str) -> (StatusCode, Value) {
    let (status, body) = get(state, uri).await;
    (status, serde_json::from_str(&body).unwrap())
}

#[tokio::test]
async fn healthz_reports_ok_and_version() {
    let (status, body) = get_json(&state().await, "/healthz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn nodes_can_be_listed_and_read() {
    let state = state().await;
    let (_, empty) = get_json(&state, "/v1/nodes").await;
    assert_eq!(empty, serde_json::json!({ "items": [] }));

    let status = NodeStatus {
        hostname: "box".into(),
        cpus: 4,
        ..NodeStatus::default()
    };
    state
        .store
        .create("box", NodeSpec {}, status)
        .await
        .unwrap();

    let (code, list) = get_json(&state, "/v1/nodes").await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(list["items"][0]["metadata"]["name"], "box");

    let (code, node) = get_json(&state, "/v1/nodes/box").await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(node["status"]["cpus"], 4);
    assert_eq!(node["metadata"]["generation"], 1);
}

#[tokio::test]
async fn missing_node_is_404_with_a_message() {
    let (status, body) = get_json(&state().await, "/v1/nodes/nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body["error"].as_str().unwrap().contains("nope"));
}

#[tokio::test]
async fn watch_streams_existing_nodes_then_changes_and_ends_on_shutdown() {
    let state = state().await;
    state
        .store
        .create("old", NodeSpec {}, NodeStatus::default())
        .await
        .unwrap();

    let response = router(state.clone())
        .oneshot(
            Request::get("/v1/nodes?watch=true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut body = response.into_body();

    let mut seen = String::new();
    read_until(&mut body, &mut seen, "\"name\":\"old\"").await;
    assert!(seen.contains("event: added"));

    let node = state.store.get::<NodeSpec>("old").await.unwrap();
    state.store.update(node).await.unwrap();
    read_until(&mut body, &mut seen, "event: modified").await;

    state.shutdown.cancel();
    assert!(body.frame().await.is_none(), "stream must end on shutdown");
}

/// Reads the event stream into `seen` until it contains `needle`.
async fn read_until(body: &mut Body, seen: &mut String, needle: &str) {
    while !seen.contains(needle) {
        let frame = body.frame().await.expect("stream open").unwrap();
        if let Ok(data) = frame.into_data() {
            seen.push_str(std::str::from_utf8(&data).unwrap());
        }
    }
}

#[tokio::test]
async fn docs_and_openapi_describe_the_node_endpoints() {
    let (status, html) = get(&state().await, "/docs").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Crustates API"));

    let spec = serde_json::to_value(crust_api::openapi()).unwrap();
    assert!(spec["paths"]["/v1/nodes"]["get"].is_object());
    assert!(spec["paths"]["/v1/nodes/{name}"]["get"].is_object());
    assert_eq!(spec["info"]["license"]["name"], "Apache-2.0");
    assert!(spec["components"]["schemas"]["Node"].is_object());

    let mut refs = Vec::new();
    collect_refs(&spec, &mut refs);
    for r in refs {
        let name = r.strip_prefix("#/components/schemas/").unwrap_or(&r);
        assert!(
            spec["components"]["schemas"][name].is_object(),
            "dangling $ref {r}"
        );
    }
}

fn collect_refs(value: &Value, refs: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(r)) = map.get("$ref") {
                refs.push(r.clone());
            }
            map.values().for_each(|v| collect_refs(v, refs));
        }
        Value::Array(items) => items.iter().for_each(|v| collect_refs(v, refs)),
        _ => {}
    }
}
