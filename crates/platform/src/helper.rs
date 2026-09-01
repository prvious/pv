#[cfg(target_os = "macos")]
use std::collections::BTreeMap;
#[cfg(any(target_os = "macos", test))]
use std::io::Read;
#[cfg(any(target_os = "macos", all(test, unix)))]
use std::io::Write;
#[cfg(target_os = "macos")]
use std::time::Duration;

use camino::Utf8Path;
#[cfg(target_os = "macos")]
use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};
#[cfg(target_os = "macos")]
use sha2::{Digest, Sha256};

use crate::{
    ActivePfRedirectInspection, KeychainCertificate, PfRedirectConfig, PlatformError,
    ResolverConfig, ResolverFileState,
};

pub const HELPER_PROTOCOL_VERSION: u32 = 1;
pub const PRIVILEGED_HELPER_VERSION: &str = "1.0.0";
pub const HELPER_EXECUTABLE_PATH: &str = "/Library/PrivilegedHelperTools/com.prvious.pv.helper";
pub const HELPER_LAUNCH_DAEMON_PATH: &str = "/Library/LaunchDaemons/com.prvious.pv.helper.plist";
pub const HELPER_METADATA_PATH: &str = "/Library/Application Support/PV/helper.json";
pub const HELPER_SOCKET_PATH: &str = "/var/run/com.prvious.pv.helper.sock";

#[cfg(target_os = "macos")]
const HELPER_SOCKET_NAME: &str = "Control";
#[cfg(target_os = "macos")]
const HELPER_LABEL: &str = "com.prvious.pv.helper";
#[cfg(target_os = "macos")]
const HELPER_STANDARD_ERROR_PATH: &str = "/var/log/com.prvious.pv.helper.err.log";
#[cfg(target_os = "macos")]
const HELPER_SUPPORT_DIRECTORY: &str = "/Library/Application Support/PV";
#[cfg(target_os = "macos")]
const HELPER_CA_CANDIDATE_PATH: &str = "/Library/Application Support/PV/ca.pem";
#[cfg(target_os = "macos")]
const HELPER_EXECUTABLE_CANDIDATE_PATH: &str =
    "/Library/PrivilegedHelperTools/.com.prvious.pv.helper.candidate";
#[cfg(target_os = "macos")]
const HELPER_METADATA_CANDIDATE_PATH: &str =
    "/Library/Application Support/PV/.helper.json.candidate";
#[cfg(target_os = "macos")]
const HELPER_PLIST_CANDIDATE_PATH: &str =
    "/Library/LaunchDaemons/.com.prvious.pv.helper.candidate.plist";
#[cfg(target_os = "macos")]
const HELPER_EXECUTABLE_ROLLBACK_PATH: &str =
    "/Library/PrivilegedHelperTools/.com.prvious.pv.helper.rollback";
#[cfg(target_os = "macos")]
const HELPER_LIFECYCLE_LOCK_PATH: &str =
    "/Library/PrivilegedHelperTools/.com.prvious.pv.helper.lifecycle.lock";
#[cfg(target_os = "macos")]
const HELPER_METADATA_ROLLBACK_PATH: &str = "/Library/Application Support/PV/.helper.json.rollback";
#[cfg(target_os = "macos")]
const HELPER_PLIST_ROLLBACK_PATH: &str =
    "/Library/LaunchDaemons/.com.prvious.pv.helper.rollback.plist";
