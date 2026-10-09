#[cfg(target_os = "macos")]
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[cfg(target_os = "macos")]
use crate::command::run_system_command_output_with_timeout;
#[cfg(target_os = "macos")]
use crate::{PlatformError, loopback_tcp_port_available};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LowPortInspection {
    pub ports: Vec<LowPortState>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LowPortState {
    pub port: u16,
    pub available: bool,
    pub owners: Vec<PortOwner>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PortOwner {
    pub pid: u32,
    pub command: String,
}

impl LowPortState {
    pub fn conflict_message(&self) -> String {
        if self.owners.is_empty() {
            return format!(
                "Loopback TCP port {} is unavailable; PV could not identify a listening process.",
                self.port
            );
        }
        let owners = self
            .owners
            .iter()
            .map(|owner| format!("{} (pid {})", owner.command, owner.pid))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "Loopback TCP port {} is unavailable. TCP listeners reported on this port: {owners}.",
            self.port
        )
    }
}

/// Root decides availability with a bind probe; `lsof` supplies diagnostics only.
#[cfg(target_os = "macos")]
pub(crate) fn inspect_loopback_ports(ports: &[u16]) -> Result<LowPortInspection, PlatformError> {
    let mut inspection = LowPortInspection {
        ports: ports
            .iter()
            .map(|&port| LowPortState {
                port,
                available: loopback_tcp_port_available(port),
                owners: Vec::new(),
            })
            .collect(),
    };
    if inspection.ports.iter().all(|port| port.available) {
        return Ok(inspection);
    }
    let port_list = ports
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(",");
    // One selector avoids failure when only some requested ports have listeners.
    let selector = format!("-iTCP:{port_list}");
    let output = run_system_command_output_with_timeout(
        "/usr/sbin/lsof",
        &["-nP", &selector, "-sTCP:LISTEN", "-F", "pcn"],
        Duration::from_secs(2),
        Some(1),
    )?;
    parse_lsof_owners(&output, &mut inspection.ports);
    Ok(inspection)
}

#[cfg(any(target_os = "macos", test))]
fn parse_lsof_owners(output: &str, ports: &mut [LowPortState]) {
    let mut pid = None;
    let mut command = None;
    for line in output.lines() {
        if let Some(value) = line.strip_prefix('p') {
            pid = value.parse::<u32>().ok().filter(|&pid| pid > 0);
            command = None;
        } else if let Some(value) = line.strip_prefix('c') {
            command = (!value.is_empty()).then_some(value);
        } else if let Some(value) = line.strip_prefix('n')
            && let Some(pid) = pid
            && let Some(command) = command
            && let Some((address, port)) = value.rsplit_once(':')
            && matches!(address, "*" | "127.0.0.1" | "[::]")
            && let Ok(port) = port.parse::<u16>()
            && let Some(state) = ports
                .iter_mut()
                .find(|state| state.port == port && !state.available)
            // Bound diagnostics so the helper reply stays within its 16 KiB frame.
            && state.owners.len() < 8
            && !state.owners.iter().any(|owner| owner.pid == pid)
        {
            state.owners.push(PortOwner {
                pid,
                command: command.to_owned(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    use std::net::TcpListener;

    use insta::assert_debug_snapshot;

    #[cfg(target_os = "macos")]
    use super::inspect_loopback_ports;
    use super::{LowPortInspection, LowPortState, parse_lsof_owners};

    #[test]
    fn lsof_fields_name_and_deduplicate_owners_without_deciding_availability() {
        for (name, fields) in [
            ("named_owner", "p412\ncnginx\nf5\nn127.0.0.1:80\n"),
            (
                "several_pids",
                "p412\ncnginx\nn*:80\np501\ncPython\nn127.0.0.1:443\n",
            ),
            (
                "nginx_master_workers",
                "p412\ncnginx\nn*:80\nn*:80\nn*:443\np413\ncnginx\nn*:80\nn*:443\np414\ncnginx\nn*:80\n",
            ),
            ("empty_output", ""),
            (
                "mixed_ipv4_and_ipv6_wildcard",
                "p412\ncnginx\nn127.0.0.1:80\np501\ncPython\nn*:80\n",
            ),
            (
                "unrelated_or_incomplete_fields",
                "p412\ncnginx\nn192.168.1.10:80\nn[::1]:443\nn*:48080\npinvalid\ncPython\nn*:80\np501\nn*:443\n",
            ),
        ] {
            let mut inspection = LowPortInspection {
                ports: [80, 443]
                    .into_iter()
                    .map(|port| LowPortState {
                        port,
                        available: false,
                        owners: Vec::new(),
                    })
                    .collect(),
            };
            parse_lsof_owners(fields, &mut inspection.ports);
            assert!(inspection.ports.iter().all(|port| !port.available));
            let messages = inspection
                .ports
                .iter()
                .map(LowPortState::conflict_message)
                .collect::<Vec<_>>();
            assert_debug_snapshot!(name, (inspection, messages));
        }
    }

    #[test]
    fn lsof_owners_are_capped_and_free_ports_have_no_owners() {
        let fields = (1..=10)
            .map(|pid| format!("p{pid}\ncnginx\nn*:80\nn*:443\n"))
            .collect::<String>();
        let mut states = vec![
            LowPortState {
                port: 80,
                available: false,
                owners: Vec::new(),
            },
            LowPortState {
                port: 443,
                available: true,
                owners: Vec::new(),
            },
        ];
        parse_lsof_owners(&fields, &mut states);
        assert_eq!(states[0].owners.len(), 8);
        assert!(states[1].owners.is_empty());
        assert_debug_snapshot!(states);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn inspection_finds_one_held_port_when_the_other_is_free() -> anyhow::Result<()> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let unused_listener = TcpListener::bind(("127.0.0.1", 0))?;
        let unused_port = unused_listener.local_addr()?.port();
        drop(unused_listener);
        let inspection = inspect_loopback_ports(&[port, unused_port])?;
        assert_eq!(inspection.ports.len(), 2);
        assert!(!inspection.ports[0].available);
        assert!(
            inspection.ports[0]
                .owners
                .iter()
                .any(|owner| owner.pid == std::process::id())
        );
        assert!(inspection.ports[1].available);
        assert!(inspection.ports[1].owners.is_empty());
        Ok(())
    }
}
