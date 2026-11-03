//! Molt updates under Crustates' control (ADR 0016). The agent checks the
//! node's channel once a day, checks the image signature, and stages new
//! images with `bootc switch` by digest. With the `auto` policy it reboots
//! into them inside the update window; with `manual` it only reports them.
//! greenboot rolls back an image that fails its health check, and the
//! agent then never stages that digest again.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crust_types::{MoltImage, MoltStatus, UpdatePolicy, Window};
use figment::Figment;
use figment::providers::{Format, Toml};
use jiff::{Timestamp, Zoned};
use serde::{Deserialize, Serialize};
use tokio::time::{MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// How often the channel is checked.
const CHECK_EVERY: Duration = Duration::from_hours(24);
/// The agent does not reboot twice for the same staged image within this
/// time, so a deployment that never takes cannot cause a reboot loop.
const REBOOT_RETRY_AFTER: Duration = Duration::from_hours(6);
/// Signature checks remembered, newest last.
const MAX_VERIFIED: usize = 32;

/// What can go wrong while updating.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A program the updater runs failed.
    #[error("{program} failed: {message}")]
    Command {
        /// The program, for example `bootc`.
        program: &'static str,
        /// What it said.
        message: String,
    },
    /// A program's output could not be understood.
    #[error("unexpected output from {program}: {message}")]
    Output {
        /// The program.
        program: &'static str,
        /// What did not fit.
        message: String,
    },
    /// The update settings are invalid.
    #[error("invalid update settings: {0}")]
    Settings(String),
    /// Reading or writing a file failed.
    #[error("{}: {source}", path.display())]
    File {
        /// The file.
        path: PathBuf,
        /// Why.
        source: std::io::Error,
    },
    /// The node cannot do what was asked; the message says why.
    #[error("{0}")]
    Refused(String),
}

/// One OS deployment, as the OS reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Deployment {
    /// The image reference it was pulled from.
    pub image: String,
    /// The digest of the image's manifest for this architecture.
    pub digest: String,
    /// The image's version label; empty if it has none.
    pub version: String,
}

/// The node's deployments.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Deployments {
    /// Running now.
    pub booted: Option<Deployment>,
    /// Boots next.
    pub staged: Option<Deployment>,
    /// The previous one.
    pub rollback: Option<Deployment>,
    /// Whether the rollback deployment boots next.
    pub rollback_queued: bool,
}

/// A channel tag resolved to digests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// What the tag points at: an image index for a multi-architecture image.
    pub digest: String,
    /// This architecture's image in it; equal to `digest` for a plain image.
    pub platform_digest: String,
    /// The image's version label; empty if it has none.
    pub version: String,
}

impl Resolved {
    fn is(&self, deployment: &Deployment) -> bool {
        deployment.digest == self.platform_digest
            || deployment.digest == self.digest
            || deployment.image.ends_with(&format!("@{}", self.digest))
    }
}

/// The OS updater engine: bootc, skopeo, cosign and mokutil on Molt
/// ([`crate::bootc::Bootc`]), [`FakeOs`] in tests.
pub trait Os: Send + Sync + 'static {
    /// The booted, staged and rollback deployments.
    fn deployments(&self) -> impl Future<Output = Result<Deployments, Error>> + Send;
    /// Resolves `repository:tag` in the registry.
    fn resolve(
        &self,
        repository: &str,
        tag: &str,
    ) -> impl Future<Output = Result<Resolved, Error>> + Send;
    /// Checks that `image` (`repository@digest`) was signed by the Crustates
    /// release workflows.
    fn verify(&self, image: &str) -> impl Future<Output = Result<(), Error>> + Send;
    /// Downloads `image` and makes it the deployment that boots next.
    fn stage(&self, image: &str) -> impl Future<Output = Result<(), Error>> + Send;
    /// Makes the rollback deployment boot next, dropping a staged one.
    fn rollback(&self) -> impl Future<Output = Result<(), Error>> + Send;
    /// Reboots the machine.
    fn reboot(&self) -> impl Future<Output = Result<(), Error>> + Send;
    /// Queues the module-signing certificate for MOK enrolment, confirmed
    /// with `password` at the next boot. Returns its SHA-256 fingerprint.
    fn enroll_key(&self, password: &str) -> impl Future<Output = Result<String, Error>> + Send;
    /// Whether the boot loader falls back from a new image that fails.
    fn automatic_rollback(&self) -> bool;
}