#[cfg(target_os = "macos")]
const HELPER_PLIST_MARKER: &str = "<!-- Managed by PV -->";
#[cfg(target_os = "macos")]
const HELPER_LIFECYCLE_PROBE: &[u8] = b"PV-HELPER-LIFECYCLE-PROBE\n";
#[cfg(any(target_os = "macos", test))]
const HELPER_LIFECYCLE_READY_PREFIX: &str = "PV-HELPER-LIFECYCLE-READY";
#[cfg(any(target_os = "macos", test))]
const MAX_MESSAGE_BYTES: usize = 16 * 1024;
#[cfg(target_os = "macos")]
const HELPER_IO_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PrivilegedHelperStatus {
    pub version: String,
    pub protocol_version: u32,
    pub owner_uid: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrivilegedHelperInstallOutcome {
    status: PrivilegedHelperStatus,
    cleanup_warning: Option<String>,
}

impl PrivilegedHelperInstallOutcome {
    pub fn successful(status: PrivilegedHelperStatus) -> Self {
        Self {
            status,
            cleanup_warning: None,
        }
    }

    pub fn with_cleanup_warning(mut self, warning: impl Into<String>) -> Self {
        self.cleanup_warning = Some(warning.into());
        self
    }

    pub const fn status(&self) -> &PrivilegedHelperStatus {
        &self.status
    }

    pub fn cleanup_warning(&self) -> Option<&str> {
        self.cleanup_warning.as_deref()
    }
}

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PrivilegedHelperMetadata {
    pub owner_uid: u32,
    pub helper_version: String,
    pub protocol_version: u32,
}

#[cfg(target_os = "macos")]
struct InstalledHelperState {
    executable: bool,
    metadata: bool,
    plist: bool,
    was_loaded: bool,
    previous_status: Option<PrivilegedHelperStatus>,
    previous_probe_error: Option<String>,
}

#[cfg(target_os = "macos")]
#[expect(
    clippy::disallowed_types,
    reason = "machine-wide helper lifecycle lock guard owns the OS-locked file handle"
)]
struct MachineHelperLifecycleLock {
    _file: std::fs::File,
}

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct HelperRequest {
    protocol_version: u32,
    operation: HelperOperation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "name",
    content = "arguments",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum HelperOperation {
    Status,
    DnsInspect { expected_port: Option<u16> },
    DnsApply { port: u16 },
    DnsRemove,
    PfInspect,
    PfApply { http_port: u16, https_port: u16 },
    PfReload,
    PfRemove,
    CaInspect,
    CaApply { fingerprint: String },
    CaRemove { fingerprint: String },
}

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct HelperResponse {
    protocol_version: u32,
    outcome: HelperOutcome,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum HelperOutcome {
    Success {
        payload: HelperPayload,
    },
    Error {
        code: HelperErrorCode,
        message: String,
    },
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum HelperErrorCode {
    AuthenticationFailed,
    ProtocolMismatch,
    InvalidRequest,
    SystemIntegrationFailed,
    InternalError,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
enum HelperPayload {
    Empty,
    Status(PrivilegedHelperStatus),
    ResolverState(ResolverFileState),
    PfInspection(ActivePfRedirectInspection),
    CaCertificates(Vec<KeychainCertificate>),
}

#[derive(Clone, Debug)]
pub struct PrivilegedHelperClient;

impl Default for PrivilegedHelperClient {
    fn default() -> Self {
        Self
    }
}

impl PrivilegedHelperClient {
    pub fn status(&self) -> Result<PrivilegedHelperStatus, PlatformError> {
        match self.call(HelperOperation::Status)? {
            HelperPayload::Status(status) => Ok(status),
            payload => Err(unexpected_payload("status", &payload)),
        }
    }

    pub fn inspect_dns(
        &self,
        expected: Option<&ResolverConfig>,
    ) -> Result<ResolverFileState, PlatformError> {
        let expected_port = expected.map(|config| config.port);
        match self.call(HelperOperation::DnsInspect { expected_port })? {
            HelperPayload::ResolverState(state) => Ok(state),
            payload => Err(unexpected_payload("DNS inspection", &payload)),
        }
    }

    pub fn apply_dns(&self, config: &ResolverConfig) -> Result<(), PlatformError> {
        expect_empty(self.call(HelperOperation::DnsApply { port: config.port })?)
    }

    pub fn remove_dns(&self) -> Result<(), PlatformError> {
        expect_empty(self.call(HelperOperation::DnsRemove)?)
    }

    pub fn inspect_pf(&self) -> Result<ActivePfRedirectInspection, PlatformError> {
        match self.call(HelperOperation::PfInspect)? {
            HelperPayload::PfInspection(inspection) => Ok(inspection),
            payload => Err(unexpected_payload("PF inspection", &payload)),
        }
    }

    pub fn apply_pf(&self, config: &PfRedirectConfig) -> Result<(), PlatformError> {
        expect_empty(self.call(HelperOperation::PfApply {
            http_port: config.http_port,
            https_port: config.https_port,
        })?)
    }

    pub fn reload_pf(&self) -> Result<(), PlatformError> {
        expect_empty(self.call(HelperOperation::PfReload)?)
    }

    pub fn remove_pf(&self) -> Result<(), PlatformError> {
        expect_empty(self.call(HelperOperation::PfRemove)?)
    }

    pub fn inspect_ca(&self) -> Result<Vec<KeychainCertificate>, PlatformError> {
        match self.call(HelperOperation::CaInspect)? {
            HelperPayload::CaCertificates(certificates) => Ok(certificates),
            payload => Err(unexpected_payload("CA inspection", &payload)),
        }
    }

    pub fn apply_ca(&self, fingerprint: &str) -> Result<(), PlatformError> {
        validate_fingerprint(fingerprint)?;
        expect_empty(self.call(HelperOperation::CaApply {
            fingerprint: fingerprint.to_string(),
        })?)
    }

    pub fn remove_ca(&self, fingerprint: &str) -> Result<(), PlatformError> {
        validate_fingerprint(fingerprint)?;
        expect_empty(self.call(HelperOperation::CaRemove {
            fingerprint: fingerprint.to_string(),
        })?)
    }

    fn call(&self, operation: HelperOperation) -> Result<HelperPayload, PlatformError> {
        call_helper(Utf8Path::new(HELPER_SOCKET_PATH), operation)
    }
}

pub fn install_privileged_helper(
    candidate_path: &Utf8Path,
    prepared_directory: &Utf8Path,
    expected_sha256: &str,
    helper_version: &str,
    protocol_version: u32,
) -> Result<PrivilegedHelperInstallOutcome, PlatformError> {
    #[cfg(target_os = "macos")]
    {
        install_privileged_helper_macos(
            candidate_path,
            prepared_directory,
            expected_sha256,
            helper_version,
            protocol_version,
        )
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (
            candidate_path,
            prepared_directory,
            expected_sha256,
            helper_version,
            protocol_version,
        );
        Err(crate::capability::unsupported(
            crate::PlatformCapability::PrivilegedHelper,
        )?)
    }
}

pub fn remove_privileged_helper() -> Result<(), PlatformError> {
    #[cfg(target_os = "macos")]
    {
        let owner_uid = rustix::process::getuid().as_raw();
        if owner_uid == 0 {
            return Err(PlatformError::PrivilegedHelperInstallation(
                "helper removal must be requested by the supported non-root account".to_string(),
            ));
        }
        if !installed_helper_artifacts_present()? {
            return Ok(());
        }
        let _machine_lifecycle_lock = acquire_machine_helper_lifecycle_lock()?;
        validate_existing_helper_owner(owner_uid)?;
        validate_helper_support_directory_for_removal()?;
        let _was_loaded = bootout_helper_if_loaded()?;
        run_sudo(&[
            "/bin/rm",
            "-f",
            HELPER_EXECUTABLE_PATH,
            HELPER_LAUNCH_DAEMON_PATH,
            HELPER_SOCKET_PATH,
            HELPER_STANDARD_ERROR_PATH,
            HELPER_EXECUTABLE_CANDIDATE_PATH,
            HELPER_PLIST_CANDIDATE_PATH,
            HELPER_EXECUTABLE_ROLLBACK_PATH,
            HELPER_PLIST_ROLLBACK_PATH,
        ])?;
        run_sudo(&["/bin/rm", "-rf", HELPER_SUPPORT_DIRECTORY])
    }

    #[cfg(not(target_os = "macos"))]
    {
        Err(crate::capability::unsupported(
            crate::PlatformCapability::PrivilegedHelper,
        )?)
    }
}

#[cfg(target_os = "macos")]
fn install_privileged_helper_macos(
    candidate_path: &Utf8Path,
    prepared_directory: &Utf8Path,
    expected_sha256: &str,
    helper_version: &str,
    protocol_version: u32,
) -> Result<PrivilegedHelperInstallOutcome, PlatformError> {
    validate_helper_candidate(candidate_path, expected_sha256)?;
    validate_helper_identity(helper_version, protocol_version)?;
    let owner_uid = rustix::process::getuid().as_raw();
    let owner_gid = rustix::process::getgid().as_raw();
    if owner_uid == 0 {
        return Err(PlatformError::PrivilegedHelperInstallation(
            "helper installation must be requested by the supported non-root account".to_string(),
        ));
    }
    let _machine_lifecycle_lock = acquire_machine_helper_lifecycle_lock()?;
    validate_existing_helper_owner(owner_uid)?;

    let metadata = PrivilegedHelperMetadata {
        owner_uid,
        helper_version: helper_version.to_string(),
        protocol_version,
    };
    let prepared_metadata_path = prepared_directory.join("helper.json");
    let prepared_plist_path = prepared_directory.join("com.prvious.pv.helper.plist");
    let metadata_json = serde_json::to_string_pretty(&metadata)?;
    state::fs::write_sensitive_file(&prepared_metadata_path, &format!("{metadata_json}\n"))
        .map_err(|error| PlatformError::PrivilegedHelperInstallation(error.to_string()))?;
    state::fs::write_sensitive_file(
        &prepared_plist_path,
        &render_launch_daemon_plist(owner_uid, owner_gid)?,
    )
    .map_err(|error| PlatformError::PrivilegedHelperInstallation(error.to_string()))?;

    run_sudo(&[
        "/usr/bin/install",
        "-d",
        "-o",
        "root",
        "-g",
        "wheel",
        "-m",
        "0755",
        "/Library/PrivilegedHelperTools",
        "/Library/LaunchDaemons",
        HELPER_SUPPORT_DIRECTORY,
    ])?;
    run_sudo(&[
        "/usr/bin/install",
        "-o",
        "root",
        "-g",
        "wheel",
        "-m",
        "0755",
        candidate_path.as_str(),
        HELPER_EXECUTABLE_CANDIDATE_PATH,
    ])?;
    run_sudo(&[
        "/usr/bin/install",
        "-o",
        "root",
        "-g",
        "wheel",
        "-m",
        "0644",
        prepared_metadata_path.as_str(),
        HELPER_METADATA_CANDIDATE_PATH,
    ])?;
    run_sudo(&[
        "/usr/bin/install",
        "-o",
        "root",
        "-g",
        "wheel",
        "-m",
        "0644",
        prepared_plist_path.as_str(),
        HELPER_PLIST_CANDIDATE_PATH,
    ])?;
    validate_root_staged_helper(
        expected_sha256,
        &format!("{metadata_json}\n"),
        &render_launch_daemon_plist(owner_uid, owner_gid)?,
    )?;
    ensure_no_retained_helper_rollback()?;
    let mut installed_state = inspect_installed_helper_state()?;
    if helper_is_loaded()? {
        installed_state.was_loaded = true;
        match probe_helper_lifecycle(Utf8Path::new(HELPER_SOCKET_PATH)) {
            Ok(status) => installed_state.previous_status = Some(status),
            Err(error) => installed_state.previous_probe_error = Some(error.to_string()),
        }
    }
    let _helper_was_loaded = bootout_helper_if_loaded()?;
    let install_result = (|| {
        move_installed_helper_to_rollback(&installed_state)?;
        move_helper_candidates_into_place()?;
        run_sudo(&[
            "/bin/launchctl",
            "bootstrap",
            "system",
            HELPER_LAUNCH_DAEMON_PATH,
        ])?;
        let status = probe_helper_lifecycle(Utf8Path::new(HELPER_SOCKET_PATH))?;
        if status.version != helper_version
            || status.protocol_version != protocol_version
            || status.owner_uid != owner_uid
        {
            return Err(PlatformError::PrivilegedHelperInstallation(format!(
                "installed helper identity did not match requested version {helper_version}, protocol {protocol_version}, owner UID {owner_uid}: {status:?}"
            )));
        }

        Ok(status)
    })();
    match install_result {
        Ok(status) => Ok(PrivilegedHelperInstallOutcome {
            status,
            cleanup_warning: clear_helper_transaction_files().err().map(|error| {
                format!(
                    "installed privileged helper but could not remove lifecycle transaction files: {error}"
                )
            }),
        }),
        Err(error) => Err(rollback_helper_installation(&installed_state, error)),
    }
}

#[cfg(target_os = "macos")]
fn acquire_machine_helper_lifecycle_lock() -> Result<MachineHelperLifecycleLock, PlatformError> {
    run_sudo(&[
        "/usr/bin/install",
        "-d",
        "-o",
        "root",
        "-g",
        "wheel",
        "-m",
        "0755",
        "/Library/PrivilegedHelperTools",
    ])?;
    run_sudo(&["/usr/bin/touch", HELPER_LIFECYCLE_LOCK_PATH])?;
    run_sudo(&["/usr/sbin/chown", "root:wheel", HELPER_LIFECYCLE_LOCK_PATH])?;
    run_sudo(&["/bin/chmod", "0644", HELPER_LIFECYCLE_LOCK_PATH])?;
    validate_root_owned_regular_file(Utf8Path::new(HELPER_LIFECYCLE_LOCK_PATH), 0o644)?;
    lock_machine_helper_lifecycle_file(Utf8Path::new(HELPER_LIFECYCLE_LOCK_PATH))
}

#[cfg(target_os = "macos")]
#[expect(
    clippy::disallowed_types,
    reason = "machine-wide helper lifecycle coordination opens and owns a fixed root-owned lock file"
)]
fn lock_machine_helper_lifecycle_file(
    path: &Utf8Path,
) -> Result<MachineHelperLifecycleLock, PlatformError> {
    let file = std::fs::File::open(path).map_err(|error| {
        PlatformError::PrivilegedHelperInstallation(format!(
            "could not open machine-wide helper lifecycle lock {path}: {error}"
        ))
    })?;
    match rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(MachineHelperLifecycleLock { _file: file }),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            Err(PlatformError::PrivilegedHelperInstallation(
                "another privileged-helper lifecycle operation is already in progress".to_string(),
            ))
        }
        Err(error) => Err(PlatformError::PrivilegedHelperInstallation(format!(
            "could not lock machine-wide helper lifecycle file {path}: {error}"
        ))),
    }
}

