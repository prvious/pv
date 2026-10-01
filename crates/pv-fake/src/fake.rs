use std::os::fd::{AsRawFd, OwnedFd};
use std::process::{Child, ExitCode, Stdio};

use anyhow::{Context, Result, anyhow, bail};
use camino::Utf8PathBuf;
use rustix::io::{FdFlags, fcntl_setfd};
use tokio::signal::unix::{Signal, SignalKind, signal};

use crate::events::{EventKind, EventLog};
use crate::{
    BUILD_ID, DESCENDANT_FLAG, Persona, Scenario, ScenarioFile, events_path, gateway, lifeline,
    scenario_path,
};

#[expect(
    clippy::disallowed_types,
    reason = "pv-fake starts its own descendant, as a runtime starts a worker process"
)]
type DescendantCommand = std::process::Command;

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
    let scenario_file: ScenarioFile<serde_json::Value> =
        serde_json::from_str(&state::fs::read_to_string(&scenario_path)?)
            .with_context(|| format!("parsing {scenario_path}"))?;
    if scenario_file.build_id != BUILD_ID {
        bail!(
            "{executable} is an older pv-fake build than the one that installed it; rebuild the \
             daemon's examples (`cargo nextest run -p daemon` without `--test`, or `cargo build -p \
             daemon --examples`)"
        );
    }
    let scenario: Scenario = serde_json::from_value(scenario_file.scenario)
        .with_context(|| format!("parsing {scenario_path}"))?;
    let events = EventLog::new(events_path(&executable));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = match argv.as_slice() {
        [_executable, flag, parent_fd] if flag == DESCENDANT_FLAG => {
            let parent_fd = parent_fd
                .parse()
                .with_context(|| format!("descendant parent pipe {parent_fd}"))?;
            runtime.block_on(run_descendant(scenario, parent_fd, events))
        }
        _ => runtime.block_on(run_persona(scenario, argv, events)),
    };
    // The lifeline watcher blocks on its pipe until the test process exits; don't wait for it.
    runtime.shutdown_background();

    result
}

async fn run_persona(scenario: Scenario, argv: Vec<String>, events: EventLog) -> Result<ExitCode> {
    // Install handlers first, so a SIGTERM that arrives during startup is handled, not fatal.
    let mut signals = Signals::new()?;
    let lifeline_armed = match scenario.lifeline_fd {
        Some(fd) => lifeline::arm(fd, events.clone(), EventKind::LifelineFired),
        None => false,
    };
    events.record(EventKind::Started {
        persona: scenario.persona,
        argv: argv.clone(),
        lifeline_armed,
    })?;
    let descendant = if scenario.settings.descendant {
        Some(spawn_descendant(&argv, &events)?)
    } else {
        None
    };

    // Signals end a fake even while it is paused during startup.
    let started = tokio::select! {
        started = start_persona(&scenario, &argv, &events) => Ok(started?),
        received = signals.next() => Err(received),
    };
    let code = match started {
        Ok(Some(code)) => code,
        Ok(None) => record_signal(signals.next().await, &events)?,
        Err(received) => record_signal(received, &events)?,
    };
    if let Some(descendant) = descendant {
        descendant.stop()?;
    }
    events.record(EventKind::Exit { code })?;

    Ok(ExitCode::from(code))
}

/// Returns an exit code to exit with now, or `None` once the persona is serving.
async fn start_persona(
    scenario: &Scenario,
    argv: &[String],
    events: &EventLog,
) -> Result<Option<u8>> {
    match scenario.persona {
        Persona::LongRunning => Ok(None),
        Persona::Caddy | Persona::FrankenPhp => {
            gateway::start(argv, &scenario.settings, events).await
        }
    }
}

fn record_signal(received: &str, events: &EventLog) -> Result<u8> {
    events.record(EventKind::Signal {
        signal: received.to_owned(),
    })?;

    Ok(0)
}

/// A descendant this fake started, with the write end of the pipe it watches.
struct Descendant {
    process: Child,
    pipe: OwnedFd,
}

impl Descendant {
    /// Closes the pipe, which makes the descendant exit, and reaps it, so it doesn't linger as a
    /// zombie where nothing reaps orphans.
    fn stop(mut self) -> Result<()> {
        drop(self.pipe);
        self.process.wait()?;

        Ok(())
    }
}

/// Starts this fake again as a descendant in its process group. The descendant watches the read
/// end of a pipe whose write end only this process holds, so it exits when this process does.
fn spawn_descendant(argv: &[String], events: &EventLog) -> Result<Descendant> {
    let Some(executable) = argv.first() else {
        bail!("started without an argv[0]");
    };
    let (read, write) = rustix::pipe::pipe()?;
    // ponytail: close-on-exec lands one syscall late, as for the lifeline; nothing else in this
    // process spawns, so no other child can inherit the write end in between.
    fcntl_setfd(&write, FdFlags::CLOEXEC)?;
    let process = DescendantCommand::new(executable)
        .args([DESCENDANT_FLAG, &read.as_raw_fd().to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()?;
    events.record(EventKind::DescendantSpawned {
        descendant_pid: i32::try_from(process.id())?,
    })?;

    Ok(Descendant {
        process,
        pipe: write,
    })
}

/// A descendant stays alive, doing nothing, until its parent fake or the installing test process
/// is gone, or it is signaled.
async fn run_descendant(scenario: Scenario, parent_fd: i32, events: EventLog) -> Result<ExitCode> {
    let mut signals = Signals::new()?;
    if let Some(fd) = scenario.lifeline_fd {
        lifeline::arm(fd, events.clone(), EventKind::LifelineFired);
    }
    lifeline::arm(parent_fd, events, EventKind::ParentExited);
    signals.next().await;

    Ok(ExitCode::SUCCESS)
}

struct Signals {
    terminate: Signal,
    interrupt: Signal,
}

impl Signals {
    fn new() -> Result<Self> {
        Ok(Self {
            terminate: signal(SignalKind::terminate())?,
            interrupt: signal(SignalKind::interrupt())?,
        })
    }

    async fn next(&mut self) -> &'static str {
        tokio::select! {
            _ = self.terminate.recv() => "SIGTERM",
            _ = self.interrupt.recv() => "SIGINT",
        }
    }
}