/// Update settings: the image's defaults, overridden from `/etc`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// The image repository, for example `ghcr.io/crustates/molt`.
    pub repository: String,
    /// The channel: the tag to follow.
    pub channel: String,
    /// When updates are applied.
    pub policy: UpdatePolicy,
    /// When `auto` may reboot, `HH:MM-HH:MM` in local time.
    pub window: String,
    /// Whether images must carry a valid Crustates signature. Only test
    /// images turn this off, for their local registry.
    #[serde(default = "yes")]
    pub verify_signatures: bool,
}

fn yes() -> bool {
    true
}

impl Settings {
    fn window(&self) -> Result<Window, Error> {
        self.window.parse().map_err(Error::Settings)
    }
}

/// Where the updater keeps its files.
#[derive(Clone, Debug)]
pub struct Paths {
    /// The image's settings, `/usr/lib/crustates/molt.toml`.
    pub defaults: PathBuf,
    /// Local settings, `/etc/crustates/molt.toml`; `crust molt policy`
    /// writes it.
    pub overrides: PathBuf,
    /// The updater's memory: bad and verified digests.
    pub state: PathBuf,
}

/// What the updater remembers across restarts and reboots.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Memory {
    /// Digests that failed their health check here.
    #[serde(default)]
    bad: Vec<String>,
    /// Digests whose signature this node checked, newest last.
    #[serde(default)]
    verified: Vec<String>,
    /// The staged digest the agent last rebooted for, and when.
    #[serde(default)]
    reboot: Option<(String, Timestamp)>,
}

/// What a channel check found.
enum Found {
    /// Nothing newer than what is booted or staged.
    UpToDate(Resolved),
    /// The newest image failed its health check here before.
    Bad(Resolved),
    /// A newer image, signature checked unless checks are off.
    New(String, Resolved),
}

/// Checks for, stages and rolls back Molt images, and reports their state.
#[derive(Debug)]
pub struct Updater<O> {
    os: O,
    paths: Paths,
    status: Mutex<MoltStatus>,
    /// One update action at a time.
    busy: tokio::sync::Mutex<()>,
}

impl<O: Os> Updater<O> {
    /// An updater that drives `os` and keeps its files at `paths`.
    #[must_use]
    pub fn new(os: O, paths: Paths) -> Self {
        Self {
            os,
            paths,
            status: Mutex::default(),
            busy: tokio::sync::Mutex::new(()),
        }
    }

    /// The last known status.
    #[must_use]
    pub fn status(&self) -> MoltStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn update_status(&self, f: impl FnOnce(&mut MoltStatus)) {
        if let Ok(mut status) = self.status.lock() {
            f(&mut status);
        }
    }

    /// Reads the settings: the image's defaults, then `/etc`.
    ///
    /// # Errors
    ///
    /// Fails if a file is invalid or the result is incomplete.
    pub fn settings(&self) -> Result<Settings, Error> {
        let settings: Settings = Figment::new()
            .merge(Toml::file(&self.paths.defaults))
            .merge(Toml::file(&self.paths.overrides))
            .extract()
            .map_err(|e| Error::Settings(e.to_string()))?;
        settings.window()?;
        let tag_char = |c: char| c.is_ascii_alphanumeric() || "._-".contains(c);
        if settings.channel.is_empty() || !settings.channel.chars().all(tag_char) {
            return Err(Error::Settings(format!(
                "channel {:?} is not an image tag",
                settings.channel
            )));
        }
        Ok(settings)
    }

