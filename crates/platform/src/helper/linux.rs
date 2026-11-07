use std::fs::DirBuilder;
use std::io::{self, Read as _, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, PermissionsExt as _};
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use rustix::fs::{AtFlags, CWD, FlockOperation, Mode, OFlags, flock, open, unlinkat};
use rustix::process::{Uid, geteuid, getuid};
use rustix::thread::set_thread_res_uid;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;
use zbus::Proxy;
use zbus::zvariant::OwnedObjectPath;

use super::{
    HELPER_EXECUTABLE_PATH, HELPER_IO_TIMEOUT, HELPER_LIFECYCLE_PROBE, HELPER_METADATA_PATH,
    HELPER_PROTOCOL_VERSION, HELPER_SOCKET_PATH, MAX_MESSAGE_BYTES, PRIVILEGED_HELPER_VERSION,
    PrivilegedHelperInstallOutcome, PrivilegedHelperMetadata, PrivilegedHelperStatus,
    parse_helper_lifecycle_response, require_matching_helper_owner, run_sudo, run_sudo_output,
    sha256_file, validate_fingerprint, validate_helper_identity, validate_regular_file_owned_by,
    validate_root_owned_regular_file,
};
use crate::PlatformError;
use crate::clock::{self, Limit};
use crate::systemd::{self, is_method_error, native_proxy, within};

const HELPER_SYSTEMD_TIMEOUT: Limit =
    Limit::new("helper systemd operation", Duration::from_secs(10));
const HELPER_JOB_TIMEOUT: Limit = Limit::new("helper systemd job", Duration::from_secs(15));
const HELPER_READINESS_TIMEOUT: Limit = Limit::new("Linux helper readiness", HELPER_IO_TIMEOUT);

const EXECUTABLE_DIRECTORY: &str = "/usr/local/libexec/pv";
const WORK_DIRECTORY: &str = "/var/lib/pv";
const CANDIDATE_PATH: &str = "/usr/local/libexec/pv/pv-helper.candidate";
const LOCK_PATH: &str = "/usr/local/libexec/.pv-helper-lifecycle.lock";
const ERROR_LOG_PATH: &str = "/var/log/pv-helper.err.log";
const SOCKET_UNIT: &str = "pv-helper.socket";
const SERVICE_UNIT: &str = "pv-helper.service";
const SOCKET_UNIT_PATH: &str = "/etc/systemd/system/pv-helper.socket";
const SERVICE_UNIT_PATH: &str = "/etc/systemd/system/pv-helper.service";
const SYSTEMCTL: &str = "/usr/bin/systemctl";
const MARKER: &str = "# Managed by PV\n";
const SERVICE: &str = "# Managed by PV\n[Unit]\nDescription=PV privileged helper\n\n[Service]\nExecStart=/usr/local/libexec/pv/pv-helper\nStandardError=append:/var/log/pv-helper.err.log\n";

#[expect(
    clippy::disallowed_types,
    reason = "root helper owns validated native file descriptors"
)]
type RootFile = std::fs::File;

#[derive(Clone, Copy)]
struct Artifact {
    path: &'static str,
    mode: u32,
}

const ARTIFACTS: [Artifact; 4] = [
    Artifact {
        path: HELPER_EXECUTABLE_PATH,
        mode: 0o755,
    },
    Artifact {
        path: HELPER_METADATA_PATH,
        mode: 0o600,
    },
    Artifact {
        path: SOCKET_UNIT_PATH,
        mode: 0o644,
    },
    Artifact {
        path: SERVICE_UNIT_PATH,
        mode: 0o644,
    },
];

struct PreviousState {
    /// Which of [`ARTIFACTS`] existed before the transaction.
    existed: [bool; 4],
    metadata: Option<PrivilegedHelperMetadata>,
    active: bool,
    enabled: bool,
}

fn installation_error(message: impl Into<String>) -> PlatformError {
    PlatformError::PrivilegedHelperInstallation(message.into())
}

fn bus_error(source: zbus::Error) -> PlatformError {
    installation_error(format!("systemd call failed: {source}"))
}

fn socket_unit(owner_uid: u32) -> String {
    format!(
        "# Managed by PV\n[Unit]\nDescription=PV privileged helper socket\n\n[Socket]\nListenStream=/run/pv-helper.sock\nSocketUser={owner_uid}\nSocketMode=0600\nAccept=no\nRemoveOnStop=yes\n\n[Install]\nWantedBy=sockets.target\n"
    )
}

