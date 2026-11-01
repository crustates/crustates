//! The REST API, its OpenAPI description and the Scalar reference at `/docs`.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use crust_store::{Error, Event, SqliteStore, Store};
use crust_types::{Node, NodeSpec};
use futures::{Stream, StreamExt, TryStreamExt, stream};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use utoipa::{IntoParams, OpenApi, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use utoipa_scalar::{Scalar, Servable};

/// What every handler needs.
#[derive(Clone, Debug)]
pub struct AppState {
    /// Where resources live.
    pub store: SqliteStore,
    /// Ends open watch streams when the server shuts down.
    pub shutdown: CancellationToken,
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Crustates API",
        description = "The declarative API of Crustates: your own cloud, on your own hardware.",
        license(name = "Apache-2.0", identifier = "Apache-2.0"),
    ),
    tags((name = "nodes", description = "Machines that run the node agent")),
)]
struct ApiDoc;

fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(healthz))
        .routes(routes!(list_nodes))
        .routes(routes!(get_node))
}

/// The OpenAPI description of the API.
#[must_use]
pub fn openapi() -> utoipa::openapi::OpenApi {
    routes().split_for_parts().1
}

/// The HTTP router: the API plus the Scalar reference at `/docs`.
pub fn router(state: AppState) -> Router {
    let (router, api) = routes().split_for_parts();
    router
        .merge(Scalar::with_url("/docs", api))
        .with_state(state)
}

/// The API is up.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
struct Health {
    /// Always `ok`.
    status: String,
    /// The control plane version.
    version: String,
}

/// Every node.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
struct NodeList {
    /// Nodes sorted by name.
    items: Vec<Node>,
}

/// An error response.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
struct ErrorBody {
    /// What went wrong.
    error: String,
}

#[derive(Debug, Deserialize, IntoParams)]
struct ListParams {
    /// Instead of the list, stream server-sent events: first an `added` event
    /// per existing node, then `added` or `modified` for every change. Each
    /// event's data is a Node. A node may appear twice around the start.
    #[serde(default)]
    watch: bool,
}

/// A store error as an HTTP response.
struct ApiError(Error);

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        Self(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            Error::NotFound { .. } => StatusCode::NOT_FOUND,
            Error::AlreadyExists { .. } | Error::Conflict { .. } => StatusCode::CONFLICT,
            e => {
                tracing::error!(error = %e, "request failed");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorBody {
                        error: "internal error".into(),
                    }),
                )
                    .into_response();
            }
        };
        (
            status,
            Json(ErrorBody {
                error: self.0.to_string(),
            }),
        )
            .into_response()
    }
}

/// Reports that the API is up, with its version.
#[utoipa::path(get, path = "/healthz", responses((status = 200, body = Health)))]
async fn healthz() -> Json<Health> {
    Json(Health {
        status: "ok".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    })
}

/// Lists all nodes, or streams changes with `watch=true`.
#[utoipa::path(
    get,
    path = "/v1/nodes",
    tag = "nodes",
    params(ListParams),
    responses(
        (status = 200, description = "All nodes, or a stream of node events with `watch=true`", content(
            (NodeList = "application/json"),
            (Node = "text/event-stream"),
        )),
        (status = 500, body = ErrorBody),
    ),
)]
async fn list_nodes(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Response, ApiError> {
    if params.watch {
        return Ok(Sse::new(watch_nodes(state))
            .keep_alive(KeepAlive::default())
            .into_response());
    }
    let items = state.store.list::<NodeSpec>().await?;
    Ok(Json(NodeList { items }).into_response())
}

/// Existing nodes as `added` events, then live changes, until shutdown.
fn watch_nodes(state: AppState) -> impl Stream<Item = Result<SseEvent, Error>> {
    // Subscribe before listing, so no change falls between the two.
    let changes = state.store.watch::<NodeSpec>().map(|event| {
        Ok(match event {
            Event::Created(node) => ("added", node),
            Event::Updated(node) => ("modified", node),
        })
    });
    let store = state.store.clone();
    let existing = stream::once(async move { store.list::<NodeSpec>().await })
        .map_ok(|nodes| {
            stream::iter(
                nodes
                    .into_iter()
                    .map(|node| Ok::<_, Error>(("added", node))),
            )
        })
        .try_flatten();
    existing
        .chain(changes)
        .map_ok(|(name, node)| sse_event(name, &node))
        .take_until(state.shutdown.cancelled_owned())
}

fn sse_event(name: &str, node: &Node) -> SseEvent {
    let event = SseEvent::default().event(name);
    match serde_json::to_string(node) {
        Ok(json) => event.data(json),
        // Unreachable for a type that just came out of the store as JSON.
        Err(e) => event.comment(format!("encoding failed: {e}")),
    }
}

/// Reads one node.
#[utoipa::path(
    get,
    path = "/v1/nodes/{name}",
    tag = "nodes",
    params(("name" = String, Path, description = "The node name")),
    responses(
        (status = 200, body = Node),
        (status = 404, body = ErrorBody),
        (status = 500, body = ErrorBody),
    ),
)]
async fn get_node(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Node>, ApiError> {
    Ok(Json(state.store.get::<NodeSpec>(&name).await?))
}
