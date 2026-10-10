use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant as StdInstant};
use std::{fmt, future::Future, io};

use camino::{Utf8Path, Utf8PathBuf};
use futures_util::{Stream, StreamExt, stream};
use platform::PlatformCapability;
#[cfg(target_os = "macos")]
use rustix::process::{
    Pid, Signal, WaitOptions, kill_process_group, test_kill_process, test_kill_process_group,
    waitpid,
};
use rustls::pki_types::ServerName;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use state::{PvPaths, StateError, fs};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{Instant, sleep, timeout};
use tokio_rustls::TlsConnector;

use crate::monitor::{self, MonitorCleanup, MonitorExit, MonitorStart, MonitorState, MonitorStop};
use crate::{DaemonError, structured_log};

const READINESS_POLL_INTERVAL: Duration = Duration::from_millis(25);
const READINESS_PROBE_TIMEOUT: Duration = Duration::from_secs(1);
/// How long past a stop's grace period to wait for its monitor: the SIGKILL wait and the final
/// group check, with room to spare.
const MONITOR_STOP_MARGIN: Duration = Duration::from_secs(5);
const PRIVATE_ENVIRONMENT_REDACTION: &str = "<redacted>";
const PRIVATE_ENVIRONMENT_FINGERPRINT_PREFIX: &str = "sha256:v1:";
const POSTGRES_RECOVERY: &str = "Restart your Mac, then run this command again.";
pub(crate) const RUNTIME_READINESS_CONCURRENCY_LIMIT: usize = 4;

/// The signal that asks a runtime to shut down gracefully, before the grace period ends in SIGKILL.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopSignal {
    /// SIGTERM.
    Terminate,
    /// SIGINT, for runtimes whose SIGTERM shutdown waits on clients, such as PostgreSQL's smart
    /// shutdown. PostgreSQL's SIGINT is its fast shutdown: it disconnects clients and still writes
    /// a shutdown checkpoint.
    Interrupt,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProcessSignal {
    Stop(StopSignal),
    Kill,
}

#[derive(Clone, Eq, PartialEq)]
pub struct ProcessSpec {
    pub name: String,
    pub command: Utf8PathBuf,
    pub arguments: Vec<String>,
    pub private_environment: BTreeMap<String, String>,
    pub config_path: Utf8PathBuf,
    pub config_fingerprint: Option<String>,
    pub log_path: Utf8PathBuf,
    pub pid_path: Utf8PathBuf,
    pub metadata_path: Utf8PathBuf,
    pub resource_name: String,
    pub track: String,
}

impl fmt::Debug for ProcessSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("ProcessSpec");
        debug.field("name", &self.name);
        debug.field("command", &self.command);
        debug.field("arguments", &self.arguments);
        if !self.private_environment.is_empty() {
            debug.field(
                "private_environment",
                &PrivateEnvironmentDebug(&self.private_environment),
            );
        }
        debug.field("config_path", &self.config_path);
        if let Some(config_fingerprint) = &self.config_fingerprint {
            debug.field("config_fingerprint", config_fingerprint);
        }
        debug.field("log_path", &self.log_path);
        debug.field("pid_path", &self.pid_path);
        debug.field("metadata_path", &self.metadata_path);
        debug.field("resource_name", &self.resource_name);
        debug.field("track", &self.track);
        debug.finish()
    }
}

pub(crate) struct PrivateEnvironmentDebug<'a>(pub(crate) &'a BTreeMap<String, String>);

impl fmt::Debug for PrivateEnvironmentDebug<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_map()
            .entries(
                self.0
                    .keys()
                    .map(|name| (name, PRIVATE_ENVIRONMENT_REDACTION)),
            )
            .finish()
    }
}

#[derive(Debug)]
pub struct ProcessSupervisor {
    paths: PvPaths,
}

