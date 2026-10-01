//! Contract tests that run the `pv-fake` binary directly, without the daemon supervisor.
#![cfg(unix)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, ExitStatus, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use pv_fake::{EventKind, FakeSettings, GatewayListeners, InstalledFake, Pause, Persona, Scenario};
use rustix::io::{FdFlags, fcntl_setfd};
use rustix::process::{Pid, Signal, kill_process};
use serde_json::json;

#[expect(
    clippy::disallowed_types,
    reason = "contract tests run the pv-fake binary directly, without the daemon supervisor"
)]
type FakeCommand = std::process::Command;

/// Kills and reaps the fake when a test returns before it exits.
struct FakeProcess(Child);

impl Drop for FakeProcess {
    fn drop(&mut self) {
        let _kill_result = self.0.kill();
        let _wait_result = self.0.wait();
    }
}

const WAIT_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

#[test]
fn long_running_fake_records_its_lifecycle_and_exits_cleanly_on_sigterm() -> Result<()> {
    let tempdir = tempdir()?;
    let fake = install_long_running(tempdir.path(), "mysqld", None)?;
    let mut child = spawn(&fake, true)?;
    let lifeline_armed = wait_for_start(&fake, &mut child)?;
    signal(&child, Signal::TERM)?;
    let status = wait_for_exit(&mut child)?;

    assert!(!lifeline_armed);
    assert_eq!(status.code(), Some(0));
    assert_eq!(
        fake.events()?.first().map(|event| &event.kind),
        Some(&EventKind::Started {
            persona: Persona::LongRunning,
            argv: vec![fake.executable().to_string()],
            lifeline_armed: false,
        })
    );
    assert_eq!(event_names(&fake)?, ["started", "signal SIGTERM", "exit 0"]);

    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn installed_fake_reports_its_install_path_as_its_process_identity() -> Result<()> {
    let tempdir = tempdir()?;
    let fake = install_long_running(tempdir.path(), "caddy", None)?;
    let mut child = spawn(&fake, true)?;
    wait_for_start(&fake, &mut child)?;
    let identity = platform::inspect_process_identity(child.0.id())?;
    signal(&child, Signal::TERM)?;
    wait_for_exit(&mut child)?;

    let identity = identity.ok_or_else(|| anyhow!("fake process identity was unavailable"))?;
    assert_eq!(identity.executable.as_path(), fake.executable());
    assert_eq!(identity.argument_zero, fake.executable().as_str());

    Ok(())
}

#[test]
fn closing_the_lifeline_kills_a_fake_that_leads_its_process_group() -> Result<()> {
    let tempdir = tempdir()?;
    let (read, write) = lifeline_pipe()?;
    let fake = install_long_running(tempdir.path(), "frankenphp", Some(read.as_raw_fd()))?;
    let mut child = spawn(&fake, true)?;
    let lifeline_armed = wait_for_start(&fake, &mut child)?;
    drop(write);
    let status = wait_for_exit(&mut child)?;

    assert!(lifeline_armed);
    assert_eq!(status.signal(), Some(Signal::KILL.as_raw()));
    assert_eq!(event_names(&fake)?, ["started", "lifeline_fired"]);

    Ok(())
}

#[test]
fn closing_the_lifeline_kills_only_a_fake_that_shares_the_test_process_group() -> Result<()> {
    let tempdir = tempdir()?;
    let (read, write) = lifeline_pipe()?;
    let fake = install_long_running(tempdir.path(), "mailpit", Some(read.as_raw_fd()))?;
    // Without its own group, the fake shares this test's group. If it signaled that group, the
    // test process would die with it.
    let mut child = spawn(&fake, false)?;
    let lifeline_armed = wait_for_start(&fake, &mut child)?;
    drop(write);
    let status = wait_for_exit(&mut child)?;

    assert!(lifeline_armed);
    assert_eq!(status.signal(), Some(Signal::KILL.as_raw()));
    assert_eq!(event_names(&fake)?, ["started", "lifeline_fired"]);

    Ok(())
}

#[test]
fn descriptor_that_is_not_a_pipe_leaves_the_lifeline_unarmed() -> Result<()> {
    let tempdir = tempdir()?;
    // The fake's stdin is /dev/null, not a pipe.
    let fake = install_long_running(tempdir.path(), "redis-server", Some(0))?;
    let mut child = spawn(&fake, true)?;
    let lifeline_armed = wait_for_start(&fake, &mut child)?;
    signal(&child, Signal::TERM)?;
    let status = wait_for_exit(&mut child)?;

    assert!(!lifeline_armed);
    assert_eq!(status.code(), Some(0));

    Ok(())
}

#[test]
fn fake_without_a_scenario_fails_with_an_actionable_error() -> Result<()> {
    let tempdir = tempdir()?;
    let fake = install_long_running(tempdir.path(), "postgres", None)?;
    let scenario_path = format!("{}.pv-fake.json", fake.executable());
    state::fs::remove_file(Utf8Path::new(&scenario_path))?;
    let output = FakeCommand::new(fake.executable())
        .stdin(Stdio::null())
        .output()?;

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr)?,
        format!(
            "pv-fake: no scenario file at {scenario_path}; install fakes with pv_fake::install\n"
        )
    );

    Ok(())
}

