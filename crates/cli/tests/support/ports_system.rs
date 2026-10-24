use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{self, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use camino::{Utf8Path, Utf8PathBuf};
use cli::{Environment, ProcessEnvironment, run_with_environment};
use daemon::gateway_config::{GatewayConfigInput, render_gateway_config};
use daemon::{MonitorStart, MonitorStop, ReadinessCheck, StopSignal, wait_for_readiness};
use insta::assert_snapshot;
use platform::{
    LowPortInspection, PfRedirectConfig, PlatformError, PrivilegedHelperClient,
    SYSTEM_SOCKET_PROXY_DIRECTORY, render_socket_proxy_units, socket_proxy_unit_names,
};
use pv_fake::Persona;
use serde_json::Value;
use state::{Database, GatewayPort, MonitorReservation, PortRequest, PvPaths, fs};
use tokio::runtime::{Builder, Runtime};

use super::{
    HelperFixture, checksum, require_units_absent, root, root_text, service_property,
    with_helper_fixture,
};

const CONTROL: &str = "/run/pv-ports-system-test";
const CLEANUP_UNIT: &str = "pv-ports-system-cleanup";
const FOREIGN_SOCKET: &str = "pv-ports-foreign.socket";
const FOREIGN_SERVICE: &str = "pv-ports-foreign.service";
const STOP: MonitorStop = MonitorStop {
    signal: StopSignal::Terminate,
    grace_ms: 1_000,
};

// Preflight proves every removed path and unit absent. Stop accepted helper work
// before removing files, including when the independent timer stops the test.
const CLEANUP_SCRIPT: &str = r#"set -eu
stop_units() {
    /usr/bin/systemctl --no-block stop "$@" || :
    /usr/bin/systemctl kill --kill-who=all --signal=KILL "$@" || :
    stopped=false
    for _attempt in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
        remaining=$(/usr/bin/systemctl list-units --all --no-legend --no-pager --plain \
            --state=active,activating,deactivating,reloading "$@")
        if [ -z "$remaining" ]; then stopped=true; break; fi
        sleep 0.5
    done
    [ "$stopped" = true ]
}
if [ "${1-}" = orphan ]; then stop_units pv-helper-system-test.service; fi
stop_units pv-helper.socket pv-helper.service
stop_units pv-http.service pv-https.service pv-http.socket pv-https.socket pv-ports-foreign.socket pv-ports-foreign.service
for name in pv-http.socket pv-http.service pv-https.socket pv-https.service pv-ports-foreign.socket pv-ports-foreign.service; do
    failed=$(/usr/bin/systemctl list-units --all --no-legend --no-pager --plain --state=failed "$name")
    if [ -n "$failed" ]; then /usr/bin/systemctl reset-failed "$name"; fi
    rm -f -- "/etc/systemd/system/$name" "/etc/systemd/system/$name.candidate" "/etc/systemd/system/$name.rollback" \
        "/etc/systemd/system/sockets.target.wants/$name" "/run/systemd/system/sockets.target.wants/$name"
    rm -rf -- "/etc/systemd/system/$name.d"
done
/usr/bin/systemctl daemon-reload
# Retain the control directory and timer journal until explicit cleanup verifies absence.
"#;

#[test]
#[ignore = "requires PV_LINUX_SYSTEM_TESTS=1, the built helper and a disposable bounded systemd service with free ports 80 and 443"]
fn systemd_socket_proxies_route_gateway_identity_repair_and_refuse_foreign_units() -> Result<()> {
    with_helper_fixture(exercise_ports)
}

fn unit_names() -> Vec<String> {
    socket_proxy_unit_names()
        .into_iter()
        .chain([FOREIGN_SOCKET.to_owned(), FOREIGN_SERVICE.to_owned()])
        .collect()
}

fn fixture_paths() -> Vec<Utf8PathBuf> {
    unit_names()
        .into_iter()
        .flat_map(|name| {
            let unit = Utf8Path::new(SYSTEM_SOCKET_PROXY_DIRECTORY).join(&name);
            [
                unit.clone(),
                format!("{unit}.candidate").into(),
                format!("{unit}.rollback").into(),
                format!("{unit}.d").into(),
                format!("/etc/systemd/system/sockets.target.wants/{name}").into(),
                format!("/run/systemd/system/sockets.target.wants/{name}").into(),
            ]
        })
        .collect()
}

fn require_absent() -> Result<()> {
    for path in fixture_paths() {
        ensure!(
            !fs::path_entry_exists(&path)?,
            "fixture path remains: {path}"
        );
    }
    let names = unit_names();
    require_units_absent(&names.iter().map(String::as_str).collect::<Vec<_>>())
}

fn socket_target_dependencies() -> Result<String> {
    let dependencies = service_property("sockets.target", "Wants")?;
    let mut sockets = dependencies
        .split_whitespace()
        .filter(|name| matches!(*name, "pv-http.socket" | "pv-https.socket"))
        .collect::<Vec<_>>();
    sockets.sort_unstable();
    Ok(sockets.join("\n"))
}

fn exercise_ports(helper_fixture: &mut HelperFixture, helper: &Utf8Path) -> Result<()> {
    require_absent()?;
    ensure!(!fs::path_entry_exists(Utf8Path::new(CONTROL))?);
    require_units_absent(&[
        "pv-ports-system-cleanup.timer",
        "pv-ports-system-cleanup.service",
    ])?;
    let paths = PvPaths::for_home(helper_fixture.scratch.path().join("ports-home"));
    let mut fixture = PortsFixture {
        paths,
        runtime: Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?,
        monitor_started: false,
        control_created: false,
        timer_attempted: false,
        cleaned: false,
    };
    let result = fixture.exercise(helper_fixture, helper);
    let cleanup = fixture.cleanup();
    match (result, cleanup) {
        (Err(error), Err(cleanup)) => {
            Err(error.context(format!("cleanup also failed: {cleanup:#}")))
        }
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

struct PortsFixture {
    paths: PvPaths,
    runtime: Runtime,
    monitor_started: bool,
    control_created: bool,
    timer_attempted: bool,
    cleaned: bool,
}

impl PortsFixture {
    fn exercise(&mut self, helper_fixture: &mut HelperFixture, helper: &Utf8Path) -> Result<()> {
        self.arm_cleanup(helper_fixture)?;
        helper_fixture.arm_cleanup()?;
        helper_fixture.install(helper, &checksum(helper)?)?;
        helper_fixture.capture_process()?;
        ensure!(
            PrivilegedHelperClient
                .inspect_low_ports()?
                .ports
                .iter()
                .all(|port| port.available)
        );
        fs::ensure_layout(&self.paths)?;
        pv_fake::install_monitor(&self.paths)?;
        let ca = platform::generate_local_ca()?;
        fs::write_sensitive_file(&self.paths.ca_certificate(), &ca.certificate_pem)?;
        fs::write_sensitive_file(&self.paths.ca_private_key(), &ca.private_key_pem)?;
        let [http_port, https_port, changed_port] = pv_fake::ports::<3>()?;
        let config = PfRedirectConfig::new(http_port, https_port);
        let mut database = Database::open(&self.paths)?;
        for (owner, port) in [
            (GatewayPort::Http, http_port),
            (GatewayPort::Https, https_port),
        ] {
            database.assign_port(PortRequest::gateway(owner, port, port, port), |_| true)?;
        }
        drop(database);

        helper_fixture.root_file(
            "foreign.socket",
            "[Socket]\nListenStream=127.0.0.1:80\n",
            &format!("{SYSTEM_SOCKET_PROXY_DIRECTORY}/{FOREIGN_SOCKET}"),
            "0644",
        )?;
        helper_fixture.root_file(
            "foreign.service",
            "[Service]\nExecStart=/usr/bin/true\n",
            &format!("{SYSTEM_SOCKET_PROXY_DIRECTORY}/{FOREIGN_SERVICE}"),
            "0644",
        )?;
        root(&["/usr/bin/systemctl", "daemon-reload"])?;
        root(&["/usr/bin/systemctl", "start", FOREIGN_SOCKET])?;
        let inspection = PrivilegedHelperClient.inspect_low_ports()?;
        let conflict = inspection
            .ports
            .iter()
            .find(|port| port.port == 80)
            .context("HTTP inspection")?;
        ensure!(!conflict.available);
        ensure!(
            !conflict.owners.is_empty(),
            "root listener diagnostics lost systemd's socket"
        );
        ensure!(PrivilegedHelperClient.apply_pf(&config).is_err());
        assert_eq!(service_property(FOREIGN_SOCKET, "ActiveState")?, "active");
        for name in socket_proxy_unit_names() {
            ensure!(!fs::path_entry_exists(
                &Utf8Path::new(SYSTEM_SOCKET_PROXY_DIRECTORY).join(name)
            )?);
        }
        root(&["/usr/bin/systemctl", "stop", FOREIGN_SOCKET])?;
        for name in [FOREIGN_SOCKET, FOREIGN_SERVICE] {
            root(&[
                "/usr/bin/rm",
                "--",
                &format!("{SYSTEM_SOCKET_PROXY_DIRECTORY}/{name}"),
            ])?;
        }
        root(&["/usr/bin/systemctl", "daemon-reload"])?;
        require_absent()?;

        self.start_gateway(&config)?;
        self.command("ports:install")?;
        self.require_active(&config)?;
        assert_snapshot!(socket_target_dependencies()?, @r"
        pv-http.socket
        pv-https.socket
        ");
        self.command("ports:install")?;
        self.require_active(&config)?;
        root(&["/usr/bin/systemctl", "disable", "pv-http.socket"])?;
        self.command("ports:install")?;
        self.require_active(&config)?;

        self.stop_gateway()?;
        self.require_status("unknown", "systemd")?;
        self.start_gateway(&config)?;
        self.require_active(&config)?;

        root(&[
            "/usr/bin/systemctl",
            "stop",
            "pv-https.service",
            "pv-https.socket",
        ])?;
        let mut previous_units = render_socket_proxy_units(&config);
        previous_units[2].1 = previous_units[2]
            .1
            .replace("127.0.0.1:443", &format!("127.0.0.1:{changed_port}"));
        helper_fixture.root_file(
            "rollback-activation.socket",
            &previous_units[2].1,
            "/etc/systemd/system/pv-https.socket",
            "0644",
        )?;
        helper_fixture.root_file(
            "rollback-conflict.socket",
            "[Socket]\nListenStream=127.0.0.1:443\n",
            &format!("{SYSTEM_SOCKET_PROXY_DIRECTORY}/{FOREIGN_SOCKET}"),
            "0644",
        )?;
        helper_fixture.root_file(
            "rollback-conflict.service",
            "[Service]\nExecStart=/usr/bin/true\n",
            &format!("{SYSTEM_SOCKET_PROXY_DIRECTORY}/{FOREIGN_SERVICE}"),
            "0644",
        )?;
        root(&["/usr/bin/systemctl", "daemon-reload"])?;
        root(&[
            "/usr/bin/systemctl",
            "start",
            "pv-https.socket",
            FOREIGN_SOCKET,
        ])?;
        self.require_forwarding(&config, 80, changed_port)?;
        let conflict = PrivilegedHelperClient
            .inspect_low_ports()?
            .ports
            .into_iter()
            .find(|port| port.port == 443 && !port.available)
            .context("foreign HTTPS listener is missing")?
            .conflict_message();
        let failure = PrivilegedHelperClient
            .apply_pf(&config)
            .err()
            .context("conflicting socket proxy apply unexpectedly succeeded")?;
        assert_snapshot!(failure.to_string().replace(&conflict, "<foreign HTTPS conflict>"), @"system integration operation failed: socket proxy apply failed: system integration operation failed: system integration operation failed: <foreign HTTPS conflict>; restored the previous PV unit files, enablement and activity; port routing may be partial and a queued or in-flight request may still complete; inspect pv-http and pv-https units, their .candidate and .rollback files with an administrator before retrying `pv ports:install` or `pv ports:uninstall`");
        for (name, content) in previous_units {
            assert_eq!(
                fs::read_to_string(&Utf8Path::new(SYSTEM_SOCKET_PROXY_DIRECTORY).join(name))?,
                content
            );
        }
        for name in ["pv-http.socket", "pv-https.socket"] {
            assert_eq!(service_property(name, "ActiveState")?, "active");
            assert_eq!(
                root_text(&["/usr/bin/systemctl", "is-enabled", name])?.trim(),
                "enabled"
            );
        }
        assert_eq!(
            service_property("pv-http.service", "ActiveState")?,
            "active"
        );
        assert_eq!(service_property(FOREIGN_SOCKET, "ActiveState")?, "active");
        self.require_forwarding(&config, 80, changed_port)?;
        assert_snapshot!(socket_target_dependencies()?, @r"
        pv-http.socket
        pv-https.socket
        ");

        root(&["/usr/bin/systemctl", "stop", FOREIGN_SOCKET])?;
        for name in [FOREIGN_SOCKET, FOREIGN_SERVICE] {
            root(&[
                "/usr/bin/rm",
                "--",
                &format!("{SYSTEM_SOCKET_PROXY_DIRECTORY}/{name}"),
            ])?;
        }
        root(&["/usr/bin/systemctl", "daemon-reload"])?;
        self.command("ports:install")?;
        self.require_active(&config)?;

        // A reload can report port 80 while the held socket still listens elsewhere.
        let socket = render_socket_proxy_units(&config)[0].1.clone();
        let moved_socket = socket.replace("127.0.0.1:80", &format!("127.0.0.1:{changed_port}"));
        helper_fixture.root_file(
            "moved.socket",
            &moved_socket,
            "/etc/systemd/system/pv-http.socket",
            "0644",
        )?;
        root(&["/usr/bin/systemctl", "daemon-reload"])?;
        root(&["/usr/bin/systemctl", "restart", "pv-http.socket"])?;
        self.require_forwarding(&config, changed_port, 443)?;
        let proxy_pid = service_property("pv-http.service", "MainPID")?;
        ensure!(proxy_pid != "0", "HTTP forwarder is not running");
        helper_fixture.root_file(
            "current.socket",
            &socket,
            "/etc/systemd/system/pv-http.socket",
            "0644",
        )?;
        root(&["/usr/bin/systemctl", "daemon-reload"])?;
        assert_eq!(service_property("pv-http.service", "MainPID")?, proxy_pid);
        self.require_forwarding(&config, changed_port, 443)?;
        let inspection = platform::inspect_active_pf_redirects_unprivileged()?;
        ensure!(inspection.pf_enabled);
        assert_eq!(inspection.pv_config, Some(config.clone()));
        self.require_status("unknown", "systemd")?;
        self.command("ports:install")?;
        self.require_active(&config)?;

        self.command("ports:uninstall")?;
        self.command("ports:uninstall")?;
        require_absent()?;
        self.require_status("inactive", "systemd")?;
        for name in socket_proxy_unit_names() {
            ensure!(!fs::path_entry_exists(
                &self.paths.ports_config().join(name)
            )?);
        }
        let foreign = "[Service]\nExecStart=/usr/bin/true\n";
        helper_fixture.root_file(
            "foreign-pv.service",
            foreign,
            "/etc/systemd/system/pv-http.service",
            "0644",
        )?;
        ensure!(PrivilegedHelperClient.apply_pf(&config).is_err());
        ensure!(PrivilegedHelperClient.remove_pf().is_err());
        assert_eq!(
            fs::read_to_string(Utf8Path::new("/etc/systemd/system/pv-http.service"))?,
            foreign
        );
        root(&["/usr/bin/rm", "--", "/etc/systemd/system/pv-http.service"])?;
        root(&["/usr/bin/systemctl", "daemon-reload"])?;
        require_absent()?;
        Ok(())
    }

    fn command(&self, command: &str) -> Result<()> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run_with_environment(
            ["pv", command],
            &PortEnvironment(&self.paths),
            &mut stdout,
            &mut stderr,
        )?;
        ensure!(
            code == ExitCode::SUCCESS,
            "{command} failed: {}{}",
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr)
        );
        Ok(())
    }

    fn require_status(&self, state: &str, evidence: &str) -> Result<()> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run_with_environment(
            ["pv", "ports:status", "--json"],
            &PortEnvironment(&self.paths),
            &mut stdout,
            &mut stderr,
        )?;
        ensure!(stderr.is_empty(), "{}", String::from_utf8_lossy(&stderr));
        let observed: Value = serde_json::from_slice(&stdout)?;
        assert_eq!(observed["state"], state);
        assert_eq!(observed["evidence"], evidence);
        assert_eq!(
            code,
            if state == "active" {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        );
        Ok(())
    }

    fn require_active(&self, config: &PfRedirectConfig) -> Result<()> {
        let inspection = platform::inspect_active_pf_redirects_unprivileged()?;
        assert_eq!(PrivilegedHelperClient.inspect_pf()?, inspection);
        ensure!(inspection.pf_enabled);
        assert_eq!(inspection.pv_config.as_ref(), Some(config));
        ensure!(platform::socket_proxy_files_current(
            config,
            &self.paths.ports_config(),
            Utf8Path::new(SYSTEM_SOCKET_PROXY_DIRECTORY)
        ));
        for name in ["pv-http.socket", "pv-https.socket"] {
            assert_eq!(
                root_text(&["/usr/bin/systemctl", "is-enabled", name])?.trim(),
                "enabled"
            );
        }
        self.require_status("active", "probe")
    }

    fn require_forwarding(
        &self,
        config: &PfRedirectConfig,
        http_port: u16,
        https_port: u16,
    ) -> Result<()> {
        self.runtime.block_on(wait_for_readiness(
            ReadinessCheck::GatewayIdentity {
                http_host: "127.0.0.1".to_owned(),
                http_port,
                https_host: "127.0.0.1".to_owned(),
                https_port,
                server_name: "pv-gateway.localhost".to_owned(),
                path: "/__pv/health".to_owned(),
                expected_body: format!(
                    "pv-gateway-health-v1:{}:{}",
                    config.http_port, config.https_port
                ),
                ca_certificate_path: self.paths.ca_certificate(),
            },
            Duration::from_secs(2),
        ))?;
        Ok(())
    }

    fn start_gateway(&mut self, config: &PfRedirectConfig) -> Result<()> {
        let fake = pv_fake::install(&self.paths.root().join("fixture/bin/caddy"), Persona::Caddy)?;
        let content = render_gateway_config(&GatewayConfigInput {
            http_port: config.http_port,
            https_port: config.https_port,
            admin_socket_path: self.paths.gateway_admin_socket(),
            ca_certificate_path: self.paths.ca_certificate(),
            ca_private_key_path: self.paths.ca_private_key(),
            storage_path: self.paths.root().join("fixture/storage"),
            access_log_path: self.paths.gateway_access_log(),
            error_log_path: self.paths.gateway_error_log(),
            projects_config_glob: self.paths.gateway_projects_config_dir().join("*.Caddyfile"),
            import_project_configs: false,
        })?;
        fs::write_sensitive_file(&self.paths.gateway_root_config(), &content)?;
        self.runtime.block_on(daemon::start_monitor(
            &self.paths,
            &self.paths.active_pv_binary(),
            MonitorStart {
                subject: "gateway".to_owned(),
                command: fake.executable().to_owned(),
                arguments: vec![
                    "run".to_owned(),
                    "--config".to_owned(),
                    self.paths.gateway_root_config().to_string(),
                    "--adapter".to_owned(),
                    "caddyfile".to_owned(),
                ],
                private_environment: BTreeMap::new(),
                log_path: self.paths.gateway_supervisor_log(),
                fallback_stop: STOP,
                lifeline_fd: Some(pv_fake::lifeline_fd()?),
            },
        ))?;
        self.monitor_started = true;
        self.require_forwarding(config, config.http_port, config.https_port)
            .with_context(|| {
                format!(
                    "fake Gateway log: {:?}; events: {:?}",
                    fs::read_to_string(&self.paths.gateway_supervisor_log()),
                    fake.events(),
                )
            })?;
        Ok(())
    }

    fn stop_gateway(&mut self) -> Result<()> {
        if self.monitor_started {
            self.runtime.block_on(async {
                daemon::stop_monitor(&self.paths, "gateway", STOP).await?;
                daemon::release_monitor(&self.paths, "gateway").await
            })?;
            self.monitor_started = false;
            ensure!(!fs::path_entry_exists(&self.paths.monitor_dir("gateway"))?);
            drop(MonitorReservation::acquire(&self.paths, "gateway")?);
            fs::remove_file_if_exists(&self.paths.gateway_admin_socket())?;
        }
        Ok(())
    }

    fn arm_cleanup(&mut self, helper_fixture: &HelperFixture) -> Result<()> {
        root(&["/usr/bin/install", "-d", "-m", "0700", CONTROL])?;
        self.control_created = true;
        helper_fixture.root_file(
            "ports-cleanup.sh",
            CLEANUP_SCRIPT,
            &format!("{CONTROL}/cleanup.sh"),
            "0700",
        )?;
        self.timer_attempted = true;
        root(&[
            "/usr/bin/systemd-run",
            "--collect",
            "--on-active=140s",
            "--unit",
            CLEANUP_UNIT,
            "--property=RuntimeMaxSec=30s",
            "/bin/sh",
            &format!("{CONTROL}/cleanup.sh"),
            "orphan",
        ])?;
        Ok(())
    }

    fn cleanup(&mut self) -> Result<()> {
        let gateway = self.stop_gateway();
        let system = self.cleanup_system();
        match (gateway, system) {
            (Err(error), Err(cleanup)) => {
                Err(error.context(format!("system cleanup also failed: {cleanup:#}")))
            }
            (Err(error), _) | (_, Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    fn cleanup_system(&mut self) -> Result<()> {
        if self.cleaned || !self.control_created {
            return Ok(());
        }
        if self.timer_attempted {
            root(&["/bin/sh", &format!("{CONTROL}/cleanup.sh")])?;
            require_absent()?;
            root(&[
                "/usr/bin/systemctl",
                "stop",
                &format!("{CLEANUP_UNIT}.timer"),
            ])?;
        }
        root(&["/usr/bin/rm", "-rf", "--", CONTROL])?;
        self.cleaned = true;
        Ok(())
    }
}

impl Drop for PortsFixture {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            let _reported = writeln!(
                io::stderr().lock(),
                "ports system fixture cleanup failed: {error:#}"
            );
        }
    }
}

struct PortEnvironment<'a>(&'a PvPaths);

impl Environment for PortEnvironment<'_> {
    fn var_os(&self, _key: &str) -> Option<OsString> {
        None
    }
    fn home_dir(&self) -> Option<PathBuf> {
        Some(self.0.home().as_std_path().to_owned())
    }
    fn current_dir(&self) -> io::Result<PathBuf> {
        Ok(self.0.home().as_std_path().to_owned())
    }
    fn current_exe(&self) -> io::Result<PathBuf> {
        Ok(self.0.active_pv_binary().into_std_path_buf())
    }
    fn stdin_is_terminal(&self) -> bool {
        false
    }
    fn open_url(&self, _url: &str) -> io::Result<()> {
        Err(io::Error::other("no browser in the system fixture"))
    }
    fn inspect_low_ports(&self) -> Result<LowPortInspection, PlatformError> {
        ProcessEnvironment.inspect_low_ports()
    }
    fn probe_gateway_redirects(
        &self,
        expected: &PfRedirectConfig,
        ca: &Utf8Path,
    ) -> Result<(), String> {
        ProcessEnvironment.probe_gateway_redirects(expected, ca)
    }
}
