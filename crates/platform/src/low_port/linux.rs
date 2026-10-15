use std::collections::BTreeMap;
use std::io;

use procfs::net::{TcpNetEntry, TcpState, tcp, tcp6};
use procfs::process::{FDTarget, Process, all_processes};
use procfs::{ProcError, ProcResult};
use rustix::fs::{Dir, OFlags};

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
    // Process::fd() hides target-read errors. Enumerate fallibly, then decode
    // each descriptor through the pinned process instead.
    let Some(directory) = diagnostic_read(
        process.open_relative_flags("fd", OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC),
    )?
    else {
        return Ok(());
    };
    let Some(descriptors) = diagnostic_read(
        Dir::read_from(&directory).map_err(|source| io::Error::from(source).into()),
    )?
    else {
        return Ok(());
    };
    let pid = u32::try_from(process.pid).map_err(|source| ProcError::Other(source.to_string()))?;
    for descriptor in descriptors {
        let Some(entry) =
            diagnostic_read(descriptor.map_err(|source| io::Error::from(source).into()))?
        else {
            continue;
        };
        let Some(number) = entry
            .file_name()
            .to_str()
            .ok()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        if let Some(descriptor) = diagnostic_read(process.fd_from_fd(number))?
            && let FDTarget::Socket(inode) = descriptor.target
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
    use std::collections::BTreeMap;
    use std::io;
    use std::net::TcpListener;
    use std::process;

    use anyhow::{Context, Result};
    use camino::Utf8Path;
    use camino_tempfile::tempdir;
    use insta::assert_debug_snapshot;
    use procfs::net::TcpNetEntries;
    use procfs::process::Process;
    use procfs::{FromReadSI, ProcError, current_system_info};
    use state::fs;

    use super::super::inspect_loopback_ports;
    use super::{LowPortState, diagnostic_read, record_process_owners, socket_ports};

    #[test]
    fn descriptor_read_failures_are_not_silently_omitted() -> Result<()> {
        let directory = tempdir()?;
        let root = directory.path().join(process::id().to_string());
        fs::write_sensitive_file(
            &root.join("stat"),
            &fs::read_to_string(Utf8Path::new("/proc/self/stat"))?,
        )?;
        fs::ensure_user_dir(&root.join("fd"))?;
        fs::symlink_file(Utf8Path::new("socket:[]"), &root.join("fd/7"))?;
        let process = Process::new_with_root(root.into_std_path_buf())?;
        let mut ports = [LowPortState {
            port: 80,
            available: false,
            owners: Vec::new(),
        }];
        let result = record_process_owners(&process, &BTreeMap::from([(7, 80)]), &mut ports);
        assert!(
            matches!(&result, Err(ProcError::Incomplete(None))),
            "unexpected descriptor inspection result: {result:?}"
        );
        assert!(ports[0].owners.is_empty());
        Ok(())
    }

    #[test]
    fn socket_candidates_only_include_listeners_on_requested_unavailable_ports() -> Result<()> {
        let entries = TcpNetEntries::from_read(
            b"sl local_address rem_address st tx_queue:rx_queue tr:tm->when retrnsmt uid timeout inode\n\
0: 0100007F:0050 00000000:0000 0A 00000000:00000000 00:00000000 00000000 1000 0 101\n\
1: 00000000:01BB 00000000:0000 0A 00000000:00000000 00:00000000 00000000 1000 0 102\n\
2: 00000000000000000000000000000000:0050 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000 1000 0 103\n\
3: 0100007F:0050 0100007F:C000 01 00000000:00000000 00:00000000 00000000 1000 0 104\n\
4: 0100007F:1F40 00000000:0000 0A 00000000:00000000 00:00000000 00000000 1000 0 105\n\
5: 010200C0:0050 00000000:0000 0A 00000000:00000000 00:00000000 00000000 1000 0 106\n".as_slice(),
            current_system_info(),
        )?;
        let ports = [
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
        assert_debug_snapshot!(socket_ports(entries.0, &ports), @r"
        {
            101: 80,
            103: 80,
            106: 80,
        }
        ");
        Ok(())
    }

    #[test]
    fn diagnostic_races_skip_only_missing_or_inaccessible_metadata() -> Result<()> {
        assert_eq!(diagnostic_read(Ok(42))?, Some(42));
        for error in [ProcError::NotFound(None), ProcError::PermissionDenied(None)] {
            assert!(matches!(diagnostic_read::<()>(Err(error)), Ok(None)));
        }
        assert!(matches!(
            diagnostic_read::<()>(Err(ProcError::Incomplete(None))),
            Err(ProcError::Incomplete(None))
        ));
        assert!(matches!(
            diagnostic_read::<()>(Err(ProcError::Io(io::Error::other("fixture read failed"), None))),
            Err(ProcError::Io(source, _)) if source.kind() == io::ErrorKind::Other
        ));
        Ok(())
    }

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
