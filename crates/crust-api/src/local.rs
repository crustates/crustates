//! The local console's way in (ADR 0018): a Unix socket that only root may
//! use, and the recovery PIN that guards the console's risky actions. The
//! lockout after wrong PINs is counted from the audit log, so it survives a
//! restart of the API.

use std::path::PathBuf;
use std::time::Duration;

use axum::Json;
use axum::extract::connect_info::Connected;
use axum::extract::{ConnectInfo, Request};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::serve::IncomingStream;
use crust_store::Store;
use crust_types::AuditEntry;
use jiff::{SignedDuration, Timestamp};
use tokio::net::UnixListener;
use tracing::{error, warn};

use crate::auth::{self, Principal};
use crate::http::ErrorBody;

/// The principal of requests through the local socket.
pub const LOCAL_CONSOLE: &str = "local console";
/// The audit action of a PIN check; its target is the action it guards.
pub const PIN_ACTION: &str = "recovery-pin";
/// Digits in a recovery PIN.
pub const PIN_DIGITS: usize = 9;
/// Wrong PINs before the lock.
const MAX_FAILURES: usize = 5;
/// How long the lock lasts after the last wrong PIN.
const LOCK: SignedDuration = SignedDuration::from_mins(15);

/// Who is at the other end of a local socket connection.
#[derive(Clone, Copy, Debug)]
pub struct Peer {
    /// The client's user id, if the OS told it.
    pub uid: Option<u32>,
}

impl Connected<IncomingStream<'_, UnixListener>> for Peer {
    fn connect_info(stream: IncomingStream<'_, UnixListener>) -> Self {
        Self {
            uid: stream.io().peer_cred().ok().map(|c| c.uid()),
        }
    }
}

/// Middleware for the local socket: only root's requests pass, as the
/// local console. Root on the machine can do anything anyway; the socket
/// spares the console a token it would have to store.
pub async fn require_root(mut request: Request, next: Next) -> Response {
    let uid = request
        .extensions()
        .get::<ConnectInfo<Peer>>()
        .and_then(|c| c.0.uid);
    if uid != Some(0) {
        warn!(uid, "refused a local socket client that is not root");
        let body = ErrorBody {
            error: "only root may use the local socket".into(),
        };
        return (StatusCode::FORBIDDEN, Json(body)).into_response();
    }
    request
        .extensions_mut()
        .insert(Principal(LOCAL_CONSOLE.into()));
    next.run(request).await
}

/// Makes a new random recovery PIN: nine digits.
///
/// # Errors
///
/// Fails if the operating system has no randomness to give.
pub fn new_pin() -> Result<String, getrandom::Error> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes)?;
    // The modulo bias of 2^64 over 10^9 is below 10^-10.
    Ok(format!("{:09}", u64::from_le_bytes(bytes) % 1_000_000_000))
}

/// The PIN as typed, without spaces or dashes; `None` unless nine digits.
#[must_use]
pub fn normalize_pin(pin: &str) -> Option<String> {
    let digits: String = pin.chars().filter(|c| !matches!(c, ' ' | '-')).collect();
    (digits.len() == PIN_DIGITS && digits.bytes().all(|b| b.is_ascii_digit())).then_some(digits)
}

/// Why a PIN check failed.
#[derive(Debug, PartialEq, Eq)]
pub enum PinError {
    /// No PIN is set up on this machine.
    NotSetUp,
    /// The PIN is wrong.
    Wrong {
        /// Attempts before the lock.
        attempts_left: usize,
    },
    /// Too many wrong PINs.
    Locked {
        /// Until the next attempt may be made.
        retry_after: Duration,
    },
    /// Reading the hash or the audit log failed.
    Internal,
}

/// Checks recovery PINs against their hash file.
#[derive(Debug)]
pub struct RecoveryPin {
    file: PathBuf,
    /// One check at a time, so parallel guesses cannot pass the lock.
    checking: tokio::sync::Mutex<()>,
}

impl RecoveryPin {
    /// Checks PINs against the argon2id hash in `file`.
    #[must_use]
    pub fn new(file: PathBuf) -> Self {
        Self {
            file,
            checking: tokio::sync::Mutex::new(()),
        }
    }

