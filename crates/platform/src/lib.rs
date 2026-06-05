mod browser;
mod ca;
mod command;
mod error;
mod launch_agent;
mod pf;
mod resolver;
mod socket;
mod trust;

pub use browser::open_url;
pub use ca::{
    CaFileState, CaRepairReason, GeneratedLocalCa, LocalCaMetadata, generate_local_ca,
    inspect_local_ca_files,
};
pub use error::PlatformError;
pub use launch_agent::{
    LAUNCH_AGENT_FILE_NAME, LAUNCH_AGENT_LABEL, LaunchAgentConfig, LaunchAgentFileState,
    bootout_launch_agent, bootstrap_launch_agent, inspect_launch_agent_file,
    kickstart_launch_agent, launch_agent_path, remove_launch_agent_file, write_launch_agent_file,
};
pub use pf::{
    PfConfReference, PfFileState, PfRedirectConfig, SYSTEM_PF_ANCHOR_PATH, SYSTEM_PF_CONF_PATH,
    inspect_pf_anchor_file, inspect_pf_conf_reference, install_pf_redirects, remove_pf_redirects,
};
pub use resolver::{
    ResolverConfig, ResolverFileState, SYSTEM_RESOLVER_TEST_PATH, inspect_resolver_file,
    install_resolver_config, remove_resolver_config,
};
pub use socket::{loopback_tcp_listener_ports, loopback_tcp_port_has_listener};
pub use trust::{
    KeychainCertificate, KeychainTrustResult, NativeSystemTrustInspector, SystemTrustInspector,
    TrustDomainState, inspect_system_ca_trust,
};

#[cfg(test)]
mod tests {
    use insta::assert_debug_snapshot;

    use crate::error::PlatformError;
    use crate::socket::parse_netstat_tcp_listener_ports;

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
    fn unsupported_platform_error_names_feature() {
        let error = PlatformError::UnsupportedPlatform {
            feature: "System keychain trust inspection",
        };

        assert_eq!(
            error.to_string(),
            "System keychain trust inspection is unsupported on this platform"
        );
    }

    #[test]
    fn netstat_tcp_listener_port_parser_covers_loopback_and_wildcard_addresses() {
        let output = r#"
Proto Recv-Q Send-Q  Local Address          Foreign Address        (state)
tcp4       0      0  *.45000                *.*                    LISTEN
tcp4       0      0  127.0.0.1.45001        *.*                    LISTEN
tcp6       0      0  ::1.45002              *.*                    LISTEN
tcp6       0      0  ::.45003               *.*                    LISTEN
tcp4       0      0  192.168.1.5.45004      *.*                    LISTEN
tcp4       0      0  127.0.0.1.45005        127.0.0.1.12345        ESTABLISHED
udp4       0      0  127.0.0.1.45006        *.*
tcp4       0      0  127.0.0.1.notaport     *.*                    LISTEN
"#;

        assert_debug_snapshot!(parse_netstat_tcp_listener_ports(output));
    }
}