/// A runtime this supervisor has just started under a monitor.
pub struct ManagedProcess {
    paths: PvPaths,
    pid: u32,
    /// The runtime's birth identity, or `None` if it had exited before the monitor reported it.
    start_identity: Option<platform::ProcessStartIdentity>,
    subject: String,
    instance: String,
    log_path: Utf8PathBuf,
    pid_path: Utf8PathBuf,
    metadata_path: Utf8PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RecordedConfigFingerprint {
    /// Bytes committed as applied; service readiness may remain unverified under the preserve policy.
    Applied(String),
    /// Exact promoted bytes prepared for a transaction; this proves neither application nor
    /// readiness and can remain useful after the process exits.
    Staged(String),
}

/// A runtime whose monitor still runs it as its record names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedRuntime {
    paths: PvPaths,
    pid: u32,
    subject: String,
    instance: String,
    command: Utf8PathBuf,
    replacement_required: bool,
    applied_config_fingerprint: Option<String>,
    desired_config_fingerprint: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdoptedProcess {
    owned: OwnedRuntime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadinessCheck {
    Tcp {
        host: String,
        port: u16,
    },
    GatewayHttps {
        http_host: String,
        http_port: u16,
        https_host: String,
        https_port: u16,
        server_name: String,
        ca_certificate_path: Utf8PathBuf,
    },
    GatewayIdentity {
        http_host: String,
        http_port: u16,
        https_host: String,
        https_port: u16,
        server_name: String,
        path: String,
        expected_body: String,
        ca_certificate_path: Utf8PathBuf,
    },
    RedisPing {
        host: String,
        port: u16,
    },
    Http {
        host: String,
        port: u16,
        path: String,
    },
}

/// A runtime's record beside its monitor: its spec identity and config proofs. A record PV wrote
/// before monitors names its process by pid file and identity instead of a monitor instance.
#[derive(Deserialize, Eq, PartialEq, Serialize)]
struct RuntimeMetadata {
    name: String,
    /// The runtime's pid when it started, for diagnostics; ownership comes from the monitor.
    pid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    monitor_instance: Option<String>,
    command: String,
    arguments: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    private_environment_fingerprint: Option<String>,
    #[serde(default)]
    config_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    config_fingerprint: Option<String>,
    #[serde(default)]
    resource_name: String,
    #[serde(default)]
    track: String,
    #[serde(default, skip_serializing_if = "is_false")]
    replacement_required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    applied_config_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    desired_config_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    staged_config_fingerprint: Option<String>,
    log_path: String,
    started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    boot_session_id: Option<platform::BootSessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    process_start_identity: Option<platform::ProcessStartIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    process_executable_identity: Option<ProcessExecutableIdentity>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ProcessExecutableIdentity {
    executable: String,
    argument_zero: String,
}

impl ProcessSupervisor {
    pub fn new(paths: PvPaths) -> Self {
        Self { paths }
    }

    /// Stops a runtime that a PV release from before monitors started, proven by its pid file and
    /// live identity. Then it removes the pid file and, unless `keep_metadata`, the record, whose
    /// config proofs a restarted runtime may still need.
    pub(crate) async fn stop_legacy_runtime(
        &self,
        pid_path: &Utf8Path,
        keep_metadata: bool,
    ) -> Result<(), DaemonError> {
        let metadata_path = pid_path.with_extension("json");
        for path in [pid_path, metadata_path.as_path()] {
            if !fs::path_is_file(path)? {
                return Err(DaemonError::InvalidRuntimeRecord {
                    path: path.to_owned(),
                });
            }
        }
        let metadata = read_runtime_metadata(&metadata_path)?.ok_or_else(|| {
            DaemonError::InvalidRuntimeRecord {
                path: metadata_path.clone(),
            }
        })?;
        let pid = read_pid_file(pid_path)?.ok_or_else(|| DaemonError::InvalidRuntimeRecord {
            path: pid_path.to_owned(),
        })?;
        let subject_matches = if pid_path == self.paths.gateway_pid() {
            metadata.resource_name == "caddy"
        } else if pid_path.parent() == Some(self.paths.run().join("workers").as_path()) {
            metadata.resource_name == "frankenphp"
                && pid_path == self.paths.worker_pid(&metadata.track)
        } else {
            !metadata.resource_name.is_empty()
                && !metadata.track.is_empty()
                && pid_path
                    == self
                        .paths
                        .resource_pid(&metadata.resource_name, &metadata.track)
        };
        if metadata.pid != pid || metadata.process_start_identity.is_none() || !subject_matches {
            return Err(DaemonError::InvalidRuntimeRecord {
                path: metadata_path,
            });
        }
        if let Some(recorded_boot) = &metadata.boot_session_id
            && recorded_boot != &platform::current_boot_session_id()?
        {
            // A previous kernel's processes cannot survive this boot. Do not inspect
            // or signal the recorded PID: it may now belong to an unrelated process.
            return remove_unchanged_runtime_records(
                pid_path,
                &metadata_path,
                &metadata,
                keep_metadata,
            );
        }
        if legacy_process_matches(pid, &metadata)? {
            // Postgres backends can leave the postmaster's process group, so an empty
            // group proves nothing about them. The postmaster exits successfully only
            // after its backends exit. Watch before signalling, then recheck ownership.
            let exit_watch = if metadata.resource_name == "postgres" {
                let watch = platform::ProcessExitWatch::new(pid)?;
                if !legacy_process_matches(pid, &metadata)? {
                    return Err(DaemonError::RuntimeProcessIdentityChanged { pid });
                }
                Some(watch)
            } else {
                None
            };
            let stop = stop_policy(&metadata.resource_name);
            // An adopted leader can be reaped during the grace period, by launchd once the
            // daemon that started it has exited. Do not signal a group whose recorded
            // leader no longer matches.
            stop_process_group(pid, stop.signal, stop_grace(stop), || {
                legacy_process_matches(pid, &metadata)
            })
            .await?;
            if let Some(mut watch) = exit_watch {
                let status = timeout(Duration::from_secs(1), async {
                    loop {
                        if let Some(status) = watch.try_exit_status()? {
                            return Ok::<_, DaemonError>(status);
                        }
                        sleep(READINESS_POLL_INTERVAL).await;
                    }
                })
                .await;
                let status = match status {
                    Ok(status) => status?,
                    Err(_elapsed) => {
                        return Err(DaemonError::RuntimeCleanupUnproven {
                            pid,
                            reason: format!(
                                "PV could not confirm that Postgres shut down cleanly, so it kept its records. {POSTGRES_RECOVERY}"
                            ),
                        });
                    }
                };
                if !status.success() {
                    return Err(DaemonError::RuntimeCleanupUnproven {
                        pid,
                        reason: format!(
                            "Postgres ended with {status}, so PV kept its records. {POSTGRES_RECOVERY}"
                        ),
                    });
                }
            }
        } else if !process_and_group_are_absent(pid)? {
            return Err(DaemonError::RuntimeProcessIdentityChanged { pid });
        } else if metadata.resource_name == "postgres" {
            return Err(DaemonError::RuntimeCleanupUnproven {
                pid,
                reason: format!(
                    "Postgres exited before PV could stop it, so PV kept its records. {POSTGRES_RECOVERY}"
                ),
            });
        }
        remove_unchanged_runtime_records(pid_path, &metadata_path, &metadata, keep_metadata)
    }

    /// Starts `spec`'s runtime under a new monitor, run by the installed `pv`, and records it.
    pub async fn start(&self, spec: ProcessSpec) -> Result<ManagedProcess, DaemonError> {
        require_process_containment()?;
        state::fs::ensure_layout(&self.paths)?;
        let subject = monitor_subject(&self.paths, &spec.metadata_path)?;
        let stop = stop_policy(&spec.resource_name);
        // A monitor left by a cancelled start or an earlier daemon must not outlive its subject.
        self.stop_subject(&spec.metadata_path, stop.signal, stop_grace(stop))
            .await?;

        let state = monitor::start_monitor(
            &self.paths,
            &self.paths.active_pv_binary(),
            MonitorStart {
                subject: subject.clone(),
                command: spec.command.clone(),
                arguments: spec.arguments.clone(),
                private_environment: spec.private_environment.clone(),
                log_path: spec.log_path.clone(),
                fallback_stop: stop,
                lifeline_fd: None,
            },
        )
        .await?;
        let instance =
            monitor::recorded_monitor_instance(&self.paths, &subject)?.ok_or_else(|| {
                DaemonError::MonitorUnavailable {
                    subject: subject.clone(),
                    reason: "the monitor published no record".to_owned(),
                }
            })?;
        let process = ManagedProcess {
            paths: self.paths.clone(),
            pid: state.runtime_pid,
            start_identity: platform::inspect_process_start_identity(state.runtime_pid)?,
            subject,
            instance,
            log_path: spec.log_path.clone(),
            pid_path: spec.pid_path.clone(),
            metadata_path: spec.metadata_path.clone(),
        };
        if let Err(error) = write_runtime_metadata(&spec, process.pid, &process.instance) {
            let _stop_result =
                stop_monitored_runtime(&self.paths, &process.subject, &process.instance, stop)
                    .await;
            return Err(error);
        }

        Ok(process)
    }

    /// Whether a monitor for `metadata_path`'s subject still runs its runtime, recorded or not.
    pub(crate) fn subject_runtime_is_live(
        &self,
        metadata_path: &Utf8Path,
    ) -> Result<bool, DaemonError> {
        require_process_containment()?;
        let subject = monitor_subject(&self.paths, metadata_path)?;

        Ok(monitor::live_runtime(&self.paths, &subject)?.is_some())
    }

    /// Stops `subject`'s monitored runtime for disable or uninstall, by its usual stop. Postgres
    /// counts as stopped only once it has exited 0: its backends can leave its process group,
    /// and the postmaster exits successfully only after they have.
    pub(crate) async fn stop_monitored_for_shutdown(
        &self,
        subject: &str,
    ) -> Result<(), DaemonError> {
        let Some(instance) = monitor::recorded_monitor_instance(&self.paths, subject)? else {
            return Ok(());
        };
        let postgres = subject_resource_name(subject) == "postgres";
        let stop = stop_policy(subject_resource_name(subject));
        match stop_monitor_instance(&self.paths, subject, &instance, stop).await? {
            MonitorStopped::Stopped(state)
                if postgres && state.exit != Some(MonitorExit::Code(0)) =>
            {
                Err(DaemonError::RuntimeCleanupUnproven {
                    pid: state.runtime_pid,
                    reason: format!(
                        "Postgres ended with {}, so PV kept its records. {POSTGRES_RECOVERY}",
                        exit_description(state.exit)
                    ),
                })
            }
            MonitorStopped::Stopped(state) => {
                structured_log::runtime_exited(&self.paths, subject, &exit_description(state.exit));
                monitor::release_monitor(&self.paths, subject).await
            }
            MonitorStopped::Unavailable if postgres => Err(DaemonError::MonitorUnavailable {
                subject: subject.to_owned(),
                reason: format!(
                    "PV could not confirm that Postgres shut down cleanly, so it kept its records. {POSTGRES_RECOVERY}"
                ),
            }),
            MonitorStopped::Unavailable => monitor::recover_monitor(&self.paths, subject).await,
            MonitorStopped::Gone => Ok(()),
        }
    }

    /// Stops and releases whatever monitor `metadata_path`'s subject has, including one with no
    /// record beside it, one whose runtime has exited, and one that died. Returns whether there
    /// was one.
    pub async fn stop_subject(
        &self,
        metadata_path: &Utf8Path,
        signal: StopSignal,
        grace_period: Duration,
    ) -> Result<bool, DaemonError> {
        require_process_containment()?;
        let subject = monitor_subject(&self.paths, metadata_path)?;
        let Some(instance) = monitor::recorded_monitor_instance(&self.paths, &subject)? else {
            return Ok(false);
        };
        stop_monitored_runtime(
            &self.paths,
            &subject,
            &instance,
            monitor_stop(signal, grace_period),
        )
        .await?;

        Ok(true)
    }

    /// The runtime `spec` describes, if its monitor still runs it as recorded.
    pub fn verify_ownership(
        &self,
        spec: &ProcessSpec,
    ) -> Result<Option<OwnedRuntime>, DaemonError> {
        require_process_containment()?;
        let Some(metadata) = read_runtime_metadata(&spec.metadata_path)? else {
            return Ok(None);
        };
        if !metadata.matches(spec) {
            return Ok(None);
        }

        self.owned_runtime(&spec.metadata_path, metadata)
    }

    /// Returns recorded config identity without proving that a process is alive.
    pub(crate) fn recorded_config_fingerprint(
        &self,
        spec: &ProcessSpec,
    ) -> Result<Option<RecordedConfigFingerprint>, DaemonError> {
        let Some(metadata) = read_runtime_metadata(&spec.metadata_path)? else {
            return Ok(None);
        };
        // A record that names neither a monitor nor a process identity is incomplete.
        if metadata.matches(spec)
            && (metadata.monitor_instance.is_some() || metadata.process_start_identity.is_some())
        {
            return Ok(metadata.recorded_config_fingerprint());
        }

        // The installed artifact changed under a live runtime: prove the prior recording
        // against its recorded spec so old applied fingerprints stay readable. Live
        // ownership decisions still use verify_ownership against the current spec.
        let Some(adopted) = self.adopt_recorded(&spec.pid_path, &spec.metadata_path)? else {
            return Ok(None);
        };

        Ok(adopted
            .into_owned()
            .applied_config_fingerprint()
            .map(|fingerprint| RecordedConfigFingerprint::Applied(fingerprint.to_owned())))
    }

    pub fn mark_replacement_required(
        &self,
        spec: &ProcessSpec,
        staged_config_fingerprint: &str,
    ) -> Result<bool, DaemonError> {
        self.set_config_application_state(
            spec,
            true,
            None,
            Some(staged_config_fingerprint),
            Some(staged_config_fingerprint),
        )
    }

    pub(crate) fn mark_restoration_required(
        &self,
        spec: &ProcessSpec,
        staged_config_fingerprint: &str,
    ) -> Result<bool, DaemonError> {
        self.set_config_application_state(spec, true, None, Some(staged_config_fingerprint), None)
    }

    pub fn clear_replacement_required(&self, spec: &ProcessSpec) -> Result<bool, DaemonError> {
        self.set_config_application_state(spec, false, None, None, None)
    }

    pub fn record_applied_config(
        &self,
        spec: &ProcessSpec,
        fingerprint: &str,
    ) -> Result<bool, DaemonError> {
        self.set_config_application_state(spec, false, Some(fingerprint), None, Some(fingerprint))
    }

    pub(crate) fn record_restored_config(
        &self,
        spec: &ProcessSpec,
        fingerprint: &str,
    ) -> Result<bool, DaemonError> {
        self.set_config_application_state(spec, false, Some(fingerprint), None, None)
    }

    fn set_config_application_state(
        &self,
        spec: &ProcessSpec,
        replacement_required: bool,
        applied_config_fingerprint: Option<&str>,
        staged_config_fingerprint: Option<&str>,
        desired_config_fingerprint: Option<&str>,
    ) -> Result<bool, DaemonError> {
        require_process_containment()?;
        let Some(mut metadata) = read_runtime_metadata(&spec.metadata_path)? else {
            return Ok(false);
        };
        if !metadata.matches(spec)
            || self
                .owned_monitor(&spec.metadata_path, &metadata)?
                .is_none()
        {
            return Ok(false);
        }

        metadata.replacement_required = replacement_required;
        metadata.applied_config_fingerprint = applied_config_fingerprint.map(str::to_owned);
        metadata.staged_config_fingerprint = staged_config_fingerprint.map(str::to_owned);
        if let Some(desired_config_fingerprint) = desired_config_fingerprint {
            metadata.desired_config_fingerprint = Some(desired_config_fingerprint.to_owned());
        } else if metadata.desired_config_fingerprint.is_none()
            && let Some(applied_config_fingerprint) = applied_config_fingerprint
        {
            metadata.desired_config_fingerprint = Some(applied_config_fingerprint.to_owned());
        }
        let encoded = serde_json::to_string(&metadata)?;
        fs::write_sensitive_file(&spec.metadata_path, &encoded)?;

        Ok(true)
    }

    pub fn adopt(&self, spec: &ProcessSpec) -> Result<Option<AdoptedProcess>, DaemonError> {
        require_process_containment()?;
        Ok(self
            .verify_ownership(spec)?
            .map(|owned| AdoptedProcess { owned }))
    }

    /// The runtime recorded at `metadata_path`, if its monitor still runs it, whatever its
    /// current spec. `_pid_path` names a record PV wrote before monitors.
    pub fn adopt_recorded(
        &self,
        _pid_path: &Utf8Path,
        metadata_path: &Utf8Path,
    ) -> Result<Option<AdoptedProcess>, DaemonError> {
        require_process_containment()?;
        let Some(metadata) = read_runtime_metadata(metadata_path)? else {
            return Ok(None);
        };

        Ok(self
            .owned_runtime(metadata_path, metadata)?
            .map(|owned| AdoptedProcess { owned }))
    }

    fn owned_runtime(
        &self,
        metadata_path: &Utf8Path,
        metadata: RuntimeMetadata,
    ) -> Result<Option<OwnedRuntime>, DaemonError> {
        let Some((subject, instance)) = self.owned_monitor(metadata_path, &metadata)? else {
            return Ok(None);
        };

        Ok(Some(OwnedRuntime {
            paths: self.paths.clone(),
            pid: metadata.pid,
            subject,
            instance,
            command: metadata.command.as_str().into(),
            replacement_required: metadata.replacement_required,
            applied_config_fingerprint: match metadata.recorded_config_fingerprint() {
                Some(RecordedConfigFingerprint::Applied(fingerprint)) => Some(fingerprint),
                Some(RecordedConfigFingerprint::Staged(_)) | None => None,
            },
            desired_config_fingerprint: metadata.desired_config_fingerprint,
        }))
    }

    /// The subject and instance of the monitor `metadata` names, while it still runs the
    /// recorded runtime.
    fn owned_monitor(
        &self,
        metadata_path: &Utf8Path,
        metadata: &RuntimeMetadata,
    ) -> Result<Option<(String, String)>, DaemonError> {
        let Some(instance) = &metadata.monitor_instance else {
            return Ok(None);
        };
        let subject = monitor_subject(&self.paths, metadata_path)?;
        let owned = monitor::live_runtime(&self.paths, &subject)?
            .is_some_and(|live| &live.instance == instance && live.runtime_pid == metadata.pid);

        Ok(owned.then(|| (subject, instance.clone())))
    }
}

impl ManagedProcess {
    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn log_path(&self) -> &Utf8Path {
        &self.log_path
    }

    pub fn pid_path(&self) -> &Utf8Path {
        &self.pid_path
    }

    pub fn metadata_path(&self) -> &Utf8Path {
        &self.metadata_path
    }

    /// Whether the runtime has exited. An identity lookup never finds an exited process, so a
    /// changed identity is enough; the monitor keeps the exit status.
    pub fn has_exited(&mut self) -> Result<bool, DaemonError> {
        let Some(start_identity) = self.start_identity else {
            return Ok(true);
        };

        Ok(platform::inspect_process_start_identity(self.pid)? != Some(start_identity))
    }

    pub async fn stop(self, grace_period: Duration) -> Result<(), DaemonError> {
        self.stop_with(StopSignal::Terminate, grace_period).await
    }

    pub async fn stop_with(
        self,
        signal: StopSignal,
        grace_period: Duration,
    ) -> Result<(), DaemonError> {
        require_process_containment()?;
        stop_monitored_runtime(
            &self.paths,
            &self.subject,
            &self.instance,
            monitor_stop(signal, grace_period),
        )
        .await
    }
}

pub(crate) async fn wait_for_started_runtime_readiness<Readiness>(
    process: &mut ManagedProcess,
    runtime_name: &str,
    readiness: Readiness,
    process_exit_poll_interval: Duration,
) -> Result<(), DaemonError>
where
    Readiness: Future<Output = Result<(), DaemonError>>,
{
    tokio::pin!(readiness);

    loop {
        tokio::select! {
            result = &mut readiness => {
                if result.is_err() && process.has_exited()? {
                    return Err(runtime_exited_before_readiness_error(runtime_name));
                }

                return result;
            }
            () = sleep(process_exit_poll_interval) => {
                if process.has_exited()? {
                    return Err(runtime_exited_before_readiness_error(runtime_name));
                }
            }
        }
    }
}

pub(crate) fn bounded_runtime_readiness<Item, Output, Wait, Readiness>(
    items: impl IntoIterator<Item = Item>,
    wait: Wait,
) -> impl Stream<Item = Output>
where
    Wait: FnMut(Item) -> Readiness,
    Readiness: Future<Output = Output>,
{
    stream::iter(items)
        .map(wait)
        .buffer_unordered(RUNTIME_READINESS_CONCURRENCY_LIMIT)
}

pub(crate) fn runtime_exited_before_readiness_error(runtime_name: &str) -> DaemonError {
    DaemonError::UnexpectedProtocolResponse {
        reason: format!("runtime `{runtime_name}` exited before readiness was verified"),
    }
}

impl OwnedRuntime {
    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn replacement_required(&self) -> bool {
        self.replacement_required
    }

    pub fn applied_config_fingerprint(&self) -> Option<&str> {
        self.applied_config_fingerprint.as_deref()
    }

    pub(crate) fn has_applied_desired_config(&self) -> bool {
        self.desired_config_fingerprint
            .as_ref()
            .is_some_and(|desired| self.applied_config_fingerprint.as_ref() == Some(desired))
    }

    /// Whether the monitor still runs this runtime as recorded.
    fn matches_live(&self) -> Result<bool, DaemonError> {
        Ok(monitor::live_runtime(&self.paths, &self.subject)?
            .is_some_and(|live| live.instance == self.instance && live.runtime_pid == self.pid))
    }
}

impl AdoptedProcess {
    pub fn pid(&self) -> u32 {
        self.owned.pid()
    }

    pub(crate) fn into_owned(self) -> OwnedRuntime {
        self.owned
    }

    pub(crate) fn uses_current_artifact(&self, artifact_root: &Utf8Path) -> bool {
        !self.owned.replacement_required() && self.owned.command.starts_with(artifact_root)
    }

    pub(crate) fn has_applied_desired_config(&self) -> bool {
        self.owned.has_applied_desired_config()
    }

    /// Kills a still-matching test process group and synchronously verifies that no live member
    /// remains. Its monitor reaps the leader and keeps the exit until released.
    #[doc(hidden)]
    pub fn kill_and_wait_for_test(&self, timeout: Duration) -> Result<(), DaemonError> {
        require_process_containment()?;
        if !self.owned.matches_live()? {
            if !process_group_has_exited(self.owned.pid)? {
                return Err(io::Error::other(format!(
                    "process {} remained after its recorded identity stopped matching",
                    self.owned.pid
                ))
                .into());
            }
            return Ok(());
        }

        signal_process_group(self.owned.pid, ProcessSignal::Kill)?;
        let deadline = StdInstant::now() + timeout;
        loop {
            if process_group_has_exited(self.owned.pid)? {
                return Ok(());
            }
            if StdInstant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "process group {} did not exit and reap after kill signal",
                        self.owned.pid
                    ),
                )
                .into());
            }
            std::thread::sleep(READINESS_POLL_INTERVAL);
        }
    }

    pub async fn stop(self, grace_period: Duration) -> Result<(), DaemonError> {
        self.stop_with(StopSignal::Terminate, grace_period).await
    }

    pub async fn stop_with(
        self,
        signal: StopSignal,
        grace_period: Duration,
    ) -> Result<(), DaemonError> {
        require_process_containment()?;
        stop_monitored_runtime(
            &self.owned.paths,
            &self.owned.subject,
            &self.owned.instance,
            monitor_stop(signal, grace_period),
        )
        .await
    }
}

