use std::io::{self, Read};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;

use camino::Utf8PathBuf;
use procfs::process::{Process, Stat, all_processes};
use procfs::{ProcError, ProcResult};
use rustix::process::{Pid, PidfdFlags, WaitId, WaitIdOptions, getuid, pidfd_open, waitid};

use super::{BootSessionId, ProcessEvents, ProcessIdentity, ProcessStartIdentity};
use crate::PlatformError;

const MAX_SNAPSHOT_ATTEMPTS: usize = 5;

pub(super) fn current_boot_session_id() -> Result<BootSessionId, PlatformError> {
    let value = procfs::sys::kernel::random::boot_id()
        .map_err(|source| PlatformError::BootSessionInspection(io::Error::other(source)))?;
    value
        .trim()
        .to_owned()
        .try_into()
        .map_err(PlatformError::BootSessionInspection)
}

/// Observes a child's exit without reaping it. Callers register this descriptor with
/// their event loop and recheck the process birth identity before signalling.
#[derive(Debug)]
pub struct ProcessExitWatch {
    descriptor: OwnedFd,
}

impl ProcessExitWatch {
    pub fn new(pid: u32) -> Result<Self, PlatformError> {
        let pid = native_pid(pid).map_err(inspection_error)?;
        let descriptor = pidfd_open(pid, PidfdFlags::empty()).map_err(inspection_error)?;
        Ok(Self { descriptor })
    }

    /// Reads the held status without consuming it. Only the child's parent can read it.
    pub fn try_exit_status(&mut self) -> Result<Option<ExitStatus>, PlatformError> {
        let Some(status) = waitid(
            WaitId::PidFd(self.descriptor.as_fd()),
            WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
        )
        .map_err(inspection_error)?
        else {
            return Ok(None);
        };
        let raw = if let Some(code) = status.exit_status() {
            code << 8
        } else if let Some(signal) = status.terminating_signal() {
            signal | if status.dumped() { 0x80 } else { 0 }
        } else {
            return Err(inspection_error(io::Error::new(
                io::ErrorKind::InvalidData,
                "exit notification omitted process status",
            )));
        };
        Ok(Some(ExitStatus::from_raw(raw)))
    }

    pub fn try_events(&mut self) -> Result<ProcessEvents, PlatformError> {
        Ok(ProcessEvents {
            exec: false,
            exit: self.try_exit_status()?,
        })
    }
}

impl AsFd for ProcessExitWatch {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.descriptor.as_fd()
    }
}

impl AsRawFd for ProcessExitWatch {
    fn as_raw_fd(&self) -> RawFd {
        self.as_fd().as_raw_fd()
    }
}

pub(super) fn inspect_process_start_identity(
    pid: u32,
) -> Result<Option<ProcessStartIdentity>, PlatformError> {
    let Some(process) = open_process(pid).map_err(inspection_error)? else {
        return Ok(None);
    };
    Ok(read_stat(&process)
        .map_err(inspection_error)?
        .map(|stat| start_identity(&stat)))
}

pub(super) fn process_is_zombie(pid: u32) -> Result<bool, PlatformError> {
    let Some(process) = open_process(pid).map_err(inspection_error)? else {
        return Ok(false);
    };
    Ok(read_stat(&process)
        .map_err(inspection_error)?
        .is_some_and(|stat| matches!(stat.state, 'Z' | 'X')))
}

pub(super) fn inspect_process_identity(pid: u32) -> Result<Option<ProcessIdentity>, PlatformError> {
    inspect_identity(pid).map_err(inspection_error)
}

fn inspect_identity(pid: u32) -> ProcResult<Option<ProcessIdentity>> {
    let Some(process) = open_process(pid)? else {
        return Ok(None);
    };
    for _attempt in 0..MAX_SNAPSHOT_ATTEMPTS {
        let Some(before) = read_stat(&process)? else {
            return Ok(None);
        };
        let (executable, argument_zero, arguments) = match process_arguments(&process) {
            Ok(arguments) => arguments,
            Err(_source) if read_stat(&process)?.is_none() => return Ok(None),
            Err(source) => return Err(source),
        };
        let Some(after) = read_stat(&process)? else {
            return Ok(None);
        };
        if before.starttime == after.starttime {
            return Ok(Some(ProcessIdentity {
                executable,
                argument_zero,
                arguments,
                start_identity: start_identity(&before),
            }));
        }
    }
    Err(ProcError::Other(format!(
        "process {pid} identity changed during {MAX_SNAPSHOT_ATTEMPTS} inspections"
    )))
}