#[test]
fn fake_from_another_build_refuses_to_start() -> Result<()> {
    let tempdir = tempdir()?;
    let fake = install_long_running(tempdir.path(), "caddy", None)?;
    let scenario_path = Utf8PathBuf::from(format!("{}.pv-fake.json", fake.executable()));
    // What a newer pv-fake library leaves for a stale binary, e.g. after `--test` skipped examples.
    state::fs::write_sensitive_file(
        &scenario_path,
        r#"{"build_id":"newer-build","persona":"long_running","lifeline_fd":null}"#,
    )?;
    let output = FakeCommand::new(fake.executable())
        .stdin(Stdio::null())
        .output()?;

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr)?,
        format!(
            "pv-fake: {} is an older pv-fake build than the one that installed it; rebuild the \
             daemon's examples (`cargo nextest run -p daemon` without `--test`, or `cargo build -p \
             daemon --examples`)\n",
            fake.executable()
        )
    );

    Ok(())
}

#[test]
fn gateway_controls_are_used_up_per_request_and_single_values_persist() -> Result<()> {
    let tempdir = tempdir()?;
    let [port] = available_ports()?;
    let gateway = GatewayFake::start(tempdir.path(), port)?;
    gateway.control(json!({
        "admin_statuses": [503],
        "load_statuses": [422, 200, 200],
        "apply_load": [false],
    }))?;
    let loads = ["rejected", "accepted", "applied"].map(|health| gateway.config(port, health));

    let admin = [
        gateway.admin("GET", "/config/", "")?,
        gateway.admin("GET", "/config/", "")?,
    ];
    let responses = loads
        .iter()
        .map(|load| gateway.admin("POST", "/load", load))
        .collect::<Result<Vec<_>>>()?;
    let health = http_get(port, "/__pv/health")?;
    let used_up = gateway.record("fake-admin-control.json")?;
    gateway.control(json!({"admin_statuses": 503}))?;
    let persisting = [
        gateway.admin("GET", "/config/", "")?,
        gateway.admin("GET", "/config/", "")?,
    ];

    assert_eq!(admin, [(503, "{}\n".to_owned()), (200, "{}\n".to_owned())]);
    assert_eq!(
        responses,
        [
            (
                422,
                "{\"error\":\"pv-fake: fake-admin-control.json rejected this load with status \
                 422\"}\n"
                    .to_owned()
            ),
            // Accepted without applying: `apply_load` is used up by accepted loads only.
            (200, UNFORMATTED_WARNING.to_owned()),
            (200, UNFORMATTED_WARNING.to_owned()),
        ]
    );
    assert_eq!(health, (200, "applied".to_owned()));
    assert_eq!(
        used_up,
        r#"{"admin_statuses":[],"apply_load":[],"load_statuses":[]}"#
    );
    assert_eq!(
        persisting,
        [(503, "{}\n".to_owned()), (503, "{}\n".to_owned())]
    );
    assert_eq!(
        gateway.requests()?,
        [
            ("GET", "/config/", 503),
            ("GET", "/config/", 200),
            ("POST", "/load", 422),
            ("POST", "/load", 200),
            ("POST", "/load", 200),
            ("GET", "/__pv/health", 200),
            ("GET", "/config/", 503),
            ("GET", "/config/", 503),
        ]
        .map(|(method, path, status)| (method.to_owned(), path.to_owned(), status))
    );
    for (number, load) in loads.iter().enumerate() {
        assert_eq!(
            &gateway.record(&format!("fake-admin-load-{number:03}.bin"))?,
            load
        );
    }
    assert_eq!(gateway.record("fake-admin-current.bin")?, loads[2]);

    Ok(())
}