    fn memory(&self) -> Memory {
        std::fs::read(&self.paths.state)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn remember(&self, f: impl FnOnce(&mut Memory)) -> Result<(), Error> {
        let mut memory = self.memory();
        f(&mut memory);
        let json = serde_json::to_vec_pretty(&memory).map_err(|e| Error::Output {
            program: "crust-agent",
            message: e.to_string(),
        })?;
        write_atomically(&self.paths.state, &json)
    }

    /// Reads the deployments and settings into the status.
    ///
    /// # Errors
    ///
    /// Fails if the OS or the settings cannot be read; the status keeps the
    /// error.
    pub async fn refresh(&self) -> Result<(Settings, Deployments), Error> {
        let result =
            async { Ok::<_, Error>((self.settings()?, self.os.deployments().await?)) }.await;
        let (settings, deployments) = match result {
            Ok(both) => both,
            Err(e) => {
                self.update_status(|s| s.last_error = e.to_string());
                return Err(e);
            }
        };
        let memory = self.memory();
        let image = |d: &Option<Deployment>| {
            d.as_ref().map(|d| MoltImage {
                image: d.image.clone(),
                digest: d.digest.clone(),
                version: d.version.clone(),
                verified: memory.verified.contains(&d.digest),
            })
        };
        let automatic_rollback = self.os.automatic_rollback();
        self.update_status(|s| {
            s.booted = image(&deployments.booted);
            s.staged = image(&deployments.staged);
            s.rollback = image(&deployments.rollback);
            s.rollback_queued = deployments.rollback_queued;
            s.repository.clone_from(&settings.repository);
            s.channel.clone_from(&settings.channel);
            s.policy = settings.policy;
            s.window.clone_from(&settings.window);
            s.bad_digests.clone_from(&memory.bad);
            s.automatic_rollback = automatic_rollback;
            // Staged or booted by now: no longer "available".
            let taken = |a: &MoltImage| {
                [&s.booted, &s.staged]
                    .into_iter()
                    .flatten()
                    .any(|d| d.digest == a.digest || d.image == a.image)
            };
            if s.available.as_ref().is_some_and(taken) {
                s.available = None;
            }
        });
        Ok((settings, deployments))
    }

    /// Resolves the channel and checks a new image's signature.
    async fn find(&self, settings: &Settings, deployments: &Deployments) -> Result<Found, Error> {
        let resolved = self
            .os
            .resolve(&settings.repository, &settings.channel)
            .await?;
        let current = [&deployments.booted, &deployments.staged];
        if current.into_iter().flatten().any(|d| resolved.is(d)) {
            return Ok(Found::UpToDate(resolved));
        }
        let memory = self.memory();
        if [&resolved.digest, &resolved.platform_digest]
            .iter()
            .any(|d| memory.bad.contains(d))
        {
            return Ok(Found::Bad(resolved));
        }
        let image = format!("{}@{}", settings.repository, resolved.digest);
        if settings.verify_signatures {
            self.os.verify(&image).await?;
            self.remember(|m| {
                for digest in [&resolved.digest, &resolved.platform_digest] {
                    if !m.verified.contains(digest) {
                        m.verified.push(digest.clone());
                    }
                }
                let excess = m.verified.len().saturating_sub(MAX_VERIFIED);
                m.verified.drain(..excess);
            })?;
        } else {
            warn!(image, "signature checks are off (test image); not checking");
        }
        Ok(Found::New(image, resolved))
    }

    /// Checks the channel; with `stage`, stages a new image. Returns what
    /// happened, for people.
    async fn check(&self, stage: bool) -> Result<String, Error> {
        let (settings, deployments) = self.refresh().await?;
        let result = self.find(&settings, &deployments).await;
        let now = Timestamp::now();
        let found = match result {
            Ok(found) => found,
            Err(e) => {
                self.update_status(|s| {
                    s.last_check = Some(now);
                    s.last_error = e.to_string();
                });
                return Err(e);
            }
        };
        let (message, available, last_error) = match found {
            Found::UpToDate(r) => {
                let message = match &deployments.staged {
                    Some(staged) if r.is(staged) => format!(
                        "the newest image, {}, is already staged; it boots at the next reboot",
                        describe(&r.version, &r.digest)
                    ),
                    _ => format!("up to date: {}", describe(&r.version, &r.digest)),
                };
                (message, None, String::new())
            }
            Found::Bad(r) => {
                let message = format!(
                    "the newest image on {}, {}, failed its health check on this node before; it is not staged again",
                    settings.channel,
                    describe(&r.version, &r.digest)
                );
                (message.clone(), None, message)
            }
            Found::New(image, r) => {
                let available = MoltImage {
                    image: image.clone(),
                    digest: r.digest.clone(),
                    version: r.version.clone(),
                    verified: settings.verify_signatures,
                };
                if stage {
                    if let Err(e) = self.os.stage(&image).await {
                        self.update_status(|s| {
                            s.last_check = Some(now);
                            s.available = Some(available);
                            s.last_error = e.to_string();
                        });
                        return Err(e);
                    }
                    info!(image, version = r.version, "staged a new Molt image");
                    let message = format!("staged {}", describe(&r.version, &r.digest));
                    (message, None, String::new())
                } else {
                    let message = format!("available: {}", describe(&r.version, &r.digest));
                    (message, Some(available), String::new())
                }
            }
        };
        self.update_status(|s| {
            s.last_check = Some(now);
            s.available = available;
            s.last_error = last_error;
        });
        self.refresh().await?;
        Ok(message)
    }

    /// Checks the channel and stages a newer image. With `reboot`, reboots
    /// into whatever is staged afterwards.
    ///
    /// # Errors
    ///
    /// Fails if the check, the signature or the download fails, or the
    /// newest image is marked bad.
    pub async fn upgrade(self: &Arc<Self>, reboot: bool) -> Result<String, Error> {
        let _busy = self.busy.lock().await;
        let message = self.check(true).await?;
        let status = self.status();
        if status.last_error.is_empty() {
            Ok(self.finish(message, status.staged.as_ref(), reboot, &status))
        } else {
            Err(Error::Refused(message))
        }
    }

    /// Words for what happens next after staging.
    fn finish(
        self: &Arc<Self>,
        message: String,
        staged: Option<&MoltImage>,
        reboot: bool,
        status: &MoltStatus,
    ) -> String {
        let Some(staged) = staged else {
            return message;
        };
        if reboot {
            self.reboot_soon();
            return format!(
                "{message}; rebooting into {} now",
                describe(&staged.version, &staged.digest)
            );
        }
        if message.contains("already staged") {
            return message;
        }
        match status.policy {
            UpdatePolicy::Auto => format!(
                "{message}; the node reboots into it between {} (local time)",
                status.window.replacen('-', " and ", 1)
            ),
            UpdatePolicy::Manual => format!("{message}; it boots at the next reboot"),
        }
    }

    /// Makes the previous image boot next. With `reboot`, reboots into it.
    ///
    /// # Errors
    ///
    /// Fails if there is no previous image or the OS refuses.
    pub async fn rollback(self: &Arc<Self>, reboot: bool) -> Result<String, Error> {
        let _busy = self.busy.lock().await;
        let (_, deployments) = self.refresh().await?;
        let Some(previous) = deployments.rollback else {
            return Err(Error::Refused(
                "there is no previous image to roll back to".into(),
            ));
        };
        if deployments.rollback_queued {
            return Err(Error::Refused(format!(
                "a rollback to {} is already queued for the next boot",
                describe(&previous.version, &previous.digest)
            )));
        }
        self.os.rollback().await?;
        self.refresh().await?;
        let target = describe(&previous.version, &previous.digest);
        let dropped = if deployments.staged.is_some() {
            "; the staged image was dropped"
        } else {
            ""
        };
        if reboot {
            self.reboot_soon();
            Ok(format!("rolling back to {target}{dropped}; rebooting now"))
        } else {
            Ok(format!("{target} boots at the next reboot{dropped}"))
        }
    }

    /// Writes the policy and, if given, the window to `/etc`.
    ///
    /// # Errors
    ///
    /// Fails if the settings file cannot be written.
    pub async fn set_policy(
        &self,
        policy: UpdatePolicy,
        window: Option<Window>,
    ) -> Result<String, Error> {
        let _busy = self.busy.lock().await;
        let path = &self.paths.overrides;
        let mut table: toml::Table = match std::fs::read_to_string(path) {
            Ok(text) => text
                .parse()
                .map_err(|e| Error::Settings(format!("{}: {e}", path.display())))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => toml::Table::new(),
            Err(source) => {
                return Err(Error::File {
                    path: path.clone(),
                    source,
                });
            }
        };
        table.insert("policy".into(), policy.to_string().into());
        if let Some(window) = window {
            table.insert("window".into(), window.to_string().into());
        }
        let text = format!(
            "# Written by crust molt policy; the image's defaults are in /usr/lib/crustates/molt.toml.\n{table}"
        );
        write_atomically(path, text.as_bytes())?;
        let (settings, _) = self.refresh().await?;
        Ok(match settings.policy {
            UpdatePolicy::Auto => format!(
                "policy auto: new images are staged when they appear, and the node reboots into them between {} (local time)",
                settings.window.replacen('-', " and ", 1)
            ),
            UpdatePolicy::Manual => {
                "policy manual: new images are only reported; stage one with crust molt upgrade"
                    .into()
            }
        })
    }

    /// Queues the module-signing key for enrolment. Returns what to do at
    /// the machine and the one-time password.
    ///
    /// # Errors
    ///
    /// Fails if Secure Boot is off, the key is enrolled already, or mokutil
    /// fails.
    pub async fn enroll_key(&self) -> Result<(String, String), Error> {
        let _busy = self.busy.lock().await;
        let password = one_time_password().map_err(|e| Error::Refused(e.to_string()))?;
        let fingerprint = self.os.enroll_key(&password).await?;
        Ok((
            format!(
                "the Crustates module-signing key (SHA-256 {fingerprint}) is queued. Reboot the machine; \
                 at its screen choose Enroll MOK, Continue, Yes, type the password, then Reboot"
            ),
            password,
        ))
    }

    /// Marks the booted image as bad, so it is never staged again. greenboot
    /// runs this when the image fails its health check.
    ///
    /// # Errors
    ///
    /// Fails if the deployments cannot be read or the state cannot be
    /// written.
    pub async fn mark_booted_bad(&self) -> Result<String, Error> {
        let deployments = self.os.deployments().await?;
        let booted = deployments
            .booted
            .ok_or_else(|| Error::Refused("no booted image".into()))?;
        let digest = booted.digest.clone();
        self.remember(|m| {
            if !m.bad.contains(&digest) {
                m.bad.push(digest.clone());
            }
        })?;
        Ok(digest)
    }

    /// One minute of the update loop: refresh, check when due, and with the
    /// `auto` policy reboot into a staged image inside the window.
    async fn tick(self: &Arc<Self>, now: &Zoned, check_due: bool) {
        let _busy = self.busy.lock().await;
        let Ok((settings, _)) = self.refresh().await else {
            return;
        };
        if check_due {
            match self.check(settings.policy == UpdatePolicy::Auto).await {
                Ok(message) => info!(message, "checked for a new Molt image"),
                Err(e) => warn!(error = %e, "checking for a new Molt image failed"),
            }
        }
        let status = self.status();
        let (Some(staged), UpdatePolicy::Auto) = (status.staged, settings.policy) else {
            return;
        };
        let Ok(window) = settings.window() else {
            return;
        };
        let minute =
            u16::try_from(now.hour()).unwrap_or(0) * 60 + u16::try_from(now.minute()).unwrap_or(0);
        if !window.contains(minute) {
            return;
        }
        let memory = self.memory();
        if let Some((digest, at)) = &memory.reboot
            && *digest == staged.digest
            && now.timestamp().duration_since(*at).unsigned_abs() < REBOOT_RETRY_AFTER
        {
            return;
        }
        let digest = staged.digest.clone();
        if let Err(e) = self.remember(|m| m.reboot = Some((digest, now.timestamp()))) {
            error!(error = %e, "cannot record the update reboot; not rebooting");
            return;
        }
        info!(
            version = staged.version,
            digest = staged.digest,
            "rebooting into the staged Molt image (update window)"
        );
        if let Err(e) = self.os.reboot().await {
            error!(error = %e, "rebooting failed");
        }
    }

    /// Reboots in a few seconds, after the command's result has gone out.
    fn reboot_soon(self: &Arc<Self>) {
        let updater = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(3)).await;
            info!("rebooting as asked");
            if let Err(e) = updater.os.reboot().await {
                error!(error = %e, "rebooting failed");
            }
        });
    }
}

