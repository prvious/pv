use std::io;
use std::mem::{self, MaybeUninit};
use std::ptr;
use std::str;
use std::thread;
use std::time::Duration;

use camino::Utf8PathBuf;
use thiserror::Error;

use super::{ProcessIdentity, ProcessStartIdentity};
use crate::PlatformError;

const MAX_SNAPSHOT_ATTEMPTS: usize = 5;
const MICROSECONDS_PER_SECOND: u64 = 1_000_000;
const SNAPSHOT_RETRY_DELAY: Duration = Duration::from_millis(1);

pub(super) fn inspect_process_identity(pid: u32) -> Result<Option<ProcessIdentity>, PlatformError> {
    inspect_process_identity_inner(pid).map_err(|source| PlatformError::ProcessIdentityInspection {
        source: Box::new(source),
    })
}

fn inspect_process_identity_inner(pid: u32) -> Result<Option<ProcessIdentity>, InspectionError> {
    let native_pid = i32::try_from(pid).map_err(|_source| InspectionError::InvalidPid { pid })?;

    for _attempt in 1..=MAX_SNAPSHOT_ATTEMPTS {
        let Some(start_identity) = process_start_identity(native_pid)? else {
            return Ok(None);
        };
        let Some((executable, argument_zero, arguments)) = process_arguments(native_pid)? else {
            return Ok(None);
        };
        let Some(confirmed_start_identity) = process_start_identity(native_pid)? else {
            return Ok(None);
        };

        if start_identity == confirmed_start_identity {
            return Ok(Some(ProcessIdentity {
                executable,
                argument_zero,
                arguments,
                start_identity,
            }));
        }
    }

    Err(InspectionError::UnstableIdentity {
        pid,
        attempts: MAX_SNAPSHOT_ATTEMPTS,
    })
}

fn process_start_identity(
    pid: libc::pid_t,
) -> Result<Option<ProcessStartIdentity>, InspectionError> {
    let expected = mem::size_of::<libc::proc_bsdinfo>();
    let buffer_size = i32::try_from(expected)
        .map_err(|_source| InspectionError::ProcessInfoTooLarge { size: expected })?;
    let mut process_info = MaybeUninit::<libc::proc_bsdinfo>::uninit();

    // SAFETY: `process_info` points to uninitialized storage large enough for one
    // `proc_bsdinfo`, and `buffer_size` describes that exact writable region.
    // The value is read only after macOS reports that it initialized every byte.
    let actual = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            process_info.as_mut_ptr().cast(),
            buffer_size,
        )
    };

    if actual <= 0 {
        let source = io::Error::last_os_error();
        if process_not_found(&source) {
            return Ok(None);
        }

        return Err(InspectionError::ProcessInfo { pid, source });
    }
    let actual = usize::try_from(actual)
        .map_err(|_source| InspectionError::InvalidProcessInfoSize { expected, actual })?;
    if actual != expected {
        return Err(InspectionError::IncompleteProcessInfo { expected, actual });
    }

    // SAFETY: `proc_pidinfo` returned the exact `proc_bsdinfo` byte size above,
    // proving that macOS initialized the complete value.
    let process_info = unsafe { process_info.assume_init() };
    let expected_pid =
        u32::try_from(pid).map_err(|_source| InspectionError::InvalidNativePid { pid })?;
    if process_info.pbi_pid != expected_pid {
        return Err(InspectionError::ProcessIdMismatch {
            expected: expected_pid,
            actual: process_info.pbi_pid,
        });
    }
    if process_info.pbi_start_tvsec == 0 || process_info.pbi_start_tvusec >= MICROSECONDS_PER_SECOND
    {
        return Err(InspectionError::InvalidStartIdentity {
            seconds: process_info.pbi_start_tvsec,
            microseconds: process_info.pbi_start_tvusec,
        });
    }

    Ok(Some(ProcessStartIdentity {
        seconds: process_info.pbi_start_tvsec,
        microseconds: process_info.pbi_start_tvusec,
    }))
}

