//! The admin token (ADR 0015): one bearer token for the whole API until
//! accounts arrive in M4. Only its argon2id hash is stored, in a file that
//! root writes and the API can only read.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use axum::Json;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tokio::sync::Semaphore;
use tracing::{error, warn};

use crate::http::ErrorBody;

/// Every admin token starts with this.
pub const TOKEN_PREFIX: &str = "crt_";
/// Crockford's base32 alphabet: no I, L, O or U, so a token copied from
/// paper is hard to get wrong.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// 20 characters of 5 bits: 100 random bits.
const TOKEN_CHARS: usize = 20;
/// Failed attempts a client may make per [`FAILURE_WINDOW`].
const MAX_FAILURES: u32 = 10;
const FAILURE_WINDOW: Duration = Duration::from_mins(1);
/// Clients remembered before old entries are dropped.
const MAX_CLIENTS: usize = 4096;

/// Who made a request; handlers write it into the audit log.
#[derive(Clone, Debug)]
pub struct Principal(pub String);

/// Makes a new random token, `crt_XXXXX-XXXXX-XXXXX-XXXXX`.
///
/// # Errors
///
/// Fails if the operating system has no randomness to give.
pub fn new_token() -> Result<String, getrandom::Error> {
    let mut bytes = [0u8; TOKEN_CHARS];
    getrandom::fill(&mut bytes)?;
    // 32 divides 256, so every character is equally likely.
    let chars: Vec<u8> = bytes
        .iter()
        .map(|b| ALPHABET[usize::from(b & 31)])
        .collect();
    Ok(format_token(&chars))
}

fn format_token(chars: &[u8]) -> String {
    let groups: Vec<&str> = chars
        .chunks(5)
        .map(|c| std::str::from_utf8(c).unwrap_or_default())
        .collect();
    format!("{TOKEN_PREFIX}{}", groups.join("-"))
}

/// The canonical form of a token as a person may type it: any case, with
/// or without dashes, `I` and `L` read as `1`, `O` as `0`. `None` if it
/// cannot be a token.
#[must_use]
pub fn normalize(token: &str) -> Option<String> {
    let token = token.trim();
    let rest = token
        .get(..TOKEN_PREFIX.len())
        .filter(|p| p.eq_ignore_ascii_case(TOKEN_PREFIX))
        .and(token.get(TOKEN_PREFIX.len()..))?;
    let mut chars = Vec::with_capacity(TOKEN_CHARS);
    for c in rest.bytes().filter(|c| *c != b'-') {
        let c = match c.to_ascii_uppercase() {
            b'I' | b'L' => b'1',
            b'O' => b'0',
            c if ALPHABET.contains(&c) => c,
            _ => return None,
        };
        chars.push(c);
    }
    (chars.len() == TOKEN_CHARS).then(|| format_token(&chars))
}

/// The argon2id hash of `token`, as a PHC string.
///
/// # Errors
///
/// Fails if hashing fails, which only happens without randomness for the salt.
pub fn hash(token: &str) -> Result<String, argon2::password_hash::Error> {
    let hash: PasswordHash = Argon2::default().hash_password(token.as_bytes())?;
    Ok(hash.to_string())
}

pub(crate) fn verify(token: &str, phc: &str) -> bool {
    PasswordHash::new(phc.trim()).is_ok_and(|hash| {
        Argon2::default()
            .verify_password(token.as_bytes(), &hash)
            .is_ok()
    })
}

/// Checks admin tokens against the hash file, and slows down guessing.
#[derive(Debug)]
pub struct Auth {
    hash_file: PathBuf,
    /// The hash and the token that last matched it. A request with the same
    /// token skips argon2 until the file changes.
    verified: Mutex<Option<(String, String)>>,
    failures: Mutex<HashMap<IpAddr, (Instant, u32)>>,
    /// argon2id takes 19 MiB and some CPU per check: at most two at once.
    hashing: Semaphore,
}