#[test]
fn rejected_loads_leave_accepted_load_controls_queued() -> Result<()> {
    let tempdir = tempdir()?;
    let [port] = available_ports()?;
    let gateway = GatewayFake::start(tempdir.path(), port)?;
    let marker = tempdir.path().join("load-accepted");
    gateway.control(json!({"load_statuses": [422], "load_accepted_marker": [marker]}))?;
    let unreadable = gateway
        .config(port, "unreadable")
        .replace(&format!("http_port {port}"), "http_port nope");

    let rejected = gateway.admin("POST", "/load", &gateway.config(port, "rejected"))?;
    let unadapted = gateway.admin("POST", "/load", &unreadable)?;
    let marked_early = state::fs::path_entry_exists(&marker)?;
    let accepted = gateway.admin("POST", "/load", &gateway.config(port, "accepted"))?;

    assert_eq!(rejected.0, 422);
    assert_eq!(
        unadapted,
        (
            400,
            "{\"error\":\"adapting config using caddyfile adapter: http_port nope: invalid digit \
             found in string\"}\n"
                .to_owned()
        )
    );
    assert!(!marked_early);
    assert_eq!(accepted, (200, UNFORMATTED_WARNING.to_owned()));
    assert_eq!(state::fs::read_to_string(&marker)?, "accepted\n");

    Ok(())
}

#[test]
fn gateway_controls_reject_unknown_keys_and_malformed_files() -> Result<()> {
    let tempdir = tempdir()?;
    let [port] = available_ports()?;
    let gateway = GatewayFake::start(tempdir.path(), port)?;

    let unknown =
        pv_fake::write_gateway_control(&gateway.config_path, json!({"load_status": [422]}));
    state::fs::write_sensitive_file(
        &gateway
            .config_path
            .with_file_name("fake-admin-control.json"),
        "{\"load_statuses\": [422",
    )?;
    let (status, body) = gateway.admin("GET", "/config/", "")?;

    let unknown = unknown.err().map(|error| format!("{error:#}"));
    assert!(
        unknown
            .as_deref()
            .is_some_and(|error| error.contains("unknown field `load_status`")),
        "{unknown:?}"
    );
    assert_eq!(status, 500);
    assert!(
        body.starts_with("pv-fake: GET /config/: parsing ")
            && body.contains("fake-admin-control.json: EOF while parsing"),
        "{body}"
    );

    Ok(())
}

#[test]
fn late_accepted_load_applies_after_its_client_disconnects() -> Result<()> {
    let tempdir = tempdir()?;
    let [port] = available_ports()?;
    let gateway = GatewayFake::start(tempdir.path(), port)?;
    gateway.control(json!({
        "late_accept": [true],
        "late_apply_delay_ms": [100],
        "load_delay_ms": [30_000],
    }))?;
    let load = gateway.config(port, "applied late");

    // Hang up once the fake has the load, as PV does when its load times out.
    let mut stream = UnixStream::connect(&gateway.admin_socket)?;
    write_request(&mut stream, "POST", "/load", &load)?;
    let received = wait_until(|| Ok(gateway.requests()?.len() == 1));
    drop(stream);
    let applied = wait_until(|| Ok(http_get(port, "/__pv/health")?.1 == "applied late"));

    received?;
    applied?;
    assert_eq!(gateway.record("fake-admin-current.bin")?, load);

    Ok(())
}

#[test]
fn held_load_is_recorded_and_leaves_the_admin_api_responsive() -> Result<()> {
    let tempdir = tempdir()?;
    let [port] = available_ports()?;
    let gateway = GatewayFake::start(tempdir.path(), port)?;
    let release = tempdir.path().join("release-load");
    gateway.control(json!({"load_response_gate": [release]}))?;
    let load = gateway.config(port, "held");

    let mut held = UnixStream::connect(&gateway.admin_socket)?;
    write_request(&mut held, "POST", "/load", &load)?;
    let recorded = wait_until(|| Ok(gateway.requests()?.len() == 1));
    let admin = gateway.admin("GET", "/config/", "");
    state::fs::write_sensitive_file(&release, "released\n")?;
    let released = read_response(held);

    recorded?;
    assert_eq!(admin?, (200, "{}\n".to_owned()));
    assert_eq!(released?, (200, UNFORMATTED_WARNING.to_owned()));
    assert_eq!(gateway.record("fake-admin-current.bin")?, load);

    Ok(())
}

