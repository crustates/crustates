//! Control plane configuration: defaults, then `crust-api.toml`, then
//! `CRUST_*` environment variables.

use std::net::SocketAddr;
use std::path::PathBuf;

use figment::Figment;
use figment::providers::{Env, Format, Serialized, Toml};
use serde::{Deserialize, Serialize};

/// Control plane settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// Address of the REST API and the `/docs` reference.
    pub listen: SocketAddr,
    /// Address node agents connect to (gRPC).
    pub agent_listen: SocketAddr,
    /// SQLite database file.
    pub database: PathBuf,
    /// Seconds without a report before a node turns `NotReady`.
    pub heartbeat_timeout_secs: u64,
    /// The argon2id hash of the admin token (ADR 0015). `crust-api
    /// new-admin-token` writes it.
    pub admin_token_file: PathBuf,
    /// The argon2id hash of the recovery PIN of the local console (ADR
    /// 0018). `crust-api new-recovery-pin` writes it.
    pub recovery_pin_file: PathBuf,
    /// The Unix socket the local console uses; only root may call through
    /// it. None: no socket.
    pub local_socket: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([127, 0, 0, 1], 8080)),
            agent_listen: SocketAddr::from(([127, 0, 0, 1], 8081)),
            database: PathBuf::from("crust.db"),
            heartbeat_timeout_secs: 15,
            admin_token_file: PathBuf::from("crust-admin-token"),
            recovery_pin_file: PathBuf::from("crust-recovery-pin"),
            local_socket: None,
        }
    }
}

impl Config {
    /// Loads the configuration. The file is `$CRUST_CONFIG`, or
    /// `crust-api.toml` in the working directory; it may be missing. Each
    /// field can be overridden as `CRUST_<FIELD>`, for example `CRUST_LISTEN`.
    ///
    /// # Errors
    ///
    /// Fails if the file or a variable holds an invalid value.
    pub fn load() -> Result<Self, Box<figment::Error>> {
        let file = std::env::var_os("CRUST_CONFIG")
            .map_or_else(|| PathBuf::from("crust-api.toml"), PathBuf::from);
        Figment::from(Serialized::defaults(Config::default()))
            .merge(Toml::file(file))
            .merge(Env::prefixed("CRUST_"))
            .extract()
            .map_err(Box::new)
    }
}
