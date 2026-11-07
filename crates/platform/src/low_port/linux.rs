use std::collections::BTreeMap;

use procfs::net::{TcpNetEntry, TcpState, tcp, tcp6};
use procfs::process::{FDTarget, Process, all_processes};
use procfs::{ProcError, ProcResult};

use super::{LowPortState, record_owner};

pub(super) fn populate_owners(ports: &mut [LowPortState]) -> ProcResult<()> {
    let ipv4 = tcp()?;
    let ipv6 = match tcp6() {
        Ok(entries) => entries,
        // Kernels without IPv6 do not expose /proc/net/tcp6.
        Err(ProcError::NotFound(_)) => Vec::new(),
        Err(error) => return Err(error),
    };
    let sockets = socket_ports(ipv4.into_iter().chain(ipv6), ports);
    if sockets.is_empty() {
        return Ok(());
    }
    for process in all_processes()? {
        let Some(process) = diagnostic_read(process)? else {
            continue;
        };
        record_process_owners(&process, &sockets, ports)?;
    }
    Ok(())
}

fn record_process_owners(
    process: &Process,
    sockets: &BTreeMap<u64, u16>,
    ports: &mut [LowPortState],
) -> ProcResult<()> {
    let Some(stat) = diagnostic_read(process.stat())? else {
        return Ok(());
    };
    let Some(descriptors) = diagnostic_read(process.fd())? else {
        return Ok(());
    };
    let pid = u32::try_from(process.pid).map_err(|source| ProcError::Other(source.to_string()))?;
    for descriptor in descriptors.flatten() {
        if let FDTarget::Socket(inode) = descriptor.target
            && let Some(port) = sockets.get(&inode)
        {
            record_owner(ports, *port, pid, &stat.comm);
        }
    }
    Ok(())
}

fn socket_ports(
    entries: impl IntoIterator<Item = TcpNetEntry>,
    ports: &[LowPortState],
) -> BTreeMap<u64, u16> {
    entries
        .into_iter()
        .filter(|entry| {
            entry.state == TcpState::Listen
                && ports
                    .iter()
                    .any(|port| !port.available && port.port == entry.local_address.port())
        })
        .map(|entry| (entry.inode, entry.local_address.port()))
        .collect()
}

fn diagnostic_read<T>(result: ProcResult<T>) -> ProcResult<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        // Processes and descriptors can disappear or deny access while enumerating.
        // Missing names never override the authoritative bind probe.
        Err(ProcError::NotFound(_) | ProcError::PermissionDenied(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::process;

    use anyhow::{Context, Result};

    use super::super::inspect_loopback_ports;

    #[test]
    fn wildcard_listener_with_duplicated_descriptors_has_one_native_owner() -> Result<()> {
        let port = pv_fake::port()?;
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        let duplicate = listener.try_clone()?;
        let inspection = inspect_loopback_ports(&[port])?;
        let state = inspection
            .ports
            .first()
            .context("port inspection missing")?;
        assert!(!state.available);
        let owners: Vec<_> = state
            .owners
            .iter()
            .filter(|owner| owner.pid == process::id())
            .collect();
        assert_eq!(owners.len(), 1);
        assert!(!owners[0].command.is_empty());
        drop((listener, duplicate));
        Ok(())
    }
}
