//! Per-runtime monitors.
//!
//! A monitor is a small `pv monitor:run` process that owns one runtime for the runtime's whole
//! life, whether or not the daemon is running. It starts the runtime through `pv monitor:gate`,
//! which execs the runtime only once the monitor has recorded it, so the runtime keeps the gate's
//! pid and birth identity. The monitor learns how the runtime exited from a kqueue watch without
//! reaping it, so the exited leader keeps its process group pinned until the group's cleanup is
//! proven. Controllers talk to the monitor over a private socket, and the monitor's reservation
//! lock proves when it has exited.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod rotation;

use std::collections::BTreeMap;
use std::fmt;
#[cfg(target_os = "macos")]
use std::time::Duration;

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

use crate::StopSignal;
use crate::supervisor::PrivateEnvironmentDebug;

#[cfg(target_os = "macos")]
pub use macos::{
    live_runtime, monitor_state, recorded_monitor_instance, recorded_monitor_subjects,
    recover_monitor, release_monitor, run_monitor_blocking, run_monitor_gate, start_monitor,
    stop_monitor,
};
#[cfg(not(target_os = "macos"))]
pub use unsupported::{
    live_runtime, monitor_state, recorded_monitor_instance, recorded_monitor_subjects,
    recover_monitor, release_monitor, run_monitor_blocking, run_monitor_gate, start_monitor,
    stop_monitor,
};

/// A monitor's runtime that is still running; see [`live_runtime`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveRuntime {
    pub instance: String,
    pub runtime_pid: u32,
}

/// The hidden command that runs a monitor.
pub const MONITOR_RUN_COMMAND: &str = "monitor:run";
/// The hidden command a monitor starts its runtime through.
pub const MONITOR_GATE_COMMAND: &str = "monitor:gate";

/// What a controller asks a new monitor to run.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct MonitorStart {
    /// Names the runtime, such as `gateway`; one monitor runs per subject.
    pub subject: String,
    pub command: Utf8PathBuf,
    pub arguments: Vec<String>,
    /// Reaches the runtime only through its environment; never recorded.
    pub private_environment: BTreeMap<String, String>,
    /// Where the runtime's stdout and stderr go.
    pub log_path: Utf8PathBuf,
    /// How the monitor stops what is left of the runtime's process group when no controller asks:
    /// after the runtime exits on its own, and once the lifeline closes.
    pub fallback_stop: MonitorStop,
    /// An inherited pipe read end. Once every write end closes, the monitor stops the runtime
    /// and exits. Tests pass their lifeline; production passes `None`.
    pub lifeline_fd: Option<i32>,
}

impl fmt::Debug for MonitorStart {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MonitorStart")
            .field("subject", &self.subject)
            .field("command", &self.command)
            .field("arguments", &self.arguments)
            .field(
                "private_environment",
                &PrivateEnvironmentDebug(&self.private_environment),
            )
            .field("log_path", &self.log_path)
            .field("fallback_stop", &self.fallback_stop)
            .field("lifeline_fd", &self.lifeline_fd)
            .finish()
    }
}

/// A graceful signal for the runtime's process group, then SIGKILL once `grace_ms` passes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MonitorStop {
    pub signal: StopSignal,
    pub grace_ms: u64,
}

#[cfg(target_os = "macos")]
impl MonitorStop {
    fn grace(self) -> Duration {
        Duration::from_millis(self.grace_ms)
    }
}

/// What a monitor knows about its runtime.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MonitorState {
    pub subject: String,
    pub monitor_pid: u32,
    pub runtime_pid: u32,
    /// The first stop a controller requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<MonitorStop>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<MonitorExit>,
    pub cleanup: MonitorCleanup,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_rotation_error: Option<String>,
}

/// How the runtime exited.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorExit {
    Code(i32),
    Signal(i32),
}

/// Whether the runtime's whole process group is proven gone.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorCleanup {
    /// The runtime is running, or its group is being stopped.
    Pending,
    /// No live member remains and the runtime leader is reaped.
    Complete,
    /// The group could not be proven empty; the monitor keeps its records.
    Unproven,
}

/// Why a monitor refused a request.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorErrorKind {
    UnsupportedVersion,
    InstanceMismatch,
    NotReleasable,
    InvalidRequest,
    StartupFailed,
}