fn sibling(path: &str, suffix: &str) -> Utf8PathBuf {
    Utf8PathBuf::from(format!("{path}.{suffix}"))
}

fn exists(path: &str) -> Result<bool, PlatformError> {
    state::fs::path_entry_exists(Utf8Path::new(path))
        .map_err(|error| installation_error(error.to_string()))
}

pub(super) fn install(
    candidate: &Utf8Path,
    expected_sha256: &str,
    helper_version: &str,
    protocol_version: u32,
) -> Result<PrivilegedHelperInstallOutcome, PlatformError> {
    let owner_uid = non_root_caller()?;
    validate_regular_file_owned_by(candidate, owner_uid)?;
    validate_helper_identity(helper_version, protocol_version)?;
    validate_fingerprint(expected_sha256)?;
    // Create missing directories with an explicit mode: sudo keeps a stricter user umask, and a
    // 0700 directory would hide the installed helper from this account.
    let mut create_directories = vec!["/usr/bin/install", "-d", "-m", "0755"];
    for directory in ["/usr/local/libexec", EXECUTABLE_DIRECTORY] {
        if !exists(directory)? {
            create_directories.push(directory);
        }
    }
    if create_directories.len() > 4 {
        run_sudo(&create_directories)?;
    }
    run_sudo(&[
        "/usr/bin/install",
        "-o",
        "root",
        "-g",
        "root",
        "-m",
        "0755",
        candidate.as_str(),
        CANDIDATE_PATH,
    ])?;
    let staged = Utf8Path::new(CANDIDATE_PATH);
    validate_root_owned_regular_file(staged, 0o755)?;
    let actual_sha256 = sha256_file(staged)?;
    if actual_sha256 != expected_sha256 {
        return Err(installation_error(format!(
            "root-staged helper checksum mismatch: expected {expected_sha256}, got {actual_sha256}"
        )));
    }
    let response = run_sudo_output(&[
        CANDIDATE_PATH,
        "helper-lifecycle",
        "--owner-uid",
        &owner_uid.to_string(),
    ])?;
    let outcome: PrivilegedHelperInstallOutcome = serde_json::from_str(&response)?;
    require_identity(&outcome.status, owner_uid, helper_version, protocol_version)?;
    Ok(outcome)
}