pub async fn wait_for_readiness(
    check: ReadinessCheck,
    readiness_timeout: Duration,
) -> Result<(), DaemonError> {
    let started_at = Instant::now();
    let mut last_error = None;

    while let Some(remaining) = remaining_timeout(started_at, readiness_timeout) {
        let probe_timeout = remaining.min(READINESS_PROBE_TIMEOUT);
        match timeout(probe_timeout, check_once(&check)).await {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(error)) => {
                last_error = Some(error.to_string());
                sleep(remaining.min(READINESS_POLL_INTERVAL)).await;
            }
            Err(elapsed) => {
                last_error = Some(elapsed.to_string());
            }
        }
    }

    Err(DaemonError::ReadinessTimedOut {
        check: check.name(),
        timeout_ms: readiness_timeout.as_millis(),
        last_error,
    })
}

pub(crate) async fn probe_readiness_once(check: &ReadinessCheck) -> Result<(), DaemonError> {
    check_once(check).await
}

pub async fn wait_for_custom_readiness<F, Fut>(
    name: &str,
    readiness_timeout: Duration,
    mut check: F,
) -> Result<(), DaemonError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let started_at = Instant::now();
    let mut last_error = None;

    while let Some(remaining) = remaining_timeout(started_at, readiness_timeout) {
        match timeout(remaining, check()).await {
            Ok(true) => return Ok(()),
            Ok(false) => {
                last_error = Some("custom readiness returned false".to_string());
                sleep(remaining.min(READINESS_POLL_INTERVAL)).await;
            }
            Err(elapsed) => {
                last_error = Some(elapsed.to_string());
                break;
            }
        }
    }

    Err(DaemonError::ReadinessTimedOut {
        check: format!("custom:{name}"),
        timeout_ms: readiness_timeout.as_millis(),
        last_error,
    })
}

