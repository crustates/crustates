//! The REST API through the local socket (ADR 0018): root's requests need
//! no token there, and risky ones a recovery PIN.

use std::path::PathBuf;
use std::time::Duration;

use crust_types::Node;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::{Method, Request, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::net::UnixStream;

/// How long a request may take; a rollback answers within seconds.
const TIMEOUT: Duration = Duration::from_secs(30);

/// A client of the API's local socket.
#[derive(Clone, Debug)]
pub struct Api {
    socket: PathBuf,
}

impl Api {
    /// A client of the socket at `socket`.
    #[must_use]
    pub fn new(socket: PathBuf) -> Self {
        Self { socket }
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<(StatusCode, Bytes), String> {
        let exchange = async {
            let stream = UnixStream::connect(&self.socket).await.map_err(|e| {
                format!("the API is not reachable at {}: {e}", self.socket.display())
            })?;
            let (mut sender, connection) =
                hyper::client::conn::http1::handshake(TokioIo::new(stream))
                    .await
                    .map_err(|e| e.to_string())?;
            tokio::spawn(connection);
            let body = body.map_or_else(Bytes::new, |b| Bytes::from(b.to_string()));
            let request = Request::builder()
                .method(method)
                .uri(path)
                .header("host", "localhost")
                .header("content-type", "application/json")
                .body(Full::new(body))
                .map_err(|e| e.to_string())?;
            let response = sender
                .send_request(request)
                .await
                .map_err(|e| e.to_string())?;
            let status = response.status();
            let bytes = response
                .into_body()
                .collect()
                .await
                .map_err(|e| e.to_string())?
                .to_bytes();
            Ok((status, bytes))
        };
        tokio::time::timeout(TIMEOUT, exchange)
            .await
            .map_err(|_| "the API did not answer in time".to_owned())?
    }

    /// The error message of a failed response.
    fn error(status: StatusCode, body: &[u8]) -> String {
        serde_json::from_slice::<Value>(body)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("the API answered {status}"))
    }

    /// Whether the API answers.
    pub async fn healthy(&self) -> bool {
        self.send(Method::GET, "/healthz", None)
            .await
            .is_ok_and(|(s, _)| s.is_success())
    }

    /// Every node.
    pub async fn nodes(&self) -> Result<Vec<Node>, String> {
        let (status, body) = self.send(Method::GET, "/v1/nodes", None).await?;
        if !status.is_success() {
            return Err(Self::error(status, &body));
        }
        let list: Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
        serde_json::from_value(list["items"].clone()).map_err(|e| e.to_string())
    }

    /// Checks the recovery PIN for the audit action `action`.
    pub async fn check_pin(&self, pin: &str, action: &str) -> Result<(), String> {
        let body = json!({ "pin": pin, "action": action });
        let (status, body) = self.send(Method::POST, "/v1/local/pin", Some(body)).await?;
        if status == StatusCode::NO_CONTENT {
            Ok(())
        } else {
            Err(Self::error(status, &body))
        }
    }

    /// Makes `node` boot its previous image, rebooting now.
    pub async fn roll_back(&self, node: &str) -> Result<String, String> {
        let path = format!("/v1/nodes/{node}/molt/rollback");
        let (status, body) = self
            .send(Method::POST, &path, Some(json!({ "reboot": true })))
            .await?;
        if !status.is_success() {
            return Err(Self::error(status, &body));
        }
        let outcome: Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
        Ok(outcome["message"]
            .as_str()
            .unwrap_or("rolling back")
            .to_owned())
    }
}