#[cfg(target_os = "macos")]
fn inspect_installed_helper_state() -> Result<InstalledHelperState, PlatformError> {
    Ok(InstalledHelperState {
        executable: root_owned_regular_file_present(Utf8Path::new(HELPER_EXECUTABLE_PATH), 0o755)?,
        metadata: root_owned_regular_file_present(Utf8Path::new(HELPER_METADATA_PATH), 0o644)?,
        plist: root_owned_regular_file_present(Utf8Path::new(HELPER_LAUNCH_DAEMON_PATH), 0o644)?,
        was_loaded: false,
        previous_status: None,
        previous_probe_error: None,
    })
}

#[cfg(target_os = "macos")]
fn move_installed_helper_to_rollback(
    installed_state: &InstalledHelperState,
) -> Result<(), PlatformError> {
    for (exists, source, destination) in [
        (
            installed_state.executable,
            HELPER_EXECUTABLE_PATH,
            HELPER_EXECUTABLE_ROLLBACK_PATH,
        ),
        (
            installed_state.metadata,
            HELPER_METADATA_PATH,
            HELPER_METADATA_ROLLBACK_PATH,
        ),
        (
            installed_state.plist,
            HELPER_LAUNCH_DAEMON_PATH,
            HELPER_PLIST_ROLLBACK_PATH,
        ),
    ] {
        if exists {
            run_sudo(&["/bin/mv", source, destination])?;
        }
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn move_helper_candidates_into_place() -> Result<(), PlatformError> {
    for (source, destination) in [
        (HELPER_METADATA_CANDIDATE_PATH, HELPER_METADATA_PATH),
        (HELPER_PLIST_CANDIDATE_PATH, HELPER_LAUNCH_DAEMON_PATH),
        (HELPER_EXECUTABLE_CANDIDATE_PATH, HELPER_EXECUTABLE_PATH),
    ] {
        run_sudo(&["/bin/mv", source, destination])?;
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn rollback_helper_installation(
    installed_state: &InstalledHelperState,
    install_error: PlatformError,
) -> PlatformError {
    let mut rollback_errors = Vec::new();
    if let Err(error) = bootout_helper_if_loaded() {
        rollback_errors.push(error.to_string());
    }
    restore_helper_file(
        installed_state.executable,
        HELPER_EXECUTABLE_PATH,
        HELPER_EXECUTABLE_ROLLBACK_PATH,
        0o755,
        &mut rollback_errors,
    );
    restore_helper_file(
        installed_state.metadata,
        HELPER_METADATA_PATH,
        HELPER_METADATA_ROLLBACK_PATH,
        0o644,
        &mut rollback_errors,
    );
    restore_helper_file(
        installed_state.plist,
        HELPER_LAUNCH_DAEMON_PATH,
        HELPER_PLIST_ROLLBACK_PATH,
        0o644,
        &mut rollback_errors,
    );
    if installed_state.was_loaded {
        if let Err(error) = run_sudo(&[
            "/bin/launchctl",
            "bootstrap",
            "system",
            HELPER_LAUNCH_DAEMON_PATH,
        ]) {
            rollback_errors.push(error.to_string());
        } else if let Some(expected_status) = &installed_state.previous_status {
            match probe_helper_lifecycle(Utf8Path::new(HELPER_SOCKET_PATH)) {
                Ok(status) if status == *expected_status => {}
                Ok(status) => rollback_errors.push(format!(
                    "restored helper identity {status:?} did not match previous identity {expected_status:?}"
                )),
                Err(error) => rollback_errors.push(format!(
                    "restored helper did not become ready: {error}"
                )),
            }
        }
    }
    if rollback_errors.is_empty()
        && let Err(error) = clear_helper_transaction_files()
    {
        rollback_errors.push(error.to_string());
    }

    if rollback_errors.is_empty() {
        let previous_health = installed_state
            .previous_probe_error
            .as_ref()
            .map(|error| {
                format!("; the previous helper was not lifecycle-ready before replacement: {error}")
            })
            .unwrap_or_default();
        PlatformError::PrivilegedHelperInstallation(format!(
            "{install_error}; restored the previous privileged helper installation{previous_health}"
        ))
    } else {
        PlatformError::PrivilegedHelperInstallation(format!(
            "{install_error}; privileged helper rollback also failed: {}; retained any remaining transaction files at the fixed .candidate and .rollback paths",
            rollback_errors.join("; ")
        ))
    }
}

#[cfg(target_os = "macos")]
fn restore_helper_file(
    existed: bool,
    destination: &str,
    rollback_path: &str,
    mode: u32,
    errors: &mut Vec<String>,
) {
    let rollback_exists = match root_owned_regular_file_present(Utf8Path::new(rollback_path), mode)
    {
        Ok(exists) => exists,
        Err(error) => {
            errors.push(error.to_string());
            return;
        }
    };
    let result = if rollback_exists {
        run_sudo(&["/bin/mv", "-f", rollback_path, destination])
    } else if existed {
        Ok(())
    } else {
        run_sudo(&["/bin/rm", "-f", destination])
    };
    if let Err(error) = result {
        errors.push(error.to_string());
    }
}

#[cfg(target_os = "macos")]
fn ensure_no_retained_helper_rollback() -> Result<(), PlatformError> {
    for (path, mode) in [
        (HELPER_EXECUTABLE_ROLLBACK_PATH, 0o755),
        (HELPER_METADATA_ROLLBACK_PATH, 0o644),
        (HELPER_PLIST_ROLLBACK_PATH, 0o644),
    ] {
        if root_owned_regular_file_present(Utf8Path::new(path), mode)? {
            return Err(PlatformError::PrivilegedHelperInstallation(format!(
                "retained privileged-helper rollback file requires administrator recovery before retrying: {path}"
            )));
        }
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn clear_helper_transaction_files() -> Result<(), PlatformError> {
    run_sudo(&[
        "/bin/rm",
        "-f",
        HELPER_EXECUTABLE_CANDIDATE_PATH,
        HELPER_METADATA_CANDIDATE_PATH,
        HELPER_PLIST_CANDIDATE_PATH,
        HELPER_EXECUTABLE_ROLLBACK_PATH,
        HELPER_METADATA_ROLLBACK_PATH,
        HELPER_PLIST_ROLLBACK_PATH,
    ])
}

#[cfg(target_os = "macos")]
fn validate_root_staged_helper(
    expected_sha256: &str,
    expected_metadata: &str,
    expected_plist: &str,
) -> Result<(), PlatformError> {
    let executable_path = Utf8Path::new(HELPER_EXECUTABLE_CANDIDATE_PATH);
    validate_root_owned_regular_file(executable_path, 0o755)?;
    let actual_sha256 = sha256_file(executable_path)?;
    if actual_sha256 != expected_sha256 {
        return Err(PlatformError::PrivilegedHelperInstallation(format!(
            "root-staged helper checksum mismatch: expected {expected_sha256}, got {actual_sha256}"
        )));
    }
    crate::command::run_system_command(
        "/usr/bin/codesign",
        &["--verify", "--strict", executable_path.as_str()],
    )?;
    validate_root_staged_file(
        Utf8Path::new(HELPER_METADATA_CANDIDATE_PATH),
        expected_metadata,
    )?;
    validate_root_staged_file(Utf8Path::new(HELPER_PLIST_CANDIDATE_PATH), expected_plist)
}

#[cfg(target_os = "macos")]
fn validate_root_staged_file(path: &Utf8Path, expected: &str) -> Result<(), PlatformError> {
    validate_root_owned_regular_file(path, 0o644)?;
    let actual = state::fs::read_to_string(path)
        .map_err(|error| PlatformError::PrivilegedHelperInstallation(error.to_string()))?;
    if actual != expected {
        return Err(PlatformError::PrivilegedHelperInstallation(format!(
            "root-staged helper file changed before installation: {path}"
        )));
    }

    Ok(())
}

#[cfg(target_os = "macos")]
#[expect(
    clippy::disallowed_methods,
    reason = "helper lifecycle inspects fixed root-owned metadata before replacement"
)]
fn validate_existing_helper_owner(owner_uid: u32) -> Result<(), PlatformError> {
    let metadata_path = Utf8Path::new(HELPER_METADATA_PATH);
    match std::fs::symlink_metadata(metadata_path) {
        Ok(_metadata) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let candidate_path = Utf8Path::new(HELPER_METADATA_CANDIDATE_PATH);
            match std::fs::symlink_metadata(candidate_path) {
                Ok(_metadata) => {
                    validate_root_owned_regular_file(candidate_path, 0o644)?;
                    let content = state::fs::read_to_string(candidate_path).map_err(|error| {
                        PlatformError::PrivilegedHelperInstallation(error.to_string())
                    })?;
                    let metadata = serde_json::from_str::<PrivilegedHelperMetadata>(&content)
                        .map_err(|error| {
                            PlatformError::PrivilegedHelperInstallation(format!(
                                "existing helper ownership cannot be determined from its staged metadata: {error}; manual administrator recovery is required"
                            ))
                        })?;

                    return require_matching_helper_owner(&metadata, owner_uid);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(PlatformError::PrivilegedHelperInstallation(format!(
                        "could not inspect existing staged helper metadata: {error}"
                    )));
                }
            }

            return validate_existing_helper_without_metadata(owner_uid);
        }
        Err(error) => {
            return Err(PlatformError::PrivilegedHelperInstallation(format!(
                "could not inspect existing helper metadata: {error}"
            )));
        }
    }
    validate_root_owned_regular_file(metadata_path, 0o644)?;
    let content = state::fs::read_to_string(metadata_path)
        .map_err(|error| PlatformError::PrivilegedHelperInstallation(error.to_string()))?;
    match serde_json::from_str::<PrivilegedHelperMetadata>(&content) {
        Ok(metadata) => require_matching_helper_owner(&metadata, owner_uid),
        Err(_error) => require_matching_plist_owner(owner_uid),
    }
}

#[cfg(target_os = "macos")]
#[expect(
    clippy::disallowed_methods,
    reason = "helper lifecycle inspects fixed root-owned installation files before replacement"
)]
fn validate_existing_helper_without_metadata(owner_uid: u32) -> Result<(), PlatformError> {
    let executable_exists = match std::fs::symlink_metadata(HELPER_EXECUTABLE_PATH) {
        Ok(_metadata) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(PlatformError::PrivilegedHelperInstallation(format!(
                "could not inspect existing helper executable: {error}"
            )));
        }
    };
    let plist_exists = match std::fs::symlink_metadata(HELPER_LAUNCH_DAEMON_PATH) {
        Ok(_metadata) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(PlatformError::PrivilegedHelperInstallation(format!(
                "could not inspect existing helper LaunchDaemon: {error}"
            )));
        }
    };
    if !executable_exists && !plist_exists {
        return Ok(());
    }
    if !plist_exists {
        return Err(PlatformError::PrivilegedHelperInstallation(
            "existing helper ownership cannot be determined; manual administrator recovery is required"
                .to_string(),
        ));
    }

    require_matching_plist_owner(owner_uid)
}