fn remaining_timeout(started_at: Instant, readiness_timeout: Duration) -> Option<Duration> {
    readiness_timeout
        .checked_sub(started_at.elapsed())
        .filter(|remaining| !remaining.is_zero())
}

impl ReadinessCheck {
    fn name(&self) -> String {
        match self {
            Self::Tcp { host, port } => format!("tcp:{host}:{port}"),
            Self::GatewayHttps {
                http_host,
                http_port,
                https_host,
                https_port,
                server_name,
                ..
            } => {
                format!(
                    "gateway:https:{server_name}:{https_host}:{https_port};tcp:{http_host}:{http_port}"
                )
            }
            Self::GatewayIdentity {
                http_host,
                http_port,
                https_host,
                https_port,
                server_name,
                path,
                ..
            } => {
                format!(
                    "gateway-identity:https:{server_name}:{https_host}:{https_port}{path};http:{http_host}:{http_port}{path}"
                )
            }
            Self::RedisPing { host, port } => format!("redis-ping:{host}:{port}"),
            Self::Http { host, port, path } => format!("http:{host}:{port}{path}"),
        }
    }
}

async fn check_once(check: &ReadinessCheck) -> Result<(), DaemonError> {
    match check {
        ReadinessCheck::Tcp { host, port } => check_tcp_once(host, *port).await,
        ReadinessCheck::GatewayHttps {
            http_host,
            http_port,
            https_host,
            https_port,
            server_name,
            ca_certificate_path,
        } => {
            check_tcp_once(http_host, *http_port).await?;
            check_https_once(https_host, *https_port, server_name, ca_certificate_path).await
        }
        ReadinessCheck::GatewayIdentity {
            http_host,
            http_port,
            https_host,
            https_port,
            server_name,
            path,
            expected_body,
            ca_certificate_path,
        } => {
            let http_stream = TcpStream::connect((http_host.as_str(), *http_port)).await?;
            check_gateway_identity_response(http_stream, server_name, path, expected_body).await?;
            let tcp_stream = TcpStream::connect((https_host.as_str(), *https_port)).await?;
            let connector = TlsConnector::from(tls_client_config(ca_certificate_path)?);
            let server_name_text = server_name.to_owned();
            let tls_server_name =
                ServerName::try_from(server_name_text.clone()).map_err(|error| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("invalid TLS server name `{server_name_text}`: {error}"),
                    )
                })?;
            let tls_stream = connector.connect(tls_server_name, tcp_stream).await?;

            check_gateway_identity_response(tls_stream, server_name, path, expected_body).await
        }
        ReadinessCheck::RedisPing { host, port } => {
            let url = format!("redis://{host}:{port}/");
            let client = redis::Client::open(url)?;
            let mut connection = client.get_multiplexed_async_connection().await?;
            let pong: String = redis::cmd("PING").query_async(&mut connection).await?;
            if pong == "PONG" {
                return Ok(());
            }

            Err(DaemonError::DaemonRejected {
                message: format!("Redis PING returned {pong}"),
            })
        }
        ReadinessCheck::Http { host, port, path } => {
            let mut stream = TcpStream::connect((host.as_str(), *port)).await?;
            let request =
                format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
            stream.write_all(request.as_bytes()).await?;

            let mut response = [0_u8; 12];
            let bytes = stream.read(&mut response).await?;
            if http_status_is_success(&response, bytes) {
                return Ok(());
            }

            Err(io::Error::other("HTTP readiness returned non-success status").into())
        }
    }
}

