use std::fmt::Write as _;
use std::io::{self, BufRead as _, BufReader, Write as _};
use std::os::fd::{AsFd as _, OwnedFd};
use std::os::unix::net::UnixStream as StdUnixStream;
use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use futures_util::StreamExt as _;
use platform::{BootSessionId, ProcessEvents, ProcessExitWatch, ProcessStartIdentity};
use protocol::{DaemonTransport, ProtocolError, transport, write_line};
use rustix::fs::FileType;
use serde::{Deserialize, Serialize};
use state::{MonitorReservation, PvPaths, StateError, fs};
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep, timeout};

use super::{
    MONITOR_GATE_COMMAND, MONITOR_RUN_COMMAND, MonitorCleanup, MonitorErrorKind, MonitorExit,
    MonitorHooks, MonitorPause, MonitorStart, MonitorState, MonitorStop, rotation,
};
use crate::DaemonError;
use crate::supervisor::{
    PHP_INI_ENVIRONMENT_KEYS, reap_process_if_child, stop_process_group,
    wait_for_process_group_exit,
};
use crate::{StopSignal, build_runtime};

const PROTOCOL_VERSION: u32 = 1;
/// macOS keeps a Unix socket path in 104 bytes, including its terminating NUL.
const SOCKET_PATH_LIMIT: usize = 103;
const RECORD_FILE: &str = "monitor.json";
const SOCKET_FILE: &str = "sock";
const STARTUP_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const STARTUP_REPLY_TIMEOUT: Duration = Duration::from_secs(30);
const EXEC_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const RELEASE_TIMEOUT: Duration = Duration::from_secs(5);
const STOP_MARGIN: Duration = Duration::from_secs(5);
const GATE_FAILURE_TIMEOUT: Duration = Duration::from_millis(100);
const EXITING_LEADER_TIMEOUT: Duration = Duration::from_secs(1);
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const ROTATION_INTERVAL: Duration = Duration::from_secs(60);
const ROTATION_BYTES: u64 = 10 * 1024 * 1024;

/// The startup request a controller writes to a new monitor's stdout socket.
#[derive(Deserialize, Serialize)]
struct StartRequest {
    version: u32,
    instance: String,
    #[serde(flatten)]
    start: MonitorStart,
}

