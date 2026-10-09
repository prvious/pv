use std::io::Write;
use std::net::SocketAddr;

use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use rustix::process::{getpgrp, getpid, getppid};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::Persona;

/// One line of a fake's event log.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Event {
    pub at: String,
    pub pid: i32,
    pub process_group: i32,
    pub parent_pid: Option<i32>,
    #[serde(flatten)]
    pub kind: EventKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum EventKind {
    Started {
        persona: Persona,
        argv: Vec<String>,
        lifeline_armed: bool,
    },
    TcpAccepted,
    TcpReady {
        address: SocketAddr,
    },
    Signal {
        signal: String,
    },
    LifelineFired,
    /// A pause began; the fake continues once `until` exists.
    Held {
        until: Utf8PathBuf,
    },
    DescendantSpawned {
        descendant_pid: i32,
    },
    /// Recorded by a descendant once its parent fake closes their pipe, on exit or by dying, just
    /// before the descendant exits too.
    ParentExited,
    Exit {
        code: u8,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct EventLog {
    path: Utf8PathBuf,
}

impl EventLog {
    pub(crate) fn new(path: Utf8PathBuf) -> Self {
        Self { path }
    }

    pub(crate) fn record(&self, kind: EventKind) -> Result<()> {
        let event = Event {
            at: OffsetDateTime::now_utc().format(&Rfc3339)?,
            pid: getpid().as_raw_nonzero().get(),
            process_group: getpgrp().as_raw_nonzero().get(),
            parent_pid: getppid().map(|pid| pid.as_raw_nonzero().get()),
            kind,
        };
        let mut line = serde_json::to_string(&event)?;
        line.push('\n');
        // A single append-mode write keeps each event on one whole line.
        state::fs::open_append_file(&self.path)?.write_all(line.as_bytes())?;

        Ok(())
    }
}

/// Reads complete events, ignoring a trailing line the fake is still writing.
pub(crate) fn read_events(path: &Utf8Path) -> Result<Vec<Event>> {
    if !state::fs::path_entry_exists(path)? {
        return Ok(Vec::new());
    }

    state::fs::read_to_string(path)?
        .split_inclusive('\n')
        .filter(|line| line.ends_with('\n'))
        .map(|line| Ok(serde_json::from_str(line)?))
        .collect()
}