async fn check_gateway_identity_response<Stream>(
    mut stream: Stream,
    server_name: &str,
    path: &str,
    expected_body: &str,
) -> Result<(), DaemonError>
where
    Stream: AsyncRead + AsyncWrite + Unpin,
{
    let request =
        format!("GET {path} HTTP/1.1\r\nHost: {server_name}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await?;

    let mut response = Vec::new();
    stream.take(4096).read_to_end(&mut response).await?;
    if http_response_has_success_body(&response, expected_body.as_bytes()) {
        return Ok(());
    }

    Err(io::Error::other("Gateway identity readiness returned an unexpected response").into())
}

fn http_response_has_success_body(response: &[u8], expected_body: &[u8]) -> bool {
    let Some(headers_end) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let body = &response[headers_end + 4..];

    http_status_is_success(response, response.len()) && body == expected_body
}

async fn check_tcp_once(host: &str, port: u16) -> Result<(), DaemonError> {
    let _stream = TcpStream::connect((host, port)).await?;

    Ok(())
}

async fn check_https_once(
    host: &str,
    port: u16,
    server_name: &str,
    ca_certificate_path: &Utf8Path,
) -> Result<(), DaemonError> {
    let tcp_stream = TcpStream::connect((host, port)).await?;
    let connector = TlsConnector::from(tls_client_config(ca_certificate_path)?);
    let server_name_text = server_name.to_owned();
    let server_name = ServerName::try_from(server_name_text.clone()).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid TLS server name `{server_name_text}`: {error}"),
        )
    })?;
    let _stream = connector.connect(server_name, tcp_stream).await?;

    Ok(())
}

fn http_status_is_success(response: &[u8], bytes: usize) -> bool {
    bytes >= 10 && (response.starts_with(b"HTTP/1.1 2") || response.starts_with(b"HTTP/1.0 2"))
}

fn tls_client_config(
    ca_certificate_path: &Utf8Path,
) -> Result<Arc<rustls::ClientConfig>, DaemonError> {
    let certificate_pem = fs::read_to_string(ca_certificate_path)?;
    let mut reader = certificate_pem.as_bytes();
    let certificates = rustls_pemfile::certs(&mut reader).collect::<Result<Vec<_>, _>>()?;
    let mut root_store = rustls::RootCertStore::empty();
    let (added, _ignored) = root_store.add_parsable_certificates(certificates);
    if added == 0 {
        return Err(io::Error::other(format!(
            "no CA certificates could be loaded from {ca_certificate_path}"
        ))
        .into());
    }

    Ok(Arc::new(
        rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|error| io::Error::other(format!("TLS protocol configuration failed: {error}")))?
        .with_root_certificates(root_store)
        .with_no_client_auth(),
    ))
}

fn require_process_containment() -> Result<(), DaemonError> {
    platform::require_capability(PlatformCapability::ProcessContainment)?;

    Ok(())
}

#[cfg(target_os = "macos")]
fn process_group_pid(pid: u32) -> Result<Pid, DaemonError> {
    let raw_pid =
        i32::try_from(pid).map_err(|source| io::Error::new(io::ErrorKind::InvalidInput, source))?;

    Pid::from_raw(raw_pid).ok_or_else(|| {
        DaemonError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "process id must be positive",
        ))
    })
}