#[derive(Debug, Deserialize, Serialize)]
struct RequestHeader {
    version: u32,
    instance: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct Request {
    version: u32,
    instance: String,
    #[serde(flatten)]
    operation: Operation,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Operation {
    State,
    Stop(MonitorStop),
    Release,
}

#[derive(Debug, Deserialize, Serialize)]
struct Reply {
    version: u32,
    instance: String,
    #[serde(flatten)]
    result: ReplyResult,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
enum ReplyResult {
    State(MonitorState),
    Accepted,
    Error {
        kind: MonitorErrorKind,
        message: String,
    },
}

impl Reply {
    fn new(instance: &str, result: ReplyResult) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            instance: instance.to_owned(),
            result,
        }
    }
}

/// What a monitor records in `monitor.json` before its runtime may start: enough for a later
/// controller to authenticate the monitor, and to stop the runtime if the monitor dies. It holds
/// no secrets.
#[derive(Debug, Deserialize, Serialize)]
struct MonitorRecord {
    version: u32,
    subject: String,
    instance: String,
    boot_session_id: BootSessionId,
    monitor: RecordedProcess,
    runtime: RecordedProcess,
    command: Utf8PathBuf,
    arguments: Vec<String>,
    log_path: Utf8PathBuf,
    fallback_stop: MonitorStop,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct RecordedProcess {
    pid: u32,
    start_identity: ProcessStartIdentity,
}

#[derive(Deserialize, Serialize)]
struct GatePermission {
    command: Utf8PathBuf,
    arguments: Vec<String>,
}

#[derive(Deserialize, Serialize)]
struct GateFailure {
    message: String,
}

/// Starts a monitor for `start.subject` with `executable`, which is `pv` or a build of the
/// `pv-monitor` example, and returns its first state once the runtime has exec'd.
///
/// The reservation passes to the monitor as its stdin, so it stays held with no gap. Any failure
/// kills the monitor and recovers what it left.
pub async fn start_monitor(
    paths: &PvPaths,
    executable: &Utf8Path,
    start: MonitorStart,
) -> Result<MonitorState, DaemonError> {
    let subject = start.subject.clone();
    monitor_socket(paths, &subject)?;
    let reservation = MonitorReservation::acquire(paths, &subject)
        .map_err(|error| reservation_error(&subject, error))?;
    if let Some(record) = read_record(paths, &subject)? {
        return Err(DaemonError::MonitorUnresolved {
            subject,
            instance: record.instance,
        });
    }
    remove_monitor_dir(paths, &subject)?;
    let request = StartRequest {
        version: PROTOCOL_VERSION,
        instance: new_instance()?,
        start,
    };
    let (controller_end, monitor_end) = StdUnixStream::pair()?;
    let mut monitor = spawn_monitor(paths, executable, &reservation, monitor_end)?;

    match complete_startup(controller_end, &request).await {
        // Tokio reaps the monitor whenever it exits.
        Ok(state) => Ok(state),
        Err(error) => {
            let _kill_result = monitor.start_kill();
            let _wait_result = monitor.wait().await;
            match recover_reserved(paths, &subject).await {
                Ok(()) => Err(error),
                Err(cleanup) => Err(DaemonError::StartupCleanupFailed {
                    source: Box::new(error),
                    cleanup: Box::new(cleanup),
                }),
            }
        }
    }
}

/// Asks the monitor of `subject` for its state.
pub async fn monitor_state(paths: &PvPaths, subject: &str) -> Result<MonitorState, DaemonError> {
    match request(paths, subject, Operation::State).await?.1 {
        ReplyResult::State(state) => Ok(state),
        ReplyResult::Accepted | ReplyResult::Error { .. } => Err(unavailable(
            subject,
            "the monitor replied without its state",
        )),
    }
}

/// Asks the monitor of `subject` to stop its runtime, then waits until cleanup is decided.
pub async fn stop_monitor(
    paths: &PvPaths,
    subject: &str,
    stop: MonitorStop,
) -> Result<MonitorState, DaemonError> {
    request(paths, subject, Operation::Stop(stop)).await?;
    let deadline = Instant::now().checked_add(stop.grace().saturating_add(STOP_MARGIN));

    loop {
        let state = monitor_state(paths, subject).await?;
        if state.cleanup != MonitorCleanup::Pending {
            return Ok(state);
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(unavailable(subject, "the runtime's cleanup did not finish"));
        }
        sleep(POLL_INTERVAL).await;
    }
}

/// Releases the monitor of `subject` once its runtime has exited and its cleanup is complete,
/// then removes the monitor's records once its reservation proves it has exited.
pub async fn release_monitor(paths: &PvPaths, subject: &str) -> Result<(), DaemonError> {
    let (record, _accepted) = request(paths, subject, Operation::Release).await?;
    let reservation = wait_for_reservation(paths, subject).await?;
    // Only this instance's records: a replacement that failed may have left its own.
    if read_record(paths, subject)?.is_some_and(|current| current.instance == record.instance) {
        remove_monitor_dir(paths, subject)?;
    }
    drop(reservation);

    Ok(())
}

/// Recovers after the monitor of `subject` died: stops its runtime if the runtime is exactly the
/// recorded process, then removes the records. A live monitor holds its reservation, so this
/// refuses to touch it.
pub async fn recover_monitor(paths: &PvPaths, subject: &str) -> Result<(), DaemonError> {
    let reservation = MonitorReservation::acquire(paths, subject)
        .map_err(|error| reservation_error(subject, error))?;
    let result = recover_reserved(paths, subject).await;
    drop(reservation);

    result
}

async fn recover_reserved(paths: &PvPaths, subject: &str) -> Result<(), DaemonError> {
    let Some(record) = read_record(paths, subject)? else {
        return remove_monitor_dir(paths, subject);
    };
    // After a reboot, nothing the record names can still be running. A runtime leader that
    // exited is at most a zombie, which no live member keeps company.
    if record.boot_session_id == platform::current_boot_session_id()?
        && platform::process_group_has_live_members(record.runtime.pid)?
    {
        let runtime = record.runtime;
        match platform::inspect_process_start_identity(runtime.pid)? {
            Some(identity) if identity == runtime.start_identity => {
                stop_process_group(
                    runtime.pid,
                    record.fallback_stop.signal,
                    record.fallback_stop.grace(),
                    || runtime_matches(runtime),
                )
                .await?;
            }
            // A leader that is exiting has no identity yet still counts as live: it may finish.
            None if wait_for_process_group_exit(runtime.pid, EXITING_LEADER_TIMEOUT).await? => {}
            Some(_) | None => {
                return Err(DaemonError::RuntimeProcessIdentityChanged { pid: runtime.pid });
            }
        }
    }

    remove_monitor_dir(paths, subject)
}

fn runtime_matches(runtime: RecordedProcess) -> Result<bool, DaemonError> {
    Ok(platform::inspect_process_start_identity(runtime.pid)? == Some(runtime.start_identity))
}

#[expect(
    clippy::disallowed_types,
    reason = "the monitor controller owns spawning monitor processes"
)]
fn spawn_monitor(
    paths: &PvPaths,
    executable: &Utf8Path,
    reservation: &MonitorReservation,
    monitor_end: StdUnixStream,
) -> Result<tokio::process::Child, DaemonError> {
    let mut command = tokio::process::Command::new(executable);
    command
        .arg(MONITOR_RUN_COMMAND)
        .env("HOME", paths.home())
        .stdin(reservation.child_stdio()?)
        .stdout(OwnedFd::from(monitor_end))
        .stderr(Stdio::null());

    // Dropping `command` closes this process's copies of the reservation and the socket end.
    Ok(command.spawn()?)
}