/// Reports whether any of the fixed Linux helper installation paths remains.
fn installed_helper_artifacts_present() -> Result<bool, PlatformError> {
    for path in [
        EXECUTABLE_DIRECTORY,
        WORK_DIRECTORY,
        SOCKET_UNIT_PATH,
        SERVICE_UNIT_PATH,
        HELPER_SOCKET_PATH,
    ] {
        if exists(path)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Stops the helper and deletes every file it left, transaction leftovers included, the way
/// macOS boots out the LaunchDaemon and removes its files.
pub(super) fn remove() -> Result<(), PlatformError> {
    let owner_uid = non_root_caller()?;
    if !installed_helper_artifacts_present()? {
        return Ok(());
    }
    require_socket_owner(owner_uid)?;
    if exists(SOCKET_UNIT_PATH)? {
        run_sudo(&[SYSTEMCTL, "disable", "--now", SOCKET_UNIT])?;
    }
    if exists(SERVICE_UNIT_PATH)? {
        run_sudo(&[SYSTEMCTL, "stop", SERVICE_UNIT])?;
    }
    let rollback_paths = ARTIFACTS.map(|artifact| sibling(artifact.path, "rollback"));
    let mut files = vec![
        "/usr/bin/rm",
        "-f",
        "--",
        SOCKET_UNIT_PATH,
        SERVICE_UNIT_PATH,
        HELPER_SOCKET_PATH,
        ERROR_LOG_PATH,
    ];
    files.extend(rollback_paths.iter().map(|path| path.as_str()));
    run_sudo(&files)?;
    run_sudo(&[
        "/usr/bin/rm",
        "-rf",
        "--",
        EXECUTABLE_DIRECTORY,
        WORK_DIRECTORY,
    ])?;
    run_sudo(&[SYSTEMCTL, "daemon-reload"])
}

/// Checks the installed helper belongs to `owner_uid`, from the socket unit anyone can read.
fn require_socket_owner(owner_uid: u32) -> Result<(), PlatformError> {
    let Some(unit) = read_root_file(Utf8Path::new(SOCKET_UNIT_PATH), 0o644)? else {
        return if exists(HELPER_EXECUTABLE_PATH)? {
            Err(installation_error(
                "existing helper ownership cannot be determined: its socket unit is missing",
            ))
        } else {
            Ok(())
        };
    };
    let socket_user = String::from_utf8_lossy(&unit)
        .lines()
        .find_map(|line| line.strip_prefix("SocketUser="))
        .and_then(|value| value.parse::<u32>().ok());
    match socket_user {
        Some(uid) => require_matching_helper_owner(
            &PrivilegedHelperMetadata {
                owner_uid: uid,
                helper_version: PRIVILEGED_HELPER_VERSION.to_owned(),
                protocol_version: HELPER_PROTOCOL_VERSION,
            },
            owner_uid,
        ),
        None => Err(installation_error(
            "existing helper ownership cannot be determined: its socket unit names no SocketUser",
        )),
    }
}

fn non_root_caller() -> Result<u32, PlatformError> {
    let uid = getuid();
    if uid.is_root() {
        Err(installation_error(
            "helper lifecycle must be requested by the supported non-root account",
        ))
    } else {
        Ok(uid.as_raw())
    }
}

/// Installs the root-staged candidate as root. A committed install may return a cleanup warning.
pub fn run_privileged_helper_lifecycle(
    owner_uid: u32,
) -> Result<PrivilegedHelperInstallOutcome, PlatformError> {
    if !getuid().is_root() || !geteuid().is_root() {
        return Err(PlatformError::PrivilegedHelperAuthentication(
            "helper lifecycle must run as root".to_owned(),
        ));
    }
    if owner_uid == 0 {
        return Err(PlatformError::PrivilegedHelperAuthentication(
            "installing UID must not be root".to_owned(),
        ));
    }
    let _lock = acquire_lock(Utf8Path::new(LOCK_PATH))?;
    let candidate = read_root_file(Utf8Path::new(CANDIDATE_PATH), 0o755)?
        .ok_or_else(|| installation_error("root-staged helper candidate is missing"))?;
    require_socket_owner(owner_uid)?;
    let metadata = read_root_file(Utf8Path::new(HELPER_METADATA_PATH), 0o600)?
        .and_then(|content| serde_json::from_slice::<PrivilegedHelperMetadata>(&content).ok());
    for artifact in ARTIFACTS {
        if exists(sibling(artifact.path, "rollback").as_str())? {
            return Err(installation_error(format!(
                "retained helper rollback requires administrator recovery: {}",
                artifact.path
            )));
        }
    }
    ensure_root_directory(Utf8Path::new(EXECUTABLE_DIRECTORY), 0o755)?;
    ensure_root_directory(Utf8Path::new(WORK_DIRECTORY), 0o700)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(PlatformError::PrivilegedHelperIo)?;
    runtime.block_on(async {
        let manager = manager().await?;
        let previous = inspect_previous(&manager, metadata).await?;
        let mut enablement_attempted = false;
        let status = match transact(
            &manager,
            owner_uid,
            &candidate,
            &previous,
            &mut enablement_attempted,
        )
        .await
        {
            Ok(status) => status,
            Err(error) => {
                return Err(rollback(&manager, &previous, enablement_attempted, error).await);
            }
        };
        let cleanup_warning = clear_transaction_files().err().map(|error| format!(
            "helper lifecycle completed, but transaction cleanup failed: {error}; administrator recovery is required"
        ));
        Ok(PrivilegedHelperInstallOutcome {
            status,
            cleanup_warning,
        })
    })
}

async fn transact(
    manager: &Proxy<'_>,
    owner_uid: u32,
    candidate: &[u8],
    previous: &PreviousState,
    enablement_attempted: &mut bool,
) -> Result<PrivilegedHelperStatus, PlatformError> {
    for unit in [SOCKET_UNIT, SERVICE_UNIT] {
        job(manager, "StopUnit", unit).await?;
    }
    for (unit, path) in [
        (SOCKET_UNIT, SOCKET_UNIT_PATH),
        (SERVICE_UNIT, SERVICE_UNIT_PATH),
    ] {
        reset_stopped_unit(manager, unit, path).await?;
    }
    for (artifact, existed) in ARTIFACTS.into_iter().zip(previous.existed) {
        if existed {
            rename(
                Utf8Path::new(artifact.path),
                &sibling(artifact.path, "rollback"),
            )?;
        }
    }
    let metadata = PrivilegedHelperMetadata {
        owner_uid,
        helper_version: PRIVILEGED_HELPER_VERSION.to_owned(),
        protocol_version: HELPER_PROTOCOL_VERSION,
    };
    for (artifact, content) in ARTIFACTS.into_iter().zip([
        candidate.to_vec(),
        serde_json::to_vec_pretty(&metadata)?,
        socket_unit(owner_uid).into_bytes(),
        SERVICE.as_bytes().to_vec(),
    ]) {
        write_root_file(Utf8Path::new(artifact.path), &content, artifact.mode)?;
    }
    reload(manager).await?;
    *enablement_attempted = true;
    enable(manager).await?;
    job(manager, "StartUnit", SOCKET_UNIT).await?;
    wait_for_ready(Utf8Path::new(HELPER_SOCKET_PATH), owner_uid).await
}

async fn rollback(
    manager: &Proxy<'_>,
    previous: &PreviousState,
    enablement_attempted: bool,
    original: PlatformError,
) -> PlatformError {
    let mut errors = Vec::new();
    let fresh_install = !previous.existed.contains(&true);
    for unit in [SOCKET_UNIT, SERVICE_UNIT] {
        if let Err(error) = job(manager, "StopUnit", unit).await {
            errors.push(error.to_string());
        }
    }
    // Disable while the installed unit file still exists; systemd rejects missing files even
    // when there are no enablement links. Earlier failures never enabled a new unit.
    if enablement_attempted && let Err(error) = disable(manager).await {
        errors.push(error.to_string());
    }
    // Failed units survive file removal and Reload. Clear stopped units while
    // their marked files and selected fragments still prove ownership.
    if enablement_attempted && errors.is_empty() {
        for (unit, path) in [
            (SOCKET_UNIT, SOCKET_UNIT_PATH),
            (SERVICE_UNIT, SERVICE_UNIT_PATH),
        ] {
            if let Err(error) = reset_stopped_unit(manager, unit, path).await {
                errors.push(error.to_string());
            }
        }
    }
    for (artifact, existed) in ARTIFACTS.into_iter().zip(previous.existed) {
        if let Err(error) = restore_file(Utf8Path::new(artifact.path), artifact.mode, existed) {
            errors.push(error.to_string());
        }
    }
    if let Err(error) = reload(manager).await {
        errors.push(error.to_string());
    }
    if previous.enabled
        && let Err(error) = enable(manager).await
    {
        errors.push(error.to_string());
    }
    // Restore activity through socket activation.
    if previous.active
        && let Err(error) = job(manager, "StartUnit", SOCKET_UNIT).await
    {
        errors.push(error.to_string());
    }
    if previous.active
        && let Some(metadata) = &previous.metadata
    {
        let result =
            match wait_for_ready(Utf8Path::new(HELPER_SOCKET_PATH), metadata.owner_uid).await {
                Ok(status) => require_identity(
                    &status,
                    metadata.owner_uid,
                    &metadata.helper_version,
                    metadata.protocol_version,
                ),
                Err(error) => Err(error),
            };
        if let Err(error) = result {
            errors.push(error.to_string());
        }
    }
    if errors.is_empty()
        && let Err(error) = clear_transaction_files()
    {
        errors.push(error.to_string());
    }
    if errors.is_empty() && fresh_install {
        for path in [WORK_DIRECTORY, EXECUTABLE_DIRECTORY] {
            if let Err(error) = remove_empty_root_directory(Utf8Path::new(path)) {
                errors.push(error.to_string());
            }
        }
    }
    if errors.is_empty() {
        installation_error(format!(
            "{original}; restored the previous privileged helper installation"
        ))
    } else {
        installation_error(format!(
            "{original}; rollback also failed: {}; retained transaction files for administrator recovery",
            errors.join("; ")
        ))
    }
}

async fn reset_stopped_unit(
    manager: &Proxy<'_>,
    unit: &str,
    path: &str,
) -> Result<(), PlatformError> {
    if unit_active(manager, unit, path).await? {
        return Err(installation_error(format!(
            "helper unit {unit} is still active during cleanup"
        )));
    }
    match within(
        &HELPER_SYSTEMD_TIMEOUT,
        manager.call::<_, _, ()>("ResetFailedUnit", &(unit,)),
    )
    .await
    .ok_or_else(|| installation_error("systemd ResetFailedUnit timed out"))?
    {
        Err(error) if is_method_error(&error, "org.freedesktop.systemd1.NoSuchUnit") => Ok(()),
        result => result.map_err(bus_error),
    }
}

async fn manager() -> Result<Proxy<'static>, PlatformError> {
    within(&HELPER_SYSTEMD_TIMEOUT, systemd::system_manager())
        .await
        .ok_or_else(|| installation_error("systemd system manager connection timed out"))?
        .map_err(bus_error)
}

async fn inspect_previous(
    manager: &Proxy<'_>,
    metadata: Option<PrivilegedHelperMetadata>,
) -> Result<PreviousState, PlatformError> {
    let mut existed = [false; 4];
    for (index, artifact) in ARTIFACTS.into_iter().enumerate() {
        let content = read_root_file(Utf8Path::new(artifact.path), artifact.mode)?;
        if index >= 2
            && let Some(content) = &content
            && !content.starts_with(MARKER.as_bytes())
        {
            return Err(installation_error(format!(
                "refusing foreign helper unit {}",
                artifact.path
            )));
        }
        existed[index] = content.is_some();
    }
    let socket_active = unit_active(manager, SOCKET_UNIT, SOCKET_UNIT_PATH).await?;
    let service_active = unit_active(manager, SERVICE_UNIT, SERVICE_UNIT_PATH).await?;
    let enabled = if existed[2] {
        bounded_call::<String>(manager, "GetUnitFileState", &(SOCKET_UNIT,)).await? == "enabled"
    } else {
        false
    };
    Ok(PreviousState {
        existed,
        metadata,
        active: socket_active || service_active,
        enabled,
    })
}

async fn unit_active(
    manager: &Proxy<'_>,
    name: &str,
    expected_path: &str,
) -> Result<bool, PlatformError> {
    within(&HELPER_SYSTEMD_TIMEOUT, async {
        let path: OwnedObjectPath = match manager.call("LoadUnit", &(name,)).await {
            Ok(path) => path,
            Err(error) if is_method_error(&error, "org.freedesktop.systemd1.NoSuchUnit") => {
                return Ok(false);
            }
            Err(error) => return Err(bus_error(error)),
        };
        let unit = native_proxy(manager.connection(), path, "org.freedesktop.systemd1.Unit")
            .await
            .map_err(bus_error)?;
        let fragment: String = unit.get_property("FragmentPath").await.map_err(bus_error)?;
        let active: String = unit.get_property("ActiveState").await.map_err(bus_error)?;
        let load: String = unit.get_property("LoadState").await.map_err(bus_error)?;
        let active = matches!(active.as_str(), "active" | "activating" | "reloading");
        // PV's fixed unit name with no file anywhere is PV's own leftover, such as a unit that
        // failed and lost its file during a rollback; it is stopped and reset, not refused.
        if fragment.is_empty() && load == "not-found" {
            return Ok(active);
        }
        if fragment != expected_path {
            return Err(installation_error(format!(
                "refusing helper unit {name} loaded from {fragment}"
            )));
        }
        let content = read_root_file(Utf8Path::new(&fragment), 0o644)?.ok_or_else(|| {
            installation_error(format!("loaded helper unit is missing: {fragment}"))
        })?;
        if !content.starts_with(MARKER.as_bytes()) {
            return Err(installation_error(format!(
                "refusing foreign helper unit {fragment}"
            )));
        }
        Ok(active)
    })
    .await
    .ok_or_else(|| installation_error("helper unit inspection timed out"))?
}

async fn bounded_call<R>(
    manager: &Proxy<'_>,
    method: &str,
    body: &(impl serde::Serialize + zbus::zvariant::DynamicType),
) -> Result<R, PlatformError>
where
    R: for<'de> serde::Deserialize<'de> + zbus::zvariant::Type,
{
    within(&HELPER_SYSTEMD_TIMEOUT, manager.call(method, body))
        .await
        .ok_or_else(|| installation_error(format!("systemd {method} timed out")))?
        .map_err(bus_error)
}

async fn reload(manager: &Proxy<'_>) -> Result<(), PlatformError> {
    bounded_call(manager, "Reload", &()).await
}
async fn enable(manager: &Proxy<'_>) -> Result<(), PlatformError> {
    let _: (bool, Vec<(String, String, String)>) = bounded_call(
        manager,
        "EnableUnitFiles",
        &([SOCKET_UNIT].as_slice(), false, false),
    )
    .await?;
    Ok(())
}
async fn disable(manager: &Proxy<'_>) -> Result<(), PlatformError> {
    if exists(SOCKET_UNIT_PATH)? {
        let _: Vec<(String, String, String)> = bounded_call(
            manager,
            "DisableUnitFiles",
            &([SOCKET_UNIT].as_slice(), false),
        )
        .await?;
    }
    Ok(())
}

async fn job(manager: &Proxy<'_>, operation: &str, unit: &str) -> Result<(), PlatformError> {
    let result = within(
        &HELPER_JOB_TIMEOUT,
        systemd::unit_job(manager, operation, unit),
    )
    .await
    .ok_or_else(|| installation_error(format!("systemd {operation} {unit} timed out")))?
    .map_err(bus_error)?;
    if result == "done" {
        Ok(())
    } else {
        Err(installation_error(format!(
            "systemd {operation} {unit} finished with {result}"
        )))
    }
}

pub(super) async fn wait_for_ready(
    socket: &Utf8Path,
    owner_uid: u32,
) -> Result<PrivilegedHelperStatus, PlatformError> {
    let original_uid = geteuid();
    if original_uid.as_raw() != owner_uid {
        set_thread_res_uid(None, Some(Uid::from_raw(owner_uid)), None)
            .map_err(|error| PlatformError::PrivilegedHelperIo(error.into()))?;
    }
    // This block runs on the lifecycle's current-thread executor. The saved UID is retained;
    // no root filesystem operation happens until the effective UID has been restored below.
    let result = within(&HELPER_READINESS_TIMEOUT, async {
        loop {
            match UnixStream::connect(socket).await {
                Ok(mut stream) => {
                    stream
                        .write_all(HELPER_LIFECYCLE_PROBE)
                        .await
                        .map_err(PlatformError::PrivilegedHelperIo)?;
                    let mut reader = BufReader::new(
                        stream.take(
                            u64::try_from(MAX_MESSAGE_BYTES + 1)
                                .map_err(|error| installation_error(error.to_string()))?,
                        ),
                    );
                    let mut response = Vec::new();
                    reader
                        .read_until(b'\n', &mut response)
                        .await
                        .map_err(PlatformError::PrivilegedHelperIo)?;
                    if response.len() > MAX_MESSAGE_BYTES {
                        return Err(installation_error(
                            "helper lifecycle response exceeds size limit",
                        ));
                    }
                    return parse_helper_lifecycle_response(&response);
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                    ) =>
                {
                    clock::poll(Duration::from_millis(25)).await;
                }
                Err(error) => return Err(PlatformError::PrivilegedHelperIo(error)),
            }
        }
    })
    .await
    .ok_or(PlatformError::PrivilegedHelperUnavailable)
    .and_then(|result| result);
    if original_uid.as_raw() != owner_uid {
        set_thread_res_uid(None, Some(original_uid), None)
            .map_err(|error| PlatformError::PrivilegedHelperIo(error.into()))?;
    }
    result
}

fn require_identity(
    status: &PrivilegedHelperStatus,
    owner_uid: u32,
    version: &str,
    protocol: u32,
) -> Result<(), PlatformError> {
    if status.owner_uid != owner_uid
        || status.version != version
        || status.protocol_version != protocol
    {
        return Err(installation_error(format!(
            "helper lifecycle identity did not match expected {version}, protocol {protocol}, owner UID {owner_uid}: {status:?}"
        )));
    }
    Ok(())
}

/// Puts back a file the transaction moved aside, or removes one it created. A file that existed
/// but has no rollback copy was never moved, because an early step failed.
fn restore_file(path: &Utf8Path, mode: u32, existed: bool) -> Result<(), PlatformError> {
    let backup = sibling(path.as_str(), "rollback");
    if exists(backup.as_str())? {
        read_root_file(&backup, mode)?;
        return rename(&backup, path);
    }
    if existed {
        return Ok(());
    }
    remove_file(path)
}

fn read_root_file(path: &Utf8Path, mode: u32) -> Result<Option<Vec<u8>>, PlatformError> {
    let descriptor = match open(
        path.as_std_path(),
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(descriptor) => descriptor,
        Err(error) if error == rustix::io::Errno::NOENT => return Ok(None),
        Err(error) => return Err(PlatformError::PrivilegedHelperIo(error.into())),
    };
    let mut file = RootFile::from(descriptor);
    let metadata = file.metadata().map_err(PlatformError::PrivilegedHelperIo)?;
    if !metadata.is_file() || metadata.uid() != 0 || metadata.permissions().mode() & 0o7777 != mode
    {
        return Err(installation_error(format!(
            "{path} must be a root-owned regular file with mode {mode:o}"
        )));
    }
    let mut content = Vec::new();
    file.read_to_end(&mut content)
        .map_err(PlatformError::PrivilegedHelperIo)?;
    Ok(Some(content))
}

#[expect(
    clippy::disallowed_methods,
    reason = "root helper sets fixed installation directory modes"
)]
fn ensure_root_directory(path: &Utf8Path, mode: u32) -> Result<(), PlatformError> {
    DirBuilder::new()
        .recursive(true)
        .mode(mode)
        .create(path)
        .map_err(PlatformError::PrivilegedHelperIo)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(PlatformError::PrivilegedHelperIo)
}

