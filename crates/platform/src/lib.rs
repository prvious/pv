mod browser;
mod ca;
mod capability;
pub mod clock;
#[cfg(any(target_os = "macos", target_os = "linux", all(test, unix)))]
mod command;
mod daemon_registration;
mod error;
mod gateway_listeners;
mod helper;
pub mod hooks;
// This is probably temporary. We should remember to remove it on the next release.
#[cfg(any(target_os = "macos", test))]
mod legacy_pf;
#[cfg(target_os = "linux")]
mod linux_host;
mod low_port;
mod port;
mod process;
mod resolver;
#[cfg(target_os = "linux")]
mod systemd;
mod target;
mod trust;
#[cfg(target_os = "linux")]
mod unprivileged_ports;

#[cfg(target_os = "linux")]
pub use process::inspect_process_comm;

pub use browser::open_url;
pub use ca::{
    CaFileState, CaRepairReason, GeneratedLocalCa, GeneratedProjectCertificate, LocalCaMetadata,
    generate_local_ca, generate_project_certificate, inspect_local_ca_files,
    project_certificate_matches,
};
pub use capability::{PlatformCapability, require_capability};
pub use daemon_registration::{
    DAEMON_REGISTRATION_FILE_NAME, DAEMON_REGISTRATION_LABEL, DAEMON_REGISTRATION_NAME,
    DaemonRegistrationConfig, DaemonRegistrationFileState, daemon_registration_enabled,
    daemon_registration_path, enable_daemon_registration, inspect_daemon_registration_file,
    remove_daemon_registration, restart_daemon_registration, stop_daemon_registration,
    write_daemon_registration_file,
};
pub use error::PlatformError;
pub use gateway_listeners::{
    GATEWAY_HTTP_PORT, GATEWAY_HTTP_SOCKET, GATEWAY_HTTPS_PORT, GATEWAY_HTTPS_SOCKET,
    GatewayListenerError, GatewayListenerSource, GatewayListeners,
};
#[cfg(target_os = "linux")]
pub use helper::run_privileged_helper_lifecycle;

pub use helper::{
    HELPER_EXECUTABLE_PATH, HELPER_LAUNCH_DAEMON_PATH, HELPER_METADATA_PATH,
    HELPER_PROTOCOL_VERSION, HELPER_SOCKET_PATH, PRIVILEGED_HELPER_VERSION, PrivilegedHelperClient,
    PrivilegedHelperInstallOutcome, PrivilegedHelperStatus, install_privileged_helper,
    remove_privileged_helper, serve_privileged_helper,
};
#[cfg(target_os = "linux")]
pub use linux_host::{check_linux_resolved_stub, check_linux_setup_host};
pub use low_port::{LowPortInspection, LowPortState, PortOwner};
// This is probably temporary. We should remember to remove it on the next release.
#[cfg(target_os = "macos")]
pub use legacy_pf::legacy_pf_present;
pub use port::loopback_tcp_port_available;
pub use process::{
    BootSessionId, ProcessEvents, ProcessExitWatch, ProcessIdentity, ProcessStartIdentity,
    current_boot_session_id, exec_replace, exec_replace_with_env, inspect_process_identity,
    inspect_process_start_identity, process_group_has_live_members, process_is_zombie,
};
pub use resolver::{
    DNS_LISTEN_ADDRESS, ResolverConfig, ResolverFileState, SYSTEM_RESOLVER_TEST_PATH,
    apply_system_resolver, inspect_system_resolver, remove_system_resolver,
};
#[cfg(target_os = "linux")]
pub use resolver::{start_resolver_link, stop_resolver_link};
pub use target::PlatformTarget;
pub use trust::{
    KeychainCertificate, KeychainTrustResult, NativeSystemTrustInspector, SystemTrustInspector,
    TrustDomainState, browser_ca_databases, inspect_browser_ca_trust, inspect_system_ca_trust,
    trust_browser_ca, trust_system_ca, trusted_pv_ca_fingerprints, untrust_browser_ca,
    untrust_system_ca,
};
#[cfg(target_os = "linux")]
pub use unprivileged_ports::{
    UNPRIVILEGED_PORTS_PATH, UnprivilegedPortsState, inspect_unprivileged_ports,
};

#[cfg(test)]
mod tests {
    use crate::capability::require_capability_for;
    use crate::error::PlatformError;
    use crate::{PlatformCapability, PlatformTarget};

    #[test]
    fn capability_check_accepts_macos_and_rejects_windows() {
        assert!(
            require_capability_for(PlatformTarget::Macos, PlatformCapability::TrustStore,).is_ok()
        );

        let error = require_capability_for(PlatformTarget::Windows, PlatformCapability::TrustStore);
        assert!(matches!(
            error,
            Err(PlatformError::Unsupported {
                capability: PlatformCapability::TrustStore,
                target: PlatformTarget::Windows,
            })
        ));
    }
}
