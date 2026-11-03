//! The REST API, called in-process through the router.

#![allow(clippy::unwrap_used, reason = "test helpers panic on failure")]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use crust_api::{AppState, Auth, Commands, auth, router};
use crust_store::{SqliteStore, Store};
use crust_types::{NodeSpec, NodeStatus};
use http_body_util::BodyExt;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

/// A fresh API and its admin token. The directory holds the token hash.
struct Api {
    state: AppState,
    token: String,
    _dir: tempfile::TempDir,
}

async fn api() -> Api {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("admin-token");
    let token = auth::new_token().unwrap();
    std::fs::write(&file, auth::hash(&token).unwrap()).unwrap();
    Api {
        state: AppState {
            store: SqliteStore::memory().await.unwrap(),
            shutdown: CancellationToken::new(),
            auth: Arc::new(Auth::new(file)),
            commands: Commands::default(),
        },
        token,
        _dir: dir,
    }
}

impl Api {
    async fn send(
        &self,
        method: &str,
        uri: &str,
        token: Option<&str>,
        body: &str,
    ) -> (StatusCode, String) {
        let mut request = Request::builder().method(method).uri(uri);
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        if !body.is_empty() {
            request = request.header("content-type", "application/json");
        }
        let response = router(self.state.clone())
            .oneshot(request.body(Body::from(body.to_owned())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    async fn get(&self, uri: &str) -> (StatusCode, String) {
        self.send("GET", uri, Some(&self.token.clone()), "").await
    }

    async fn get_json(&self, uri: &str) -> (StatusCode, Value) {
        let (status, body) = self.get(uri).await;
        (status, serde_json::from_str(&body).unwrap())
    }
}

#[tokio::test]
async fn healthz_reports_ok_and_version_without_a_token() {
    let api = api().await;
    let (status, body) = api.send("GET", "/healthz", None, "").await;
    assert_eq!(status, StatusCode::OK);
    let body: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn v1_needs_the_admin_token() {
    let api = api().await;
    let (status, body) = api.send("GET", "/v1/nodes", None, "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.contains("admin token"), "{body}");

    let wrong = auth::new_token().unwrap();
    let (status, _) = api.send("GET", "/v1/nodes", Some(&wrong), "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Typed in lower case without dashes, it still works.
    let typed = api.token.to_lowercase().replace('-', "");
    let (status, _) = api.send("GET", "/v1/nodes", Some(&typed), "").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn without_a_token_hash_the_api_says_it_is_not_set_up() {
    let api = api().await;
    std::fs::remove_file(api.state.auth.hash_file()).unwrap();
    let (status, body) = api.get("/v1/nodes").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body.contains("first boot"), "{body}");
}

#[tokio::test]
async fn nodes_can_be_listed_and_read() {
    let api = api().await;
    let (_, empty) = api.get_json("/v1/nodes").await;
    assert_eq!(empty, serde_json::json!({ "items": [] }));

    let status = NodeStatus {
        hostname: "box".into(),
        cpus: 4,
        ..NodeStatus::default()
    };
    api.state
        .store
        .create("box", NodeSpec {}, status)
        .await
        .unwrap();

    let (code, list) = api.get_json("/v1/nodes").await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(list["items"][0]["metadata"]["name"], "box");

    let (code, node) = api.get_json("/v1/nodes/box").await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(node["status"]["cpus"], 4);
    assert_eq!(node["metadata"]["generation"], 1);
}

#[tokio::test]
async fn missing_node_is_404_with_a_message() {
    let (status, body) = api().await.get_json("/v1/nodes/nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body["error"].as_str().unwrap().contains("nope"));
}

#[tokio::test]
async fn molt_commands_are_audited_even_when_they_fail() {
    let api = api().await;
    let token = api.token.clone();
    let (status, _) = api
        .send("POST", "/v1/nodes/ghost/molt/upgrade", Some(&token), "{}")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    api.state
        .store
        .create("box", NodeSpec {}, NodeStatus::default())
        .await
        .unwrap();
    // No agent connected.
    let (status, body) = api
        .send(
            "POST",
            "/v1/nodes/box/molt/upgrade",
            Some(&token),
            r#"{"reboot":true}"#,
        )
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");

    let (status, body) = api
        .send(
            "PUT",
            "/v1/nodes/box/molt/policy",
            Some(&token),
            r#"{"policy":"auto","window":"3am"}"#,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("HH:MM-HH:MM"), "{body}");

    let (status, log) = api.get_json("/v1/audit?limit=10").await;
    assert_eq!(status, StatusCode::OK);
    let entry = &log["items"][0];
    assert_eq!(entry["action"], "molt.upgrade");
    assert_eq!(entry["target"], "node/box");
    assert_eq!(entry["principal"], "admin token");
    assert!(entry["outcome"].as_str().unwrap().contains("not connected"));

    let (status, _) = api.get("/v1/audit?limit=0").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn watch_streams_existing_nodes_then_changes_and_ends_on_shutdown() {
    let api = api().await;
    let state = &api.state;
    state
        .store
        .create("old", NodeSpec {}, NodeStatus::default())
        .await
        .unwrap();

    let response = router(state.clone())
        .oneshot(
            Request::get("/v1/nodes?watch=true")
                .header("authorization", format!("Bearer {}", api.token))
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
async fn docs_and_openapi_describe_the_endpoints_and_the_token() {
    let api = api().await;
    let (status, html) = api.send("GET", "/docs", None, "").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Crustates API"));

    let spec = serde_json::to_value(crust_api::openapi()).unwrap();
    assert!(spec["paths"]["/v1/nodes"]["get"].is_object());
    assert!(spec["paths"]["/v1/nodes/{name}"]["get"].is_object());
    assert!(spec["paths"]["/v1/nodes/{name}/molt/upgrade"]["post"].is_object());
    assert!(spec["paths"]["/v1/audit"]["get"].is_object());
    assert_eq!(spec["info"]["license"]["name"], "Apache-2.0");
    assert!(spec["components"]["schemas"]["Node"].is_object());
    assert_eq!(
        spec["components"]["securitySchemes"]["admin_token"]["scheme"],
        "bearer"
    );
    assert_eq!(spec["security"][0]["admin_token"], serde_json::json!([]));
    // /healthz is open: an empty requirement overrides the global one.
    assert_eq!(
        spec["paths"]["/healthz"]["get"]["security"],
        serde_json::json!([{}])
    );

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
