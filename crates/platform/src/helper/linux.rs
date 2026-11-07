use std::fs::DirBuilder;
use std::io::{self, Read as _, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, PermissionsExt as _};
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use futures_util::StreamExt as _;
use rustix::fs::{AtFlags, CWD, FlockOperation, Mode, OFlags, flock, open, unlinkat};
use rustix::process::{Uid, geteuid, getuid};
use rustix::thread::set_thread_res_uid;
use sha2::{Digest, Sha256};
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
use crate::systemd::{self, native_proxy, within};

const HELPER_SYSTEMD_TIMEOUT: Limit =
    Limit::new("helper systemd operation", Duration::from_secs(10));
const HELPER_JOB_TIMEOUT: Limit = Limit::new("helper systemd job", Duration::from_secs(15));
const HELPER_READINESS_TIMEOUT: Limit = Limit::new("Linux helper readiness", HELPER_IO_TIMEOUT);

const EXECUTABLE_DIRECTORY: &str = "/usr/local/libexec/pv";
const WORK_DIRECTORY: &str = "/var/lib/pv";
const CANDIDATE_PATH: &str = "/usr/local/libexec/pv/pv-helper.candidate";
const LOCK_PATH: &str = "/usr/local/libexec/.pv-helper-lifecycle.lock";
const SOCKET_UNIT: &str = "pv-helper.socket";
const SERVICE_UNIT: &str = "pv-helper.service";
const SOCKET_UNIT_PATH: &str = "/etc/systemd/system/pv-helper.socket";
const SERVICE_UNIT_PATH: &str = "/etc/systemd/system/pv-helper.service";
const MARKER: &str = "# Managed by PV\n";
const SERVICE: &str = "# Managed by PV\n[Unit]\nDescription=PV privileged helper\n\n[Service]\nExecStart=/usr/local/libexec/pv/pv-helper\nStandardError=append:/var/log/pv-helper.err.log\n";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrivilegedHelperLifecycleAction {
    Install,
    Replace,
    Remove,
}

impl std::str::FromStr for PrivilegedHelperLifecycleAction {
    type Err = PlatformError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "install" => Ok(Self::Install),
            "replace" => Ok(Self::Replace),
            "remove" => Ok(Self::Remove),
            _ => Err(PlatformError::PrivilegedHelperRejected {
                message: "helper lifecycle action must be install, replace, or remove".to_owned(),
            }),
        }
    }
}

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
    checksums: [Option<String>; 4],
    metadata: Option<PrivilegedHelperMetadata>,
    active: bool,
    enabled: bool,
}

fn installation_error(message: impl Into<String>) -> PlatformError {
    PlatformError::PrivilegedHelperInstallation(message.into())
}

fn bus_error(source: zbus::Error) -> PlatformError {
    PlatformError::PrivilegedHelperBus(Box::new(source))
}

fn socket_unit(owner_uid: u32) -> String {
    format!(
        "# Managed by PV\n[Unit]\nDescription=PV privileged helper socket\n\n[Socket]\nListenStream=/run/pv-helper.sock\nSocketUser={owner_uid}\nSocketMode=0600\nAccept=no\nRemoveOnStop=yes\n\n[Install]\nWantedBy=sockets.target\n"
    )
}

