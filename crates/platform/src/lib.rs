mod ca;
mod error;
mod pf;
mod resolver;
mod socket;
mod trust;

pub use ca::{
    CaFileState, CaRepairReason, GeneratedLocalCa, LocalCaMetadata, generate_local_ca,
    inspect_local_ca_files,
};
pub use error::PlatformError;
pub use pf::{
    PfConfReference, PfFileState, PfRedirectConfig, SYSTEM_PF_ANCHOR_PATH, SYSTEM_PF_CONF_PATH,
    inspect_pf_anchor_file, inspect_pf_conf_reference,
};
pub use resolver::{
    ResolverConfig, ResolverFileState, SYSTEM_RESOLVER_TEST_PATH, inspect_resolver_file,
};
pub use socket::{loopback_tcp_listener_ports, loopback_tcp_port_has_listener};
pub use trust::{
    KeychainCertificate, KeychainTrustResult, NativeSystemTrustInspector, SystemTrustInspector,
    TrustDomainState, inspect_system_ca_trust,
};

#[cfg(test)]
mod tests {
    use insta::assert_debug_snapshot;

    use crate::socket::parse_netstat_tcp_listener_ports;

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