#[cfg(target_os = "macos")]
fn signal_process_group(pid: u32, signal: ProcessSignal) -> Result<(), DaemonError> {
    let process_group = process_group_pid(pid)?;
    let signal = match signal {
        ProcessSignal::Stop(StopSignal::Terminate) => Signal::TERM,
        ProcessSignal::Stop(StopSignal::Interrupt) => Signal::INT,
        ProcessSignal::Kill => Signal::KILL,
    };

    match kill_process_group(process_group, signal) {
        Ok(()) => Ok(()),
        Err(source) => {
            let error = io::Error::from(source);
            // macOS answers EPERM, not ESRCH, when no member can take a signal: each is an
            // unreaped zombie or already exiting, as a killed runtime's children briefly are.
            // Every caller then waits for the group to end, which decides the stop.
            if process_not_found(&error) || error.kind() == io::ErrorKind::PermissionDenied {
                return Ok(());
            }

            Err(error.into())
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn signal_process_group(_pid: u32, _signal: ProcessSignal) -> Result<(), DaemonError> {
    require_process_containment()
}

/// Signals `pid`'s process group with `signal`, then SIGKILL once `grace_period` passes, and
/// waits until no live member remains. Escalates only while `leader_still_owned` proves the
/// group still belongs to the runtime.
pub(crate) async fn stop_process_group(
    pid: u32,
    signal: StopSignal,
    grace_period: Duration,
    leader_still_owned: impl Fn() -> Result<bool, DaemonError>,
) -> Result<(), DaemonError> {
    signal_process_group(pid, ProcessSignal::Stop(signal))?;

    if wait_for_process_group_exit(pid, grace_period).await? {
        return Ok(());
    }

    if !leader_still_owned()? {
        return Err(DaemonError::RuntimeCleanupUnproven {
            pid,
            reason: "runtime leader changed before escalation; group ownership is unproven"
                .to_owned(),
        });
    }
    signal_process_group(pid, ProcessSignal::Kill)?;

    if wait_for_process_group_exit(pid, Duration::from_secs(1)).await? {
        return Ok(());
    }

    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("process {pid} did not exit after signal"),
    )
    .into())
}

pub(crate) async fn wait_for_process_group_exit(
    pid: u32,
    readiness_timeout: Duration,
) -> Result<bool, DaemonError> {
    let started_at = Instant::now();

    while let Some(remaining) = remaining_timeout(started_at, readiness_timeout) {
        if process_group_has_exited(pid)? {
            return Ok(true);
        }

        sleep(remaining.min(READINESS_POLL_INTERVAL)).await;
    }

    process_group_has_exited(pid)
}

/// The monitor subject of the runtime recorded at `metadata_path`: the record's path under
/// `run/` without its extension, such as `gateway`, `workers/php-8.4` or `resources/redis/7`.
pub(crate) fn monitor_subject(
    paths: &PvPaths,
    metadata_path: &Utf8Path,
) -> Result<String, DaemonError> {
    metadata_path
        .strip_prefix(paths.run())
        .ok()
        .map(|relative| relative.with_extension(""))
        .filter(|subject| !subject.as_str().is_empty())
        .map(Into::into)
        .ok_or_else(|| DaemonError::InvalidRuntimeRecord {
            path: metadata_path.to_owned(),
        })
}

/// The resource a monitor subject runs: the Gateway's Caddy, a worker's FrankenPHP, or the
/// Managed Resource named in `resources/<name>/<track>`.
fn subject_resource_name(subject: &str) -> &str {
    if subject == "gateway" {
        "caddy"
    } else if subject.starts_with("workers/") {
        "frankenphp"
    } else {
        subject
            .strip_prefix("resources/")
            .and_then(|resource| resource.split('/').next())
            .unwrap_or(subject)
    }
}

/// How a runtime is stopped when nobody chooses otherwise: Postgres by its fast shutdown, and
/// the Gateway and workers with less grace than the data stores.
fn stop_policy(resource_name: &str) -> MonitorStop {
    MonitorStop {
        signal: if resource_name == "postgres" {
            StopSignal::Interrupt
        } else {
            StopSignal::Terminate
        },
        grace_ms: if matches!(resource_name, "caddy" | "frankenphp") {
            1_000
        } else {
            10_000
        },
    }
}

fn monitor_stop(signal: StopSignal, grace_period: Duration) -> MonitorStop {
    MonitorStop {
        signal,
        grace_ms: u64::try_from(grace_period.as_millis()).unwrap_or(u64::MAX),
    }
}

fn stop_grace(stop: MonitorStop) -> Duration {
    Duration::from_millis(stop.grace_ms)
}

/// Stops the runtime of `subject`'s monitor `instance`, logs how it exited, and releases the
/// monitor. A monitor whose record names another instance, or none, is not this runtime's and is
/// left alone; a monitor that died is recovered from its record.
async fn stop_monitored_runtime(
    paths: &PvPaths,
    subject: &str,
    instance: &str,
    stop: MonitorStop,
) -> Result<(), DaemonError> {
    match stop_monitor_instance(paths, subject, instance, stop).await? {
        MonitorStopped::Stopped(state) => {
            structured_log::runtime_exited(paths, subject, &exit_description(state.exit));
            monitor::release_monitor(paths, subject).await
        }
        MonitorStopped::Unavailable => monitor::recover_monitor(paths, subject).await,
        MonitorStopped::Gone => Ok(()),
    }
}

pub(crate) enum MonitorStopped {
    /// The runtime exited and its whole process group is proven gone; the monitor awaits release.
    Stopped(MonitorState),
    /// The monitor could not be reached; it may have died.
    Unavailable,
    /// The record no longer names this instance: it was released, or replaced.
    Gone,
}

/// Has `subject`'s monitor `instance` stop its runtime, waiting up to the grace period plus a
/// margin. Cleanup that the monitor cannot prove keeps the monitor and its records, and fails.
pub(crate) async fn stop_monitor_instance(
    paths: &PvPaths,
    subject: &str,
    instance: &str,
    stop: MonitorStop,
) -> Result<MonitorStopped, DaemonError> {
    if monitor::recorded_monitor_instance(paths, subject)?.as_deref() != Some(instance) {
        return Ok(MonitorStopped::Gone);
    }
    let state = match timeout(
        stop_grace(stop).saturating_add(MONITOR_STOP_MARGIN),
        monitor::stop_monitor(paths, subject, stop),
    )
    .await
    {
        Ok(Ok(state)) => state,
        Ok(Err(DaemonError::MonitorUnavailable { .. })) => return Ok(MonitorStopped::Unavailable),
        Ok(Err(error)) => return Err(error),
        Err(_elapsed) => {
            return Err(DaemonError::MonitorUnavailable {
                subject: subject.to_owned(),
                reason: "the monitor did not finish stopping its runtime".to_owned(),
            });
        }
    };
    if state.cleanup != MonitorCleanup::Complete {
        return Err(DaemonError::RuntimeCleanupUnproven {
            pid: state.runtime_pid,
            reason: format!(
                "PV could not prove that the runtime's processes exited, so it kept its monitor and records. {POSTGRES_RECOVERY}"
            ),
        });
    }

    Ok(MonitorStopped::Stopped(state))
}

pub(crate) fn exit_description(exit: Option<MonitorExit>) -> String {
    match exit {
        Some(MonitorExit::Code(code)) => format!("exit code {code}"),
        Some(MonitorExit::Signal(signal)) => format!("signal {signal}"),
        None => "unknown".to_owned(),
    }
}

/// Whether the process at `pid` is still the one a record from before monitors names.
fn legacy_process_matches(pid: u32, metadata: &RuntimeMetadata) -> Result<bool, DaemonError> {
    let Some(start_identity) = metadata.process_start_identity else {
        return Ok(false);
    };

    live_process_matches(
        pid,
        Utf8Path::new(&metadata.command),
        &metadata.arguments,
        start_identity,
        metadata.process_executable_identity.as_ref(),
    )
}

fn write_runtime_metadata(
    spec: &ProcessSpec,
    pid: u32,
    monitor_instance: &str,
) -> Result<(), DaemonError> {
    let started_at = timestamp()?;
    let metadata = RuntimeMetadata {
        name: spec.name.clone(),
        pid,
        monitor_instance: Some(monitor_instance.to_owned()),
        command: spec.command.to_string(),
        arguments: spec.arguments.clone(),
        private_environment_fingerprint: private_environment_fingerprint(&spec.private_environment),
        config_path: spec.config_path.to_string(),
        config_fingerprint: spec.config_fingerprint.clone(),
        resource_name: spec.resource_name.clone(),
        track: spec.track.clone(),
        replacement_required: false,
        applied_config_fingerprint: None,
        desired_config_fingerprint: None,
        staged_config_fingerprint: None,
        log_path: spec.log_path.to_string(),
        started_at,
        boot_session_id: None,
        process_start_identity: None,
        process_executable_identity: None,
    };
    let encoded = serde_json::to_string(&metadata)?;

    fs::write_sensitive_file(&spec.metadata_path, &encoded)?;

    Ok(())
}

fn remove_unchanged_runtime_records(
    pid_path: &Utf8Path,
    metadata_path: &Utf8Path,
    metadata: &RuntimeMetadata,
    keep_metadata: bool,
) -> Result<(), DaemonError> {
    // Admission is closed and the daemon has stopped. Still compare both records
    // before removal so another instance's evidence cannot be removed by this stop.
    if read_pid_file(pid_path)? != Some(metadata.pid)
        || read_runtime_metadata(metadata_path)?.is_none_or(|current| current != *metadata)
    {
        return Err(DaemonError::RuntimeProcessIdentityChanged { pid: metadata.pid });
    }
    fs::remove_file_if_exists(pid_path)?;
    if !keep_metadata {
        fs::remove_file_if_exists(metadata_path)?;
    }
    Ok(())
}

fn read_pid_file(path: &Utf8Path) -> Result<Option<u32>, DaemonError> {
    let Some(content) = read_optional_file(path)? else {
        return Ok(None);
    };
    let pid = content
        .trim()
        .parse::<u32>()
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;

    Ok(Some(pid))
}

fn read_runtime_metadata(path: &Utf8Path) -> Result<Option<RuntimeMetadata>, DaemonError> {
    let Some(content) = read_optional_file(path)? else {
        return Ok(None);
    };

    Ok(Some(serde_json::from_str(&content)?))
}

fn read_optional_file(path: &Utf8Path) -> Result<Option<String>, DaemonError> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(target_os = "macos")]
fn process_group_has_exited(pid: u32) -> Result<bool, DaemonError> {
    Ok(!platform::process_group_has_live_members(pid)?)
}

#[cfg(target_os = "macos")]
fn process_and_group_are_absent(pid: u32) -> Result<bool, DaemonError> {
    let process = process_group_pid(pid)?;
    let process_absent = match test_kill_process(process) {
        Err(rustix::io::Errno::SRCH) => true,
        Ok(()) | Err(rustix::io::Errno::PERM) => false,
        Err(source) => return Err(io::Error::from(source).into()),
    };
    let group_absent = match test_kill_process_group(process) {
        Err(rustix::io::Errno::SRCH) => true,
        Ok(()) | Err(rustix::io::Errno::PERM) => false,
        Err(source) => return Err(io::Error::from(source).into()),
    };

    Ok(process_absent && group_absent)
}

