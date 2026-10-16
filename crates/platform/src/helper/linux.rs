use std::io::{self, Read as _, Write as _};
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use async_io::Async;
use camino::{Utf8Path, Utf8PathBuf};
use futures_lite::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use futures_util::StreamExt as _;
use rustix::fs::{AtFlags, CWD, FlockOperation, Mode, OFlags, flock, open, unlinkat};
use rustix::process::{Uid, geteuid, getuid};
use rustix::thread::set_thread_res_uid;
use sha2::{Digest, Sha256};
use zbus::zvariant::OwnedObjectPath;
use zbus::{Proxy, connection};

use super::{
    HELPER_EXECUTABLE_PATH, HELPER_IO_TIMEOUT, HELPER_LIFECYCLE_PROBE, HELPER_METADATA_PATH,
    HELPER_PROTOCOL_VERSION, HELPER_SOCKET_PATH, MAX_MESSAGE_BYTES, PRIVILEGED_HELPER_VERSION,
    PrivilegedHelperInstallOutcome, PrivilegedHelperMetadata, PrivilegedHelperStatus,
    parse_helper_lifecycle_response, require_matching_helper_owner, run_sudo, run_sudo_output,
    sha256_file, validate_fingerprint, validate_helper_identity, validate_regular_file_owned_by,
    validate_root_owned_regular_file,
};
use crate::PlatformError;
use crate::systemd::{native_proxy, within};

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
    socket_active: bool,
    service_active: bool,
    // Some(false) is persistent enablement, Some(true) is runtime-only, None is disabled.
    enabled: Option<bool>,
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
    let cleanup_warning = stage_and_run(
        candidate,
        if state::fs::path_entry_exists(Utf8Path::new(HELPER_EXECUTABLE_PATH))
            .map_err(|error| installation_error(error.to_string()))?
        {
            "replace"
        } else {
            "install"
        },
        owner_uid,
        expected_sha256,
        helper_version,
        protocol_version,
    )?;
    let status = async_io::block_on(wait_for_ready(Utf8Path::new(HELPER_SOCKET_PATH), owner_uid))?;
    require_identity(&status, owner_uid, helper_version, protocol_version)?;
    let mut outcome = PrivilegedHelperInstallOutcome::successful(status);
    if let Some(warning) = cleanup_warning {
        outcome = outcome.with_cleanup_warning(warning);
    }
    Ok(outcome)
}

pub(super) fn remove() -> Result<(), PlatformError> {
    let owner_uid = non_root_caller()?;
    let mut present = false;
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
            present = true;
            break;
        }
    }
    if !present {
        return Ok(());
    }
    let executable = Utf8Path::new(HELPER_EXECUTABLE_PATH);
    validate_root_owned_regular_file(executable, 0o755).map_err(|error| {
        installation_error(format!(
            "existing helper executable cannot be verified; administrator recovery is required: {error}"
        ))
    })?;
    let checksum = sha256_file(executable)?;
    stage_and_run(
        executable,
        "remove",
        owner_uid,
        &checksum,
        PRIVILEGED_HELPER_VERSION,
        HELPER_PROTOCOL_VERSION,
    )
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
    action: &str,
    owner_uid: u32,
    checksum: &str,
    helper_version: &str,
    protocol_version: u32,
) -> Result<Option<String>, PlatformError> {
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
        action,
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
) -> Result<Option<String>, PlatformError> {
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
    let candidate = read_root_file(Utf8Path::new(CANDIDATE_PATH), 0o755)?
        .ok_or_else(|| installation_error("root-staged helper candidate is missing"))?;
    let actual = checksum_bytes(&candidate);
    if actual != expected_sha256 {
        return Err(installation_error(
            "root-staged helper changed before lifecycle installation",
        ));
    }
    let metadata = validate_existing_owner(owner_uid)?;
    if action == PrivilegedHelperLifecycleAction::Remove
        && let Some(metadata) = &metadata
        && (metadata.helper_version != PRIVILEGED_HELPER_VERSION
            || metadata.protocol_version != HELPER_PROTOCOL_VERSION)
    {
        return Err(installation_error(
            "installed helper metadata does not match the staged removal binary",
        ));
    }
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
    async_io::block_on(async {
        let manager = manager().await?;
        let previous = inspect_previous(&manager, metadata).await?;
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
        if let Err(error) = result {
            return Err(rollback(&manager, &previous, enablement_attempted, error).await);
        }
        if action == PrivilegedHelperLifecycleAction::Remove {
            remove_file(Utf8Path::new("/var/log/pv-helper.err.log"))?;
            clear_transaction_files()?;
            remove_empty_root_directory(Utf8Path::new(WORK_DIRECTORY))?;
            remove_empty_root_directory(Utf8Path::new(EXECUTABLE_DIRECTORY))?;
            return Ok(None);
        }
        Ok(clear_transaction_files().err().map(|error| format!(
            "helper lifecycle completed, but transaction cleanup failed: {error}; administrator recovery is required"
        )))
    })
}