fn process_arguments(
    pid: libc::pid_t,
) -> Result<Option<(Utf8PathBuf, String, Vec<String>)>, InspectionError> {
    for _attempt in 1..=MAX_SNAPSHOT_ATTEMPTS {
        let capacity = match query_process_arguments(pid, None) {
            Ok(capacity) => capacity,
            Err(source) if process_not_found(&source) || argument_query_is_unavailable(&source) => {
                return Ok(None);
            }
            Err(source) if argument_query_is_transient(&source) => {
                thread::sleep(SNAPSHOT_RETRY_DELAY);
                continue;
            }
            Err(source) => return Err(InspectionError::ArgumentSize { pid, source }),
        };
        let mut buffer = vec![0; capacity];

        match query_process_arguments(pid, Some(&mut buffer)) {
            Ok(actual) if actual <= capacity => {
                buffer.truncate(actual);
                return parse_process_arguments(&buffer).map(Some);
            }
            Ok(actual) => {
                return Err(InspectionError::InvalidArgumentSize { capacity, actual });
            }
            Err(source) if process_not_found(&source) || argument_query_is_unavailable(&source) => {
                return Ok(None);
            }
            Err(source) if argument_query_is_transient(&source) => {
                thread::sleep(SNAPSHOT_RETRY_DELAY);
            }
            Err(source) => return Err(InspectionError::ArgumentRead { pid, source }),
        }
    }

    Err(InspectionError::ArgumentSnapshotUnstable {
        pid,
        attempts: MAX_SNAPSHOT_ATTEMPTS,
    })
}

fn query_process_arguments(pid: libc::pid_t, buffer: Option<&mut [u8]>) -> io::Result<usize> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut length = buffer.as_ref().map_or(0, |bytes| bytes.len());
    let pointer = buffer.map_or(ptr::null_mut(), |bytes| bytes.as_mut_ptr().cast());

    // SAFETY: `mib` contains the documented read-only KERN_PROCARGS2 query,
    // `length` points to valid writable storage, and `pointer` is either null for
    // the size query or covers `length` writable bytes for the data query. Both
    // new-value arguments are null/zero because this query does not mutate state.
    let status = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as libc::c_uint,
            pointer,
            &mut length,
            ptr::null_mut(),
            0,
        )
    };

    if status == 0 {
        Ok(length)
    } else {
        Err(io::Error::last_os_error())
    }
}

fn parse_process_arguments(
    buffer: &[u8],
) -> Result<(Utf8PathBuf, String, Vec<String>), InspectionError> {
    let argument_count_size = mem::size_of::<libc::c_int>();
    let Some(argument_count_bytes) = buffer.get(..argument_count_size) else {
        return Err(InspectionError::ArgumentBufferTooShort {
            minimum: argument_count_size,
            actual: buffer.len(),
        });
    };
    let mut encoded_argument_count = [0; mem::size_of::<libc::c_int>()];
    encoded_argument_count.copy_from_slice(argument_count_bytes);
    let argument_count = libc::c_int::from_ne_bytes(encoded_argument_count);
    let argument_count = usize::try_from(argument_count)
        .map_err(|_source| InspectionError::InvalidArgumentCount { argument_count })?;
    if argument_count == 0 {
        return Err(InspectionError::InvalidArgumentCount { argument_count: 0 });
    }

    let encoded = &buffer[argument_count_size..];
    let Some(executable_end) = encoded.iter().position(|byte| *byte == 0) else {
        return Err(InspectionError::MissingExecutableTerminator);
    };
    if executable_end == 0 {
        return Err(InspectionError::MissingExecutable);
    }
    let executable =
        str::from_utf8(&encoded[..executable_end]).map_err(InspectionError::NonUtf8Executable)?;
    let mut encoded_arguments = &encoded[executable_end + 1..];
    while encoded_arguments.first() == Some(&0) {
        encoded_arguments = &encoded_arguments[1..];
    }

    let mut arguments = Vec::with_capacity(argument_count);
    for index in 0..argument_count {
        let Some(argument_end) = encoded_arguments.iter().position(|byte| *byte == 0) else {
            return Err(InspectionError::MissingArgumentTerminator { index });
        };
        let argument = str::from_utf8(&encoded_arguments[..argument_end])
            .map_err(|source| InspectionError::NonUtf8Argument { index, source })?;
        arguments.push(argument.to_string());
        encoded_arguments = &encoded_arguments[argument_end + 1..];
    }

    let mut arguments = arguments.into_iter();
    let Some(argument_zero) = arguments.next() else {
        return Err(InspectionError::InvalidArgumentCount { argument_count: 0 });
    };

    Ok((
        Utf8PathBuf::from(executable),
        argument_zero,
        arguments.collect(),
    ))
}

