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
use camino::Utf8Path;
use camino::Utf8PathBuf;
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
pub use gateway::write_gateway_control;
#[cfg(unix)]
pub use install::{InstalledFake, binary, install, install_with, install_with_settings};

/// The argument a fake starts its descendant with, followed by the descendant's parent pipe.
#[cfg(unix)]
const DESCENDANT_FLAG: &str = "--pv-fake-descendant";

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
    #[serde(default)]
    pub settings: FakeSettings,
}

/// Behavior a test chooses on top of its persona. The default is the recorded behavior.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FakeSettings {
    /// Which listeners a Gateway persona's `run` opens.
    pub gateway_listeners: GatewayListeners,
    /// Pauses a Gateway persona's `validate` before it checks the config.
    pub validate_pause: Option<Pause>,
    /// Makes a Gateway persona's `validate` exit with this code instead of checking the config.
    pub validate_exit_code: Option<u8>,
    /// Pauses a Gateway persona's `run` after its HTTP and HTTPS ports open and before its admin
    /// socket does.
    pub run_pause: Option<Pause>,
    /// Starts one child process in the fake's process group, as runtimes start workers. It exits
    /// when the fake does.
    pub descendant: bool,
}

/// Holds a fake at a known point, recording a `held` event, until a test creates `until`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Pause {
    pub until: Utf8PathBuf,
}

/// The listeners a Gateway persona opens, for tests of runtimes that never become ready.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayListeners {
    /// HTTP, HTTPS and the admin socket, as configured.
    #[default]
    All,
    /// Only the admin socket; loads are accepted, but no HTTP or HTTPS port ever opens.
    AdminOnly,
    /// Nothing: the process stays alive without serving.
    Nothing,
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