async fn complete_startup(
    stream: StdUnixStream,
    request: &StartRequest,
) -> Result<MonitorState, DaemonError> {
    let subject = &request.start.subject;
    stream.set_nonblocking(true)?;
    let mut startup = transport(UnixStream::from_std(stream)?);
    write_line(&mut startup, request).await?;
    let reply = timeout(STARTUP_REPLY_TIMEOUT, read_reply(&mut startup))
        .await
        .map_err(|_elapsed| DaemonError::ProtocolTimedOut {
            phase: "monitor startup",
        })??
        .ok_or_else(|| startup_failed(subject, "the monitor exited before it started"))?;
    if reply.version != PROTOCOL_VERSION || reply.instance != request.instance {
        return Err(startup_failed(
            subject,
            "the monitor replied for another instance",
        ));
    }

    match reply.result {
        ReplyResult::State(state) => Ok(state),
        ReplyResult::Error { message, .. } => Err(startup_failed(subject, message)),
        ReplyResult::Accepted => Err(startup_failed(
            subject,
            "the monitor replied without its state",
        )),
    }
}

/// Sends `operation` to the monitor of `subject` after proving the socket is served by the
/// recorded monitor process, from this boot, and returns its record with the reply.
async fn request(
    paths: &PvPaths,
    subject: &str,
    operation: Operation,
) -> Result<(MonitorRecord, ReplyResult), DaemonError> {
    let record = read_record(paths, subject)?
        .ok_or_else(|| unavailable(subject, "no monitor is recorded"))?;
    let socket = monitor_socket(paths, subject)?;
    let stream = timeout(REQUEST_TIMEOUT, UnixStream::connect(&socket))
        .await
        .map_err(|_elapsed| unavailable(subject, "connecting timed out"))?
        .map_err(|error| unavailable(subject, &format!("cannot connect: {error}")))?;
    if !peer_is_monitor(&stream, &record)? {
        return Err(unavailable(
            subject,
            "the socket is not served by the recorded monitor",
        ));
    }
    let mut connection = transport(stream);
    let request = Request {
        version: PROTOCOL_VERSION,
        instance: record.instance.clone(),
        operation,
    };
    write_line(&mut connection, &request).await?;
    let reply = timeout(REQUEST_TIMEOUT, read_reply(&mut connection))
        .await
        .map_err(|_elapsed| unavailable(subject, "the monitor did not reply"))??
        .ok_or_else(|| unavailable(subject, "the monitor closed the connection"))?;
    if reply.instance != record.instance {
        return Err(unavailable(subject, "the reply came from another instance"));
    }
    if let ReplyResult::Error { kind, message } = reply.result {
        return Err(DaemonError::MonitorRejected {
            subject: subject.to_owned(),
            kind,
            message,
        });
    }

    Ok((record, reply.result))
}

fn peer_is_monitor(stream: &UnixStream, record: &MonitorRecord) -> Result<bool, DaemonError> {
    let credentials = stream.peer_cred()?;
    let peer_pid = credentials.pid().and_then(|pid| u32::try_from(pid).ok());

    Ok(credentials.uid() == rustix::process::getuid().as_raw()
        && peer_pid == Some(record.monitor.pid)
        && platform::inspect_process_start_identity(record.monitor.pid)?
            == Some(record.monitor.start_identity)
        && platform::current_boot_session_id()? == record.boot_session_id)
}

async fn read_reply(
    connection: &mut DaemonTransport<UnixStream>,
) -> Result<Option<Reply>, DaemonError> {
    let Some(line) = connection.next().await else {
        return Ok(None);
    };

    Ok(Some(serde_json::from_str(
        &line.map_err(ProtocolError::from)?,
    )?))
}

async fn wait_for_reservation(
    paths: &PvPaths,
    subject: &str,
) -> Result<MonitorReservation, DaemonError> {
    let deadline = Instant::now() + RELEASE_TIMEOUT;

    loop {
        match MonitorReservation::acquire(paths, subject) {
            Ok(reservation) => return Ok(reservation),
            Err(StateError::CoordinationLockHeld { .. }) if Instant::now() < deadline => {
                sleep(POLL_INTERVAL).await;
            }
            Err(error) => return Err(reservation_error(subject, error)),
        }
    }
}

/// Runs a monitor: `pv monitor:run`. Its stdin is the reservation and its stdout the startup
/// socket. Returns once a controller releases it, or the lifeline closes and the runtime is
/// cleaned up.
pub fn run_monitor_blocking(paths: PvPaths, hooks: MonitorHooks) -> Result<(), DaemonError> {
    platform::require_capability(platform::PlatformCapability::ProcessContainment)?;
    // A session of its own, so a signal for the controller's group or terminal never reaches it.
    rustix::process::setsid().map_err(io::Error::from)?;
    let runtime = build_runtime()?;
    let result = runtime.block_on(run_monitor(&paths, &hooks));
    // A held rotation or the lifeline reader must not keep the monitor alive.
    runtime.shutdown_background();

    result
}