fn process_not_found(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound || error.raw_os_error() == Some(libc::ESRCH)
}

fn argument_query_is_unavailable(error: &io::Error) -> bool {
    // KERN_PROCARGS2 reports EINVAL when the target vanished or has no user stack.
    error.raw_os_error() == Some(libc::EINVAL)
}

fn argument_query_is_transient(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(libc::EIO) | Some(libc::ENOMEM))
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::argument_query_is_unavailable;

    #[test]
    fn argument_query_einval_means_process_identity_is_unavailable() {
        assert!(argument_query_is_unavailable(
            &io::Error::from_raw_os_error(libc::EINVAL)
        ));
        assert!(!argument_query_is_unavailable(
            &io::Error::from_raw_os_error(libc::EACCES)
        ));
    }
}

#[derive(Debug, Error)]
enum InspectionError {
    #[error("process id {pid} exceeds the macOS process id range")]
    InvalidPid { pid: u32 },

    #[error("native process id {pid} cannot be represented as an unsigned process id")]
    InvalidNativePid { pid: libc::pid_t },

    #[error("macOS process information size {size} exceeds the native API limit")]
    ProcessInfoTooLarge { size: usize },

    #[error("could not read macOS process information for pid {pid}: {source}")]
    ProcessInfo {
        pid: libc::pid_t,
        #[source]
        source: io::Error,
    },

    #[error("macOS returned invalid process information size {actual}; expected {expected}")]
    InvalidProcessInfoSize {
        expected: usize,
        actual: libc::c_int,
    },

    #[error("macOS returned {actual} process information bytes; expected {expected}")]
    IncompleteProcessInfo { expected: usize, actual: usize },

    #[error("macOS returned process id {actual} while inspecting pid {expected}")]
    ProcessIdMismatch { expected: u32, actual: u32 },

    #[error(
        "macOS returned invalid process-start identity {seconds} seconds and {microseconds} microseconds"
    )]
    InvalidStartIdentity { seconds: u64, microseconds: u64 },

    #[error("could not query argument size for pid {pid}: {source}")]
    ArgumentSize {
        pid: libc::pid_t,
        #[source]
        source: io::Error,
    },

    #[error("could not read arguments for pid {pid}: {source}")]
    ArgumentRead {
        pid: libc::pid_t,
        #[source]
        source: io::Error,
    },

    #[error("macOS returned {actual} argument bytes for a {capacity}-byte buffer")]
    InvalidArgumentSize { capacity: usize, actual: usize },

    #[error("macOS process arguments changed during {attempts} consecutive reads for pid {pid}")]
    ArgumentSnapshotUnstable { pid: libc::pid_t, attempts: usize },

    #[error("process identity changed during {attempts} consecutive reads for pid {pid}")]
    UnstableIdentity { pid: u32, attempts: usize },

    #[error("process argument buffer is too short: expected at least {minimum}, received {actual}")]
    ArgumentBufferTooShort { minimum: usize, actual: usize },

    #[error("process argument buffer reported invalid argument count {argument_count}")]
    InvalidArgumentCount { argument_count: libc::c_int },

    #[error("process argument buffer is missing the executable path terminator")]
    MissingExecutableTerminator,

    #[error("process argument buffer contains an empty executable path")]
    MissingExecutable,

    #[error("process executable path is not valid UTF-8: {0}")]
    NonUtf8Executable(#[source] str::Utf8Error),

    #[error("process argument {index} is missing its terminator")]
    MissingArgumentTerminator { index: usize },

    #[error("process argument {index} is not valid UTF-8: {source}")]
    NonUtf8Argument {
        index: usize,
        #[source]
        source: str::Utf8Error,
    },
}
