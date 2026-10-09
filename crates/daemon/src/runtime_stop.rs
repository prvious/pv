use std::collections::BTreeSet;
use std::io;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use state::{PvPaths, fs};

use crate::{DaemonError, ProcessSupervisor, RuntimeReconciliationFailure};

/// Captured process identity to wait on after unloading the LaunchAgent.
#[derive(Debug, Deserialize, PartialEq, Serialize)]
pub struct DaemonProcess {
    pid: u32,
    start_identity: platform::ProcessStartIdentity,
}

impl DaemonProcess {
    pub fn wait_for_exit(&self, grace: Duration) -> Result<(), DaemonError> {
        let deadline = Instant::now() + grace;
        while platform::inspect_process_start_identity(self.pid)? == Some(self.start_identity)
            && !platform::process_is_zombie(self.pid)?
        {
            if Instant::now() >= deadline {
                return Err(DaemonError::RuntimeCleanupUnproven {
                    pid: self.pid,
                    reason: "daemon is still running after LaunchAgent unload".to_owned(),
                });
            }
            thread::sleep(Duration::from_millis(25));
        }
        Ok(())
    }
}

pub(crate) fn record_daemon_process(paths: &PvPaths) -> Result<DaemonProcess, DaemonError> {
    // Shared admission permits concurrent bootstraps. Listener acquisition
    // serializes publication, so repeat the early check after acquiring it.
    require_previous_daemon_exited(paths)?;
    let pid = std::process::id();
    let start_identity = platform::inspect_process_start_identity(pid)?.ok_or_else(|| {
        DaemonError::MissingProcessIdentity {
            name: "daemon".to_owned(),
            pid,
        }
    })?;
    let record = DaemonProcess {
        pid,
        start_identity,
    };
    fs::write_sensitive_file(
        &paths.daemon_process_record(),
        &serde_json::to_string(&record)?,
    )?;
    Ok(record)
}

fn read_daemon_process_record(paths: &PvPaths) -> Result<Option<DaemonProcess>, DaemonError> {
    let path = paths.daemon_process_record();
    if fs::path_entry_exists(&path)? && !fs::path_is_file(&path)? {
        return Err(DaemonError::InvalidRuntimeRecord { path });
    }
    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content)
            .map(Some)
            .map_err(|source| DaemonError::InvalidDaemonProcessRecord { path, source }),
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn require_previous_daemon_exited(paths: &PvPaths) -> Result<(), DaemonError> {
    if let Some(record) = read_daemon_process_record(paths)?
        && platform::inspect_process_start_identity(record.pid)? == Some(record.start_identity)
        && !platform::process_is_zombie(record.pid)?
    {
        return Err(DaemonError::RuntimeCleanupUnproven {
            pid: record.pid,
            reason: "previous daemon is still running; its process record must remain available"
                .to_owned(),
        });
    }
    Ok(())
}

/// Captures the daemon before bootout, including a daemon whose listener has closed.
pub fn daemon_process_for_stop(paths: &PvPaths) -> Result<Option<DaemonProcess>, DaemonError> {
    platform::require_capability(platform::PlatformCapability::DaemonIpc)?;
    if let Some(record) = read_daemon_process_record(paths)?
        && platform::inspect_process_start_identity(record.pid)? == Some(record.start_identity)
    {
        if verify_daemon_process(paths, record.pid)?.start_identity != record.start_identity {
            return Err(DaemonError::RuntimeProcessIdentityChanged { pid: record.pid });
        }
        return Ok(Some(record));
    }
    #[cfg(target_os = "macos")]
    return super::build_runtime()?.block_on(async {
        match tokio::time::timeout(Duration::from_secs(3), crate::ipc::connect(paths)).await {
            Ok(Ok(stream)) => {
                let peer_pid = stream
                    .peer_cred()?
                    .pid()
                    .and_then(|pid| u32::try_from(pid).ok())
                    .ok_or_else(|| DaemonError::InvalidRuntimeRecord {
                        path: paths.daemon_socket(),
                    })?;
                let identity = verify_daemon_process(paths, peer_pid)?;
                Ok(Some(DaemonProcess {
                    pid: peer_pid,
                    start_identity: identity.start_identity,
                }))
            }
            Ok(Err(DaemonError::Io(error)))
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) =>
            {
                Ok(None)
            }
            Ok(Err(error)) => Err(error),
            Err(error) => Err(io::Error::new(io::ErrorKind::TimedOut, error).into()),
        }
    });
    #[cfg(not(target_os = "macos"))]
    Ok(None)
}