async fn run_monitor(paths: &PvPaths, hooks: &MonitorHooks) -> Result<(), DaemonError> {
    let startup = StdUnixStream::from(io::stdout().as_fd().try_clone_to_owned()?);
    startup.set_nonblocking(true)?;
    let mut startup = transport(UnixStream::from_std(startup)?);
    let line = timeout(STARTUP_REQUEST_TIMEOUT, startup.next())
        .await
        .map_err(|_elapsed| DaemonError::ProtocolTimedOut {
            phase: "monitor startup request",
        })?
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the controller closed before sending a startup request",
            )
        })?;
    let request: StartRequest = serde_json::from_str(&line.map_err(ProtocolError::from)?)?;

    let monitor = match start_runtime(paths, hooks, &request).await {
        Ok(monitor) => monitor,
        Err(error) => {
            let message = match &error {
                DaemonError::MonitorStartupFailed { message, .. } => message.clone(),
                error => error.to_string(),
            };
            let reply = Reply::new(
                &request.instance,
                ReplyResult::Error {
                    kind: MonitorErrorKind::StartupFailed,
                    message,
                },
            );
            let _reply_result = write_line(&mut startup, &reply).await;
            return Err(error);
        }
    };
    let reply = Reply::new(&request.instance, ReplyResult::State(monitor.state()));
    // The runtime is running: keep supervising it even if the controller has gone.
    let _reply_result = write_line(&mut startup, &reply).await;
    drop(startup);

    monitor.run(hooks).await
}

async fn start_runtime(
    paths: &PvPaths,
    hooks: &MonitorHooks,
    request: &StartRequest,
) -> Result<Monitor, DaemonError> {
    let start = &request.start;
    let subject = &start.subject;
    if request.version != PROTOCOL_VERSION {
        return Err(startup_failed(
            subject,
            format!(
                "this monitor speaks protocol version {PROTOCOL_VERSION}, not {}",
                request.version
            ),
        ));
    }
    let reservation =
        MonitorReservation::adopt(paths, subject, io::stdin().as_fd().try_clone_to_owned()?)?;
    let socket = monitor_socket(paths, subject)?;
    let log = fs::open_private_log_file(&start.log_path)?;
    let lifeline = start.lifeline_fd.map(lifeline_path).transpose()?;
    fs::ensure_user_dir(&paths.monitor_dir(subject))?;
    let (gate, gate_end) = StdUnixStream::pair()?;
    let pid = spawn_gate(start, &log, gate_end)?;

    match supervise_gate(paths, hooks, request, pid, gate, &socket).await {
        Ok((watch, exit, record, listener)) => Ok(Monitor {
            paths: paths.clone(),
            record,
            watch,
            listener,
            log,
            lifeline,
            _reservation: reservation,
            stop: None,
            exit,
            cleanup: MonitorCleanup::Pending,
            stopped: false,
            executor: None,
            rotation: None,
            log_rotation_error: None,
        }),
        Err(error) => {
            abandon_gate(paths, subject, pid).await;
            Err(error)
        }
    }
}

type SupervisedGate = (
    Option<AsyncFd<ProcessExitWatch>>,
    Option<MonitorExit>,
    MonitorRecord,
    UnixListener,
);

/// Records the gate, lets it exec the runtime, and confirms the exec.
async fn supervise_gate(
    paths: &PvPaths,
    hooks: &MonitorHooks,
    request: &StartRequest,
    pid: u32,
    gate: StdUnixStream,
    socket: &Utf8Path,
) -> Result<SupervisedGate, DaemonError> {
    let start = &request.start;
    let subject = &start.subject;
    // A kqueue descriptor is only ever readable; registering it as writable fails.
    let mut watch = AsyncFd::with_interest(ProcessExitWatch::with_exec(pid)?, Interest::READABLE)?;
    let record = MonitorRecord {
        version: PROTOCOL_VERSION,
        subject: subject.clone(),
        instance: request.instance.clone(),
        boot_session_id: platform::current_boot_session_id()?,
        monitor: recorded_process(std::process::id())?,
        runtime: recorded_process(pid)?,
        command: start.command.clone(),
        arguments: start.arguments.clone(),
        log_path: start.log_path.clone(),
        fallback_stop: start.fallback_stop,
    };
    pause(hooks, MonitorPause::BeforePublication).await;
    fs::write_sensitive_file(
        &record_path(paths, subject),
        &serde_json::to_string_pretty(&record)?,
    )?;
    pause(hooks, MonitorPause::BeforePermission).await;
    gate.set_nonblocking(true)?;
    let mut gate = transport(UnixStream::from_std(gate)?);
    let permission = GatePermission {
        command: start.command.clone(),
        arguments: start.arguments.clone(),
    };
    write_line(&mut gate, &permission).await?;
    let events = timeout(EXEC_TIMEOUT, confirm_exec(&mut watch, &mut gate, subject))
        .await
        .map_err(|_elapsed| startup_failed(subject, "the runtime did not start in time"))??;
    let exit = events.exit.map(MonitorExit::from);
    // An exit event ends the kqueue watch.
    let watch = exit.is_none().then_some(watch);
    fs::remove_file_if_exists(socket)?;
    let listener = UnixListener::bind(socket)?;
    fs::secure_sensitive_file(socket)?;

    Ok((watch, exit, record, listener))
}