fn acquire_lock(path: &Utf8Path) -> Result<RootFile, PlatformError> {
    let descriptor = open(
        path.as_std_path(),
        OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::from_bits_truncate(0o600),
    )
    .map_err(|error| PlatformError::PrivilegedHelperIo(error.into()))?;
    let file = RootFile::from(descriptor);
    let metadata = file.metadata().map_err(PlatformError::PrivilegedHelperIo)?;
    if !metadata.is_file() || metadata.uid() != 0 || metadata.permissions().mode() & 0o7777 != 0o600
    {
        return Err(installation_error(
            "machine-wide helper lock is not a private root-owned regular file",
        ));
    }
    flock(&file, FlockOperation::NonBlockingLockExclusive).map_err(|error| {
        installation_error(format!(
            "could not acquire machine-wide helper lifecycle lock: {error}"
        ))
    })?;
    Ok(file)
}

fn write_root_file(path: &Utf8Path, content: &[u8], mode: u32) -> Result<(), PlatformError> {
    // Units are stopped and originals are already in rollback files. Create exclusively from
    // the pinned bytes: another foreground caller can replace the shared staging candidate.
    let descriptor = open(
        path.as_std_path(),
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::from_bits_truncate(mode),
    )
    .map_err(|error| PlatformError::PrivilegedHelperIo(error.into()))?;
    let mut file = RootFile::from(descriptor);
    file.write_all(content)
        .map_err(PlatformError::PrivilegedHelperIo)?;
    file.set_permissions(std::fs::Permissions::from_mode(mode))
        .map_err(PlatformError::PrivilegedHelperIo)?;
    Ok(())
}

