//! The REST API, its OpenAPI description and the Scalar reference at `/docs`.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json, Router};
use crust_proto::agent::v1 as pb;
use crust_store::{Error, Event, SqliteStore, Store};
use crust_types::{AuditEntry, Node, NodeSpec, UpdatePolicy, Window};
use futures::{Stream, StreamExt, TryStreamExt, stream};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{IntoParams, Modify, OpenApi, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use utoipa_scalar::{Scalar, Servable};

use crate::auth::{Auth, Principal, require_token};
use crate::commands::{CommandError, Commands};

/// How long an upgrade may take: the agent downloads the new image first.
const UPGRADE_TIMEOUT: Duration = Duration::from_mins(30);
/// How long any other command may take.
const COMMAND_TIMEOUT: Duration = Duration::from_mins(2);

/// What every handler needs.
#[derive(Clone, Debug)]
pub struct AppState {
    /// Where resources live.
    pub store: SqliteStore,
    /// Ends open watch streams when the server shuts down.
    pub shutdown: CancellationToken,
    /// Checks the admin token on `/v1/` requests.
    pub auth: Arc<Auth>,
    /// Sends commands to node agents.
    pub commands: Commands,
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Crustates API",
        description = "The declarative API of Crustates: your own cloud, on your own hardware.",
        license(name = "Apache-2.0", identifier = "Apache-2.0"),
    ),
    tags(
        (name = "nodes", description = "Machines that run the node agent"),
        (name = "molt", description = "The Molt OS image of each node and its updates"),
        (name = "audit", description = "Who changed what"),
    ),
    modifiers(&AdminToken),
    security(("admin_token" = [])),
)]
struct ApiDoc;

/// Declares the admin token as the API's bearer token.
struct AdminToken;

impl Modify for AdminToken {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let scheme = HttpBuilder::new()
            .scheme(HttpAuthScheme::Bearer)
            .description(Some(
                "The admin token, `crt_…`, shown on the machine's screen at its first boot.",
            ))
            .build();
        openapi
            .components
            .get_or_insert_with(Default::default)
            .add_security_scheme("admin_token", SecurityScheme::Http(scheme));
    }
}

fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(healthz))
        .routes(routes!(list_nodes))
        .routes(routes!(get_node))
        .routes(routes!(molt_upgrade))
        .routes(routes!(molt_rollback))
        .routes(routes!(molt_policy))
        .routes(routes!(molt_enroll_key))
        .routes(routes!(audit_log))
}

/// The OpenAPI description of the API.
#[must_use]
pub fn openapi() -> utoipa::openapi::OpenApi {
    routes().split_for_parts().1
}

/// The HTTP router: the API plus the Scalar reference at `/docs`. Every
/// `/v1/` route needs the admin token.
pub fn router(state: AppState) -> Router {
    let (router, api) = routes().split_for_parts();
    let auth = state.auth.clone();
    router
        .merge(Scalar::with_url("/docs", api))
        .layer(axum::middleware::from_fn_with_state(auth, require_token))
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
pub(crate) struct ErrorBody {
    /// What went wrong.
    pub(crate) error: String,
}

#[derive(Debug, Deserialize, IntoParams)]
struct ListParams {
    /// Instead of the list, stream server-sent events: first an `added` event
    /// per existing node, then `added` or `modified` for every change. Each
    /// event's data is a Node. A node may appear twice around the start.
    #[serde(default)]
    watch: bool,
}

/// An error as an HTTP response.
#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: String) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
        }
    }
}

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        let status = match &e {
            Error::NotFound { .. } => StatusCode::NOT_FOUND,
            Error::AlreadyExists { .. } | Error::Conflict { .. } => StatusCode::CONFLICT,
            e => {
                tracing::error!(error = %e, "request failed");
                return Self {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    message: "internal error".into(),
                };
            }
        };
        Self {
            status,
            message: e.to_string(),
        }
    }
}

impl From<CommandError> for ApiError {
    fn from(e: CommandError) -> Self {
        let status = match &e {
            CommandError::NotConnected(_) => StatusCode::SERVICE_UNAVAILABLE,
            CommandError::Timeout(_) => StatusCode::GATEWAY_TIMEOUT,
            CommandError::Failed(_) => StatusCode::UNPROCESSABLE_ENTITY,
        };
        Self {
            status,
            message: e.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                error: self.message,
            }),
        )
            .into_response()
    }
}