#[cfg(target_os = "macos")]
fn require_matching_plist_owner(owner_uid: u32) -> Result<(), PlatformError> {
    let plist_path = Utf8Path::new(HELPER_LAUNCH_DAEMON_PATH);
    validate_root_owned_regular_file(plist_path, 0o644)?;
    let content = state::fs::read_to_string(plist_path)
        .map_err(|error| PlatformError::PrivilegedHelperInstallation(error.to_string()))?;
    let plist =
        plist::from_bytes::<HelperLaunchDaemonPlist>(content.as_bytes()).map_err(|error| {
            PlatformError::PrivilegedHelperInstallation(format!(
                "existing helper ownership cannot be determined from its LaunchDaemon: {error}"
            ))
        })?;
    let socket = plist.sockets.get(HELPER_SOCKET_NAME).ok_or_else(|| {
        PlatformError::PrivilegedHelperInstallation(
            "existing helper LaunchDaemon does not define its control socket".to_string(),
        )
    })?;
    if socket.owner_uid != owner_uid {
        return Err(PlatformError::PrivilegedHelperInstallation(format!(
            "helper belongs to installing UID {}; uninstall it from that account before setting up UID {owner_uid}",
            socket.owner_uid
        )));
    }

    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn require_matching_helper_owner(
    metadata: &PrivilegedHelperMetadata,
    owner_uid: u32,
) -> Result<(), PlatformError> {
    if metadata.owner_uid == owner_uid {
        return Ok(());
    }

    Err(PlatformError::PrivilegedHelperInstallation(format!(
        "helper belongs to installing UID {}; uninstall it from that account before setting up UID {owner_uid}",
        metadata.owner_uid
    )))
}

#[cfg(any(target_os = "macos", test))]
fn validate_helper_identity(
    helper_version: &str,
    protocol_version: u32,
) -> Result<(), PlatformError> {
    let components = helper_version.split('.').collect::<Vec<_>>();
    let valid_version = components.len() == 3
        && components.iter().all(|component| {
            !component.is_empty()
                && component.bytes().all(|byte| byte.is_ascii_digit())
                && (*component == "0" || !component.starts_with('0'))
        });
    if !valid_version || protocol_version == 0 {
        return Err(PlatformError::PrivilegedHelperInstallation(
            "helper version and protocol identity are invalid".to_string(),
        ));
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn validate_helper_candidate(
    candidate_path: &Utf8Path,
    expected_sha256: &str,
) -> Result<(), PlatformError> {
    validate_regular_file_owned_by(candidate_path, rustix::process::getuid().as_raw())?;
    validate_fingerprint(expected_sha256)?;
    let actual_sha256 = sha256_file(candidate_path)?;
    if actual_sha256 != expected_sha256 {
        return Err(PlatformError::PrivilegedHelperInstallation(format!(
            "helper checksum mismatch: expected {expected_sha256}, got {actual_sha256}"
        )));
    }
    crate::command::run_system_command(
        "/usr/bin/codesign",
        &["--verify", "--strict", candidate_path.as_str()],
    )
}

#[cfg(target_os = "macos")]
#[derive(Deserialize, Serialize)]
struct HelperLaunchDaemonPlist {
    #[serde(rename = "Label")]
    label: String,
    #[serde(rename = "ProgramArguments")]
    program_arguments: Vec<String>,
    #[serde(rename = "Sockets")]
    sockets: BTreeMap<String, HelperSocketPlist>,
    #[serde(rename = "KeepAlive")]
    keep_alive: bool,
    #[serde(rename = "RunAtLoad")]
    run_at_load: bool,
    #[serde(rename = "ProcessType")]
    process_type: String,
    #[serde(rename = "StandardErrorPath")]
    standard_error_path: String,
}

#[cfg(target_os = "macos")]
#[derive(Deserialize, Serialize)]
struct HelperSocketPlist {
    #[serde(rename = "SockPathName")]
    path: String,
    #[serde(rename = "SockPathOwner")]
    owner_uid: u32,
    #[serde(rename = "SockPathGroup")]
    owner_gid: u32,
    #[serde(rename = "SockPathMode")]
    mode: u32,
}

#[cfg(target_os = "macos")]
fn render_launch_daemon_plist(owner_uid: u32, owner_gid: u32) -> Result<String, PlatformError> {
    let plist = HelperLaunchDaemonPlist {
        label: HELPER_LABEL.to_string(),
        program_arguments: vec![HELPER_EXECUTABLE_PATH.to_string()],
        sockets: BTreeMap::from([(
            HELPER_SOCKET_NAME.to_string(),
            HelperSocketPlist {
                path: HELPER_SOCKET_PATH.to_string(),
                owner_uid,
                owner_gid,
                mode: 0o600,
            },
        )]),
        keep_alive: false,
        run_at_load: false,
        process_type: "Interactive".to_string(),
        standard_error_path: HELPER_STANDARD_ERROR_PATH.to_string(),
    };
    let mut content = Vec::new();
    plist::to_writer_xml(&mut content, &plist).map_err(|error| {
        PlatformError::PrivilegedHelperInstallation(format!(
            "could not render LaunchDaemon plist: {error}"
        ))
    })?;
    let content = String::from_utf8(content).map_err(|error| {
        PlatformError::PrivilegedHelperInstallation(format!(
            "LaunchDaemon plist was not UTF-8: {error}"
        ))
    })?;
    let declaration_end = content.find("?>").ok_or_else(|| {
        PlatformError::PrivilegedHelperInstallation(
            "LaunchDaemon plist is missing its XML declaration".to_string(),
        )
    })? + 2;

    Ok(format!(
        "{}\n{HELPER_PLIST_MARKER}{}",
        &content[..declaration_end],
        &content[declaration_end..]
    ))
}

#[cfg(target_os = "macos")]
fn run_sudo(args: &[&str]) -> Result<(), PlatformError> {
    crate::command::run_system_command("/usr/bin/sudo", args)
}

#[cfg(target_os = "macos")]
fn bootout_helper_if_loaded() -> Result<bool, PlatformError> {
    if !helper_is_loaded()? {
        return Ok(false);
    }

    let service_target = format!("system/{HELPER_LABEL}");
    run_sudo(&["/bin/launchctl", "bootout", &service_target])?;
    Ok(true)
}

#[cfg(target_os = "macos")]
fn helper_is_loaded() -> Result<bool, PlatformError> {
    let service_target = format!("system/{HELPER_LABEL}");
    match crate::command::run_system_command_output("/bin/launchctl", &["print", &service_target]) {
        Ok(_output) => Ok(true),
        Err(PlatformError::SystemIntegrationCommandStatus { status, .. })
            if status == "exit status: 113" || status.starts_with("exit status: 113:") =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

#[cfg(target_os = "macos")]
fn installed_helper_artifacts_present() -> Result<bool, PlatformError> {
    helper_artifacts_present(&[
        Utf8Path::new(HELPER_EXECUTABLE_PATH),
        Utf8Path::new(HELPER_LAUNCH_DAEMON_PATH),
        Utf8Path::new(HELPER_METADATA_PATH),
        Utf8Path::new(HELPER_SOCKET_PATH),
        Utf8Path::new(HELPER_STANDARD_ERROR_PATH),
        Utf8Path::new(HELPER_SUPPORT_DIRECTORY),
        Utf8Path::new(HELPER_EXECUTABLE_CANDIDATE_PATH),
        Utf8Path::new(HELPER_PLIST_CANDIDATE_PATH),
        Utf8Path::new(HELPER_EXECUTABLE_ROLLBACK_PATH),
        Utf8Path::new(HELPER_PLIST_ROLLBACK_PATH),
    ])
}

#[cfg(any(target_os = "macos", test))]
#[expect(
    clippy::disallowed_methods,
    reason = "helper removal checks fixed system artifact paths without following symlinks"
)]
fn helper_artifacts_present(paths: &[&Utf8Path]) -> Result<bool, PlatformError> {
    for path in paths {
        match std::fs::symlink_metadata(path) {
            Ok(_metadata) => return Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(PlatformError::PrivilegedHelperInstallation(format!(
                    "could not inspect helper artifact {path}: {error}"
                )));
            }
        }
    }

    Ok(false)
}

#[cfg(target_os = "macos")]
#[expect(
    clippy::disallowed_methods,
    reason = "helper removal checks fixed system directories without following symlinks"
)]
fn validate_helper_support_directory_for_removal() -> Result<(), PlatformError> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let path = Utf8Path::new(HELPER_SUPPORT_DIRECTORY);
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(PlatformError::PrivilegedHelperInstallation(format!(
                "could not inspect helper support directory {path}: {error}"
            )));
        }
    };
    let mode = metadata.permissions().mode() & 0o777;
    if !metadata.file_type().is_dir() || metadata.uid() != 0 || mode != 0o755 {
        return Err(PlatformError::PrivilegedHelperInstallation(format!(
            "{path} must be a root-owned directory with mode 755 before removal"
        )));
    }

    Ok(())
}