fn rename(source: &Utf8Path, destination: &Utf8Path) -> Result<(), PlatformError> {
    state::fs::rename(source, destination).map_err(|error| installation_error(error.to_string()))
}
fn remove_file(path: &Utf8Path) -> Result<(), PlatformError> {
    state::fs::remove_file_if_exists(path).map_err(|error| installation_error(error.to_string()))
}
fn clear_transaction_files() -> Result<(), PlatformError> {
    remove_file(Utf8Path::new(CANDIDATE_PATH))?;
    for artifact in ARTIFACTS {
        remove_file(&sibling(artifact.path, "rollback"))?;
    }
    Ok(())
}
fn remove_empty_root_directory(path: &Utf8Path) -> Result<(), PlatformError> {
    unlinkat(CWD, path.as_std_path(), AtFlags::REMOVEDIR).map_err(|error| {
        installation_error(format!(
            "could not remove empty helper directory {path}: {error}"
        ))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use camino_tempfile::tempdir;
    use rustix::fs::{CWD, Mode, mkfifoat};

    use super::read_root_file;
    use crate::PlatformError;

    #[test]
    fn root_file_validation_rejects_user_owned_files_and_symlinks() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("file");
        state::fs::write_sensitive_file(&path, "not root-owned")?;
        assert!(read_root_file(&path, 0o600).is_err());
        let link = directory.path().join("link");
        state::fs::symlink_file(&path, &link)?;
        assert!(read_root_file(&link, 0o600).is_err());
        let fifo = directory.path().join("fifo");
        mkfifoat(CWD, fifo.as_std_path(), Mode::RUSR | Mode::WUSR)?;
        assert!(matches!(
            read_root_file(&fifo, 0o600),
            Err(PlatformError::PrivilegedHelperInstallation(_))
        ));
        assert!(read_root_file(&directory.path().join("missing"), 0o600)?.is_none());
        Ok(())
    }
}