#[cfg(target_os = "macos")]
pub(crate) fn reap_process_if_child(pid: u32) -> Result<(), DaemonError> {
    let process = process_group_pid(pid)?;
    match waitpid(Some(process), WaitOptions::NOHANG) {
        Ok(_) | Err(rustix::io::Errno::CHILD) => Ok(()),
        Err(error) => Err(io::Error::from(error).into()),
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn process_group_has_exited(_pid: u32) -> Result<bool, DaemonError> {
    require_process_containment()?;

    Ok(true)
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn process_and_group_are_absent(_pid: u32) -> Result<bool, DaemonError> {
    require_process_containment()?;

    Ok(false)
}

fn live_process_matches(
    pid: u32,
    command: &Utf8Path,
    arguments: &[String],
    process_start_identity: platform::ProcessStartIdentity,
    process_executable_identity: Option<&ProcessExecutableIdentity>,
) -> Result<bool, DaemonError> {
    let Some(process_identity) = platform::inspect_process_identity(pid)? else {
        return Ok(false);
    };
    if process_identity.start_identity != process_start_identity {
        return Ok(false);
    }

    Ok(process_identity_matches(
        &process_identity,
        command,
        arguments,
        process_executable_identity,
    ))
}

fn process_identity_matches(
    process_identity: &platform::ProcessIdentity,
    command: &Utf8Path,
    arguments: &[String],
    process_executable_identity: Option<&ProcessExecutableIdentity>,
) -> bool {
    let direct_arguments_match = process_identity.arguments == arguments;
    let direct_command_matches = executable_matches(process_identity, command);
    let script_arguments_match = script_arguments_match(process_identity, command, arguments);
    let script_identity_matches = script_arguments_match
        && fs::read_to_string(command).is_ok_and(|source| source.starts_with("#!"))
        && process_executable_identity.is_some_and(|expected| {
            expected.executable == process_identity.executable.as_str()
                && expected.argument_zero == process_identity.argument_zero
        });

    (direct_command_matches && direct_arguments_match) || script_identity_matches
}

fn script_arguments_match(
    process_identity: &platform::ProcessIdentity,
    command: &Utf8Path,
    arguments: &[String],
) -> bool {
    process_identity
        .arguments
        .split_first()
        .is_some_and(|(script, live_arguments)| {
            script == command.as_str() && live_arguments == arguments
        })
}

fn executable_matches(process_identity: &platform::ProcessIdentity, command: &Utf8Path) -> bool {
    process_identity.executable == command
        || (command == Utf8Path::new("/bin/sh")
            && process_identity.executable == Utf8Path::new("/bin/bash")
            && process_identity.argument_zero == command.as_str())
}

#[cfg(target_os = "macos")]
fn process_not_found(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound || error.raw_os_error() == Some(3)
}

impl RuntimeMetadata {
    fn recorded_config_fingerprint(&self) -> Option<RecordedConfigFingerprint> {
        match (
            self.replacement_required,
            self.applied_config_fingerprint.as_deref(),
            self.staged_config_fingerprint.as_deref(),
        ) {
            (false, Some(fingerprint), None) => {
                Some(RecordedConfigFingerprint::Applied(fingerprint.to_owned()))
            }
            (true, None, Some(fingerprint)) => {
                Some(RecordedConfigFingerprint::Staged(fingerprint.to_owned()))
            }
            _ => None,
        }
    }

    fn matches(&self, spec: &ProcessSpec) -> bool {
        self.name == spec.name
            && self.command == spec.command.as_str()
            && self.arguments == spec.arguments
            && self.config_path == spec.config_path.as_str()
            && self.config_fingerprint == spec.config_fingerprint
            && self.resource_name == spec.resource_name
            && self.track == spec.track
            && self.log_path == spec.log_path.as_str()
            && self.private_environment_fingerprint.as_deref()
                == private_environment_fingerprint(&spec.private_environment).as_deref()
    }
}

fn is_false(value: &bool) -> bool {
    !value
}

fn private_environment_fingerprint(environment: &BTreeMap<String, String>) -> Option<String> {
    if environment.is_empty() {
        return None;
    }

    let mut hasher = Sha256::new();
    for (key, value) in environment {
        hasher.update((key.len() as u64).to_be_bytes());
        hasher.update(key.as_bytes());
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }

    Some(format!(
        "{PRIVATE_ENVIRONMENT_FINGERPRINT_PREFIX}{:x}",
        hasher.finalize()
    ))
}

fn timestamp() -> Result<String, DaemonError> {
    let format =
        time::macros::format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");

    Ok(time::OffsetDateTime::now_utc().format(format)?)
}

#[cfg(test)]
mod bounded_readiness_tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

    use anyhow::{Result, anyhow};
    use tokio::sync::Semaphore;
    use tokio::time::timeout;

    use super::bounded_runtime_readiness;
    use futures_util::StreamExt;

    #[tokio::test]
    async fn readiness_waits_overlap_at_the_fixed_bound() -> Result<()> {
        let started = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let maximum_active = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(Semaphore::new(0));
        let task = tokio::spawn(
            bounded_runtime_readiness(0..8, {
                let started = Arc::clone(&started);
                let active = Arc::clone(&active);
                let maximum_active = Arc::clone(&maximum_active);
                let gate = Arc::clone(&gate);

                move |item| {
                    let started = Arc::clone(&started);
                    let active = Arc::clone(&active);
                    let maximum_active = Arc::clone(&maximum_active);
                    let gate = Arc::clone(&gate);

                    async move {
                        started.fetch_add(1, Ordering::SeqCst);
                        let active_now = active.fetch_add(1, Ordering::SeqCst) + 1;
                        maximum_active.fetch_max(active_now, Ordering::SeqCst);
                        let permit = gate
                            .acquire_owned()
                            .await
                            .map_err(|error| anyhow!("readiness gate closed: {error}"))?;
                        permit.forget();
                        active.fetch_sub(1, Ordering::SeqCst);

                        Ok::<_, anyhow::Error>(item)
                    }
                }
            })
            .collect::<Vec<_>>(),
        );

        timeout(Duration::from_secs(1), async {
            while started.load(Ordering::SeqCst) < 4 {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        for _attempt in 0..10 {
            tokio::task::yield_now().await;
        }
        assert_eq!(started.load(Ordering::SeqCst), 4);
        assert_eq!(active.load(Ordering::SeqCst), 4);
        assert_eq!(maximum_active.load(Ordering::SeqCst), 4);

        gate.add_permits(8);
        let outcomes = timeout(Duration::from_secs(1), task)
            .await??
            .into_iter()
            .collect::<Result<Vec<_>>>()?;

        assert_eq!(outcomes.len(), 8);
        assert_eq!(maximum_active.load(Ordering::SeqCst), 4);

        Ok(())
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::net::TcpListener;
    use std::time::Duration;

    use anyhow::{Context, Result, anyhow, bail};
    use camino::{Utf8Path, Utf8PathBuf};
    use camino_tempfile::tempdir;
    use rustix::process::{Pid, Signal, kill_process_group, test_kill_process};
    use tokio::time::sleep;

    use super::{
        ProcessSpec, ProcessSupervisor, RecordedConfigFingerprint, monitor_subject,
        process_group_has_exited, read_runtime_metadata, remove_unchanged_runtime_records,
    };
    use crate::StopSignal;
    use crate::monitor::{self, MonitorStart, MonitorStop};
    use state::PvPaths;

    /// Keeps a shell alive until it is stopped, but for 150 s at most, so a test that dies before
    /// stopping it leaves nothing running for long. That outlasts the CI profile's 120 s limit on
    /// a test, so a stalled test can't pass because its fixture exited on its own.
    const IDLE_SHELL_LOOP: &str = "i=0; while [ $i -lt 150 ]; do sleep 1; i=$((i + 1)); done";

    #[test]
    fn shutdown_cleanup_preserves_runtime_records_changed_after_inspection() -> Result<()> {
        for change_pid in [false, true] {
            let tempdir = tempdir()?;
            let pid_path = tempdir.path().join("runtime.pid");
            let metadata_path = tempdir.path().join("runtime.json");
            let mut metadata = serde_json::json!({
                "name": "postgres", "pid": 42, "command": "/owned/postgres",
                "arguments": [], "log_path": "/owned/postgres.log", "started_at": "unused",
            });
            state::fs::write_sensitive_file(&pid_path, "42\n")?;
            state::fs::write_sensitive_file(&metadata_path, &serde_json::to_string(&metadata)?)?;
            let inspected = read_runtime_metadata(&metadata_path)?
                .ok_or_else(|| anyhow!("runtime metadata was missing"))?;
            let current_pid = if change_pid { "43\n" } else { "42\n" };
            metadata["name"] = serde_json::json!("replacement");
            state::fs::write_sensitive_file(&pid_path, current_pid)?;
            if !change_pid {
                state::fs::write_sensitive_file(
                    &metadata_path,
                    &serde_json::to_string(&metadata)?,
                )?;
            }
            let before = state::fs::read_to_string(&metadata_path)?;
            assert!(matches!(
                remove_unchanged_runtime_records(&pid_path, &metadata_path, &inspected, false),
                Err(crate::DaemonError::RuntimeProcessIdentityChanged { pid: 42 }),
            ));
            assert_eq!(state::fs::read_to_string(&pid_path)?, current_pid);
            assert_eq!(state::fs::read_to_string(&metadata_path)?, before);
        }
        Ok(())
    }

    #[tokio::test]
    async fn start_replaces_a_monitor_left_without_a_record() -> Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        state::fs::ensure_layout(&paths)?;
        pv_fake::install_monitor(&paths)?;
        let spec = descendant_spec(
            &paths,
            "orphaned",
            "/bin/sh".into(),
            vec!["-c".to_owned(), IDLE_SHELL_LOOP.to_owned()],
        );
        // As a start cancelled after its monitor began, before the runtime's record was written.
        let orphan = monitor::start_monitor(
            &paths,
            &paths.active_pv_binary(),
            MonitorStart {
                subject: monitor_subject(&paths, &spec.metadata_path)?,
                command: spec.command.clone(),
                arguments: spec.arguments.clone(),
                private_environment: spec.private_environment.clone(),
                log_path: spec.log_path.clone(),
                fallback_stop: MonitorStop {
                    signal: StopSignal::Terminate,
                    grace_ms: 1_000,
                },
                lifeline_fd: None,
            },
        )
        .await?;
        let supervisor = ProcessSupervisor::new(paths.clone());

        let process = supervisor.start(spec.clone()).await?;
        let orphan_exited = wait_for_test_process_exit(orphan.runtime_pid).await?;
        let owned = supervisor.verify_ownership(&spec)?;
        let replacement_pid = process.pid();
        process.stop(Duration::from_secs(5)).await?;

        assert!(orphan_exited);
        assert_ne!(replacement_pid, orphan.runtime_pid);
        assert_eq!(owned.map(|owned| owned.pid()), Some(replacement_pid));

        Ok(())
    }

    #[tokio::test]
    async fn recorded_fingerprint_accepts_prior_proof_only_while_process_is_live() -> Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        state::fs::ensure_layout(&paths)?;
        pv_fake::install_monitor(&paths)?;
        let supervisor = ProcessSupervisor::new(paths.clone());
        let spec = descendant_spec(
            &paths,
            "prior-proof",
            "/bin/sleep".into(),
            vec!["60".to_string()],
        );
        let process = supervisor.start(spec.clone()).await?;
        assert!(supervisor.record_applied_config(&spec, "sha256:v1:applied")?);

        // The installed artifact changed: strict proof against the new spec fails, but the
        // live prior recording still proves the old applied fingerprint.
        let mut changed_spec = spec.clone();
        changed_spec.command = "/bin/changed-artifact".into();
        assert_eq!(
            supervisor.recorded_config_fingerprint(&changed_spec)?,
            Some(RecordedConfigFingerprint::Applied(
                "sha256:v1:applied".to_owned()
            ))
        );

        // Once the recorded process is dead, prior proof is refused even though the
        // record is intact.
        process.stop(Duration::from_secs(5)).await?;
        assert!(spec.metadata_path.exists());
        assert!(
            supervisor
                .recorded_config_fingerprint(&changed_spec)?
                .is_none()
        );

        Ok(())
    }

    #[test]
    fn adopted_process_kill_waits_for_group_and_listener_exit_without_runtime() -> Result<()> {
        let tempdir = tempdir().context("create temp directory")?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        state::fs::ensure_layout(&paths).context("create test layout")?;
        pv_fake::install_monitor(&paths)?;
        let descendant_pid_path = paths.run().join("adopted-kill-descendant.pid");
        let listener_ready_path = paths.run().join("adopted-kill-listener.ready");
        let listener = TcpListener::bind(("127.0.0.1", 0)).context("reserve listener port")?;
        let listener_port = listener.local_addr().context("read listener port")?.port();
        drop(listener);

        let spec = descendant_spec(
            &paths,
            "adopted-kill",
            "/bin/sh".into(),
            listener_descendant_arguments(
                listener_port,
                &listener_ready_path,
                &descendant_pid_path,
            ),
        );
        let pid_path = spec.pid_path.clone();
        let metadata_path = spec.metadata_path.clone();
        let supervisor = ProcessSupervisor::new(paths);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("build startup runtime")?;
        let process = runtime
            .block_on(supervisor.start(spec))
            .context("start recorded process")?;
        let leader_pid = process.pid();
        runtime.block_on(wait_for_test_path(&descendant_pid_path));
        runtime.block_on(wait_for_test_path(&listener_ready_path));
        let descendant_pid = state::fs::read_to_string(&descendant_pid_path)
            .context("read descendant pid")?
            .trim()
            .parse::<u32>()?;
        drop(process);
        drop(runtime);

        let cleanup_result: Result<()> = (|| {
            let process = supervisor
                .adopt_recorded(&pid_path, &metadata_path)?
                .ok_or_else(|| anyhow!("recorded process was not adoptable"))?;
            process
                .kill_and_wait_for_test(Duration::from_secs(1))
                .context("kill and wait for adopted process")?;
            Ok(())
        })();
        if cleanup_result.is_err() {
            let _kill_result = kill_process_group(test_pid(leader_pid)?, Signal::KILL);
            let _group_exited = wait_for_test_process_group_exit_synchronously(leader_pid)?;
        }
        cleanup_result?;

        // The monitor keeps the leader unreaped until it is released.
        assert!(process_group_has_exited(leader_pid)?);
        assert!(wait_for_test_process_exit_synchronously(descendant_pid)?);
        assert!(wait_for_test_listener_release(listener_port));

        Ok(())
    }

    fn descendant_spec(
        paths: &PvPaths,
        name: &str,
        command: Utf8PathBuf,
        arguments: Vec<String>,
    ) -> ProcessSpec {
        ProcessSpec {
            name: name.to_string(),
            command,
            arguments,
            private_environment: Default::default(),
            config_path: paths.config().join(format!("{name}.json")),
            config_fingerprint: None,
            log_path: paths.logs().join(format!("{name}.log")),
            pid_path: paths.run().join(format!("{name}.pid")),
            metadata_path: paths.run().join(format!("{name}-metadata.json")),
            resource_name: name.to_string(),
            track: "test".to_string(),
        }
    }

    fn listener_descendant_arguments(
        port: u16,
        listener_ready_path: &Utf8Path,
        descendant_pid_path: &Utf8Path,
    ) -> Vec<String> {
        vec![
            "-c".to_string(),
            format!(
                "python3 -c 'import os, pathlib, socket, sys, time; pathlib.Path(sys.argv[3]).write_text(str(os.getpid()) + \"\\n\"); listener = socket.socket(); listener.bind((\"127.0.0.1\", int(sys.argv[1]))); listener.listen(); open(sys.argv[2], \"w\").close(); time.sleep(60)' \"$1\" \"$2\" \"$3\" & {IDLE_SHELL_LOOP}"
            ),
            "runtime-teardown".to_string(),
            port.to_string(),
            listener_ready_path.to_string(),
            descendant_pid_path.to_string(),
        ]
    }

    async fn wait_for_test_path(path: &Utf8Path) {
        for _attempt in 0..50 {
            if path.exists() {
                return;
            }

            sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_test_process_exit(pid: u32) -> Result<bool> {
        let pid = test_pid(pid)?;
        for _attempt in 0..50 {
            match test_kill_process(pid) {
                Err(rustix::io::Errno::SRCH) => return Ok(true),
                Ok(()) => {}
                Err(error) => bail!("failed to inspect process {pid}: {error}"),
            }

            sleep(Duration::from_millis(20)).await;
        }

        Ok(false)
    }

    fn wait_for_test_process_group_exit_synchronously(pid: u32) -> Result<bool> {
        for _attempt in 0..100 {
            if process_group_has_exited(pid)? {
                return Ok(true);
            }

            std::thread::sleep(Duration::from_millis(10));
        }

        Ok(false)
    }

    fn wait_for_test_process_exit_synchronously(pid: u32) -> Result<bool> {
        let pid = test_pid(pid)?;
        for _attempt in 0..100 {
            match test_kill_process(pid) {
                Err(rustix::io::Errno::SRCH) => return Ok(true),
                Ok(()) => {}
                Err(error) => bail!("failed to inspect process {pid}: {error}"),
            }

            std::thread::sleep(Duration::from_millis(10));
        }

        Ok(false)
    }

    fn wait_for_test_listener_release(port: u16) -> bool {
        for _attempt in 0..100 {
            if TcpListener::bind(("127.0.0.1", port)).is_ok() {
                return true;
            }

            std::thread::sleep(Duration::from_millis(10));
        }

        false
    }

    fn test_pid(pid: u32) -> Result<Pid> {
        let raw_pid = i32::try_from(pid)?;

        Pid::from_raw(raw_pid).ok_or_else(|| anyhow!("invalid process id {raw_pid}"))
    }
}