impl fmt::Display for MonitorErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedVersion => "unsupported_version",
            Self::InstanceMismatch => "instance_mismatch",
            Self::NotReleasable => "not_releasable",
            Self::InvalidRequest => "invalid_request",
            Self::StartupFailed => "startup_failed",
        })
    }
}

/// Test-only behavior for the `pv-monitor` example. Production `pv` uses the defaults.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct MonitorHooks {
    /// Where a paused point writes `<point>.reached`, holding its pid, then waits for
    /// `<point>.continue`.
    pub pause_dir: Option<Utf8PathBuf>,
    pub pauses: Vec<MonitorPause>,
    /// Makes the final process-group check report an unknown outcome.
    pub unknown_group_check: bool,
    pub rotation_bytes: Option<u64>,
    pub rotation_interval_ms: Option<u64>,
    /// The test process's lifeline, for monitors whose controller passes none, such as those a
    /// daemon running inside a test starts.
    pub lifeline_fd: Option<i32>,
}

/// A point where the monitor or its gate can be held for a test.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorPause {
    /// Before the monitor writes its recovery record.
    BeforePublication,
    /// After the record, before the gate may exec the runtime.
    BeforePermission,
    /// In the gate, after permission and before exec.
    GateAfterPermission,
    /// During a log rotation, before the copy.
    RotationCopy,
}

#[cfg(target_os = "macos")]
impl MonitorPause {
    const fn name(self) -> &'static str {
        match self {
            Self::BeforePublication => "before_publication",
            Self::BeforePermission => "before_permission",
            Self::GateAfterPermission => "gate_after_permission",
            Self::RotationCopy => "rotation_copy",
        }
    }
}

/// Monitors need process containment, which only macOS supports yet. Every entry point fails
/// before any side effect.
#[cfg(not(target_os = "macos"))]
mod unsupported {
    use camino::Utf8Path;
    use platform::PlatformCapability;
    use state::PvPaths;

    use super::{LiveRuntime, MonitorHooks, MonitorStart, MonitorState, MonitorStop};
    use crate::DaemonError;

    fn require_process_containment() -> Result<(), DaemonError> {
        platform::require_capability(PlatformCapability::ProcessContainment)?;

        Ok(())
    }

    fn unavailable(subject: &str) -> DaemonError {
        DaemonError::MonitorUnavailable {
            subject: subject.to_owned(),
            reason: "monitors are not supported on this platform".to_owned(),
        }
    }

    pub fn run_monitor_blocking(_paths: PvPaths, _hooks: MonitorHooks) -> Result<(), DaemonError> {
        require_process_containment()
    }

    pub fn run_monitor_gate(_hooks: &MonitorHooks) -> Result<(), DaemonError> {
        require_process_containment()
    }

    pub async fn start_monitor(
        _paths: &PvPaths,
        _executable: &Utf8Path,
        start: MonitorStart,
    ) -> Result<MonitorState, DaemonError> {
        require_process_containment()?;

        Err(unavailable(&start.subject))
    }

    pub async fn monitor_state(
        _paths: &PvPaths,
        subject: &str,
    ) -> Result<MonitorState, DaemonError> {
        require_process_containment()?;

        Err(unavailable(subject))
    }

    pub async fn stop_monitor(
        _paths: &PvPaths,
        subject: &str,
        _stop: MonitorStop,
    ) -> Result<MonitorState, DaemonError> {
        require_process_containment()?;

        Err(unavailable(subject))
    }

    pub async fn release_monitor(_paths: &PvPaths, _subject: &str) -> Result<(), DaemonError> {
        require_process_containment()
    }

    pub async fn recover_monitor(_paths: &PvPaths, _subject: &str) -> Result<(), DaemonError> {
        require_process_containment()
    }

    pub fn live_runtime(
        _paths: &PvPaths,
        _subject: &str,
    ) -> Result<Option<LiveRuntime>, DaemonError> {
        require_process_containment()?;

        Ok(None)
    }

    pub fn recorded_monitor_instance(
        _paths: &PvPaths,
        _subject: &str,
    ) -> Result<Option<String>, DaemonError> {
        require_process_containment()?;

        Ok(None)
    }

    pub fn recorded_monitor_subjects(_paths: &PvPaths) -> Result<Vec<String>, DaemonError> {
        require_process_containment()?;

        Ok(Vec::new())
    }
}