/// Runs the update loop until `shutdown` fires: the first check 5 to 20
/// minutes after start, then every 24 hours.
pub async fn run<O: Os>(updater: Arc<Updater<O>>, shutdown: CancellationToken) {
    let mut next_check =
        Timestamp::now() + Duration::from_mins(5) + jitter(Duration::from_mins(15));
    let mut ticks = interval(Duration::from_mins(1));
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = shutdown.cancelled() => return,
            _ = ticks.tick() => {
                let now = Zoned::now();
                let due = now.timestamp() >= next_check;
                if due {
                    next_check = now.timestamp() + CHECK_EVERY;
                }
                updater.tick(&now, due).await;
            }
        }
    }
}

/// A random duration below `max`, so nodes do not all check at once.
fn jitter(max: Duration) -> Duration {
    let mut bytes = [0u8; 8];
    let _ = getrandom::fill(&mut bytes);
    let max = u64::try_from(max.as_millis()).unwrap_or(u64::MAX).max(1);
    Duration::from_millis(u64::from_le_bytes(bytes) % max)
}

/// Ten characters for the MOK manager, which has a US keyboard layout: lower
/// case letters and digits, without look-alikes.
fn one_time_password() -> Result<String, getrandom::Error> {
    const CHARS: &[u8; 32] = b"abcdefghjkmnpqrstuvwxyz023456789";
    let mut bytes = [0u8; 10];
    getrandom::fill(&mut bytes)?;
    Ok(bytes
        .iter()
        .map(|b| char::from(CHARS[usize::from(b & 31)]))
        .collect())
}