    /// Checks `pin` before `action` and writes the attempt to the audit log.
    ///
    /// # Errors
    ///
    /// Fails if the PIN is wrong, locked, not set up, or cannot be checked.
    pub async fn check<S: Store>(
        &self,
        store: &S,
        pin: &str,
        action: &str,
    ) -> Result<(), PinError> {
        let _one = self.checking.lock().await;
        let log = store
            .audit_log(
                u32::try_from(MAX_FAILURES).unwrap_or(5) * 4,
                Some(PIN_ACTION),
            )
            .await
            .map_err(|e| {
                error!(error = %e, "reading the audit log failed");
                PinError::Internal
            })?;
        let now = Timestamp::now();
        let (failures, newest) = recent_failures(&log, now);
        let outcome = if failures >= MAX_FAILURES {
            Err(PinError::Locked {
                retry_after: newest
                    .map_or(LOCK, |t| LOCK - now.duration_since(t))
                    .unsigned_abs(),
            })
        } else {
            let phc = match tokio::fs::read_to_string(&self.file).await {
                Ok(phc) => phc,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err(PinError::NotSetUp);
                }
                Err(e) => {
                    error!(file = %self.file.display(), error = %e, "reading the recovery PIN hash failed");
                    return Err(PinError::Internal);
                }
            };
            let pin = normalize_pin(pin).unwrap_or_default();
            let ok = tokio::task::spawn_blocking(move || auth::verify(&pin, &phc))
                .await
                .map_err(|_| PinError::Internal)?;
            if ok {
                Ok(())
            } else {
                Err(PinError::Wrong {
                    attempts_left: MAX_FAILURES - failures - 1,
                })
            }
        };
        let entry = AuditEntry {
            time: now,
            principal: LOCAL_CONSOLE.into(),
            action: PIN_ACTION.into(),
            target: action.into(),
            outcome: match &outcome {
                Ok(()) => "ok".into(),
                Err(PinError::Locked { .. }) => "locked".into(),
                Err(_) => "wrong PIN".into(),
            },
        };
        if let Err(e) = store.audit(entry).await {
            error!(error = %e, "writing the audit log failed");
            return Err(PinError::Internal);
        }
        if outcome.is_err() {
            warn!(action, "a wrong recovery PIN was entered");
        }
        outcome
    }
}

/// Failed attempts since the last good PIN and within the lock time, and
/// the newest of them. `log` is newest first.
fn recent_failures(log: &[AuditEntry], now: Timestamp) -> (usize, Option<Timestamp>) {
    let failures: Vec<Timestamp> = log
        .iter()
        .take_while(|e| e.outcome != "ok" && now.duration_since(e.time) < LOCK)
        .map(|e| e.time)
        .collect();
    (failures.len(), failures.first().copied())
}

impl IntoResponse for PinError {
    fn into_response(self) -> Response {
        let (status, error) = match &self {
            PinError::NotSetUp => (
                StatusCode::SERVICE_UNAVAILABLE,
                "no recovery PIN is set up on this machine".to_owned(),
            ),
            PinError::Wrong { attempts_left: 0 } => (
                StatusCode::FORBIDDEN,
                "wrong PIN; locked for 15 minutes".to_owned(),
            ),
            PinError::Wrong { attempts_left } => (
                StatusCode::FORBIDDEN,
                format!("wrong PIN; {attempts_left} attempts left, then locked for 15 minutes"),
            ),
            PinError::Locked { retry_after } => (
                StatusCode::TOO_MANY_REQUESTS,
                format!(
                    "too many wrong PINs; try again in {} minutes",
                    retry_after.as_secs().div_ceil(60)
                ),
            ),
            PinError::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal error".to_owned(),
            ),
        };
        let body = Json(ErrorBody { error });
        match self {
            PinError::Locked { retry_after } => (
                status,
                [(
                    axum::http::header::RETRY_AFTER,
                    retry_after.as_secs().to_string(),
                )],
                body,
            )
                .into_response(),
            _ => (status, body).into_response(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_are_nine_digits_and_typing_is_forgiven() {
        let pin = new_pin().unwrap();
        assert_eq!(normalize_pin(&pin).as_deref(), Some(pin.as_str()));
        assert_eq!(normalize_pin("123 456-789").as_deref(), Some("123456789"));
        for bad in ["", "12345678", "1234567890", "12345678a"] {
            assert_eq!(normalize_pin(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn failures_count_back_to_the_last_good_pin_within_the_lock() {
        let now: Timestamp = "2026-11-03T12:00:00Z".parse().unwrap();
        let entry = |mins: i64, outcome: &str| AuditEntry {
            time: now - SignedDuration::from_mins(mins),
            principal: LOCAL_CONSOLE.into(),
            action: PIN_ACTION.into(),
            target: "power.reboot".into(),
            outcome: outcome.into(),
        };
        let log = [
            entry(1, "wrong PIN"),
            entry(2, "wrong PIN"),
            entry(3, "ok"),
            entry(4, "wrong PIN"),
        ];
        assert_eq!(recent_failures(&log, now), (2, Some(log[0].time)));
        let log = [entry(1, "wrong PIN"), entry(16, "wrong PIN")];
        assert_eq!(recent_failures(&log, now).0, 1, "older than the lock");
        assert_eq!(recent_failures(&[], now), (0, None));
    }
}