/// Reports that the API is up, with its version. Needs no token.
#[utoipa::path(get, path = "/healthz", security(()), responses((status = 200, body = Health)))]
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
        (status = 401, body = ErrorBody),
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

/// Reads one node, including its Molt status.
#[utoipa::path(
    get,
    path = "/v1/nodes/{name}",
    tag = "nodes",
    params(("name" = String, Path, description = "The node name")),
    responses(
        (status = 200, body = Node),
        (status = 401, body = ErrorBody),
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

/// Whether to reboot right after the change.
#[derive(Debug, Default, Deserialize, ToSchema)]
struct RebootChoice {
    /// Reboot right away instead of at the next reboot (or, with the
    /// `auto` policy, in the update window).
    #[serde(default)]
    reboot: bool,
}

/// What a node did.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
struct Outcome {
    /// What happened, for people.
    message: String,
}

/// An update policy to set.
#[derive(Debug, Deserialize, ToSchema)]
struct PolicyChange {
    /// `auto` stages new images and reboots in the window; `manual` only
    /// reports them.
    policy: UpdatePolicy,
    /// When automatic updates may reboot, `HH:MM-HH:MM` in the node's local
    /// time. Unchanged if left out.
    window: Option<String>,
}

/// A queued key enrolment.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
struct Enrolment {
    /// What to do next, at the machine.
    message: String,
    /// The one-time password to type in the MOK manager at the next boot.
    password: String,
}

#[derive(Debug, Deserialize, IntoParams)]
struct AuditParams {
    /// How many entries, newest first (1 to 1000).
    #[serde(default = "default_audit_limit")]
    limit: u32,
}

fn default_audit_limit() -> u32 {
    100
}

/// The newest audit entries.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
struct AuditLog {
    /// Newest first.
    items: Vec<AuditEntry>,
}

/// Runs a command on the agent of node `name` and writes the audit log.
async fn run_command(
    state: &AppState,
    principal: &Principal,
    name: &str,
    action: &str,
    kind: pb::command::Kind,
    timeout: Duration,
) -> Result<pb::CommandResult, ApiError> {
    state.store.get::<NodeSpec>(name).await?;
    let result = state
        .commands
        .run(name, kind, timeout)
        .await
        .map_err(ApiError::from);
    let outcome = match &result {
        Ok(_) => "ok".to_owned(),
        Err(e) => e.message.clone(),
    };
    let entry = AuditEntry {
        time: Timestamp::now(),
        principal: principal.0.clone(),
        action: action.into(),
        target: format!("node/{name}"),
        outcome,
    };
    if let Err(e) = state.store.audit(entry).await {
        tracing::error!(error = %e, action, node = name, "writing the audit log failed");
    }
    result
}

/// Checks the node's channel and stages a newer image if there is one. Its
/// signature is checked first. With `reboot`, the node reboots into it.
#[utoipa::path(
    post,
    path = "/v1/nodes/{name}/molt/upgrade",
    tag = "molt",
    params(("name" = String, Path, description = "The node name")),
    request_body = RebootChoice,
    responses(
        (status = 200, body = Outcome),
        (status = 404, body = ErrorBody, description = "No such node"),
        (status = 422, body = ErrorBody, description = "The node could not stage the image"),
        (status = 503, body = ErrorBody, description = "The node's agent is not connected"),
    ),
)]
async fn molt_upgrade(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(name): Path<String>,
    Json(choice): Json<RebootChoice>,
) -> Result<Json<Outcome>, ApiError> {
    let kind = pb::command::Kind::MoltUpgrade(pb::MoltUpgrade {
        reboot: choice.reboot,
    });
    let result = run_command(
        &state,
        &principal,
        &name,
        "molt.upgrade",
        kind,
        UPGRADE_TIMEOUT,
    )
    .await?;
    Ok(Json(Outcome {
        message: result.message,
    }))
}

