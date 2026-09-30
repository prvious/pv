//! Ties each fake's lifetime to the test process that installed it.
//!
//! The test process owns a pipe. The write end is close-on-exec, so it never leaves the test
//! process. The read end is inherited by everything the test spawns, including runtimes the
//! daemon supervisor starts. A fake blocks reading it, and the read returns end-of-file only once
//! every write end is closed. That happens when the test process exits for any reason, SIGKILL
//! included.

use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::{Mutex, PoisonError};

use anyhow::Result;
use camino::Utf8PathBuf;
use rustix::fs::FileType;
use rustix::io::{FdFlags, fcntl_setfd};
use rustix::process::{Signal, getpgrp, getpid, kill_process, kill_process_group};

use crate::events::{EventKind, EventLog};

struct Lifeline {
    read: OwnedFd,
    _write: OwnedFd,
}

static LIFELINE: Mutex<Option<Lifeline>> = Mutex::new(None);

/// Returns the inheritable read end of this test process's lifeline, creating it on first use.
pub(crate) fn test_process_read_fd() -> Result<i32> {
    let mut lifeline = LIFELINE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(lifeline) = lifeline.as_ref() {
        return Ok(lifeline.read.as_raw_fd());
    }

    let (read, write) = rustix::pipe::pipe()?;
    // ponytail: macOS has no pipe2, so close-on-exec lands one syscall late. A process spawned in
    // that instant would hold the pipe open and keep fakes alive. If a leak is ever traced here,
    // switch to a per-install named FIFO that the test opens with O_CLOEXEC and fakes open by path.
    fcntl_setfd(&write, FdFlags::CLOEXEC)?;
    let read_fd = read.as_raw_fd();
    *lifeline = Some(Lifeline {
        read,
        _write: write,
    });

    Ok(read_fd)
}

/// Watches the inherited lifeline `fd` in the background. Returns whether it is armed.
pub(crate) fn arm(fd: i32, events: EventLog) -> bool {
    // Opening /dev/fd/N duplicates descriptor N, so the watcher never owns a raw descriptor.
    let path = Utf8PathBuf::from(format!("/dev/fd/{fd}"));
    let is_pipe = rustix::fs::stat(path.as_std_path())
        .is_ok_and(|stat| FileType::from_raw_mode(stat.st_mode) == FileType::Fifo);
    if !is_pipe {
        // Nothing to watch, e.g. a fake started by hand; it must not treat this as parent loss.
        return false;
    }

    tokio::task::spawn_blocking(move || {
        // Returns once every write end is closed: the installing test process is gone.
        let _read_result = state::fs::read_to_string(&path);
        let _record_result = events.record(EventKind::LifelineFired);
        terminate();
    });

    true
}

fn terminate() {
    let pid = getpid();
    // A supervised runtime leads its own process group, so take the whole group down, as the
    // daemon's config validation anchor does. A fake started directly by a test is in the test's
    // group and only kills itself.
    if getpgrp() == pid {
        let _group_result = kill_process_group(pid, Signal::KILL);
    }
    let _process_result = kill_process(pid, Signal::KILL);
}