/// Waits for the gate's exec. The gate's socket closing proves nothing: only the kqueue exec
/// event does, or the gate's own report that the exec failed.
async fn confirm_exec(
    watch: &mut AsyncFd<ProcessExitWatch>,
    gate: &mut DaemonTransport<UnixStream>,
    subject: &str,
) -> Result<ProcessEvents, DaemonError> {
    let mut gate_open = true;

    loop {
        tokio::select! {
            events = next_events(watch) => {
                let events = events?;
                if events.exec {
                    return Ok(events);
                }
                if let Some(status) = events.exit {
                    let message = timeout(GATE_FAILURE_TIMEOUT, gate_failure(gate))
                        .await
                        .ok()
                        .flatten()
                        .unwrap_or_else(|| format!("the gate exited before the runtime started ({status})"));
                    return Err(startup_failed(subject, message));
                }
            }
            line = gate.next(), if gate_open => match line {
                Some(line) => {
                    let line = line.map_err(ProtocolError::from)?;
                    return Err(startup_failed(subject, failure_message(&line)));
                }
                None => gate_open = false,
            },
        }
    }
}

async fn gate_failure(gate: &mut DaemonTransport<UnixStream>) -> Option<String> {
    let line = gate.next().await?.ok()?;

    Some(failure_message(&line))
}

fn failure_message(line: &str) -> String {
    serde_json::from_str::<GateFailure>(line).map_or_else(
        |_error| "the gate reported an unreadable failure".to_owned(),
        |failure| failure.message,
    )
}

async fn next_events(watch: &mut AsyncFd<ProcessExitWatch>) -> Result<ProcessEvents, DaemonError> {
    loop {
        let mut ready = watch.readable_mut().await?;
        let events = ready.get_inner_mut().try_events()?;
        if events != ProcessEvents::default() {
            return Ok(events);
        }
        ready.clear_ready();
    }
}

/// Kills a gate whose startup failed, reaps it, and removes the records once its group is gone.
async fn abandon_gate(paths: &PvPaths, subject: &str, pid: u32) {
    let stopped = stop_process_group(pid, StopSignal::Terminate, Duration::ZERO, || Ok(true)).await;
    if stopped.is_ok() && reap_process_if_child(pid).is_ok() {
        let _remove_result = remove_monitor_dir(paths, subject);
    }
}

#[expect(
    clippy::disallowed_types,
    reason = "the monitor spawns its gate with std, so no Tokio waiter can reap the runtime"
)]
#[expect(
    clippy::disallowed_methods,
    reason = "the gate is this same monitor executable"
)]
fn spawn_gate(
    start: &MonitorStart,
    log: &std::fs::File,
    gate_end: StdUnixStream,
) -> Result<u32, DaemonError> {
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command
        .arg(MONITOR_GATE_COMMAND)
        .process_group(0)
        .stdin(OwnedFd::from(gate_end))
        .stdout(log.try_clone()?)
        .stderr(log.try_clone()?);
    for key in PHP_INI_ENVIRONMENT_KEYS {
        command.env_remove(key);
    }
    command.envs(&start.private_environment);

    // Dropping the std child neither waits for nor kills it.
    Ok(command.spawn()?.id())
}

/// Starts the runtime: `pv monitor:gate`. Its stdin is the monitor's gate socket. It waits for
/// the monitor's permission, then execs the runtime, so the runtime keeps this process's pid and
/// birth identity. Returns only if it cannot.
pub fn run_monitor_gate(hooks: &MonitorHooks) -> Result<(), DaemonError> {
    platform::require_capability(platform::PlatformCapability::ProcessContainment)?;
    let mut socket = StdUnixStream::from(io::stdin().as_fd().try_clone_to_owned()?);
    let mut line = String::new();
    BufReader::new(&socket).read_line(&mut line)?;
    if line.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the monitor closed before permitting the runtime to start",
        )
        .into());
    }
    let permission: GatePermission = serde_json::from_str(&line)?;
    pause_blocking(hooks, MonitorPause::GateAfterPermission);

    let error = runtime_command(&permission).exec();
    let failure = GateFailure {
        message: format!("cannot start {}: {error}", permission.command),
    };
    let _report_result = writeln!(socket, "{}", serde_json::to_string(&failure)?);

    Err(error.into())
}

#[expect(
    clippy::disallowed_types,
    reason = "the gate replaces itself with the runtime"
)]
fn runtime_command(permission: &GatePermission) -> std::process::Command {
    let mut command = std::process::Command::new(&permission.command);
    command.args(&permission.arguments).stdin(Stdio::null());

    command
}

