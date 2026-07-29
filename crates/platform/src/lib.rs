mod browser;
mod ca;
mod capability;
mod command;
mod error;
mod launch_agent;
mod listener;
mod pf;
mod process;
mod resolver;
mod target;
mod trust;

pub use browser::open_url;
pub use ca::{
    CaFileState, CaRepairReason, GeneratedLocalCa, GeneratedProjectCertificate, LocalCaMetadata,
    generate_local_ca, generate_project_certificate, inspect_local_ca_files,
    project_certificate_matches,
};
pub use capability::{PlatformCapability, require_capability};
pub use error::PlatformError;
pub use launch_agent::{
    LAUNCH_AGENT_FILE_NAME, LAUNCH_AGENT_LABEL, LaunchAgentConfig, LaunchAgentFileState,
    bootout_launch_agent, bootstrap_launch_agent, inspect_launch_agent_file,
    kickstart_launch_agent, launch_agent_path, remove_launch_agent_file, write_launch_agent_file,
};
pub use listener::{loopback_tcp_listener_ports, loopback_tcp_port_has_listener};
pub use pf::{
    PfConfReference, PfFileState, PfRedirectConfig, SYSTEM_PF_ANCHOR_PATH, SYSTEM_PF_CONF_PATH,
    active_pf_redirect_config, active_pf_redirect_config_with_privilege_mode,
    inspect_pf_anchor_file, inspect_pf_conf_reference, install_pf_redirects, remove_pf_redirects,
};
pub use process::{exec_replace, exec_replace_with_env};
pub use resolver::{
    ResolverConfig, ResolverFileState, SYSTEM_RESOLVER_TEST_PATH, inspect_resolver_file,
    install_resolver_config, remove_resolver_config,
};
pub use target::PlatformTarget;
pub use trust::{
    KeychainCertificate, KeychainTrustResult, NativeSystemTrustInspector, PrivilegeMode,
    SystemTrustInspector, TrustDomainState, inspect_system_ca_trust, trust_system_ca,
    trusted_pv_ca_fingerprints, untrust_system_ca,
};

#[cfg(test)]
mod tests {
    use crate::capability::require_capability_for;
    use crate::error::PlatformError;
    use crate::{PlatformCapability, PlatformTarget};

    #[test]
    fn browser_open_helper_reports_failed_status() {
        let result = crate::browser::open_url_with_launcher("https://example.test", |_| Ok(false));

        assert!(matches!(
            result,
            Err(PlatformError::BrowserOpenStatus { status })
                if status == "exit status: unsuccessful"
        ));
    }

    #[test]
    fn unsupported_error_names_capability_and_target() {
        let error = PlatformError::Unsupported {
            capability: PlatformCapability::DaemonRegistration,
            target: PlatformTarget::Linux,
        };

        assert!(matches!(
            &error,
            PlatformError::Unsupported {
                capability: PlatformCapability::DaemonRegistration,
                target: PlatformTarget::Linux,
            }
        ));
        assert_eq!(
            error.to_string(),
            "daemon registration is unsupported on linux"
        );
    }

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