/// Makes the previous image the one that boots next. With `reboot`, the
/// node reboots into it.
#[utoipa::path(
    post,
    path = "/v1/nodes/{name}/molt/rollback",
    tag = "molt",
    params(("name" = String, Path, description = "The node name")),
    request_body = RebootChoice,
    responses(
        (status = 200, body = Outcome),
        (status = 404, body = ErrorBody, description = "No such node"),
        (status = 422, body = ErrorBody, description = "There is nothing to roll back to"),
        (status = 503, body = ErrorBody, description = "The node's agent is not connected"),
    ),
)]
async fn molt_rollback(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(name): Path<String>,
    Json(choice): Json<RebootChoice>,
) -> Result<Json<Outcome>, ApiError> {
    let kind = pb::command::Kind::MoltRollback(pb::MoltRollback {
        reboot: choice.reboot,
    });
    let result = run_command(
        &state,
        &principal,
        &name,
        "molt.rollback",
        kind,
        COMMAND_TIMEOUT,
    )
    .await?;
    Ok(Json(Outcome {
        message: result.message,
    }))
}

/// Sets how the node applies updates.
#[utoipa::path(
    put,
    path = "/v1/nodes/{name}/molt/policy",
    tag = "molt",
    params(("name" = String, Path, description = "The node name")),
    request_body = PolicyChange,
    responses(
        (status = 200, body = Outcome),
        (status = 400, body = ErrorBody, description = "The window is not HH:MM-HH:MM"),
        (status = 404, body = ErrorBody, description = "No such node"),
        (status = 503, body = ErrorBody, description = "The node's agent is not connected"),
    ),
)]
async fn molt_policy(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(name): Path<String>,
    Json(change): Json<PolicyChange>,
) -> Result<Json<Outcome>, ApiError> {
    let window = match change.window {
        Some(w) => w
            .parse::<Window>()
            .map_err(ApiError::bad_request)?
            .to_string(),
        None => String::new(),
    };
    let policy = match change.policy {
        UpdatePolicy::Auto => pb::UpdatePolicy::Auto,
        UpdatePolicy::Manual => pb::UpdatePolicy::Manual,
    };
    let kind = pb::command::Kind::MoltPolicy(pb::MoltPolicy {
        policy: policy.into(),
        window,
    });
    let result = run_command(
        &state,
        &principal,
        &name,
        "molt.policy",
        kind,
        COMMAND_TIMEOUT,
    )
    .await?;
    Ok(Json(Outcome {
        message: result.message,
    }))
}

/// Queues the Crustates module-signing certificate for enrolment as a
/// Machine Owner Key, so the node's own modules load with Secure Boot on.
/// Someone at the machine confirms it at the next boot with the returned
/// one-time password.
#[utoipa::path(
    post,
    path = "/v1/nodes/{name}/molt/enroll-key",
    tag = "molt",
    params(("name" = String, Path, description = "The node name")),
    responses(
        (status = 200, body = Enrolment),
        (status = 404, body = ErrorBody, description = "No such node"),
        (status = 422, body = ErrorBody, description = "Secure Boot is off, or the key cannot be enrolled"),
        (status = 503, body = ErrorBody, description = "The node's agent is not connected"),
    ),
)]
async fn molt_enroll_key(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(name): Path<String>,
) -> Result<Json<Enrolment>, ApiError> {
    let kind = pb::command::Kind::MoltEnrollKey(pb::MoltEnrollKey {});
    let result = run_command(
        &state,
        &principal,
        &name,
        "molt.enroll-key",
        kind,
        COMMAND_TIMEOUT,
    )
    .await?;
    Ok(Json(Enrolment {
        message: result.message,
        password: result.secret,
    }))
}

/// Reads the audit log, newest first.
#[utoipa::path(
    get,
    path = "/v1/audit",
    tag = "audit",
    params(AuditParams),
    responses(
        (status = 200, body = AuditLog),
        (status = 400, body = ErrorBody),
        (status = 401, body = ErrorBody),
    ),
)]
async fn audit_log(
    State(state): State<AppState>,
    Query(params): Query<AuditParams>,
) -> Result<Json<AuditLog>, ApiError> {
    if !(1..=1000).contains(&params.limit) {
        return Err(ApiError::bad_request("limit must be 1 to 1000".into()));
    }
    let items = state.store.audit_log(params.limit).await?;
    Ok(Json(AuditLog { items }))
}