#[cfg(target_os = "macos")]
#[expect(
    clippy::disallowed_methods,
    reason = "helper installer validates the candidate without following symlinks"
)]
fn validate_regular_file_owned_by(path: &Utf8Path, expected_uid: u32) -> Result<(), PlatformError> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        PlatformError::PrivilegedHelperInstallation(format!(
            "could not inspect helper candidate {path}: {error}"
        ))
    })?;
    if !metadata.file_type().is_file() || metadata.uid() != expected_uid {
        return Err(PlatformError::PrivilegedHelperInstallation(format!(
            "helper candidate must be a regular file owned by UID {expected_uid}: {path}"
        )));
    }

    Ok(())
}

#[cfg(target_os = "macos")]
#[expect(
    clippy::disallowed_types,
    reason = "helper installer hashes the candidate before privileged installation"
)]
fn sha256_file(path: &Utf8Path) -> Result<String, PlatformError> {
    let mut file = std::fs::File::open(path).map_err(|error| {
        PlatformError::PrivilegedHelperInstallation(format!(
            "could not open helper candidate {path}: {error}"
        ))
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            PlatformError::PrivilegedHelperInstallation(format!(
                "could not hash helper candidate {path}: {error}"
            ))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(hasher
        .finalize()
        .into_iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn expect_empty(payload: HelperPayload) -> Result<(), PlatformError> {
    match payload {
        HelperPayload::Empty => Ok(()),
        payload => Err(unexpected_payload("mutation", &payload)),
    }
}

fn unexpected_payload(operation: &str, payload: &HelperPayload) -> PlatformError {
    PlatformError::PrivilegedHelperRejected {
        message: format!("unexpected response payload for {operation}: {payload:?}"),
    }
}

#[cfg(target_os = "macos")]
fn call_helper(
    socket_path: &Utf8Path,
    operation: HelperOperation,
) -> Result<HelperPayload, PlatformError> {
    let mut stream = connect_helper(socket_path)?;

    let request = HelperRequest {
        protocol_version: HELPER_PROTOCOL_VERSION,
        operation,
    };
    write_message(&mut stream, &request)?;
    let response: HelperResponse = read_message(&mut stream)?;

    if response.protocol_version != HELPER_PROTOCOL_VERSION {
        return Err(PlatformError::PrivilegedHelperProtocolMismatch {
            expected: HELPER_PROTOCOL_VERSION,
            actual: response.protocol_version,
        });
    }

    match response.outcome {
        HelperOutcome::Success { payload } => Ok(payload),
        HelperOutcome::Error { code, message } => Err(match code {
            HelperErrorCode::AuthenticationFailed => {
                PlatformError::PrivilegedHelperAuthentication(message)
            }
            HelperErrorCode::ProtocolMismatch => PlatformError::PrivilegedHelperRemote {
                code: "protocol_mismatch".to_string(),
                message,
            },
            HelperErrorCode::InvalidRequest => PlatformError::PrivilegedHelperRejected { message },
            HelperErrorCode::SystemIntegrationFailed => PlatformError::SystemIntegration(message),
            HelperErrorCode::InternalError => PlatformError::PrivilegedHelperRemote {
                code: "internal_error".to_string(),
                message,
            },
        }),
    }
}

#[cfg(target_os = "macos")]
fn connect_helper(socket_path: &Utf8Path) -> Result<std::os::unix::net::UnixStream, PlatformError> {
    let stream = std::os::unix::net::UnixStream::connect(socket_path).map_err(|source| {
        if matches!(
            source.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
        ) {
            PlatformError::PrivilegedHelperUnavailable
        } else if source.kind() == std::io::ErrorKind::PermissionDenied {
            PlatformError::PrivilegedHelperAuthentication(
                "caller cannot access the installing account's helper socket".to_string(),
            )
        } else {
            PlatformError::PrivilegedHelperIo(source)
        }
    })?;
    stream
        .set_read_timeout(Some(HELPER_IO_TIMEOUT))
        .map_err(PlatformError::PrivilegedHelperIo)?;
    stream
        .set_write_timeout(Some(HELPER_IO_TIMEOUT))
        .map_err(PlatformError::PrivilegedHelperIo)?;

    Ok(stream)
}

#[cfg(target_os = "macos")]
fn probe_helper_lifecycle(socket_path: &Utf8Path) -> Result<PrivilegedHelperStatus, PlatformError> {
    let mut stream = connect_helper(socket_path)?;
    stream
        .write_all(HELPER_LIFECYCLE_PROBE)
        .map_err(PlatformError::PrivilegedHelperIo)?;
    let response = read_frame(&mut stream)?;
    parse_helper_lifecycle_response(&response)
}

#[cfg(any(target_os = "macos", test))]
fn parse_helper_lifecycle_response(
    response: &[u8],
) -> Result<PrivilegedHelperStatus, PlatformError> {
    let response = std::str::from_utf8(response).map_err(|error| {
        PlatformError::PrivilegedHelperInstallation(format!(
            "helper lifecycle response was not UTF-8: {error}"
        ))
    })?;
    let response = response.strip_suffix('\n').ok_or_else(|| {
        PlatformError::PrivilegedHelperInstallation(
            "helper lifecycle response was not newline-delimited".to_string(),
        )
    })?;
    let mut fields = response.split('\t');
    let prefix = fields.next();
    let version = fields.next();
    let protocol_version = fields.next();
    let owner_uid = fields.next();
    if prefix != Some(HELPER_LIFECYCLE_READY_PREFIX) || fields.next().is_some() {
        return Err(PlatformError::PrivilegedHelperInstallation(
            "helper lifecycle response had an invalid shape".to_string(),
        ));
    }
    let (Some(version), Some(protocol_version), Some(owner_uid)) =
        (version, protocol_version, owner_uid)
    else {
        return Err(PlatformError::PrivilegedHelperInstallation(
            "helper lifecycle response was incomplete".to_string(),
        ));
    };
    let protocol_version = protocol_version.parse::<u32>().map_err(|error| {
        PlatformError::PrivilegedHelperInstallation(format!(
            "helper lifecycle protocol version was invalid: {error}"
        ))
    })?;
    let owner_uid = owner_uid.parse::<u32>().map_err(|error| {
        PlatformError::PrivilegedHelperInstallation(format!(
            "helper lifecycle owner UID was invalid: {error}"
        ))
    })?;
    validate_helper_identity(version, protocol_version)?;

    Ok(PrivilegedHelperStatus {
        version: version.to_string(),
        protocol_version,
        owner_uid,
    })
}

#[cfg(not(target_os = "macos"))]
fn call_helper(
    _socket_path: &Utf8Path,
    _operation: HelperOperation,
) -> Result<HelperPayload, PlatformError> {
    Err(crate::capability::unsupported(
        crate::PlatformCapability::PrivilegedHelper,
    )?)
}

#[cfg(any(target_os = "macos", all(test, unix)))]
fn write_message(writer: &mut impl Write, message: &impl Serialize) -> Result<(), PlatformError> {
    let message = serde_json::to_vec(message)?;
    if message.len() > MAX_MESSAGE_BYTES {
        return Err(PlatformError::PrivilegedHelperRejected {
            message: "message exceeds the helper protocol size limit".to_string(),
        });
    }

    writer
        .write_all(&message)
        .and_then(|()| writer.write_all(b"\n"))
        .map_err(PlatformError::PrivilegedHelperIo)
}

#[cfg(any(target_os = "macos", test))]
fn read_message<T: for<'de> Deserialize<'de>>(reader: &mut impl Read) -> Result<T, PlatformError> {
    let bytes = read_frame(reader)?;

    Ok(serde_json::from_slice(&bytes)?)
}

#[cfg(any(target_os = "macos", test))]
fn read_frame(reader: &mut impl Read) -> Result<Vec<u8>, PlatformError> {
    let mut bytes = Vec::new();
    let mut byte = [0_u8; 1];
    while bytes.len() <= MAX_MESSAGE_BYTES {
        let count = reader
            .read(&mut byte)
            .map_err(PlatformError::PrivilegedHelperIo)?;
        if count == 0 {
            return Err(PlatformError::PrivilegedHelperIo(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "helper protocol connection closed before a complete frame was received",
            )));
        }

        bytes.push(byte[0]);
        if byte[0] == b'\n' {
            break;
        }
    }

    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(PlatformError::PrivilegedHelperRejected {
            message: "helper protocol message exceeds the size limit".to_string(),
        });
    }

    Ok(bytes)
}

fn validate_fingerprint(fingerprint: &str) -> Result<(), PlatformError> {
    if fingerprint.len() == 64
        && fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Ok(());
    }

    Err(PlatformError::PrivilegedHelperRejected {
        message: "CA fingerprint must be 64 lowercase hexadecimal characters".to_string(),
    })
}

#[cfg(target_os = "macos")]
pub fn serve_privileged_helper() -> Result<(), PlatformError> {
    use std::os::fd::FromRawFd;
    use std::os::unix::net::UnixListener;

    if !rustix::process::getuid().is_root() {
        return Err(PlatformError::PrivilegedHelperAuthentication(
            "helper must run as root".to_string(),
        ));
    }

    let metadata = read_helper_metadata()?;
    validate_helper_metadata(&metadata)?;
    let socket_fds = raunch::activate_socket(HELPER_SOCKET_NAME).map_err(|error| {
        PlatformError::PrivilegedHelperInstallation(format!(
            "could not activate launchd socket: {error}"
        ))
    })?;
    let [socket_fd] = socket_fds.as_slice() else {
        return Err(PlatformError::PrivilegedHelperInstallation(format!(
            "launchd provided {} helper sockets, expected exactly one",
            socket_fds.len()
        )));
    };

    // SAFETY: launchd returned this owned listening socket descriptor exactly once for
    // the named service, and the descriptor is transferred to [`UnixListener`] here.
    let listener = unsafe { UnixListener::from_raw_fd(*socket_fd) };

    loop {
        serve_next_helper_connection(&listener, &metadata)?;
    }
}

#[cfg(target_os = "macos")]
fn serve_next_helper_connection(
    listener: &std::os::unix::net::UnixListener,
    metadata: &PrivilegedHelperMetadata,
) -> Result<(), PlatformError> {
    let (mut stream, _address) = listener
        .accept()
        .map_err(PlatformError::PrivilegedHelperIo)?;
    if let Err(error) = serve_helper_connection(&mut stream, metadata) {
        let mut standard_error = std::io::stderr().lock();
        writeln!(standard_error, "pv-helper: {error}")
            .map_err(PlatformError::PrivilegedHelperIo)?;
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn serve_helper_connection(
    stream: &mut std::os::unix::net::UnixStream,
    metadata: &PrivilegedHelperMetadata,
) -> Result<(), PlatformError> {
    stream
        .set_read_timeout(Some(HELPER_IO_TIMEOUT))
        .map_err(PlatformError::PrivilegedHelperIo)?;
    stream
        .set_write_timeout(Some(HELPER_IO_TIMEOUT))
        .map_err(PlatformError::PrivilegedHelperIo)?;
    let response = handle_connection(stream, metadata);
    match response {
        HelperConnectionResponse::Operational(response) => write_message(stream, &response)?,
        HelperConnectionResponse::Lifecycle(response) => stream
            .write_all(response.as_bytes())
            .map_err(PlatformError::PrivilegedHelperIo)?,
    }

    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn serve_privileged_helper() -> Result<(), PlatformError> {
    Err(crate::capability::unsupported(
        crate::PlatformCapability::PrivilegedHelper,
    )?)
}

#[cfg(target_os = "macos")]
enum HelperConnectionResponse {
    Operational(HelperResponse),
    Lifecycle(String),
}

#[cfg(target_os = "macos")]
fn handle_connection(
    stream: &mut std::os::unix::net::UnixStream,
    metadata: &PrivilegedHelperMetadata,
) -> HelperConnectionResponse {
    let frame =
        match authenticate_peer(stream, metadata.owner_uid).and_then(|()| read_frame(stream)) {
            Ok(frame) => frame,
            Err(error) => {
                return HelperConnectionResponse::Operational(error_response(error));
            }
        };
    if frame == HELPER_LIFECYCLE_PROBE {
        return HelperConnectionResponse::Lifecycle(format!(
            "{HELPER_LIFECYCLE_READY_PREFIX}\t{PRIVILEGED_HELPER_VERSION}\t{HELPER_PROTOCOL_VERSION}\t{}\n",
            metadata.owner_uid
        ));
    }
    let outcome = serde_json::from_slice::<HelperRequest>(&frame)
        .map_err(PlatformError::from)
        .and_then(|request| dispatch_request(request, metadata));

    HelperConnectionResponse::Operational(match outcome {
        Ok(payload) => successful_response(payload),
        Err(error) => error_response(error),
    })
}

#[cfg(target_os = "macos")]
fn authenticate_peer(
    stream: &std::os::unix::net::UnixStream,
    owner_uid: u32,
) -> Result<(), PlatformError> {
    let (peer_uid, _peer_gid) = nix::unistd::getpeereid(stream).map_err(|error| {
        PlatformError::PrivilegedHelperAuthentication(format!(
            "could not inspect Unix peer credentials: {error}"
        ))
    })?;
    if peer_uid.as_raw() != owner_uid {
        return Err(PlatformError::PrivilegedHelperAuthentication(format!(
            "caller UID {} does not match installing UID {owner_uid}",
            peer_uid.as_raw()
        )));
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn dispatch_request(
    request: HelperRequest,
    metadata: &PrivilegedHelperMetadata,
) -> Result<HelperPayload, PlatformError> {
    if request.protocol_version != HELPER_PROTOCOL_VERSION {
        return Err(PlatformError::PrivilegedHelperProtocolMismatch {
            expected: HELPER_PROTOCOL_VERSION,
            actual: request.protocol_version,
        });
    }

    match request.operation {
        HelperOperation::Status => Ok(HelperPayload::Status(PrivilegedHelperStatus {
            version: PRIVILEGED_HELPER_VERSION.to_string(),
            protocol_version: HELPER_PROTOCOL_VERSION,
            owner_uid: metadata.owner_uid,
        })),
        HelperOperation::DnsInspect { expected_port } => {
            let expected = expected_port.map(ResolverConfig::new);
            Ok(HelperPayload::ResolverState(crate::inspect_resolver_file(
                Utf8Path::new(crate::SYSTEM_RESOLVER_TEST_PATH),
                expected.as_ref(),
            )))
        }
        HelperOperation::DnsApply { port } => {
            validate_high_port("DNS", port)?;
            crate::resolver::apply_resolver_config_privileged(&ResolverConfig::new(port))?;
            Ok(HelperPayload::Empty)
        }
        HelperOperation::DnsRemove => {
            crate::resolver::remove_resolver_config_privileged()?;
            Ok(HelperPayload::Empty)
        }
        HelperOperation::PfInspect => Ok(HelperPayload::PfInspection(
            crate::pf::inspect_active_pf_redirects_privileged()?,
        )),
        HelperOperation::PfApply {
            http_port,
            https_port,
        } => {
            validate_high_port("Gateway HTTP", http_port)?;
            validate_high_port("Gateway HTTPS", https_port)?;
            if http_port == https_port {
                return Err(PlatformError::PrivilegedHelperRejected {
                    message: "Gateway HTTP and HTTPS ports must be different".to_string(),
                });
            }
            crate::pf::apply_pf_redirects_privileged(&PfRedirectConfig::new(
                http_port, https_port,
            ))?;
            Ok(HelperPayload::Empty)
        }
        HelperOperation::PfReload => {
            crate::pf::reload_pf_redirects_privileged()?;
            Ok(HelperPayload::Empty)
        }
        HelperOperation::PfRemove => {
            crate::pf::remove_pf_redirects_privileged()?;
            Ok(HelperPayload::Empty)
        }
        HelperOperation::CaInspect => {
            use crate::SystemTrustInspector as _;

            let certificates = crate::NativeSystemTrustInspector.trusted_certificates()?;
            Ok(HelperPayload::CaCertificates(certificates))
        }
        HelperOperation::CaApply { fingerprint } => {
            validate_fingerprint(&fingerprint)?;
            let certificate_path = owner_ca_certificate_path(metadata.owner_uid)?;
            let certificate_pem = state::fs::read_to_string(&certificate_path)
                .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;
            let local = crate::LocalCaMetadata::from_certificate_pem(&certificate_pem)?;
            if local.fingerprint != fingerprint || !crate::ca::is_pv_ca_metadata(&local) {
                return Err(PlatformError::PrivilegedHelperRejected {
                    message: "local CA does not match the requested PV fingerprint".to_string(),
                });
            }
            let root_candidate = Utf8Path::new(HELPER_CA_CANDIDATE_PATH);
            write_root_work_file(root_candidate, &certificate_pem, "0755")?;
            let trust_result = (|| {
                let staged_pem = state::fs::read_to_string(root_candidate)
                    .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;
                let staged = crate::LocalCaMetadata::from_certificate_pem(&staged_pem)?;
                if staged.fingerprint != fingerprint || !crate::ca::is_pv_ca_metadata(&staged) {
                    return Err(PlatformError::SystemIntegration(
                        "root-staged local CA changed before trust installation".to_string(),
                    ));
                }
                crate::trust::trust_system_ca_privileged(root_candidate)
            })();
            let cleanup_result =
                crate::command::run_system_command("/bin/rm", &["-f", root_candidate.as_str()]);
            match (trust_result, cleanup_result) {
                (Ok(()), Ok(())) => {}
                (Ok(()), Err(error)) => return Err(error),
                (Err(error), Ok(())) => return Err(error),
                (Err(error), Err(cleanup_error)) => {
                    return Err(PlatformError::SystemIntegration(format!(
                        "{error}; additionally failed to remove root CA candidate: {cleanup_error}"
                    )));
                }
            }
            Ok(HelperPayload::Empty)
        }
        HelperOperation::CaRemove { fingerprint } => {
            validate_fingerprint(&fingerprint)?;
            crate::trust::untrust_system_ca_privileged(&fingerprint)?;
            Ok(HelperPayload::Empty)
        }
    }
}

#[cfg(target_os = "macos")]
fn owner_ca_certificate_path(owner_uid: u32) -> Result<Utf8PathBuf, PlatformError> {
    let user = nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(owner_uid))
        .map_err(|error| {
            PlatformError::PrivilegedHelperAuthentication(format!(
                "could not resolve installing user: {error}"
            ))
        })?
        .ok_or_else(|| {
            PlatformError::PrivilegedHelperAuthentication(format!(
                "installing UID {owner_uid} does not resolve to a local account"
            ))
        })?;
    let home = Utf8PathBuf::from_path_buf(user.dir).map_err(|path| {
        PlatformError::PrivilegedHelperAuthentication(format!(
            "installing user's home is not valid UTF-8: {}",
            path.to_string_lossy()
        ))
    })?;

    Ok(home.join(".pv/certificates/ca.pem"))
}

#[cfg(target_os = "macos")]
fn read_helper_metadata() -> Result<PrivilegedHelperMetadata, PlatformError> {
    validate_root_owned_regular_file(Utf8Path::new(HELPER_METADATA_PATH), 0o644)?;
    let content = state::fs::read_to_string(Utf8Path::new(HELPER_METADATA_PATH))
        .map_err(|error| PlatformError::PrivilegedHelperInstallation(error.to_string()))?;

    Ok(serde_json::from_str(&content)?)
}

#[cfg(target_os = "macos")]
fn validate_helper_metadata(metadata: &PrivilegedHelperMetadata) -> Result<(), PlatformError> {
    if metadata.helper_version != PRIVILEGED_HELPER_VERSION {
        return Err(PlatformError::PrivilegedHelperInstallation(format!(
            "metadata helper version {} does not match binary version {PRIVILEGED_HELPER_VERSION}",
            metadata.helper_version
        )));
    }
    if metadata.protocol_version != HELPER_PROTOCOL_VERSION {
        return Err(PlatformError::PrivilegedHelperProtocolMismatch {
            expected: HELPER_PROTOCOL_VERSION,
            actual: metadata.protocol_version,
        });
    }
    if metadata.owner_uid == 0 {
        return Err(PlatformError::PrivilegedHelperAuthentication(
            "installing user UID must not be root".to_string(),
        ));
    }

    Ok(())
}

#[cfg(target_os = "macos")]
#[expect(
    clippy::disallowed_methods,
    reason = "privileged helper validates root-owned installation metadata before use"
)]
fn validate_root_owned_regular_file(
    path: &Utf8Path,
    expected_mode: u32,
) -> Result<(), PlatformError> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        PlatformError::PrivilegedHelperInstallation(format!("could not inspect {path}: {error}"))
    })?;
    let actual_mode = metadata.permissions().mode() & 0o777;
    if !metadata.file_type().is_file() || metadata.uid() != 0 || actual_mode != expected_mode {
        return Err(PlatformError::PrivilegedHelperInstallation(format!(
            "{path} must be a root-owned regular file with mode {expected_mode:o}"
        )));
    }

    Ok(())
}