fn verify_daemon_process(
    paths: &PvPaths,
    pid: u32,
) -> Result<platform::ProcessIdentity, DaemonError> {
    let identity = platform::inspect_process_identity(pid)?
        .ok_or(DaemonError::RuntimeProcessIdentityChanged { pid })?;
    if identity.arguments != ["daemon:run"] || !identity.executable.starts_with(paths.bin()) {
        return Err(DaemonError::RuntimeProcessIdentityChanged { pid });
    }
    Ok(identity)
}

/// Stops recorded runtimes without opening the database or loading artifacts.
/// The caller must first close lifecycle admission and verify daemon termination.
pub fn stop_recorded_runtimes_blocking(paths: PvPaths) -> Result<(), DaemonError> {
    super::build_runtime()?.block_on(stop_recorded_runtimes(paths))
}

/// Stops exact recorded runtimes without opening the database.
/// The caller must hold exclusive lifecycle admission and verify daemon termination.
pub async fn stop_recorded_runtimes(paths: PvPaths) -> Result<(), DaemonError> {
    platform::require_capability(platform::PlatformCapability::ProcessContainment)?;
    if !fs::path_entry_exists(paths.run())? {
        return Ok(());
    }
    require_directory(paths.run())?;
    let _jobs_lock = state::JobsLock::acquire(&paths)?;
    let mut records = BTreeSet::new();
    for path in [paths.gateway_pid(), paths.gateway_runtime_metadata()] {
        if fs::path_entry_exists(&path)? {
            records.insert(path.with_extension("pid"));
        }
    }
    collect_records(&paths.run().join("workers"), &mut records)?;
    let resources = paths.run().join("resources");
    if fs::path_entry_exists(&resources)? {
        require_directory(&resources)?;
    }
    for resource in fs::read_dir_paths(&resources)? {
        // MySQL sockets are ancillary entries, not runtime record directories.
        if resource.extension() == Some("sock") {
            continue;
        }
        // MySQL keeps its initialization SQL beside the per-resource directories.
        if fs::path_is_file(&resource)? && !matches!(resource.extension(), Some("pid" | "json")) {
            continue;
        }
        require_directory(&resource)?;
        collect_records(&resource, &mut records)?;
    }
    let supervisor = ProcessSupervisor::new(paths);
    let mut failures = Vec::new();
    for pid_path in records {
        if let Err(error) = supervisor.stop_recorded_for_shutdown(&pid_path).await {
            failures.push(RuntimeReconciliationFailure::new(
                pid_path.to_string(),
                error,
            ));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(DaemonError::RuntimeStopFailures { failures })
    }
}

fn collect_records(
    directory: &Utf8Path,
    records: &mut BTreeSet<Utf8PathBuf>,
) -> Result<(), DaemonError> {
    if !fs::path_entry_exists(directory)? {
        return Ok(());
    }
    require_directory(directory)?;
    for path in fs::read_dir_paths(directory)? {
        if matches!(path.extension(), Some("pid" | "json")) {
            if !fs::path_is_file(&path)? {
                return Err(DaemonError::InvalidRuntimeRecord { path });
            }
            records.insert(path.with_extension("pid"));
        }
    }
    Ok(())
}

fn require_directory(path: &Utf8Path) -> Result<(), DaemonError> {
    if !fs::path_is_directory(path)? {
        return Err(DaemonError::InvalidRuntimeRecord {
            path: path.to_owned(),
        });
    }
    Ok(())
}
