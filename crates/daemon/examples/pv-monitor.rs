//! Builds a runtime monitor with test hooks along with the daemon's tests; see
//! `pv_fake::example_binary`. It runs the entry points of `pv monitor:run` and `pv monitor:gate`
//! with the [`daemon::MonitorHooks`] a test writes to `pv-monitor-hooks.json` in its PV home.

use std::io::{self, Write as _};
use std::process::ExitCode;

use daemon::{DaemonError, MONITOR_GATE_COMMAND, MONITOR_RUN_COMMAND, MonitorHooks};
use state::PvPaths;

const HOOKS_FILE: &str = "pv-monitor-hooks.json";

fn main() -> ExitCode {
    let command = std::env::args().nth(1);
    let result = PvPaths::default_home()
        .map_err(DaemonError::from)
        .and_then(|paths| {
            let hooks = hooks(&paths);
            match command.as_deref() {
                Some(MONITOR_RUN_COMMAND) => daemon::run_monitor_blocking(paths, hooks),
                Some(MONITOR_GATE_COMMAND) => daemon::run_monitor_gate(&hooks),
                _ => Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("usage: pv-monitor {MONITOR_RUN_COMMAND}|{MONITOR_GATE_COMMAND}"),
                )
                .into()),
            }
        });

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _write_result = writeln!(io::stderr(), "pv-monitor: {error}");
            ExitCode::FAILURE
        }
    }
}

fn hooks(paths: &PvPaths) -> MonitorHooks {
    state::fs::read_to_string(&paths.home().join(HOOKS_FILE))
        .ok()
        .and_then(|hooks| serde_json::from_str(&hooks).ok())
        .unwrap_or_default()
}