#[cfg(target_os = "macos")]
#[expect(
    clippy::disallowed_methods,
    reason = "helper lifecycle inspects fixed root-owned installation and rollback files"
)]
fn root_owned_regular_file_present(
    path: &Utf8Path,
    expected_mode: u32,
) -> Result<bool, PlatformError> {
    match std::fs::symlink_metadata(path) {
        Ok(_metadata) => {
            validate_root_owned_regular_file(path, expected_mode)?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(PlatformError::PrivilegedHelperInstallation(format!(
            "could not inspect {path}: {error}"
        ))),
    }
}

#[cfg(target_os = "macos")]
#[expect(
    clippy::disallowed_methods,
    reason = "privileged helper validates fixed system destinations before mutation"
)]
pub(crate) fn validate_root_owned_file_if_present(path: &Utf8Path) -> Result<bool, PlatformError> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(PlatformError::SystemIntegration(format!(
                "could not inspect fixed system path {path}: {error}"
            )));
        }
    };
    if !metadata.file_type().is_file() || metadata.uid() != 0 {
        return Err(PlatformError::SystemIntegration(format!(
            "fixed system path must be a root-owned regular file: {path}"
        )));
    }

    Ok(true)
}

#[cfg(target_os = "macos")]
#[expect(
    clippy::disallowed_methods,
    reason = "root helper atomically writes fixed root-owned preparation files"
)]
pub(crate) fn write_root_work_file(
    path: &Utf8Path,
    content: &str,
    parent_mode: &str,
) -> Result<(), PlatformError> {
    use std::os::unix::fs::PermissionsExt as _;

    let parent = path.parent().ok_or_else(|| {
        PlatformError::SystemIntegration(format!("root work file has no parent: {path}"))
    })?;
    crate::command::run_system_command(
        "/usr/bin/install",
        &[
            "-d",
            "-o",
            "root",
            "-g",
            "wheel",
            "-m",
            parent_mode,
            parent.as_str(),
        ],
    )?;
    validate_root_owned_file_if_present(path)?;
    let temporary_path = path.with_extension("pv-helper-tmp");
    validate_root_owned_file_if_present(&temporary_path)?;
    std::fs::write(&temporary_path, content).map_err(|error| {
        PlatformError::SystemIntegration(format!("could not write {temporary_path}: {error}"))
    })?;
    std::fs::set_permissions(&temporary_path, std::fs::Permissions::from_mode(0o600)).map_err(
        |error| {
            PlatformError::SystemIntegration(format!(
                "could not secure root work file {temporary_path}: {error}"
            ))
        },
    )?;
    std::fs::rename(&temporary_path, path).map_err(|error| {
        PlatformError::SystemIntegration(format!("could not replace {path}: {error}"))
    })?;
    validate_root_owned_regular_file(path, 0o600)
}

