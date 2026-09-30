use std::process::ExitCode;

use anyhow::{Context, Result, anyhow, bail};
use camino::Utf8PathBuf;
use tokio::signal::unix::{SignalKind, signal};

use crate::events::{EventKind, EventLog};
use crate::{Persona, Scenario, events_path, lifeline, scenario_path};

pub(crate) fn run() -> Result<ExitCode> {
    let argv = std::env::args_os()
        .map(|argument| {
            argument
                .into_string()
                .map_err(|argument| anyhow!("argument {argument:?} is not UTF-8"))
        })
        .collect::<Result<Vec<_>>>()?;
    let Some(executable) = argv.first().map(Utf8PathBuf::from) else {
        bail!("started without an argv[0]");
    };
    if !executable.is_absolute() {
        bail!(
            "started as {executable}; start pv-fake by absolute path so it can find its scenario"
        );
    }
    let scenario_path = scenario_path(&executable);
    if !state::fs::path_is_file(&scenario_path)? {
        bail!("no scenario file at {scenario_path}; install fakes with pv_fake::install");
    }
    let scenario: Scenario = serde_json::from_str(&state::fs::read_to_string(&scenario_path)?)
        .with_context(|| format!("parsing {scenario_path}"))?;
    let events = EventLog::new(events_path(&executable));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(run_persona(scenario, argv, events));
    // The lifeline watcher blocks on its pipe until the test process exits; don't wait for it.
    runtime.shutdown_background();

    result
}

async fn run_persona(scenario: Scenario, argv: Vec<String>, events: EventLog) -> Result<ExitCode> {
    // Install handlers first, so a SIGTERM that arrives during startup is handled, not fatal.
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    let lifeline_armed = match scenario.lifeline_fd {
        Some(fd) => lifeline::arm(fd, events.clone()),
        None => false,
    };
    events.record(EventKind::Started {
        persona: scenario.persona,
        argv,
        lifeline_armed,
    })?;

    let code = match scenario.persona {
        Persona::LongRunning => {
            let received = tokio::select! {
                _ = terminate.recv() => "SIGTERM",
                _ = interrupt.recv() => "SIGINT",
            };
            events.record(EventKind::Signal {
                signal: received.to_owned(),
            })?;
            0
        }
    };
    events.record(EventKind::Exit { code })?;

    Ok(ExitCode::from(code))
}