/// A running monitor's view of its runtime.
#[expect(
    clippy::disallowed_types,
    reason = "the monitor retains the runtime log's append descriptor for rotation"
)]
struct Monitor {
    paths: PvPaths,
    record: MonitorRecord,
    /// The runtime's kqueue watch until its exit is known.
    watch: Option<AsyncFd<ProcessExitWatch>>,
    listener: UnixListener,
    log: std::fs::File,
    lifeline: Option<Utf8PathBuf>,
    _reservation: MonitorReservation,
    stop: Option<MonitorStop>,
    exit: Option<MonitorExit>,
    cleanup: MonitorCleanup,
    /// Whether the stop executor proved the group empty, before the exit event arrived.
    stopped: bool,
    executor: Option<JoinHandle<Result<(), DaemonError>>>,
    rotation: Option<JoinHandle<Result<(), DaemonError>>>,
    log_rotation_error: Option<String>,
}

enum Command {
    State(oneshot::Sender<MonitorState>),
    Stop(MonitorStop),
    Release(oneshot::Sender<Result<(), String>>),
    Exit,
}

impl Monitor {
    fn state(&self) -> MonitorState {
        MonitorState {
            subject: self.record.subject.clone(),
            monitor_pid: self.record.monitor.pid,
            runtime_pid: self.record.runtime.pid,
            stop: self.stop,
            exit: self.exit,
            cleanup: self.cleanup,
            log_rotation_error: self.log_rotation_error.clone(),
        }
    }

    async fn run(mut self, hooks: &MonitorHooks) -> Result<(), DaemonError> {
        let (commands, mut requests) = mpsc::channel(16);
        let mut lifeline = self.lifeline.take().map(watch_lifeline).transpose()?;
        let mut self_release = false;
        let rotation_period = hooks
            .rotation_interval_ms
            .map_or(ROTATION_INTERVAL, Duration::from_millis);
        let mut rotation_tick = interval_at(Instant::now() + rotation_period, rotation_period);
        rotation_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        if self.exit.is_some() {
            self.start_cleanup(self.record.fallback_stop);
        }

        loop {
            if self_release {
                match self.cleanup {
                    MonitorCleanup::Complete => {
                        return remove_monitor_dir(&self.paths, &self.record.subject);
                    }
                    // Keep the records as evidence for recovery.
                    MonitorCleanup::Unproven => return Ok(()),
                    MonitorCleanup::Pending => {}
                }
            }

            tokio::select! {
                events = next_optional_events(&mut self.watch) => self.observe(events?, hooks),
                accepted = self.listener.accept() => {
                    if let Ok((stream, _address)) = accepted {
                        let instance = self.record.instance.clone();
                        tokio::spawn(serve_connection(stream, instance, commands.clone()));
                    }
                }
                Some(command) = requests.recv() => match command {
                    Command::State(reply) => {
                        let _send_result = reply.send(self.state());
                    }
                    Command::Stop(stop) => {
                        self.stop.get_or_insert(stop);
                        self.start_cleanup(stop);
                    }
                    Command::Release(reply) => {
                        let _send_result = reply.send(self.releasable());
                    }
                    Command::Exit => return Ok(()),
                },
                stopped = join_optional(&mut self.executor) => {
                    self.executor = None;
                    self.finish_stop(stopped, hooks);
                }
                () = lifeline_closed(&mut lifeline) => {
                    lifeline = None;
                    self_release = true;
                    self.start_cleanup(self.record.fallback_stop);
                }
                _tick = rotation_tick.tick(), if self.rotation.is_none() => {
                    match self.start_rotation(hooks) {
                        Ok(rotation) => self.rotation = Some(rotation),
                        Err(error) => self.log_rotation_error = Some(error.to_string()),
                    }
                }
                rotated = join_optional(&mut self.rotation) => {
                    self.rotation = None;
                    self.log_rotation_error = rotated.err().map(|error| error.to_string());
                }
            }
        }
    }

    fn observe(&mut self, events: ProcessEvents, hooks: &MonitorHooks) {
        let Some(status) = events.exit else {
            return;
        };
        self.watch = None;
        self.exit = Some(MonitorExit::from(status));
        if self.stopped {
            self.complete_cleanup(hooks);
        } else {
            // The runtime exited on its own: stop whatever it left in its group.
            self.start_cleanup(self.record.fallback_stop);
        }
    }

    /// Starts the one stop executor of this instance, unless it already ran. A repeated stop
    /// never resets its deadline, and a client disconnecting never cancels it.
    fn start_cleanup(&mut self, stop: MonitorStop) {
        if self.executor.is_some() || self.stopped || self.cleanup != MonitorCleanup::Pending {
            return;
        }
        let pid = self.record.runtime.pid;
        // The unreaped leader pins the group's id, so the group is still the runtime's.
        self.executor = Some(tokio::spawn(async move {
            stop_process_group(pid, stop.signal, stop.grace(), || Ok(true)).await
        }));
    }

    fn finish_stop(&mut self, stopped: Result<(), DaemonError>, hooks: &MonitorHooks) {
        if stopped.is_err() {
            self.cleanup = MonitorCleanup::Unproven;
            return;
        }
        self.stopped = true;
        // The leader is a zombie now, so its exit event is already queued if not yet observed.
        if self.exit.is_some() {
            self.complete_cleanup(hooks);
        }
    }