fn process_arguments(process: &Process) -> ProcResult<(Utf8PathBuf, String, Vec<String>)> {
    let executable = process
        .exe()?
        .into_os_string()
        .into_string()
        .map_err(|path| ProcError::Other(format!("process executable is not UTF-8: {path:?}")))?;
    let executable =
        Utf8PathBuf::from(executable.strip_suffix(" (deleted)").unwrap_or(&executable));
    // Process::cmdline drops empty arguments. Read through the same pinned proc
    // directory so empty values and argument boundaries survive inspection.
    let mut command_line = String::new();
    process
        .open_relative("cmdline")?
        .read_to_string(&mut command_line)?;
    let command_line = command_line.strip_suffix('\0').ok_or_else(|| {
        ProcError::Other(format!(
            "process {} has no terminated command line",
            process.pid
        ))
    })?;
    let mut arguments = command_line.split('\0');
    let argument_zero = arguments
        .next()
        .ok_or_else(|| ProcError::Other(format!("process {} has no argument zero", process.pid)))?;
    Ok((
        executable,
        argument_zero.to_owned(),
        arguments.map(str::to_owned).collect(),
    ))
}

pub(super) fn process_group_has_live_members(process_group: u32) -> Result<bool, PlatformError> {
    let process_group = native_pid(process_group)
        .map_err(inspection_error)?
        .as_raw_pid();
    // A process can fork and exit during enumeration. Only two complete scans
    // with no live member prove that the group has stopped.
    Ok(
        group_has_live_members(process_group).map_err(inspection_error)?
            || group_has_live_members(process_group).map_err(inspection_error)?,
    )
}

fn group_has_live_members(process_group: i32) -> ProcResult<bool> {
    for process in all_processes()? {
        let process = match process {
            Ok(process) => process,
            Err(ProcError::NotFound(_)) => continue,
            Err(source) => return Err(source),
        };
        let stat = match process.stat() {
            Ok(stat) => stat,
            Err(ProcError::NotFound(_)) => continue,
            Err(ProcError::PermissionDenied(_)) if process.uid()? != getuid().as_raw() => continue,
            Err(source) => return Err(source),
        };
        if stat.pgrp == process_group && !matches!(stat.state, 'Z' | 'X') {
            return Ok(true);
        }
    }
    Ok(false)
}

fn open_process(pid: u32) -> ProcResult<Option<Process>> {
    match Process::new(native_pid(pid)?.as_raw_pid()) {
        Ok(process) => Ok(Some(process)),
        Err(ProcError::NotFound(_)) => Ok(None),
        Err(source) => Err(source),
    }
}

fn read_stat(process: &Process) -> ProcResult<Option<Stat>> {
    match process.stat() {
        Ok(stat) => Ok(Some(stat)),
        Err(ProcError::NotFound(_)) => Ok(None),
        Err(source) => Err(source),
    }
}

fn native_pid(pid: u32) -> ProcResult<Pid> {
    i32::try_from(pid)
        .ok()
        .and_then(Pid::from_raw)
        .ok_or_else(|| ProcError::Other(format!("invalid process id {pid}")))
}

fn start_identity(stat: &Stat) -> ProcessStartIdentity {
    let ticks_per_second = procfs::ticks_per_second();
    ProcessStartIdentity {
        seconds: stat.starttime / ticks_per_second,
        microseconds: (stat.starttime % ticks_per_second) * 1_000_000 / ticks_per_second,
    }
}

fn inspection_error(source: impl std::error::Error + Send + Sync + 'static) -> PlatformError {
    PlatformError::ProcessIdentityInspection {
        source: Box::new(source),
    }
}