#[test]
fn retained_listeners_serve_an_applied_load_on_the_previous_port() -> Result<()> {
    let tempdir = tempdir()?;
    let [port, moved_port] = available_ports()?;
    let gateway = GatewayFake::start(tempdir.path(), port)?;
    gateway.control(json!({"retain_previous_listeners": [true]}))?;
    let load = gateway.config(moved_port, "moved");

    let response = gateway.admin("POST", "/load", &load)?;

    assert_eq!(response, (200, UNFORMATTED_WARNING.to_owned()));
    assert_eq!(http_get(port, "/__pv/health")?, (200, "moved".to_owned()));
    assert!(TcpStream::connect(("127.0.0.1", moved_port)).is_err());
    assert_eq!(gateway.record("fake-admin-current.bin")?, load);

    Ok(())
}

#[test]
fn stopped_service_closes_the_http_port_and_keeps_the_admin_api() -> Result<()> {
    let tempdir = tempdir()?;
    let [port] = available_ports()?;
    let gateway = GatewayFake::start(tempdir.path(), port)?;
    gateway.control(json!({"stop_service": true}))?;

    let closed = wait_until(|| Ok(TcpStream::connect(("127.0.0.1", port)).is_err()));

    closed?;
    assert_eq!(
        gateway.admin("GET", "/config/", "")?,
        (200, "{}\n".to_owned())
    );

    Ok(())
}

#[test]
fn admin_off_serves_http_without_an_admin_socket() -> Result<()> {
    let tempdir = tempdir()?;
    let [port] = available_ports()?;
    let admin_socket = tempdir.path().join("admin.sock");
    let config = gateway_config(&admin_socket, port, "legacy")
        .replace(&format!("admin \"unix/{admin_socket}|0600\""), "admin off");
    let (_fake, _process) = spawn_gateway(tempdir.path(), &config, FakeSettings::default())?;

    // The admin socket would be bound before the first HTTP connection is served.
    wait_until(|| Ok(http_get(port, "/__pv/health")?.1 == "legacy"))?;
    let admin_over_http = [
        http_get(port, "/config/")?,
        exchange(
            TcpStream::connect(("127.0.0.1", port))?,
            "POST",
            "/load",
            &config,
        )?,
    ];

    assert!(!state::fs::path_entry_exists(&admin_socket)?);
    assert_eq!(admin_over_http.map(|(status, _body)| status), [404, 404]);

    Ok(())
}

#[test]
fn admin_only_listeners_accept_loads_without_opening_ports() -> Result<()> {
    let tempdir = tempdir()?;
    let [port, moved_port] = available_ports()?;
    let gateway = GatewayFake::start_with(
        tempdir.path(),
        port,
        FakeSettings {
            gateway_listeners: GatewayListeners::AdminOnly,
            ..FakeSettings::default()
        },
    )?;
    let load = gateway.config(moved_port, "moved");

    let response = gateway.admin("POST", "/load", &load)?;

    assert_eq!(response, (200, UNFORMATTED_WARNING.to_owned()));
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    assert!(TcpStream::connect(("127.0.0.1", moved_port)).is_err());
    assert_eq!(gateway.record("fake-admin-current.bin")?, load);

    Ok(())
}

#[test]
fn listeners_set_to_nothing_stay_alive_without_serving() -> Result<()> {
    let tempdir = tempdir()?;
    let [port] = available_ports()?;
    let admin_socket = tempdir.path().join("admin.sock");
    let (fake, mut process) = spawn_gateway(
        tempdir.path(),
        &gateway_config(&admin_socket, port, "unused"),
        FakeSettings {
            gateway_listeners: GatewayListeners::Nothing,
            ..FakeSettings::default()
        },
    )?;

    wait_for_start(&fake, &mut process)?;
    signal(&process, Signal::TERM)?;
    let status = wait_for_exit(&mut process)?;

    // The socket file outlives the fake, so its absence after exit shows it was never bound.
    assert_eq!(status.code(), Some(0));
    assert!(!state::fs::path_entry_exists(&admin_socket)?);
    assert_eq!(event_names(&fake)?, ["started", "signal SIGTERM", "exit 0"]);

    Ok(())
}