/// `0.1.0 (sha256:0123456789ab)`.
fn describe(version: &str, digest: &str) -> String {
    let short = digest
        .get(..19)
        .filter(|_| digest.starts_with("sha256:"))
        .unwrap_or(digest);
    if version.is_empty() {
        short.to_owned()
    } else {
        format!("{version} ({short})")
    }
}

/// Writes `bytes` next to `path` and renames it over `path`.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let file_error = |source| Error::File {
        path: path.to_owned(),
        source,
    };
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(file_error)?;
    }
    let partial = path.with_extension("partial");
    std::fs::write(&partial, bytes).map_err(file_error)?;
    std::fs::rename(&partial, path).map_err(file_error)
}

/// An [`Os`] in memory, for tests: a registry of tags and the deployments
/// a real node would have.
#[cfg(any(test, feature = "fake"))]
#[derive(Debug, Default)]
pub struct FakeOs {
    /// Everything the fake knows; tests read and change it.
    pub state: Mutex<FakeState>,
}

/// The state of a [`FakeOs`].
#[cfg(any(test, feature = "fake"))]
#[derive(Debug, Default)]
pub struct FakeState {
    /// The deployments.
    pub deployments: Deployments,
    /// `repository:tag` → what it resolves to.
    pub tags: std::collections::HashMap<String, Resolved>,
    /// Images (`repository@digest`) whose signature check fails.
    pub unsigned: Vec<String>,
    /// How often [`Os::reboot`] was called.
    pub reboots: u32,
    /// Whether Secure Boot is on.
    pub secure_boot: bool,
}