    /// Proves the group empty one final time, then reaps the leader, exactly once.
    fn complete_cleanup(&mut self, hooks: &MonitorHooks) {
        let pid = self.record.runtime.pid;
        let group_empty = !hooks.unknown_group_check
            && matches!(platform::process_group_has_live_members(pid), Ok(false));
        self.cleanup = if group_empty && reap_process_if_child(pid).is_ok() {
            MonitorCleanup::Complete
        } else {
            MonitorCleanup::Unproven
        };
    }

    fn releasable(&self) -> Result<(), String> {
        if self.exit.is_some() && self.cleanup == MonitorCleanup::Complete {
            return Ok(());
        }

        Err("the runtime has not exited with its cleanup complete".to_owned())
    }

    fn start_rotation(
        &self,
        hooks: &MonitorHooks,
    ) -> Result<JoinHandle<Result<(), DaemonError>>, DaemonError> {
        let log = self.log.try_clone()?;
        let path = self.record.log_path.clone();
        let threshold = hooks.rotation_bytes.unwrap_or(ROTATION_BYTES);
        let hooks = hooks.clone();

        Ok(tokio::task::spawn_blocking(move || {
            rotation::rotate_if_needed(&log, &path, threshold, || {
                pause_blocking(&hooks, MonitorPause::RotationCopy);
            })
        }))
    }
}

async fn serve_connection(stream: UnixStream, instance: String, commands: mpsc::Sender<Command>) {
    let _answer_result = answer(stream, &instance, &commands).await;
}

async fn answer(
    stream: UnixStream,
    instance: &str,
    commands: &mpsc::Sender<Command>,
) -> Result<(), DaemonError> {
    if stream.peer_cred()?.uid() != rustix::process::getuid().as_raw() {
        return Ok(());
    }
    let mut connection = transport(stream);
    let Some(line) = timeout(REQUEST_TIMEOUT, connection.next())
        .await
        .map_err(|_elapsed| DaemonError::ProtocolTimedOut {
            phase: "monitor request",
        })?
    else {
        return Ok(());
    };
    let line = line.map_err(ProtocolError::from)?;
    let mut released = false;
    let result = match parse_request(&line, instance) {
        Err((kind, message)) => ReplyResult::Error { kind, message },
        Ok(Operation::State) => {
            let (reply, state) = oneshot::channel();
            send(commands, Command::State(reply)).await?;
            ReplyResult::State(state.await.map_err(|_closed| monitor_closing())?)
        }
        Ok(Operation::Stop(stop)) => {
            send(commands, Command::Stop(stop)).await?;
            ReplyResult::Accepted
        }
        Ok(Operation::Release) => {
            let (reply, releasable) = oneshot::channel();
            send(commands, Command::Release(reply)).await?;
            match releasable.await.map_err(|_closed| monitor_closing())? {
                Ok(()) => {
                    released = true;
                    ReplyResult::Accepted
                }
                Err(message) => ReplyResult::Error {
                    kind: MonitorErrorKind::NotReleasable,
                    message,
                },
            }
        }
    };
    let written = write_line(&mut connection, &Reply::new(instance, result)).await;
    if released {
        send(commands, Command::Exit).await?;
    }
    written?;

    Ok(())
}

fn parse_request(line: &str, instance: &str) -> Result<Operation, (MonitorErrorKind, String)> {
    let invalid = |error: serde_json::Error| (MonitorErrorKind::InvalidRequest, error.to_string());
    let header: RequestHeader = serde_json::from_str(line).map_err(invalid)?;
    if header.version != PROTOCOL_VERSION {
        return Err((
            MonitorErrorKind::UnsupportedVersion,
            format!(
                "this monitor speaks protocol version {PROTOCOL_VERSION}, not {}",
                header.version
            ),
        ));
    }
    if header.instance != instance {
        return Err((
            MonitorErrorKind::InstanceMismatch,
            "the request names another monitor instance".to_owned(),
        ));
    }
    let request: Request = serde_json::from_str(line).map_err(invalid)?;

    Ok(request.operation)
}

async fn send(commands: &mpsc::Sender<Command>, command: Command) -> Result<(), DaemonError> {
    commands
        .send(command)
        .await
        .map_err(|_closed| monitor_closing())
}

fn monitor_closing() -> DaemonError {
    io::Error::other("the monitor is shutting down").into()
}

async fn next_optional_events(
    watch: &mut Option<AsyncFd<ProcessExitWatch>>,
) -> Result<ProcessEvents, DaemonError> {
    match watch {
        Some(watch) => next_events(watch).await,
        None => std::future::pending().await,
    }
}

async fn join_optional(
    handle: &mut Option<JoinHandle<Result<(), DaemonError>>>,
) -> Result<(), DaemonError> {
    match handle {
        Some(handle) => handle.await?,
        None => std::future::pending().await,
    }
}