#[test]
fn validate_pause_holds_then_exits_with_the_chosen_code() -> Result<()> {
    let tempdir = tempdir()?;
    let release = tempdir.path().join("release-validate");
    let (fake, mut process) = spawn_gateway_command(
        tempdir.path(),
        "validate",
        "{\n}\n",
        FakeSettings {
            validate_pause: Some(Pause {
                until: release.clone(),
            }),
            validate_exit_code: Some(7),
            ..FakeSettings::default()
        },
    )?;

    wait_for_event(&fake, is_held)?;
    let held_running = process.0.try_wait()?.is_none();
    state::fs::write_sensitive_file(&release, "release\n")?;
    let status = wait_for_exit(&mut process)?;

    assert!(held_running);
    assert_eq!(status.code(), Some(7));
    assert_eq!(event_names(&fake)?, ["started", "held", "exit 7"]);

    Ok(())
}

#[test]
fn run_pause_holds_with_http_open_and_no_admin_socket() -> Result<()> {
    let tempdir = tempdir()?;
    let [port] = available_ports()?;
    let admin_socket = tempdir.path().join("admin.sock");
    let release = tempdir.path().join("release-run");
    let (fake, _process) = spawn_gateway(
        tempdir.path(),
        &gateway_config(&admin_socket, port, "paused"),
        FakeSettings {
            run_pause: Some(Pause {
                until: release.clone(),
            }),
            ..FakeSettings::default()
        },
    )?;

    wait_for_event(&fake, is_held)?;
    let health = http_get(port, "/__pv/health")?;
    let admin_socket_while_held = state::fs::path_entry_exists(&admin_socket)?;
    state::fs::write_sensitive_file(&release, "release\n")?;
    let admin_opened = wait_until(|| Ok(UnixStream::connect(&admin_socket).is_ok()));

    assert_eq!(health, (200, "paused".to_owned()));
    assert!(!admin_socket_while_held);
    admin_opened?;

    Ok(())
}

#[test]
fn paused_fake_exits_cleanly_on_sigterm() -> Result<()> {
    let tempdir = tempdir()?;
    let (fake, mut process) = spawn_gateway_command(
        tempdir.path(),
        "validate",
        "{\n}\n",
        FakeSettings {
            validate_pause: Some(Pause {
                until: tempdir.path().join("never-released"),
            }),
            ..FakeSettings::default()
        },
    )?;

    wait_for_event(&fake, is_held)?;
    signal(&process, Signal::TERM)?;
    let status = wait_for_exit(&mut process)?;

    assert_eq!(status.code(), Some(0));
    assert_eq!(
        event_names(&fake)?,
        ["started", "held", "signal SIGTERM", "exit 0"]
    );

    Ok(())
}

#[test]
fn descendant_joins_the_process_group_and_exits_with_its_parent() -> Result<()> {
    let tempdir = tempdir()?;
    let fake = pv_fake::install_with(
        Utf8Path::new(env!("CARGO_BIN_EXE_pv-fake")),
        &tempdir.path().join("bin/frankenphp"),
        &Scenario {
            persona: Persona::LongRunning,
            lifeline_fd: None,
            settings: FakeSettings {
                descendant: true,
                ..FakeSettings::default()
            },
        },
    )?;
    let mut parent = spawn(&fake, true)?;

    let descendant = wait_for_event(&fake, |kind| match kind {
        EventKind::DescendantSpawned { descendant_pid } => Some(*descendant_pid),
        _ => None,
    })?;
    let descendant_pid =
        Pid::from_raw(descendant).ok_or_else(|| anyhow!("invalid descendant pid {descendant}"))?;
    let descendant_group = rustix::process::getpgid(Some(descendant_pid))?;
    signal(&parent, Signal::TERM)?;
    wait_for_exit(&mut parent)?;
    let descendant_exited =
        wait_until(|| Ok(rustix::process::test_kill_process(descendant_pid).is_err()));

    assert_eq!(
        u32::try_from(descendant_group.as_raw_nonzero().get())?,
        parent.0.id()
    );
    descendant_exited?;
    assert!(
        fake.events()?
            .iter()
            .any(|event| { event.pid == descendant && event.kind == EventKind::ParentExited })
    );

    Ok(())
}