async fn transact(
    manager: &Proxy<'_>,
    action: PrivilegedHelperLifecycleAction,
    owner_uid: u32,
    candidate: &[u8],
    previous: &PreviousState,
    enablement_attempted: &mut bool,
) -> Result<(), PlatformError> {
    job(manager, "StopUnit", SOCKET_UNIT).await?;
    job(manager, "StopUnit", SERVICE_UNIT).await?;
    if action == PrivilegedHelperLifecycleAction::Remove {
        disable(manager).await?;
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
        return Ok(());
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
    enable(manager, false).await?;
    job(manager, "StartUnit", SOCKET_UNIT).await?;
    let status = wait_for_ready(Utf8Path::new(HELPER_SOCKET_PATH), owner_uid).await?;
    require_identity(
        &status,
        owner_uid,
        PRIVILEGED_HELPER_VERSION,
        HELPER_PROTOCOL_VERSION,
    )
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
    // Failed units survive file removal and Reload. Clear only newly introduced
    // stopped units, while their marked files and selected fragments still prove ownership.
    if enablement_attempted && errors.is_empty() {
        for ((unit, path), checksum) in [
            (SOCKET_UNIT, SOCKET_UNIT_PATH),
            (SERVICE_UNIT, SERVICE_UNIT_PATH),
        ]
        .into_iter()
        .zip(&previous.checksums[2..])
        {
            if checksum.is_some() {
                continue;
            }
            if let Err(error) = async {
                if unit_active(manager, unit, path).await? {
                    return Err(installation_error(format!(
                        "helper unit {unit} is still active during rollback"
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
            .await
            {
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
    if let Some(runtime) = previous.enabled
        && let Err(error) = enable(manager, runtime).await
    {
        errors.push(error.to_string());
    }
    if previous.socket_active
        && let Err(error) = job(manager, "StartUnit", SOCKET_UNIT).await
    {
        errors.push(error.to_string());
    }
    if previous.service_active
        && let Err(error) = job(manager, "StartUnit", SERVICE_UNIT).await
    {
        errors.push(error.to_string());
    }
    if (previous.socket_active || previous.service_active)
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

async fn manager() -> Result<Proxy<'static>, PlatformError> {
    within(Duration::from_secs(10), async {
        // Root lifecycle always uses the real system bus, never a caller-supplied address.
        let connection = connection::Builder::address("unix:path=/run/dbus/system_bus_socket")
            .map_err(bus_error)?
            .build()
            .await
            .map_err(bus_error)?;
        let proxy = native_proxy(
            &connection,
            OwnedObjectPath::try_from("/org/freedesktop/systemd1")
                .map_err(zbus::Error::from)
                .map_err(bus_error)?,
            "org.freedesktop.systemd1.Manager",
        )
        .await
        .map_err(bus_error)?;
        proxy
            .get_property::<String>("Version")
            .await
            .map_err(bus_error)?;
        proxy
            .call::<_, _, ()>("Subscribe", &())
            .await
            .map_err(bus_error)?;
        Ok(proxy)
    })
    .await
    .ok_or_else(|| installation_error("systemd system manager connection timed out"))?
}

async fn inspect_previous(
    manager: &Proxy<'_>,
    metadata: Option<PrivilegedHelperMetadata>,
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
    let socket_active = unit_active(manager, SOCKET_UNIT, SOCKET_UNIT_PATH).await?;
    let service_active = unit_active(manager, SERVICE_UNIT, SERVICE_UNIT_PATH).await?;
    let enabled = if checksums[2].is_some() {
        let state: String = bounded_call(manager, "GetUnitFileState", &(SOCKET_UNIT,)).await?;
        match state.as_str() {
            "enabled" => Some(false),
            "enabled-runtime" => Some(true),
            "disabled" | "static" => None,
            _ => {
                return Err(installation_error(format!(
                    "unexpected helper socket enablement state: {state}"
                )));
            }
        }
    } else {
        None
    };
    Ok(PreviousState {
        checksums,
        metadata,
        socket_active,
        service_active,
        enabled,
    })
}

async fn unit_active(
    manager: &Proxy<'_>,
    name: &str,
    expected_path: &str,
) -> Result<bool, PlatformError> {
    within(Duration::from_secs(10), async {
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
    within(Duration::from_secs(10), manager.call(method, body))
        .await
        .ok_or_else(|| installation_error(format!("systemd {method} timed out")))?
        .map_err(bus_error)
}

async fn reload(manager: &Proxy<'_>) -> Result<(), PlatformError> {
    bounded_call(manager, "Reload", &()).await
}
async fn enable(manager: &Proxy<'_>, runtime: bool) -> Result<(), PlatformError> {
    let _: (bool, Vec<(String, String, String)>) = bounded_call(
        manager,
        "EnableUnitFiles",
        &([SOCKET_UNIT].as_slice(), runtime, false),
    )
    .await?;
    Ok(())
}
async fn disable(manager: &Proxy<'_>) -> Result<(), PlatformError> {
    for runtime in [false, true] {
        let _: Vec<(String, String, String)> = bounded_call(
            manager,
            "DisableUnitFiles",
            &([SOCKET_UNIT].as_slice(), runtime),
        )
        .await?;
    }
    Ok(())
}

async fn job(manager: &Proxy<'_>, operation: &str, unit: &str) -> Result<(), PlatformError> {
    within(Duration::from_secs(15), async {
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
    let result = within(HELPER_IO_TIMEOUT, async {
        loop {
            match Async::<UnixStream>::connect(socket).await {
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
                    async_io::Timer::after(Duration::from_millis(25)).await;
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
                std::fs::create_dir(directory).map_err(PlatformError::PrivilegedHelperIo)?;
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
    file.sync_all().map_err(PlatformError::PrivilegedHelperIo)?;
    state::fs::sync_parent_directory(path).map_err(|error| installation_error(error.to_string()))
}

fn rename(source: &Utf8Path, destination: &Utf8Path) -> Result<(), PlatformError> {
    state::fs::rename(source, destination)
        .and_then(|()| state::fs::sync_parent_directory(destination))
        .map_err(|error| installation_error(error.to_string()))
}
fn remove_file(path: &Utf8Path) -> Result<(), PlatformError> {
    state::fs::remove_file_if_exists(path)
        .and_then(|()| state::fs::sync_parent_directory(path))
        .map_err(|error| installation_error(error.to_string()))
}
fn clear_transaction_files() -> Result<(), PlatformError> {
    for artifact in ARTIFACTS {
        for suffix in ["candidate", "rollback"] {
            let path = sibling(artifact.path, suffix);
            if read_root_file(&path, artifact.mode)?.is_some() {
                remove_file(&path)?;
            }
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
    state::fs::sync_parent_directory(path).map_err(|error| installation_error(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixListener;
    use std::time::Instant;

    use anyhow::{anyhow, ensure};
    use async_io::{Async, Timer};
    use camino_tempfile::tempdir;
    use futures_lite::future::zip;
    use futures_lite::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use insta::assert_snapshot;
    use rustix::fs::{CWD, Mode, chmod, chown, link, mkfifoat};
    use rustix::net::sockopt::socket_peercred;
    use rustix::process::{Uid, geteuid, getuid};

    use super::{
        HELPER_IO_TIMEOUT, HELPER_LIFECYCLE_PROBE, PlatformError, SERVICE, acquire_lock,
        checksum_bytes, ensure_root_directory, read_root_file, remove_file, rename, restore_file,
        sibling, socket_unit, validate_directory_chain, wait_for_ready, within, write_root_file,
    };

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

    #[test]
    #[ignore = "run as root in an ordinary disposable Linux container; uses only temporary files"]
    fn root_files_preserve_modes_lock_identity_and_verified_rollback() -> anyhow::Result<()> {
        ensure!(getuid().is_root() && geteuid().is_root(), "requires root");
        let directory = camino_tempfile::tempdir_in("/run")?;
        ensure_root_directory(directory.path(), 0o700)?;
        let path = directory.path().join("helper");
        let backup = sibling(path.as_str(), "rollback");
        let checksum = checksum_bytes(b"original");
        let staged = sibling(path.as_str(), "candidate");
        write_root_file(&staged, b"other foreground candidate", 0o755)?;

        write_root_file(&path, b"original", 0o755)?;
        assert_eq!(read_root_file(&path, 0o755)?, Some(b"original".to_vec()));
        assert_eq!(
            read_root_file(&staged, 0o755)?,
            Some(b"other foreground candidate".to_vec())
        );
        assert!(write_root_file(&path, b"cannot overwrite", 0o755).is_err());
        assert_eq!(read_root_file(&path, 0o755)?, Some(b"original".to_vec()));
        assert!(read_root_file(&path, 0o600).is_err());
        let symbolic_link = directory.path().join("symbolic-link");
        state::fs::symlink_file(&path, &symbolic_link)?;
        assert!(read_root_file(&symbolic_link, 0o755).is_err());
        restore_file(&path, 0o755, Some(&checksum))?;
        rename(&path, &backup)?;
        write_root_file(&path, b"replacement", 0o755)?;
        restore_file(&path, 0o755, Some(&checksum))?;
        assert_eq!(read_root_file(&path, 0o755)?, Some(b"original".to_vec()));
        assert!(!state::fs::path_entry_exists(&backup)?);

        rename(&path, &backup)?;
        write_root_file(&path, b"replacement", 0o755)?;
        assert!(restore_file(&path, 0o755, Some("wrong checksum")).is_err());
        assert_eq!(read_root_file(&backup, 0o755)?, Some(b"original".to_vec()));
        assert_eq!(read_root_file(&path, 0o755)?, Some(b"replacement".to_vec()));
        remove_file(&backup)?;
        assert!(restore_file(&path, 0o755, Some(&checksum)).is_err());
        assert_eq!(read_root_file(&path, 0o755)?, Some(b"replacement".to_vec()));
        remove_file(&path)?;
        assert!(restore_file(&path, 0o755, Some(&checksum)).is_err());

        write_root_file(&path, b"new install", 0o600)?;
        restore_file(&path, 0o600, None)?;
        assert!(read_root_file(&path, 0o600)?.is_none());
        write_root_file(&path, b"one link", 0o600)?;
        let other_link = directory.path().join("other-link");
        link(path.as_std_path(), other_link.as_std_path())?;
        assert!(read_root_file(&path, 0o600).is_err());
        remove_file(&other_link)?;
        assert!(read_root_file(&path, 0o600)?.is_some());

        let lock = directory.path().join("lifecycle.lock");
        let first = acquire_lock(&lock)?;
        assert!(acquire_lock(&lock).is_err());
        drop(first);
        let _next = acquire_lock(&lock)?;
        let writable = directory.path().join("writable");
        ensure_root_directory(&writable, 0o700)?;
        chmod(writable.as_std_path(), Mode::from_bits_truncate(0o777))?;
        assert!(validate_directory_chain(&writable, false).is_err());
        Ok(())
    }

    #[test]
    #[ignore = "run as root in an ordinary disposable Linux container; switches only this thread's effective UID"]
    fn root_readiness_authenticates_as_owner_and_restores_root_on_every_result()
    -> anyhow::Result<()> {
        ensure!(getuid().is_root() && geteuid().is_root(), "requires root");
        let directory = camino_tempfile::tempdir_in("/run")?;
        ensure_root_directory(directory.path(), 0o755)?;
        let socket = directory.path().join("helper.sock");
        let owner_uid = 1000;
        for response in [
            Some(b"PV-HELPER-LIFECYCLE-READY\t2.0.0\t2\t1000\n".as_slice()),
            Some(b"invalid\n".as_slice()),
            None,
        ] {
            let listener = Async::new(UnixListener::bind(&socket)?)?;
            chown(socket.as_std_path(), Some(Uid::from_raw(owner_uid)), None)?;
            let started = Instant::now();
            let server = within(
                HELPER_IO_TIMEOUT + std::time::Duration::from_secs(1),
                async {
                    let (mut stream, _) = listener.accept().await?;
                    let peer_uid = socket_peercred(stream.get_ref())?.uid.as_raw();
                    let mut probe = vec![0; HELPER_LIFECYCLE_PROBE.len()];
                    stream.read_exact(&mut probe).await?;
                    ensure!(
                        probe == HELPER_LIFECYCLE_PROBE,
                        "unexpected lifecycle probe"
                    );
                    if let Some(response) = response {
                        stream.write_all(response).await?;
                    } else {
                        Timer::after(HELPER_IO_TIMEOUT + std::time::Duration::from_millis(100))
                            .await;
                    }
                    Ok::<_, anyhow::Error>(peer_uid)
                },
            );
            let (result, observed_uid) =
                async_io::block_on(zip(wait_for_ready(&socket, owner_uid), server));
            assert!(
                geteuid().is_root(),
                "lifecycle must restore root before returning"
            );
            assert_eq!(
                observed_uid.ok_or_else(|| anyhow!("bounded fixture did not finish"))??,
                owner_uid
            );
            match response {
                Some(response) if response.starts_with(b"PV-HELPER-LIFECYCLE-READY") => {
                    let status = result?;
                    assert_eq!(status.owner_uid, owner_uid);
                    assert_eq!(status.version, "2.0.0");
                    assert_eq!(status.protocol_version, 2);
                }
                Some(_) => assert!(matches!(
                    result,
                    Err(PlatformError::PrivilegedHelperInstallation(_))
                )),
                None => {
                    assert!(matches!(
                        result,
                        Err(PlatformError::PrivilegedHelperUnavailable)
                    ));
                    assert!(started.elapsed() >= HELPER_IO_TIMEOUT);
                }
            }
            drop(listener);
            remove_file(&socket)?;
        }
        Ok(())
    }
}