#[cfg(any(test, feature = "fake"))]
impl FakeOs {
    fn with<T>(&self, f: impl FnOnce(&mut FakeState) -> T) -> T {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut state)
    }
}

#[cfg(any(test, feature = "fake"))]
#[allow(
    clippy::unused_async_trait_impl,
    reason = "the trait is async; the fake answers at once"
)]
impl Os for FakeOs {
    async fn deployments(&self) -> Result<Deployments, Error> {
        Ok(self.with(|s| s.deployments.clone()))
    }

    async fn resolve(&self, repository: &str, tag: &str) -> Result<Resolved, Error> {
        self.with(|s| s.tags.get(&format!("{repository}:{tag}")).cloned())
            .ok_or_else(|| Error::Command {
                program: "skopeo",
                message: format!("{repository}:{tag}: manifest unknown"),
            })
    }

    async fn verify(&self, image: &str) -> Result<(), Error> {
        if self.with(|s| s.unsigned.iter().any(|u| u == image)) {
            return Err(Error::Command {
                program: "cosign",
                message: "no matching signatures".into(),
            });
        }
        Ok(())
    }

    async fn stage(&self, image: &str) -> Result<(), Error> {
        self.with(|s| {
            let resolved = s
                .tags
                .values()
                .find(|r| image.ends_with(&format!("@{}", r.digest)))
                .cloned();
            let Some(resolved) = resolved else {
                return Err(Error::Command {
                    program: "bootc",
                    message: format!("{image}: manifest unknown"),
                });
            };
            s.deployments.staged = Some(Deployment {
                image: image.to_owned(),
                digest: resolved.platform_digest,
                version: resolved.version,
            });
            Ok(())
        })
    }