fn sibling(path: &str, suffix: &str) -> Utf8PathBuf {
    Utf8PathBuf::from(format!("{path}.{suffix}"))
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
    if sha256_file(candidate)? != expected_sha256 {
        return Err(installation_error("helper candidate checksum mismatch"));
    }
    stage_and_run(
        candidate,
        owner_uid,
        expected_sha256,
        helper_version,
        protocol_version,
    )
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
        if state::fs::path_entry_exists(Utf8Path::new(path))
            .map_err(|error| installation_error(error.to_string()))?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn remove() -> Result<(), PlatformError> {
    let owner_uid = non_root_caller()?;
    if !installed_helper_artifacts_present()? {
        return Ok(());
    }
    let executable = Utf8Path::new(HELPER_EXECUTABLE_PATH);
    let mut files_present = false;
    for path in [HELPER_EXECUTABLE_PATH, SOCKET_UNIT_PATH, SERVICE_UNIT_PATH] {
        if state::fs::path_entry_exists(Utf8Path::new(path))
            .map_err(|error| installation_error(error.to_string()))?
        {
            files_present = true;
            break;
        }
    }
    if !files_present {
        // -d removes only empty fixed directories, preserving metadata or unexpected contents.
        return run_sudo(&[
            "/usr/bin/rm",
            "-f",
            "-d",
            "--",
            CANDIDATE_PATH,
            EXECUTABLE_DIRECTORY,
            WORK_DIRECTORY,
        ]);
    }
    validate_root_owned_regular_file(executable, 0o755)?;
    let checksum = sha256_file(executable)?;
    run_sudo_output(&[
        HELPER_EXECUTABLE_PATH,
        "helper-lifecycle",
        "remove",
        "--owner-uid",
        &owner_uid.to_string(),
        "--sha256",
        &checksum,
        "--helper-version",
        PRIVILEGED_HELPER_VERSION,
        "--protocol-version",
        &HELPER_PROTOCOL_VERSION.to_string(),
    ])
    .map(|_| ())
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

fn stage_and_run(
    candidate: &Utf8Path,
    owner_uid: u32,
    checksum: &str,
    helper_version: &str,
    protocol_version: u32,
) -> Result<PrivilegedHelperInstallOutcome, PlatformError> {
    validate_directory_chain(Utf8Path::new(EXECUTABLE_DIRECTORY), false)?;
    run_sudo(&[
        "/usr/bin/install",
        "-D",
        "-o",
        "root",
        "-g",
        "root",
        "-m",
        "0755",
        candidate.as_str(),
        CANDIDATE_PATH,
    ])?;
    validate_root_owned_regular_file(Utf8Path::new(CANDIDATE_PATH), 0o755)?;
    if sha256_file(Utf8Path::new(CANDIDATE_PATH))? != checksum {
        return Err(installation_error("root-staged helper checksum mismatch"));
    }
    let response = run_sudo_output(&[
        CANDIDATE_PATH,
        "helper-lifecycle",
        "install",
        "--owner-uid",
        &owner_uid.to_string(),
        "--sha256",
        checksum,
        "--helper-version",
        helper_version,
        "--protocol-version",
        &protocol_version.to_string(),
    ])?;
    Ok(serde_json::from_str(&response)?)
}

/// Runs the fixed Linux helper lifecycle as root. A committed install may return a cleanup warning.
pub fn run_privileged_helper_lifecycle(
    action: PrivilegedHelperLifecycleAction,
    owner_uid: u32,
    expected_sha256: &str,
    helper_version: &str,
    protocol_version: u32,
) -> Result<Option<PrivilegedHelperInstallOutcome>, PlatformError> {
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
    validate_helper_identity(helper_version, protocol_version)?;
    if action != PrivilegedHelperLifecycleAction::Remove
        && (helper_version != PRIVILEGED_HELPER_VERSION
            || protocol_version != HELPER_PROTOCOL_VERSION)
    {
        return Err(installation_error(
            "candidate helper identity does not match its lifecycle request",
        ));
    }
    validate_fingerprint(expected_sha256)?;
    validate_directory_chain(Utf8Path::new(EXECUTABLE_DIRECTORY), false)?;
    let _lock = acquire_lock(Utf8Path::new(LOCK_PATH))?;
    let source = if action == PrivilegedHelperLifecycleAction::Remove {
        HELPER_EXECUTABLE_PATH
    } else {
        CANDIDATE_PATH
    };
    let candidate = read_root_file(Utf8Path::new(source), 0o755)?
        .ok_or_else(|| installation_error("root-staged helper candidate is missing"))?;
    let actual = checksum_bytes(&candidate);
    if actual != expected_sha256 {
        return Err(installation_error(
            "root-staged helper changed before lifecycle installation",
        ));
    }
    let metadata = validate_existing_owner(owner_uid)?;
    for artifact in ARTIFACTS {
        if read_root_file(&sibling(artifact.path, "rollback"), artifact.mode)?.is_some() {
            return Err(installation_error(format!(
                "retained helper rollback requires administrator recovery: {}",
                artifact.path
            )));
        }
    }
    ensure_root_directory(Utf8Path::new(EXECUTABLE_DIRECTORY), 0o755)?;
    ensure_root_directory(Utf8Path::new(WORK_DIRECTORY), 0o700)?;
    validate_directory_chain(Utf8Path::new("/etc/systemd/system"), true)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(PlatformError::PrivilegedHelperIo)?;
    runtime.block_on(async {
        let manager = manager().await?;
        let previous = inspect_previous(&manager, metadata, action).await?;
        let mut enablement_attempted = false;
        let result = transact(
            &manager,
            action,
            owner_uid,
            &candidate,
            &previous,
            &mut enablement_attempted,
        )
        .await;
        let status = match result {
            Ok(status) => status,
            Err(error) => {
                return Err(rollback(&manager, &previous, enablement_attempted, error).await);
            }
        };
        if action == PrivilegedHelperLifecycleAction::Remove {
            remove_file(Utf8Path::new("/var/log/pv-helper.err.log"))?;
            clear_transaction_files()?;
            remove_empty_root_directory(Utf8Path::new(WORK_DIRECTORY))?;
            remove_empty_root_directory(Utf8Path::new(EXECUTABLE_DIRECTORY))?;
            return Ok(None);
        }
        let cleanup_warning = clear_transaction_files().err().map(|error| format!(
            "helper lifecycle completed, but transaction cleanup failed: {error}; administrator recovery is required"
        ));
        Ok(status.map(|status| PrivilegedHelperInstallOutcome {
            status,
            cleanup_warning,
        }))
    })
}

async fn transact(
    manager: &Proxy<'_>,
    action: PrivilegedHelperLifecycleAction,
    owner_uid: u32,
    candidate: &[u8],
    previous: &PreviousState,
    enablement_attempted: &mut bool,
) -> Result<Option<PrivilegedHelperStatus>, PlatformError> {
    for (index, unit) in [SOCKET_UNIT, SERVICE_UNIT].into_iter().enumerate() {
        if action != PrivilegedHelperLifecycleAction::Remove
            || previous.checksums[index + 2].is_some()
        {
            job(manager, "StopUnit", unit).await?;
        }
    }
    if action == PrivilegedHelperLifecycleAction::Remove {
        disable(manager).await?;
    }
    for (index, (unit, path)) in [
        (SOCKET_UNIT, SOCKET_UNIT_PATH),
        (SERVICE_UNIT, SERVICE_UNIT_PATH),
    ]
    .into_iter()
    .enumerate()
    {
        if action != PrivilegedHelperLifecycleAction::Remove
            || previous.checksums[index + 2].is_some()
        {
            reset_stopped_unit(manager, unit, path).await?;
        }
    }
    for (artifact, checksum) in ARTIFACTS.into_iter().zip(&previous.checksums) {
        if checksum.is_some() {
            rename(
                Utf8Path::new(artifact.path),
                &sibling(artifact.path, "rollback"),
            )?;
        }
    }
    if action == PrivilegedHelperLifecycleAction::Remove {
        reload(manager).await?;
        return Ok(None);
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
    let status = wait_for_ready(Utf8Path::new(HELPER_SOCKET_PATH), owner_uid).await?;
    require_identity(
        &status,
        owner_uid,
        PRIVILEGED_HELPER_VERSION,
        HELPER_PROTOCOL_VERSION,
    )?;
    Ok(Some(status))
}

async fn rollback(
    manager: &Proxy<'_>,
    previous: &PreviousState,
    enablement_attempted: bool,
    original: PlatformError,
) -> PlatformError {
    let mut errors = Vec::new();
    let fresh_install = previous.checksums.iter().all(Option::is_none);
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
    for (artifact, checksum) in ARTIFACTS.into_iter().zip(&previous.checksums) {
        if let Err(error) = restore_file(
            Utf8Path::new(artifact.path),
            artifact.mode,
            checksum.as_deref(),
        ) {
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
    match bounded_call::<()>(manager, "ResetFailedUnit", &(unit,)).await {
        Err(PlatformError::PrivilegedHelperBus(source))
            if matches!(source.as_ref(), zbus::Error::MethodError(name, _, _)
                if name.as_str() == "org.freedesktop.systemd1.NoSuchUnit") =>
        {
            Ok(())
        }
        result => result,
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
    action: PrivilegedHelperLifecycleAction,
) -> Result<PreviousState, PlatformError> {
    let mut checksums = [const { None }; 4];
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
        checksums[index] = content.as_deref().map(checksum_bytes);
    }
    let socket_active = (action != PrivilegedHelperLifecycleAction::Remove
        || checksums[2].is_some())
        && unit_active(manager, SOCKET_UNIT, SOCKET_UNIT_PATH).await?;
    let service_active = (action != PrivilegedHelperLifecycleAction::Remove
        || checksums[3].is_some())
        && unit_active(manager, SERVICE_UNIT, SERVICE_UNIT_PATH).await?;
    let enabled = if checksums[2].is_some() {
        bounded_call::<String>(manager, "GetUnitFileState", &(SOCKET_UNIT,)).await? == "enabled"
    } else {
        false
    };
    Ok(PreviousState {
        checksums,
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
            Err(zbus::Error::MethodError(name, _, _))
                if name.as_str() == "org.freedesktop.systemd1.NoSuchUnit" =>
            {
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
        if fragment.is_empty() && load == "not-found" && active == "inactive" {
            return Ok(false);
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
        Ok(matches!(
            active.as_str(),
            "active" | "activating" | "reloading"
        ))
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
    if read_root_file(Utf8Path::new(SOCKET_UNIT_PATH), 0o644)?.is_some() {
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
    within(&HELPER_JOB_TIMEOUT, async {
        let mut removed = manager
            .receive_signal("JobRemoved")
            .await
            .map_err(bus_error)?;
        let job: OwnedObjectPath = match manager.call(operation, &(unit, "replace")).await {
            Ok(job) => job,
            Err(zbus::Error::MethodError(name, _, _))
                if operation == "StopUnit"
                    && name.as_str() == "org.freedesktop.systemd1.NoSuchUnit" =>
            {
                return Ok(());
            }
            Err(error) => return Err(bus_error(error)),
        };
        while let Some(message) = removed.next().await {
            let (_, completed, _, result): (u32, OwnedObjectPath, String, String) =
                message.body().deserialize().map_err(bus_error)?;
            if completed == job {
                return if result == "done" {
                    Ok(())
                } else {
                    Err(installation_error(format!(
                        "systemd {operation} {unit} finished with {result}"
                    )))
                };
            }
        }
        Err(installation_error(
            "systemd disconnected before helper job completion",
        ))
    })
    .await
    .ok_or_else(|| installation_error(format!("systemd {operation} {unit} timed out")))?
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

fn validate_existing_owner(
    owner_uid: u32,
) -> Result<Option<PrivilegedHelperMetadata>, PlatformError> {
    if let Some(content) = read_root_file(Utf8Path::new(HELPER_METADATA_PATH), 0o600)? {
        let metadata: PrivilegedHelperMetadata = serde_json::from_slice(&content)?;
        require_matching_helper_owner(&metadata, owner_uid)?;
        return Ok(Some(metadata));
    }
    for artifact in ARTIFACTS {
        if state::fs::path_entry_exists(Utf8Path::new(artifact.path))
            .map_err(|error| installation_error(error.to_string()))?
        {
            return Err(installation_error(
                "existing helper ownership cannot be determined; administrator recovery is required",
            ));
        }
    }
    Ok(None)
}

fn checksum_bytes(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}

fn restore_file(path: &Utf8Path, mode: u32, checksum: Option<&str>) -> Result<(), PlatformError> {
    let backup = sibling(path.as_str(), "rollback");
    if let Some(content) = read_root_file(&backup, mode)? {
        if checksum != Some(checksum_bytes(&content).as_str()) {
            return Err(installation_error(format!(
                "helper rollback checksum does not match the previous file: {backup}"
            )));
        }
        return rename(&backup, path);
    }
    let Some(checksum) = checksum else {
        return remove_file(path);
    };
    // An early failure can leave an original unmoved. Prove it is still that exact file.
    if let Some(content) = read_root_file(path, mode)?
        && checksum_bytes(&content) == checksum
    {
        return Ok(());
    }
    Err(installation_error(format!(
        "previous helper file and its rollback could not be verified: {path}"
    )))
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
    if !metadata.is_file()
        || metadata.uid() != 0
        || metadata.gid() != 0
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o7777 != mode
    {
        return Err(installation_error(format!(
            "{path} must be a root:root regular file with one link and mode {mode:o}"
        )));
    }
    let mut content = Vec::new();
    file.read_to_end(&mut content)
        .map_err(PlatformError::PrivilegedHelperIo)?;
    Ok(Some(content))
}

#[expect(
    clippy::disallowed_methods,
    reason = "root helper validates fixed destination directory ancestry"
)]
fn validate_directory_chain(path: &Utf8Path, create: bool) -> Result<(), PlatformError> {
    for directory in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        let metadata = match std::fs::symlink_metadata(directory) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound && !create => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                DirBuilder::new()
                    .mode(0o755)
                    .create(directory)
                    .map_err(PlatformError::PrivilegedHelperIo)?;
                std::fs::symlink_metadata(directory).map_err(PlatformError::PrivilegedHelperIo)?
            }
            Err(error) => return Err(PlatformError::PrivilegedHelperIo(error)),
        };
        if !metadata.is_dir()
            || metadata.uid() != 0
            || metadata.gid() != 0
            || metadata.permissions().mode() & 0o022 != 0
        {
            return Err(installation_error(format!(
                "root helper directory is not trusted: {directory}"
            )));
        }
    }
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "root helper sets fixed installation directory modes"
)]
fn ensure_root_directory(path: &Utf8Path, mode: u32) -> Result<(), PlatformError> {
    validate_directory_chain(path, true)?;
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
    if !metadata.is_file()
        || metadata.uid() != 0
        || metadata.gid() != 0
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o7777 != 0o600
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
    let candidate = Utf8Path::new(CANDIDATE_PATH);
    if read_root_file(candidate, 0o755)?.is_some() {
        remove_file(candidate)?;
    }
    for artifact in ARTIFACTS {
        let path = sibling(artifact.path, "rollback");
        if read_root_file(&path, artifact.mode)?.is_some() {
            remove_file(&path)?;
        }
    }
    Ok(())
}
fn remove_empty_root_directory(path: &Utf8Path) -> Result<(), PlatformError> {
    validate_directory_chain(path, false)?;
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
    use insta::assert_snapshot;
    use rustix::fs::{CWD, Mode, mkfifoat};

    use super::{SERVICE, read_root_file, socket_unit};
    use crate::PlatformError;

    #[test]
    fn helper_units_use_owned_socket_activation_and_the_installing_uid() {
        assert_snapshot!(socket_unit(1000), @r#"
        # Managed by PV
        [Unit]
        Description=PV privileged helper socket

        [Socket]
        ListenStream=/run/pv-helper.sock
        SocketUser=1000
        SocketMode=0600
        Accept=no
        RemoveOnStop=yes

        [Install]
        WantedBy=sockets.target
        "#);
        assert_snapshot!(SERVICE, @r#"
        # Managed by PV
        [Unit]
        Description=PV privileged helper

        [Service]
        ExecStart=/usr/local/libexec/pv/pv-helper
        StandardError=append:/var/log/pv-helper.err.log
        "#);
    }

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