/// Why a request was turned away.
#[derive(Debug, PartialEq)]
enum Reject {
    Missing,
    Invalid,
    NotSetUp,
    TooMany,
    Internal,
}

impl Auth {
    /// Checks tokens against the hash in `hash_file`, which is read on every
    /// request, so a new token takes effect without a restart.
    #[must_use]
    pub fn new(hash_file: PathBuf) -> Self {
        Self {
            hash_file,
            verified: Mutex::default(),
            failures: Mutex::default(),
            hashing: Semaphore::new(2),
        }
    }

    /// The file the token hash is read from.
    #[must_use]
    pub fn hash_file(&self) -> &Path {
        &self.hash_file
    }

    async fn check(&self, presented: Option<&str>, client: IpAddr) -> Result<(), Reject> {
        if self.limited(client) {
            return Err(Reject::TooMany);
        }
        let presented = presented.ok_or(Reject::Missing)?;
        let Some(token) = normalize(presented) else {
            self.failed(client);
            return Err(Reject::Invalid);
        };
        let phc = match tokio::fs::read_to_string(&self.hash_file).await {
            Ok(phc) => phc,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(Reject::NotSetUp),
            Err(e) => {
                error!(file = %self.hash_file.display(), error = %e, "reading the admin token hash failed");
                return Err(Reject::Internal);
            }
        };
        if let Ok(verified) = self.verified.lock()
            && let Some((hash, known)) = verified.as_ref()
            && *hash == phc
            && constant_time_eq(known.as_bytes(), token.as_bytes())
        {
            return Ok(());
        }
        let _permit = self.hashing.acquire().await.map_err(|_| Reject::Internal)?;
        let (ok, token, phc) =
            tokio::task::spawn_blocking(move || (verify(&token, &phc), token, phc))
                .await
                .map_err(|_| Reject::Internal)?;
        if !ok {
            self.failed(client);
            return Err(Reject::Invalid);
        }
        if let Ok(mut verified) = self.verified.lock() {
            *verified = Some((phc, token));
        }
        Ok(())
    }

    fn limited(&self, client: IpAddr) -> bool {
        let Ok(failures) = self.failures.lock() else {
            return true;
        };
        failures
            .get(&client_key(client))
            .is_some_and(|(since, count)| {
                since.elapsed() < FAILURE_WINDOW && *count >= MAX_FAILURES
            })
    }

    fn failed(&self, client: IpAddr) {
        let Ok(mut failures) = self.failures.lock() else {
            return;
        };
        if failures.len() >= MAX_CLIENTS {
            failures.retain(|_, (since, _)| since.elapsed() < FAILURE_WINDOW);
        }
        let entry = failures
            .entry(client_key(client))
            .or_insert((Instant::now(), 0));
        if entry.0.elapsed() >= FAILURE_WINDOW {
            *entry = (Instant::now(), 0);
        }
        entry.1 += 1;
        warn!(client = %client, failures = entry.1, "rejected an admin token");
    }
}

/// IPv6 clients usually own a whole /64, so they are limited per /64.
fn client_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(Ipv6Addr::from(u128::from(v6) & !u128::from(u64::MAX))),
        },
        v4 @ IpAddr::V4(_) => v4,
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then_some(token)
}

/// Paths anyone may call: the health check and the API reference.
const OPEN_PATHS: [&str; 3] = ["/healthz", "/docs", "/docs/"];

/// Middleware: every request needs the admin token as a bearer token,
/// except the [`OPEN_PATHS`]. Fails closed: a new route is protected unless
/// it is added there.
pub async fn require_token(
    State(auth): State<Arc<Auth>>,
    mut request: Request,
    next: Next,
) -> Response {
    if OPEN_PATHS.contains(&request.uri().path()) {
        return next.run(request).await;
    }
    let client = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map_or(IpAddr::V6(Ipv6Addr::UNSPECIFIED), |c| c.0.ip());
    match auth.check(bearer(request.headers()), client).await {
        Ok(()) => {
            request
                .extensions_mut()
                .insert(Principal("admin token".into()));
            next.run(request).await
        }
        Err(reject) => reject.into_response(),
    }
}