async fn lifeline_closed(lifeline: &mut Option<oneshot::Receiver<()>>) {
    match lifeline {
        Some(closed) => {
            let _closed_result = closed.await;
        }
        None => std::future::pending().await,
    }
}

fn lifeline_path(fd: i32) -> Result<Utf8PathBuf, DaemonError> {
    // Opening /dev/fd/N duplicates descriptor N, so the monitor never owns a raw descriptor.
    let path = Utf8PathBuf::from(format!("/dev/fd/{fd}"));
    let is_pipe = fd > 2
        && rustix::fs::stat(path.as_std_path())
            .is_ok_and(|stat| FileType::from_raw_mode(stat.st_mode) == FileType::Fifo);
    if !is_pipe {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("lifeline descriptor {fd} is not an inherited pipe"),
        )
        .into());
    }

    Ok(path)
}

fn watch_lifeline(path: Utf8PathBuf) -> Result<oneshot::Receiver<()>, DaemonError> {
    let (closed, receiver) = oneshot::channel();
    std::thread::Builder::new()
        .name("pv-monitor-lifeline".to_owned())
        .spawn(move || {
            // Returns once every write end is closed: the controller holding it is gone.
            let _read_result = fs::read_to_string(&path);
            let _send_result = closed.send(());
        })?;

    Ok(receiver)
}

async fn pause(hooks: &MonitorHooks, point: MonitorPause) {
    if let Some((reached, resume)) = pause_files(hooks, point) {
        announce(&reached);
        while !fs::path_exists(&resume) {
            sleep(POLL_INTERVAL).await;
        }
    }
}

fn pause_blocking(hooks: &MonitorHooks, point: MonitorPause) {
    if let Some((reached, resume)) = pause_files(hooks, point) {
        announce(&reached);
        while !fs::path_exists(&resume) {
            std::thread::sleep(POLL_INTERVAL);
        }
    }
}

fn pause_files(hooks: &MonitorHooks, point: MonitorPause) -> Option<(Utf8PathBuf, Utf8PathBuf)> {
    let dir = hooks.pause_dir.as_ref()?;
    if !hooks.pauses.contains(&point) {
        return None;
    }
    let name = point.name();

    Some((
        dir.join(format!("{name}.reached")),
        dir.join(format!("{name}.continue")),
    ))
}

fn announce(reached: &Utf8Path) {
    let _write_result = fs::write_sensitive_file(reached, &format!("{}\n", std::process::id()));
}

impl From<ExitStatus> for MonitorExit {
    fn from(status: ExitStatus) -> Self {
        match status.code() {
            Some(code) => Self::Code(code),
            None => Self::Signal(status.signal().unwrap_or_default()),
        }
    }
}

fn recorded_process(pid: u32) -> Result<RecordedProcess, DaemonError> {
    let start_identity = platform::inspect_process_start_identity(pid)?.ok_or_else(|| {
        DaemonError::MissingProcessIdentity {
            name: "monitor".to_owned(),
            pid,
        }
    })?;

    Ok(RecordedProcess {
        pid,
        start_identity,
    })
}

fn monitor_socket(paths: &PvPaths, subject: &str) -> Result<Utf8PathBuf, DaemonError> {
    let path = paths.monitor_dir(subject).join(SOCKET_FILE);
    if path.as_str().len() > SOCKET_PATH_LIMIT {
        return Err(DaemonError::MonitorSocketPathTooLong {
            path,
            limit: SOCKET_PATH_LIMIT,
        });
    }

    Ok(path)
}

fn record_path(paths: &PvPaths, subject: &str) -> Utf8PathBuf {
    paths.monitor_dir(subject).join(RECORD_FILE)
}

fn read_record(paths: &PvPaths, subject: &str) -> Result<Option<MonitorRecord>, DaemonError> {
    match fs::read_to_string(&record_path(paths, subject)) {
        Ok(content) => Ok(Some(serde_json::from_str(&content)?)),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

fn remove_monitor_dir(paths: &PvPaths, subject: &str) -> Result<(), DaemonError> {
    match fs::delete_dir_all(&paths.monitor_dir(subject)) {
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(())
        }
        result => Ok(result?),
    }
}

fn new_instance() -> Result<String, DaemonError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    let mut instance = String::with_capacity(32);
    for byte in bytes {
        let _write_result = write!(instance, "{byte:02x}");
    }

    Ok(instance)
}

fn reservation_error(subject: &str, error: StateError) -> DaemonError {
    match error {
        StateError::CoordinationLockHeld { .. } => {
            unavailable(subject, "a monitor or controller holds its reservation")
        }
        error => error.into(),
    }
}

fn startup_failed(subject: &str, message: impl Into<String>) -> DaemonError {
    DaemonError::MonitorStartupFailed {
        subject: subject.to_owned(),
        message: message.into(),
    }
}

fn unavailable(subject: &str, reason: &str) -> DaemonError {
    DaemonError::MonitorUnavailable {
        subject: subject.to_owned(),
        reason: reason.to_owned(),
    }
}
