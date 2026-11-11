//! Linux: lets PV's daemon open ports 80 and 443 as the user through the kernel's
//! `net.ipv4.ip_unprivileged_port_start` setting, as Lerd and rootless Docker and Podman do.

use camino::{Utf8Path, Utf8PathBuf};

use crate::PlatformError;
use crate::systemd::{read_owned_unit, remove_leftover_candidate, write_owned_unit};

pub const UNPRIVILEGED_PORTS_PATH: &str = "/etc/sysctl.d/99-pv-ports.conf";
const SETTING: &str = "net.ipv4.ip_unprivileged_port_start";
const LIVE_PATH: &str = "/proc/sys/net/ipv4/ip_unprivileged_port_start";
const PV_PORT_START: u32 = 80;
const KERNEL_DEFAULT_PORT_START: u32 = 1024;
/// Where `systemd-sysctl` reads settings at boot, besides `/etc/sysctl.conf`.
const SYSCTL_DIRECTORIES: [&str; 4] = [
    "/etc/sysctl.d",
    "/run/sysctl.d",
    "/usr/local/lib/sysctl.d",
    "/usr/lib/sysctl.d",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UnprivilegedPortsState {
    /// PV's file is in place and the kernel lets unprivileged programs open port 80.
    Current,
    /// PV's file is missing.
    Missing,
    /// PV's file is in place, but the kernel still reserves port 80; `files` also set it.
    Overridden {
        value: u32,
        files: Vec<Utf8PathBuf>,
    },
    Unreadable {
        message: String,
    },
}

/// Reads PV's file and the live value, unprivileged.
pub fn inspect_unprivileged_ports() -> UnprivilegedPortsState {
    let inspected = || -> Result<UnprivilegedPortsState, PlatformError> {
        let present = state::fs::path_is_file(Utf8Path::new(UNPRIVILEGED_PORTS_PATH))
            .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?
            && state::fs::read_to_string(Utf8Path::new(UNPRIVILEGED_PORTS_PATH))
                .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?
                .lines()
                .any(|line| line.trim() == "# Managed by PV");
        if !present {
            return Ok(UnprivilegedPortsState::Missing);
        }
        let value = live_value()?;
        if value <= PV_PORT_START {
            return Ok(UnprivilegedPortsState::Current);
        }

        Ok(UnprivilegedPortsState::Overridden {
            value,
            files: other_files_setting_ports()?,
        })
    };

    inspected().unwrap_or_else(|error| UnprivilegedPortsState::Unreadable {
        message: error.to_string(),
    })
}

/// Writes PV's file and applies it now. A value that is already lower, such as one a rootless
/// container setup chose, is kept now and persisted, since PV's file sorts last at boot.
pub(crate) fn apply_unprivileged_ports_privileged() -> Result<(), PlatformError> {
    let live = live_value()?;
    let persisted = live.min(PV_PORT_START);
    write_owned_unit(
        Utf8Path::new(UNPRIVILEGED_PORTS_PATH),
        &format!("# Managed by PV\n{SETTING}={persisted}\n"),
    )?;
    if live > PV_PORT_START {
        write_live_value(PV_PORT_START)?;
    }

    Ok(())
}

/// Deletes PV's file, and restores the kernel default only when PV's file was there, nothing
/// else sets the value, and it is still PV's. A value an administrator set without PV stays.
pub(crate) fn remove_unprivileged_ports_privileged() -> Result<(), PlatformError> {
    let path = Utf8Path::new(UNPRIVILEGED_PORTS_PATH);
    let owned = read_owned_unit(path)?.is_some();
    if owned {
        state::fs::remove_file_if_exists(path)
            .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;
    }
    remove_leftover_candidate(path)?;
    if owned && other_files_setting_ports()?.is_empty() && live_value()? == PV_PORT_START {
        write_live_value(KERNEL_DEFAULT_PORT_START)?;
    }

    Ok(())
}

fn live_value() -> Result<u32, PlatformError> {
    let value = state::fs::read_to_string(Utf8Path::new(LIVE_PATH))
        .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;
    value.trim().parse().map_err(|error| {
        PlatformError::SystemIntegration(format!("{LIVE_PATH} holds {value:?}: {error}"))
    })
}

fn write_live_value(value: u32) -> Result<(), PlatformError> {
    use rustix::fs::{Mode, OFlags, open};

    let written = open(LIVE_PATH, OFlags::WRONLY | OFlags::CLOEXEC, Mode::empty())
        .and_then(|file| rustix::io::write(&file, format!("{value}\n").as_bytes()));
    written.map(|_length| ()).map_err(|error| {
        PlatformError::SystemIntegration(format!("could not write {LIVE_PATH}: {error}"))
    })
}

/// The sysctl files besides PV's that set the setting, in the places `systemd-sysctl` reads.
fn other_files_setting_ports() -> Result<Vec<Utf8PathBuf>, PlatformError> {
    let mut candidates = vec![Utf8PathBuf::from("/etc/sysctl.conf")];
    for directory in SYSCTL_DIRECTORIES.map(Utf8Path::new) {
        if state::fs::path_is_directory(directory)
            .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?
        {
            candidates.extend(
                state::fs::read_dir_paths(directory)
                    .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?
                    .into_iter()
                    .filter(|path| path.extension() == Some("conf")),
            );
        }
    }
    let mut files = Vec::new();
    for path in candidates {
        if path == UNPRIVILEGED_PORTS_PATH
            || !state::fs::path_is_file(&path)
                .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?
        {
            continue;
        }
        let content = state::fs::read_to_string(&path)
            .map_err(|error| PlatformError::SystemIntegration(error.to_string()))?;
        if content.lines().any(sets_ports) {
            files.push(path);
        }
    }
    files.sort();

    Ok(files)
}

/// A `key = value` line for the setting, in either of sysctl.d's key spellings.
fn sets_ports(line: &str) -> bool {
    line.split_once('=').is_some_and(|(key, _value)| {
        key.trim().trim_start_matches('-').replace('/', ".") == SETTING
    })
}