impl IntoResponse for Reject {
    fn into_response(self) -> Response {
        let (status, error) = match self {
            Reject::Missing => (
                StatusCode::UNAUTHORIZED,
                "this needs the admin token: send it as `Authorization: Bearer crt_…`",
            ),
            Reject::Invalid => (StatusCode::UNAUTHORIZED, "the admin token is wrong"),
            Reject::NotSetUp => (
                StatusCode::SERVICE_UNAVAILABLE,
                "no admin token is set up yet; it is made on the machine's first boot",
            ),
            Reject::TooMany => (
                StatusCode::TOO_MANY_REQUESTS,
                "too many wrong tokens from this address; wait a minute",
            ),
            Reject::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "internal error"),
        };
        let body = Json(ErrorBody {
            error: error.into(),
        });
        match self {
            Reject::Missing | Reject::Invalid => {
                (status, [(header::WWW_AUTHENTICATE, "Bearer")], body).into_response()
            }
            Reject::TooMany => (status, [(header::RETRY_AFTER, "60")], body).into_response(),
            _ => (status, body).into_response(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_tokens_are_canonical_and_differ() {
        let a = new_token().unwrap();
        assert_eq!(a.len(), TOKEN_PREFIX.len() + TOKEN_CHARS + 3);
        assert_eq!(normalize(&a).as_deref(), Some(a.as_str()));
        assert_ne!(a, new_token().unwrap());
    }

    #[test]
    fn typed_tokens_are_forgiven_case_dashes_and_lookalikes() {
        let canonical = "crt_ABCDE-FGH1J-KMNP0-QRSTV";
        for typed in [
            "crt_abcde-fgh1j-kmnp0-qrstv",
            "CRT_ABCDEFGHIJKMNPOQRSTV",
            " crt_ABCDE-FGHLJ-KMNPO-QRSTV\n",
        ] {
            assert_eq!(normalize(typed).as_deref(), Some(canonical), "{typed:?}");
        }
        for bad in [
            "",
            "crt_",
            "tok_ABCDE-FGH1J-KMNP0-QRSTV",
            "crt_ABCDE-FGH1J-KMNP0-QRSTU",
            "crt_ABCDE",
        ] {
            assert_eq!(normalize(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn hashes_verify_only_their_token() {
        let token = new_token().unwrap();
        let phc = hash(&token).unwrap();
        assert!(phc.starts_with("$argon2id$"));
        assert!(verify(&token, &phc));
        assert!(!verify(&new_token().unwrap(), &phc));
        assert!(!verify(&token, "not a hash"));
    }

    #[tokio::test]
    async fn ten_wrong_tokens_lock_the_client_out_for_a_minute() {
        let dir = std::env::temp_dir().join(format!("crust-auth-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("admin-token");
        let token = new_token().unwrap();
        std::fs::write(&file, hash(&token).unwrap()).unwrap();
        let auth = Auth::new(file);
        let (attacker, other) = ("192.0.2.1".parse().unwrap(), "192.0.2.2".parse().unwrap());

        for _ in 0..MAX_FAILURES {
            assert_eq!(
                auth.check(Some("crt_nope"), attacker).await,
                Err(Reject::Invalid)
            );
        }
        // Locked out, even with the right token; others are not.
        assert_eq!(
            auth.check(Some(&token), attacker).await,
            Err(Reject::TooMany)
        );
        assert_eq!(auth.check(Some(&token), other).await, Ok(()));
        // The second check with the same token comes from the cache.
        assert_eq!(auth.check(Some(&token), other).await, Ok(()));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ipv6_clients_share_a_limit_per_64() {
        let a: IpAddr = "2001:db8:1:2::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2:ffff::9".parse().unwrap();
        let mapped: IpAddr = "::ffff:192.0.2.7".parse().unwrap();
        assert_eq!(client_key(a), client_key(b));
        assert_eq!(client_key(mapped), "192.0.2.7".parse::<IpAddr>().unwrap());
    }
}