#[cfg(any(target_os = "macos", test))]
fn validate_high_port(label: &str, port: u16) -> Result<(), PlatformError> {
    if port >= 1024 {
        return Ok(());
    }

    Err(PlatformError::PrivilegedHelperRejected {
        message: format!("{label} port {port} is not an unprivileged high port"),
    })
}

#[cfg(target_os = "macos")]
fn successful_response(payload: HelperPayload) -> HelperResponse {
    HelperResponse {
        protocol_version: HELPER_PROTOCOL_VERSION,
        outcome: HelperOutcome::Success { payload },
    }
}

#[cfg(target_os = "macos")]
fn error_response(error: PlatformError) -> HelperResponse {
    HelperResponse {
        protocol_version: HELPER_PROTOCOL_VERSION,
        outcome: HelperOutcome::Error {
            code: helper_error_code(&error),
            message: error.to_string(),
        },
    }
}

#[cfg(target_os = "macos")]
fn helper_error_code(error: &PlatformError) -> HelperErrorCode {
    match error {
        PlatformError::PrivilegedHelperAuthentication(_) => HelperErrorCode::AuthenticationFailed,
        PlatformError::PrivilegedHelperProtocolMismatch { .. } => HelperErrorCode::ProtocolMismatch,
        PlatformError::PrivilegedHelperRejected { .. } => HelperErrorCode::InvalidRequest,
        PlatformError::SystemIntegration(_)
        | PlatformError::SystemIntegrationCommand { .. }
        | PlatformError::SystemIntegrationCommandStatus { .. } => {
            HelperErrorCode::SystemIntegrationFailed
        }
        _ => HelperErrorCode::InternalError,
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, ErrorKind};
    #[cfg(target_os = "macos")]
    use std::thread;
    #[cfg(unix)]
    use std::time::Duration;

    #[cfg(target_os = "macos")]
    use anyhow::anyhow;
    use camino_tempfile::tempdir;

    #[cfg(unix)]
    use super::{HELPER_PROTOCOL_VERSION, HelperOperation, write_message};
    #[cfg(target_os = "macos")]
    use super::{
        HELPER_SOCKET_NAME, HELPER_STANDARD_ERROR_PATH, HelperLaunchDaemonPlist, HelperPayload,
        PRIVILEGED_HELPER_VERSION, call_helper, lock_machine_helper_lifecycle_file,
        probe_helper_lifecycle, render_launch_daemon_plist, serve_next_helper_connection,
    };
    use super::{
        HelperRequest, MAX_MESSAGE_BYTES, PrivilegedHelperMetadata, helper_artifacts_present,
        parse_helper_lifecycle_response, read_frame, read_message, require_matching_helper_owner,
        validate_fingerprint, validate_helper_identity, validate_high_port,
    };
    use crate::PlatformError;

    #[cfg(target_os = "macos")]
    #[test]
    fn machine_helper_lifecycle_lock_rejects_a_concurrent_holder() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let path = tempdir.path().join("helper-lifecycle.lock");
        state::fs::write_sensitive_file(&path, "")?;

        let _first = lock_machine_helper_lifecycle_file(&path)?;
        let second = lock_machine_helper_lifecycle_file(&path);

        assert!(matches!(
            second,
            Err(PlatformError::PrivilegedHelperInstallation(message))
                if message.contains("already in progress")
        ));

        Ok(())
    }

    #[test]
    fn request_validation_rejects_low_ports_and_malformed_fingerprints() {
        assert!(validate_high_port("DNS", 1024).is_ok());
        assert!(validate_high_port("DNS", 53).is_err());
        assert!(validate_fingerprint(&"a".repeat(64)).is_ok());
        assert!(validate_fingerprint(&"A".repeat(64)).is_err());
        assert!(validate_fingerprint("abc").is_err());
    }

    #[test]
    fn helper_identity_requires_canonical_version_and_nonzero_protocol() {
        assert!(validate_helper_identity("1.0.0", 1).is_ok());
        assert!(validate_helper_identity("01.0.0", 1).is_err());
        assert!(validate_helper_identity("1.0", 1).is_err());
        assert!(validate_helper_identity("1.0.0", 0).is_err());
    }

    #[test]
    fn helper_replacement_rejects_a_different_installing_account() {
        let metadata = PrivilegedHelperMetadata {
            owner_uid: 501,
            helper_version: "1.0.0".to_string(),
            protocol_version: 1,
        };

        assert!(require_matching_helper_owner(&metadata, 501).is_ok());
        assert!(require_matching_helper_owner(&metadata, 502).is_err());
    }

    #[test]
    fn lifecycle_response_is_independent_from_the_operational_protocol_schema() -> anyhow::Result<()>
    {
        let status =
            parse_helper_lifecycle_response(b"PV-HELPER-LIFECYCLE-READY\t2.0.0\t9\t501\n")?;

        assert_eq!(status.version, "2.0.0");
        assert_eq!(status.protocol_version, 9);
        assert_eq!(status.owner_uid, 501);
        assert!(
            parse_helper_lifecycle_response(b"PV-HELPER-LIFECYCLE-READY\t2.0.0\t9\t501\textra\n")
                .is_err()
        );

        Ok(())
    }

    #[test]
    fn protocol_reader_reports_premature_eof_as_io_failure() {
        for frame in [Vec::new(), b"partial frame".to_vec()] {
            let error = read_frame(&mut Cursor::new(frame));

            assert!(matches!(
                error,
                Err(PlatformError::PrivilegedHelperIo(source))
                    if source.kind() == ErrorKind::UnexpectedEof
            ));
        }
    }

    #[test]
    fn helper_artifact_presence_detects_existing_paths() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let absent = tempdir.path().join("absent");
        let present = tempdir.path().join("present");

        assert!(!helper_artifacts_present(&[&absent])?);
        state::fs::write_sensitive_file(&present, "")?;
        assert!(helper_artifacts_present(&[&absent, &present])?);

        Ok(())
    }

    #[test]
    fn protocol_reader_rejects_oversized_frames() {
        let error = read_frame(&mut Cursor::new(vec![b'a'; MAX_MESSAGE_BYTES + 1]));

        assert!(matches!(
            error,
            Err(PlatformError::PrivilegedHelperRejected { message })
                if message == "helper protocol message exceeds the size limit"
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn launch_daemon_uses_reboot_safe_socket_and_stderr_log() -> anyhow::Result<()> {
        let content = render_launch_daemon_plist(501, 20)?;
        let plist = plist::from_bytes::<HelperLaunchDaemonPlist>(content.as_bytes())?;
        let socket = plist
            .sockets
            .get(HELPER_SOCKET_NAME)
            .ok_or_else(|| anyhow!("rendered LaunchDaemon is missing its control socket"))?;

        assert_eq!(socket.path, "/var/run/com.prvious.pv.helper.sock");
        assert_eq!(plist.standard_error_path, HELPER_STANDARD_ERROR_PATH);

        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "helper test fixture needs a blocking Unix socket server"
    )]
    fn helper_serves_sequential_connections_without_restarting() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let socket_path = tempdir.path().join("helper.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket_path)?;
        let owner_uid = rustix::process::getuid().as_raw();
        let metadata = PrivilegedHelperMetadata {
            owner_uid,
            helper_version: PRIVILEGED_HELPER_VERSION.to_string(),
            protocol_version: HELPER_PROTOCOL_VERSION,
        };
        let server = thread::spawn(move || {
            serve_next_helper_connection(&listener, &metadata)?;
            serve_next_helper_connection(&listener, &metadata)?;
            serve_next_helper_connection(&listener, &metadata)
        });

        let disconnected_client = std::os::unix::net::UnixStream::connect(&socket_path)?;
        drop(disconnected_client);
        let lifecycle_status = probe_helper_lifecycle(&socket_path)?;
        let status_payload = call_helper(&socket_path, HelperOperation::Status)?;
        let server_result = server
            .join()
            .map_err(|_error| anyhow!("helper fixture thread panicked"))?;
        server_result?;

        assert_eq!(lifecycle_status.owner_uid, owner_uid);
        assert!(matches!(
            status_payload,
            HelperPayload::Status(status) if status.owner_uid == owner_uid
        ));

        Ok(())
    }

    #[test]
    fn protocol_rejects_unknown_fields() {
        let mut message = Cursor::new(
            br#"{"protocol_version":1,"operation":{"name":"status"},"command":"/bin/sh"}
"#,
        );

        assert!(matches!(
            read_message::<HelperRequest>(&mut message),
            Err(crate::PlatformError::PrivilegedHelperProtocol(_))
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn connection_authenticates_before_reading_a_request() -> anyhow::Result<()> {
        use super::{
            HelperConnectionResponse, HelperErrorCode, HelperOutcome, HelperResponse,
            handle_connection,
        };

        let (mut server, _client) = std::os::unix::net::UnixStream::pair()?;
        let metadata = PrivilegedHelperMetadata {
            owner_uid: rustix::process::getuid().as_raw().saturating_add(1),
            helper_version: "1.0.0".to_string(),
            protocol_version: HELPER_PROTOCOL_VERSION,
        };

        let response = handle_connection(&mut server, &metadata);

        assert!(matches!(
            response,
            HelperConnectionResponse::Operational(HelperResponse {
                outcome: HelperOutcome::Error {
                    code: HelperErrorCode::AuthenticationFailed,
                    ..
                },
                ..
            })
        ));

        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn protocol_reader_returns_after_one_newline_delimited_message() -> anyhow::Result<()> {
        let (mut reader, mut writer) = std::os::unix::net::UnixStream::pair()?;
        reader.set_read_timeout(Some(Duration::from_secs(1)))?;
        let request = HelperRequest {
            protocol_version: HELPER_PROTOCOL_VERSION,
            operation: HelperOperation::DnsApply { port: 35_353 },
        };

        write_message(&mut writer, &request)?;
        let decoded: HelperRequest = read_message(&mut reader)?;

        assert_eq!(decoded.protocol_version, HELPER_PROTOCOL_VERSION);
        assert!(matches!(
            decoded.operation,
            HelperOperation::DnsApply { port: 35_353 }
        ));

        Ok(())
    }
}