#[test]
fn fake_whose_lifeline_closed_before_it_started_exits_at_once() -> Result<()> {
    let tempdir = tempdir()?;
    let (read, write) = lifeline_pipe()?;
    let fake = install_long_running(tempdir.path(), "redis-server", Some(read.as_raw_fd()))?;
    // The installing test is already gone by the time the fake starts.
    drop(write);
    let mut child = spawn(&fake, true)?;
    let status = wait_for_exit(&mut child)?;

    // The watcher can fire before or after `started` is recorded.
    assert_eq!(status.signal(), Some(Signal::KILL.as_raw()));
    assert!(
        event_names(&fake)?
            .iter()
            .any(|name| name == "lifeline_fired")
    );

    Ok(())
}

#[test]
fn gateway_records_restart_with_each_process() -> Result<()> {
    let tempdir = tempdir()?;
    let [port] = available_ports()?;
    let first = GatewayFake::start(tempdir.path(), port)?;
    first.admin("POST", "/load", &first.config(port, "first process"))?;
    drop(first);
    let second = GatewayFake::start(tempdir.path(), port)?;
    let load = second.config(port, "second process");

    let initial = second.record("fake-admin-current.bin")?;
    second.admin("POST", "/load", &load)?;

    assert_eq!(initial, second.config(port, "started"));
    assert_eq!(second.record("fake-admin-load-000.bin")?, load);
    assert!(!state::fs::path_entry_exists(
        &second.config_path.with_file_name("fake-admin-load-001.bin")
    )?);

    Ok(())
}

#[test]
fn validate_records_each_validated_config() -> Result<()> {
    let tempdir = tempdir()?;
    let fake = install_gateway(tempdir.path(), FakeSettings::default())?;
    let config_path = tempdir.path().join("config/Caddyfile");
    state::fs::write_sensitive_file(&config_path, "{\n}\n")?;
    let validate = |config_path: &Utf8Path| {
        FakeCommand::new(fake.executable())
            .args([
                "validate",
                "--config",
                config_path.as_str(),
                "--adapter",
                "caddyfile",
            ])
            .stdin(Stdio::null())
            .output()
    };

    let valid = [validate(&config_path)?, validate(&config_path)?];
    let missing = validate(&tempdir.path().join("config/missing.Caddyfile"))?;

    assert_eq!(valid.map(|output| output.status.code()), [Some(0), Some(0)]);
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(
        state::fs::read_to_string(&tempdir.path().join("config/fake-validator-spawns.log"))?,
        format!("{config_path}\n{config_path}\n")
    );

    Ok(())
}

/// Caddy's adapter warning for PV's space-indented configs, which accepted loads return.
const UNFORMATTED_WARNING: &str = r#"[{"file":"Caddyfile","line":2,"message":"Caddyfile input is not formatted; run 'caddy fmt --overwrite' to fix inconsistencies"}]"#;

/// A `caddy run` fake serving a minimal Gateway config, killed when dropped.
struct GatewayFake {
    config_path: Utf8PathBuf,
    admin_socket: Utf8PathBuf,
    _process: FakeProcess,
}

impl GatewayFake {
    fn start(root: &Utf8Path, http_port: u16) -> Result<Self> {
        Self::start_with(root, http_port, FakeSettings::default())
    }

    fn start_with(root: &Utf8Path, http_port: u16, settings: FakeSettings) -> Result<Self> {
        let admin_socket = root.join("admin.sock");
        state::fs::remove_file_if_exists(&admin_socket)?;
        let config_path = root.join("config/Caddyfile");
        let (fake, mut process) = spawn_gateway(
            root,
            &gateway_config(&admin_socket, http_port, "started"),
            settings,
        )?;
        // Connecting without a request leaves no record.
        if let Err(error) = wait_until(|| Ok(UnixStream::connect(&admin_socket).is_ok())) {
            let stderr = state::fs::read_to_string(&root.join("caddy.stderr")).unwrap_or_default();
            let events = fake.events()?;
            bail!(
                "the Gateway fake didn't start ({error}); exit status {:?}; events {events:?}; stderr:\n{stderr}",
                process.0.try_wait()?
            );
        }

        Ok(Self {
            config_path,
            admin_socket,
            _process: process,
        })
    }

    fn config(&self, http_port: u16, health: &str) -> String {
        gateway_config(&self.admin_socket, http_port, health)
    }

    fn control(&self, control: serde_json::Value) -> Result<()> {
        pv_fake::write_gateway_control(&self.config_path, control)
    }