    async fn rollback(&self) -> Result<(), Error> {
        self.with(|s| {
            if s.deployments.rollback.is_none() {
                return Err(Error::Command {
                    program: "bootc",
                    message: "no rollback available".into(),
                });
            }
            s.deployments.staged = None;
            s.deployments.rollback_queued = true;
            Ok(())
        })
    }

    async fn reboot(&self) -> Result<(), Error> {
        self.with(|s| s.reboots += 1);
        Ok(())
    }

    async fn enroll_key(&self, password: &str) -> Result<String, Error> {
        if !self.with(|s| s.secure_boot) {
            return Err(Error::Refused("Secure Boot is off".into()));
        }
        if password.len() != 10 {
            return Err(Error::Refused(format!("unexpected password {password:?}")));
        }
        Ok("AA:BB".into())
    }

    fn automatic_rollback(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO: &str = "ghcr.io/crustates/molt";

    fn deployment(version: &str, digest: &str) -> Deployment {
        Deployment {
            image: format!("{REPO}:beta"),
            digest: digest.into(),
            version: version.into(),
        }
    }

    fn resolved(version: &str, digest: &str) -> Resolved {
        Resolved {
            digest: format!("{digest}-index"),
            platform_digest: digest.into(),
            version: version.into(),
        }
    }

    /// An updater on a node that boots 1.0 and follows beta, where beta
    /// holds `beta`.
    fn updater(dir: &Path, policy: &str, beta: Resolved) -> Arc<Updater<FakeOs>> {
        let defaults = dir.join("defaults.toml");
        std::fs::write(
            &defaults,
            format!("repository = \"{REPO}\"\nchannel = \"beta\"\npolicy = \"{policy}\"\nwindow = \"03:00-05:00\"\n"),
        )
        .unwrap();
        let os = FakeOs::default();
        os.with(|s| {
            s.deployments.booted = Some(deployment("1.0", "sha256:10"));
            s.deployments.rollback = Some(deployment("0.9", "sha256:09"));
            s.tags.insert(format!("{REPO}:beta"), beta);
        });
        Arc::new(Updater::new(
            os,
            Paths {
                defaults,
                overrides: dir.join("etc/molt.toml"),
                state: dir.join("state/molt.json"),
            },
        ))
    }

    #[tokio::test]
    async fn upgrade_stages_the_newer_image_by_digest() {
        let dir = tempfile::tempdir().unwrap();
        let u = updater(dir.path(), "manual", resolved("1.1", "sha256:11"));
        let message = u.upgrade(false).await.unwrap();
        assert_eq!(
            message,
            "staged 1.1 (sha256:11-index); it boots at the next reboot"
        );
        let status = u.status();
        let staged = status.staged.unwrap();
        assert_eq!(staged.image, format!("{REPO}@sha256:11-index"));
        assert!(staged.verified, "the signature was checked before staging");
        assert_eq!(status.available, None);
        assert_eq!(u.os.with(|s| s.reboots), 0);

        // Nothing newer: up to date, and the staged image is named.
        let again = u.upgrade(false).await.unwrap();
        assert!(again.contains("already staged"), "{again}");
    }

    #[tokio::test]
    async fn unsigned_images_are_refused_and_reported() {
        let dir = tempfile::tempdir().unwrap();
        let u = updater(dir.path(), "manual", resolved("1.1", "sha256:11"));
        u.os.with(|s| s.unsigned.push(format!("{REPO}@sha256:11-index")));
        let err = u.upgrade(false).await.unwrap_err();
        assert!(err.to_string().contains("no matching signatures"), "{err}");
        let status = u.status();
        assert_eq!(status.staged, None);
        assert!(status.last_error.contains("cosign"));
    }

    #[tokio::test]
    async fn images_marked_bad_are_never_staged_again() {
        let dir = tempfile::tempdir().unwrap();
        let u = updater(dir.path(), "auto", resolved("1.1", "sha256:11"));
        // 1.1 boots, fails its health check, and greenboot marks it bad.
        u.os.with(|s| s.deployments.booted = Some(deployment("1.1", "sha256:11")));
        assert_eq!(u.mark_booted_bad().await.unwrap(), "sha256:11");
        // After the rollback 1.0 runs again, and beta still holds 1.1.
        u.os.with(|s| s.deployments.booted = Some(deployment("1.0", "sha256:10")));
        let err = u.upgrade(false).await.unwrap_err();
        assert!(err.to_string().contains("failed its health check"), "{err}");
        assert_eq!(u.status().staged, None);
        assert_eq!(u.status().bad_digests, ["sha256:11"]);
    }

    #[tokio::test]
    async fn manual_checks_only_report_and_auto_checks_stage() {
        let dir = tempfile::tempdir().unwrap();
        let u = updater(dir.path(), "manual", resolved("1.1", "sha256:11"));
        let noon = "2026-11-03T12:00:00[UTC]".parse::<Zoned>().unwrap();
        u.tick(&noon, true).await;
        let status = u.status();
        assert_eq!(status.staged, None);
        assert_eq!(status.available.unwrap().version, "1.1");

        u.set_policy(UpdatePolicy::Auto, None).await.unwrap();
        u.tick(&noon, true).await;
        let status = u.status();
        assert_eq!(status.staged.unwrap().version, "1.1");
        assert_eq!(status.available, None);
        // Outside the window: no reboot yet.
        assert_eq!(u.os.with(|s| s.reboots), 0);
    }

    #[tokio::test]
    async fn auto_reboots_once_inside_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let u = updater(dir.path(), "auto", resolved("1.1", "sha256:11"));
        let night = "2026-11-03T03:30:00[UTC]".parse::<Zoned>().unwrap();
        u.tick(&night, true).await;
        assert_eq!(u.os.with(|s| s.reboots), 1);
        // The reboot did not take (the image is still only staged): no loop.
        let later = "2026-11-03T03:31:00[UTC]".parse::<Zoned>().unwrap();
        u.tick(&later, false).await;
        assert_eq!(u.os.with(|s| s.reboots), 1);
    }

