mod browser;
mod ca;
mod capability;
#[cfg(any(target_os = "macos", target_os = "linux", all(test, unix)))]
mod command;
mod daemon_registration;

#[cfg(target_os = "linux")]
pub use daemon_registration::{
    DaemonRegistrationStatus, daemon_registration_session_address, inspect_daemon_registration,
};
mod error;
mod helper;
mod low_port;
mod pf;
mod port;
mod process;
mod resolver;
#[cfg(target_os = "linux")]
mod systemd;
mod target;
mod trust;

#[cfg(target_os = "linux")]
pub use process::inspect_process_comm;

#[cfg(target_os = "linux")]
pub use daemon_registration::reload_daemon_registration;

pub use browser::open_url;
pub use ca::{
    CaFileState, CaRepairReason, GeneratedLocalCa, GeneratedProjectCertificate, LocalCaMetadata,
    generate_local_ca, generate_project_certificate, inspect_local_ca_files,
    project_certificate_matches,
};
pub use capability::{PlatformCapability, require_capability};
pub use daemon_registration::{
    DAEMON_REGISTRATION_FILE_NAME, DAEMON_REGISTRATION_LABEL, DAEMON_REGISTRATION_NAME,
    DaemonRegistrationConfig, DaemonRegistrationFileState, daemon_registration_path,
    enable_daemon_registration, inspect_daemon_registration_file, remove_daemon_registration,
    restart_daemon_registration, stop_daemon_registration, write_daemon_registration_file,
};
pub use error::PlatformError;
#[cfg(target_os = "linux")]
pub use helper::{
    PrivilegedHelperLifecycleAction, installed_helper_artifacts_present,
    run_privileged_helper_lifecycle,
};

pub use helper::{
    HELPER_EXECUTABLE_PATH, HELPER_LAUNCH_DAEMON_PATH, HELPER_METADATA_PATH,
    HELPER_PROTOCOL_VERSION, HELPER_SOCKET_PATH, PRIVILEGED_HELPER_VERSION, PrivilegedHelperClient,
    PrivilegedHelperInstallOutcome, PrivilegedHelperStatus, install_privileged_helper,
    remove_privileged_helper, serve_privileged_helper,
};
pub use low_port::{LowPortInspection, LowPortState, PortOwner};
pub use pf::{
    ActivePfRedirectInspection, PfConfReference, PfFileState, PfRedirectConfig,
    SYSTEM_PF_ANCHOR_PATH, SYSTEM_PF_CONF_PATH, active_pf_redirect_config,
    inspect_active_pf_redirects_unprivileged, inspect_pf_anchor_file, inspect_pf_conf_reference,
    install_pf_redirects, remove_pf_redirects,
};
#[cfg(target_os = "linux")]
pub use pf::{
    SYSTEM_SOCKET_PROXY_DIRECTORY, inspect_socket_proxy_unit, render_socket_proxy_units,
    repairable_socket_proxy_ports, socket_proxy_files_current, socket_proxy_unit_names,
};
pub use port::loopback_tcp_port_available;
pub use process::{
    BootSessionId, ProcessEvents, ProcessExitWatch, ProcessIdentity, ProcessStartIdentity,
    current_boot_session_id, exec_replace, exec_replace_with_env, inspect_process_identity,
    inspect_process_start_identity, process_group_has_live_members, process_is_zombie,
};
pub use resolver::{
    ResolverConfig, ResolverFileState, SYSTEM_RESOLVER_TEST_PATH, inspect_resolver_file,
    install_resolver_config, remove_resolver_config,
};
#[cfg(target_os = "linux")]
pub use resolver::{start_resolver_link, stop_resolver_link};
pub use target::PlatformTarget;
pub use trust::{
    KeychainCertificate, KeychainTrustResult, NativeSystemTrustInspector, SystemTrustInspector,
    TrustDomainState, inspect_system_ca_trust, trust_system_ca, trusted_pv_ca_fingerprints,
    untrust_system_ca,
};

#[cfg(test)]
mod tests {
    use crate::capability::require_capability_for;
    use crate::error::PlatformError;
    use crate::{PlatformCapability, PlatformTarget};

    #[test]
    fn unsupported_error_names_capability_and_target() {
        let error = PlatformError::Unsupported {
            capability: PlatformCapability::DaemonRegistration,
            target: PlatformTarget::Windows,
        };

        assert!(matches!(
            &error,
            PlatformError::Unsupported {
                capability: PlatformCapability::DaemonRegistration,
                target: PlatformTarget::Windows,
            }
        ));
        assert_eq!(
            error.to_string(),
            "daemon registration is unsupported on windows"
        );
    }

    #[test]
    fn capability_check_supports_only_implemented_host_boundaries() {
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
        assert!(
            require_capability_for(PlatformTarget::Linux, PlatformCapability::BrowserHandoff)
                .is_ok()
        );
        assert!(matches!(
            require_capability_for(PlatformTarget::Windows, PlatformCapability::BrowserHandoff),
            Err(PlatformError::Unsupported {
                capability: PlatformCapability::BrowserHandoff,
                target: PlatformTarget::Windows,
            })
        ));
        assert!(
            require_capability_for(PlatformTarget::Linux, PlatformCapability::ProcessInspection)
                .is_ok()
        );
        assert!(
            require_capability_for(
                PlatformTarget::Linux,
                PlatformCapability::ProcessContainment
            )
            .is_ok()
        );
        assert!(
            require_capability_for(PlatformTarget::Linux, PlatformCapability::DaemonIpc).is_ok()
        );
        assert!(
            require_capability_for(
                PlatformTarget::Linux,
                PlatformCapability::DaemonRegistration
            )
            .is_ok()
        );
        assert!(
            require_capability_for(PlatformTarget::Linux, PlatformCapability::PrivilegedHelper)
                .is_ok()
        );
        assert!(
            require_capability_for(
                PlatformTarget::Linux,
                PlatformCapability::ResolverIntegration
            )
            .is_ok()
        );
        assert!(
            require_capability_for(PlatformTarget::Linux, PlatformCapability::LowPortFrontend)
                .is_ok()
        );
    }
}