    fn admin(&self, method: &str, path: &str, body: &str) -> Result<(u16, String)> {
        exchange(UnixStream::connect(&self.admin_socket)?, method, path, body)
    }

    fn record(&self, name: &str) -> Result<String> {
        Ok(state::fs::read_to_string(
            &self.config_path.with_file_name(name),
        )?)
    }

    fn requests(&self) -> Result<Vec<(String, String, u16)>> {
        if !state::fs::path_entry_exists(
            &self.config_path.with_file_name("fake-admin-requests.jsonl"),
        )? {
            return Ok(Vec::new());
        }
        self.record("fake-admin-requests.jsonl")?
            .lines()
            .map(|line| {
                let request: serde_json::Value = serde_json::from_str(line)?;
                Ok((
                    request["method"].as_str().unwrap_or_default().to_owned(),
                    request["path"].as_str().unwrap_or_default().to_owned(),
                    u16::try_from(request["status"].as_u64().unwrap_or_default())?,
                ))
            })
            .collect()
    }
}

fn gateway_config(admin_socket: &Utf8Path, http_port: u16, health: &str) -> String {
    format!(
        "{{\n    admin \"unix/{admin_socket}|0600\"\n    http_port {http_port}\n}}\n\n\
         http://pv-gateway.localhost {{\n    respond /__pv/health \"{health}\" 200\n}}\n"
    )
}

/// Starts `caddy run` with `config` at `<root>/config/Caddyfile`.
fn spawn_gateway(
    root: &Utf8Path,
    config: &str,
    settings: FakeSettings,
) -> Result<(InstalledFake, FakeProcess)> {
    spawn_gateway_command(root, "run", config, settings)
}

/// Starts `caddy <subcommand>` with `config` at `<root>/config/Caddyfile`.
fn spawn_gateway_command(
    root: &Utf8Path,
    subcommand: &str,
    config: &str,
    settings: FakeSettings,
) -> Result<(InstalledFake, FakeProcess)> {
    let fake = install_gateway(root, settings)?;
    let config_path = root.join("config/Caddyfile");
    state::fs::write_sensitive_file(&config_path, config)?;
    let process = FakeProcess(
        FakeCommand::new(fake.executable())
            .args([
                subcommand,
                "--config",
                config_path.as_str(),
                "--adapter",
                "caddyfile",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(state::fs::open_append_file(&root.join("caddy.stderr"))?)
            .process_group(0)
            .spawn()?,
    );

    Ok((fake, process))
}

fn install_gateway(root: &Utf8Path, settings: FakeSettings) -> Result<InstalledFake> {
    pv_fake::install_with(
        Utf8Path::new(env!("CARGO_BIN_EXE_pv-fake")),
        &root.join("bin/caddy"),
        &Scenario {
            persona: Persona::Caddy,
            lifeline_fd: None,
            settings,
        },
    )
}

fn available_ports<const COUNT: usize>() -> Result<[u16; COUNT]> {
    let listeners = (0..COUNT)
        .map(|_index| TcpListener::bind(("127.0.0.1", 0)))
        .collect::<Result<Vec<_>, _>>()?;
    let ports = listeners
        .iter()
        .map(|listener| Ok(listener.local_addr()?.port()))
        .collect::<Result<Vec<_>>>()?;

    ports
        .try_into()
        .map_err(|_ports| anyhow!("expected {COUNT} ports"))
}

fn http_get(port: u16, path: &str) -> Result<(u16, String)> {
    exchange(TcpStream::connect(("127.0.0.1", port))?, "GET", path, "")
}

/// One HTTP/1.1 exchange that closes the connection afterwards.
fn exchange<S: Read + Write>(
    mut stream: S,
    method: &str,
    path: &str,
    body: &str,
) -> Result<(u16, String)> {
    write_request(&mut stream, method, path, body)?;

    read_response(stream)
}

fn read_response(mut stream: impl Read) -> Result<(u16, String)> {
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    let Some((head, body)) = response.split_once("\r\n\r\n") else {
        bail!("malformed response {response:?}");
    };
    let status = head
        .split(' ')
        .nth(1)
        .ok_or_else(|| anyhow!("malformed status line in {head:?}"))?
        .parse()?;

    Ok((status, body.to_owned()))
}

fn write_request(stream: &mut impl Write, method: &str, path: &str, body: &str) -> Result<()> {
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;

    Ok(())
}

fn wait_until(mut condition: impl FnMut() -> Result<bool>) -> Result<()> {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        if condition().unwrap_or(false) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("condition not met within {WAIT_TIMEOUT:?}");
        }
        sleep(POLL_INTERVAL);
    }
}

