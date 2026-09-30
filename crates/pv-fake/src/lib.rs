//! Native fake runtimes for PV's daemon tests.
//!
//! Tests put `pv-fake` at a runtime's executable path with [`install`]. The fake reads its
//! [`Scenario`] from `<executable>.pv-fake.json`, appends [`Event`]s to
//! `<executable>.pv-fake.events.jsonl`, and exits when the test process that installed it dies.
//!
//! Design: `docs/superpowers/specs/2026-09-29-pv-fake-design.md`.

use std::io::{self, Write};
use std::process::ExitCode;

#[cfg(unix)]
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

#[cfg(unix)]
mod events;
#[cfg(unix)]
mod fake;
#[cfg(unix)]
mod gateway;
#[cfg(unix)]
mod install;
#[cfg(unix)]
mod lifeline;

#[cfg(unix)]
pub use events::{Event, EventKind};
#[cfg(unix)]
pub use install::{InstalledFake, binary, install, install_with};

/// Identifies this pv-fake build; see `build.rs`.
#[cfg(unix)]
const BUILD_ID: &str = env!("PV_FAKE_BUILD_ID");

/// A scenario file: the scenario plus the pv-fake build that wrote it.
#[cfg(unix)]
#[derive(Deserialize, Serialize)]
struct ScenarioFile<S> {
    build_id: String,
    #[serde(flatten)]
    scenario: S,
}

/// How an installed fake behaves.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Scenario {
    pub persona: Persona,
    /// Descriptor number of the inherited lifeline pipe read end, if the installer has one.
    pub lifeline_fd: Option<i32>,
}

/// The runtime a fake emulates.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Persona {
    /// Stays alive until SIGTERM or SIGINT, then exits 0.
    LongRunning,
    /// Caddy's `validate` and `run`: HTTP, HTTPS, the admin socket, and PV's health route.
    Caddy,
    /// FrankenPHP embeds Caddy, so this behaves like [`Persona::Caddy`].
    #[serde(rename = "frankenphp")]
    FrankenPhp,
}

/// Entry point shared by the `pv-fake` binary and the daemon's `pv-fake` example.
pub fn main() -> ExitCode {
    #[cfg(unix)]
    let result = fake::run();
    #[cfg(not(unix))]
    let result: anyhow::Result<ExitCode> = Err(anyhow::anyhow!("pv-fake only runs on Unix"));

    match result {
        Ok(code) => code,
        Err(error) => {
            let _write_result = writeln!(io::stderr(), "pv-fake: {error:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(unix)]
fn scenario_path(executable: &Utf8Path) -> Utf8PathBuf {
    Utf8PathBuf::from(format!("{executable}.pv-fake.json"))
}

#[cfg(unix)]
fn events_path(executable: &Utf8Path) -> Utf8PathBuf {
    Utf8PathBuf::from(format!("{executable}.pv-fake.events.jsonl"))
}