    #[tokio::test]
    async fn policy_changes_land_in_etc_and_keep_other_settings() {
        let dir = tempfile::tempdir().unwrap();
        let u = updater(dir.path(), "auto", resolved("1.0", "sha256:10"));
        std::fs::create_dir_all(dir.path().join("etc")).unwrap();
        std::fs::write(dir.path().join("etc/molt.toml"), "channel = \"stable\"\n").unwrap();
        let message = u
            .set_policy(UpdatePolicy::Manual, Some("22:00-23:30".parse().unwrap()))
            .await
            .unwrap();
        assert!(message.starts_with("policy manual"), "{message}");
        let settings = u.settings().unwrap();
        assert_eq!(settings.policy, UpdatePolicy::Manual);
        assert_eq!(settings.window, "22:00-23:30");
        assert_eq!(settings.channel, "stable");
    }

    #[tokio::test]
    async fn rollback_needs_a_previous_image_and_drops_the_staged_one() {
        let dir = tempfile::tempdir().unwrap();
        let u = updater(dir.path(), "manual", resolved("1.1", "sha256:11"));
        u.upgrade(false).await.unwrap();
        let message = u.rollback(false).await.unwrap();
        assert_eq!(
            message,
            "0.9 (sha256:09) boots at the next reboot; the staged image was dropped"
        );
        assert!(u.status().rollback_queued);
        assert!(u.rollback(false).await.is_err(), "already queued");

        u.os.with(|s| s.deployments.rollback = None);
        let err = u.rollback(false).await.unwrap_err();
        assert!(err.to_string().contains("no previous image"));
    }

    #[test]
    fn invalid_settings_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let u = updater(dir.path(), "auto", resolved("1.0", "sha256:10"));
        std::fs::create_dir_all(dir.path().join("etc")).unwrap();
        for bad in [
            "window = \"3am\"",
            "channel = \"../x\"",
            "policy = \"sometimes\"",
        ] {
            std::fs::write(dir.path().join("etc/molt.toml"), bad).unwrap();
            assert!(u.settings().is_err(), "{bad}");
        }
    }

    #[test]
    fn passwords_and_descriptions() {
        let pw = one_time_password().unwrap();
        assert_eq!(pw.len(), 10);
        assert!(
            pw.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        );
        assert_eq!(
            describe("0.2.0", "sha256:0123456789abcdef"),
            "0.2.0 (sha256:0123456789ab)"
        );
        assert_eq!(describe("", "sha256:01"), "sha256:01");
    }
}