fn install_long_running(
    root: &Utf8Path,
    name: &str,
    lifeline_fd: Option<i32>,
) -> Result<InstalledFake> {
    pv_fake::install_with(
        Utf8Path::new(env!("CARGO_BIN_EXE_pv-fake")),
        &root.join("bin").join(name),
        &Scenario {
            persona: Persona::LongRunning,
            lifeline_fd,
            settings: FakeSettings::default(),
        },
    )
}

fn spawn(fake: &InstalledFake, own_process_group: bool) -> Result<FakeProcess> {
    let mut command = FakeCommand::new(fake.executable());
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(state::fs::open_append_file(&stderr_path(fake))?);
    if own_process_group {
        command.process_group(0);
    }

    Ok(FakeProcess(command.spawn()?))
}

fn stderr_path(fake: &InstalledFake) -> Utf8PathBuf {
    Utf8PathBuf::from(format!("{}.stderr", fake.executable()))
}

/// A pipe whose read end children inherit and whose write end they never do.
fn lifeline_pipe() -> Result<(OwnedFd, OwnedFd)> {
    let (read, write) = rustix::pipe::pipe()?;
    fcntl_setfd(&write, FdFlags::CLOEXEC)?;

    Ok((read, write))
}

/// Waits for the fake's `started` event and returns whether its lifeline is armed.
fn wait_for_start(fake: &InstalledFake, child: &mut FakeProcess) -> Result<bool> {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        let started = fake
            .events()?
            .into_iter()
            .find_map(|event| match event.kind {
                EventKind::Started { lifeline_armed, .. } => Some(lifeline_armed),
                _ => None,
            });
        if let Some(lifeline_armed) = started {
            return Ok(lifeline_armed);
        }
        if Instant::now() >= deadline {
            let stderr = state::fs::read_to_string(&stderr_path(fake)).unwrap_or_default();
            let events = state::fs::read_to_string(&Utf8PathBuf::from(format!(
                "{}.pv-fake.events.jsonl",
                fake.executable()
            )))
            .unwrap_or_default();
            bail!(
                "fake {} did not record a started event; exit status {:?}; events:\n{events}\nstderr:\n{stderr}",
                fake.executable(),
                child.0.try_wait()?
            );
        }
        sleep(POLL_INTERVAL);
    }
}

fn wait_for_exit(child: &mut FakeProcess) -> Result<ExitStatus> {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        if let Some(status) = child.0.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            bail!("fake process {} did not exit", child.0.id());
        }
        sleep(POLL_INTERVAL);
    }
}

fn signal(child: &FakeProcess, signal: Signal) -> Result<()> {
    let raw_pid = i32::try_from(child.0.id())?;
    let pid = Pid::from_raw(raw_pid).ok_or_else(|| anyhow!("invalid process id {raw_pid}"))?;
    kill_process(pid, signal)?;

    Ok(())
}

fn event_names(fake: &InstalledFake) -> Result<Vec<String>> {
    Ok(fake
        .events()?
        .into_iter()
        .map(|event| match event.kind {
            EventKind::Started { .. } => "started".to_owned(),
            EventKind::Signal { signal } => format!("signal {signal}"),
            EventKind::LifelineFired => "lifeline_fired".to_owned(),
            EventKind::Held { .. } => "held".to_owned(),
            EventKind::DescendantSpawned { .. } => "descendant_spawned".to_owned(),
            EventKind::ParentExited => "parent_exited".to_owned(),
            EventKind::Exit { code } => format!("exit {code}"),
        })
        .collect())
}

/// Waits until `find` matches one of the fake's events and returns what it found.
fn wait_for_event<T>(
    fake: &InstalledFake,
    mut find: impl FnMut(&EventKind) -> Option<T>,
) -> Result<T> {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        let events = fake.events()?;
        if let Some(found) = events.iter().find_map(|event| find(&event.kind)) {
            return Ok(found);
        }
        if Instant::now() >= deadline {
            bail!(
                "fake {} did not record the expected event; events {events:?}",
                fake.executable()
            );
        }
        sleep(POLL_INTERVAL);
    }
}

fn is_held(kind: &EventKind) -> Option<()> {
    matches!(kind, EventKind::Held { .. }).then_some(())
}
