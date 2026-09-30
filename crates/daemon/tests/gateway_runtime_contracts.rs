//! Contracts for the Gateway and PHP worker runtimes, written with PV's own config renderers,
//! supervisor, admin client, and readiness checks. Each contract runs against the `pv-fake` Caddy
//! and FrankenPHP personas, and against the real artifacts when `PV_E2E_REAL_ARTIFACTS=1`, so the
//! fakes can't drift from what PV actually depends on.
#![cfg(target_os = "macos")]

use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::process::Output;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use camino::Utf8Path;
use camino_tempfile::{Utf8TempDir, tempdir};
use daemon::gateway::{
    CaddyCliCommand, PhpWorkerRuntimePlan, RuntimeProject, gateway_process_spec,
    worker_process_spec,
};
use daemon::gateway_config::{
    GatewayConfigInput, PhpWorkerConfigInput, PhpWorkerProject, render_gateway_config,
    render_php_worker_config, render_php_worker_project_config,
};
use daemon::{
    CaddyAdminClient, CaddyAdminEndpoint, CaddyAdminError, ProcessSupervisor, ReadinessCheck,
    wait_for_readiness,
};
use pv_fake::Persona;
use resources::{
    ManagedResourceCommands, TargetPlatform, TrackSelector, caddy_adapter, frankenphp_adapter,
    php_adapter,
};
use state::PvPaths;

#[expect(
    clippy::disallowed_types,
    reason = "runtime contracts run the runtime CLI the way PV's validation step does"
)]
type RuntimeCommand = std::process::Command;

const READINESS_TIMEOUT: Duration = Duration::from_secs(30);
/// Caddy switches listeners before `/load` responds; this only absorbs scheduling delays.
const RELOAD_TIMEOUT: Duration = Duration::from_secs(5);
const STOP_GRACE_PERIOD: Duration = Duration::from_secs(10);
const GATEWAY_SERVER_NAME: &str = "pv-gateway.localhost";
const WORKER_PHP_TRACK: &str = "8.4";

#[tokio::test]
async fn fake_caddy_satisfies_the_gateway_contract() -> Result<()> {
    let (_tempdir, paths) = contract_paths()?;
    let executable = paths.home().join("caddy-release/bin/caddy");
    pv_fake::install(&executable, Persona::Caddy)?;

    gateway_contract(&paths, &executable).await
}

#[tokio::test]
#[ignore = "requires PV_E2E_REAL_ARTIFACTS=1 and PV_E2E_ARTIFACT_MANIFEST_URL"]
async fn real_caddy_satisfies_the_gateway_contract() -> Result<()> {
    let Some(manifest_url) = real_artifact_manifest_url()? else {
        return Ok(());
    };
    let (_tempdir, paths) = contract_paths()?;
    let commands = ManagedResourceCommands::new(paths.clone(), manifest_url, target_platform());
    let client = resources::UreqResourceHttpClient::new();
    let caddy = commands.install(&caddy_adapter()?, TrackSelector::Latest, &client)?;
    let executable = caddy_adapter()?.executable_path(caddy.current_artifact_path());

    gateway_contract(&paths, &executable).await
}

#[tokio::test]
async fn fake_frankenphp_satisfies_the_worker_contract() -> Result<()> {
    let (_tempdir, paths) = contract_paths()?;
    let artifact_root = paths.home().join("frankenphp-release");
    pv_fake::install(&artifact_root.join("bin/frankenphp"), Persona::FrankenPhp)?;

    worker_contract(&paths, &artifact_root, WORKER_PHP_TRACK).await
}

#[tokio::test]
#[ignore = "requires PV_E2E_REAL_ARTIFACTS=1 and PV_E2E_ARTIFACT_MANIFEST_URL"]
async fn real_frankenphp_satisfies_the_worker_contract() -> Result<()> {
    let Some(manifest_url) = real_artifact_manifest_url()? else {
        return Ok(());
    };
    let (_tempdir, paths) = contract_paths()?;
    let commands = ManagedResourceCommands::new(paths.clone(), manifest_url, target_platform());
    let client = resources::UreqResourceHttpClient::new();
    let php = commands.install(&php_adapter()?, TrackSelector::Latest, &client)?;
    let frankenphp = commands.install(
        &frankenphp_adapter()?,
        TrackSelector::Track(php.track().clone()),
        &client,
    )?;

    worker_contract(
        &paths,
        frankenphp.current_artifact_path(),
        frankenphp.track().as_str(),
    )
    .await
}

/// PV's Gateway lifecycle: validate the rendered root config, start it through the supervisor,
/// wait for the admin API and the public identity route over HTTP and HTTPS, reload it onto new
/// ports, keep serving through a reload that can't bind its port, and stop within the grace
/// period.
async fn gateway_contract(paths: &PvPaths, executable: &Utf8Path) -> Result<()> {
    seed_local_ca(paths)?;
    let [http_port, https_port, moved_http_port, moved_https_port] = available_ports()?;
    state::fs::ensure_user_dir(&paths.gateway_projects_config_dir())?;
    let config = gateway_config(paths, http_port, https_port)?;
    let config_path = paths.gateway_root_config();
    state::fs::write_sensitive_file(&config_path, &config)?;
    let command = CaddyCliCommand::caddy(executable);
    let endpoint = CaddyAdminEndpoint::new(paths.gateway_admin_socket());

    assert_validation_contract(paths, executable, &command, &config_path)?;
    let process = ProcessSupervisor::new(paths.clone())
        .start(gateway_process_spec(paths, &command))
        .await?;
    let serving = async {
        CaddyAdminClient::new()
            .wait_until_ready(&endpoint, READINESS_TIMEOUT)
            .await?;
        wait_for_readiness(
            gateway_identity(paths, http_port, https_port),
            READINESS_TIMEOUT,
        )
        .await?;

        // Caddy has closed the old ports and serves the new identity by the time `/load` returns.
        CaddyAdminClient::new()
            .load_caddyfile(
                &endpoint,
                gateway_config(paths, moved_http_port, moved_https_port)?.as_bytes(),
            )
            .await?;
        ensure_ports_closed(&[http_port, https_port])?;
        wait_for_readiness(
            gateway_identity(paths, moved_http_port, moved_https_port),
            RELOAD_TIMEOUT,
        )
        .await?;

        // A reload that can't bind its port fails after Caddy's adapter warnings, with a 200, and
        // leaves the previous config serving.
        let busy = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let busy_port = busy.local_addr()?.port();
        let busy_load = CaddyAdminClient::new()
            .load_caddyfile(
                &endpoint,
                gateway_config(paths, busy_port, moved_https_port)?.as_bytes(),
            )
            .await;
        ensure!(
            matches!(
                &busy_load,
                Err(CaddyAdminError::LoadReportedFailure { status: 200, detail, .. })
                    if detail.ends_with("bind: address already in use")
            ),
            "expected a reported bind failure, got {busy_load:?}"
        );
        wait_for_readiness(
            gateway_identity(paths, moved_http_port, moved_https_port),
            RELOAD_TIMEOUT,
        )
        .await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let stop_started = Instant::now();
    process.stop(STOP_GRACE_PERIOD).await?;
    let stopped_within = stop_started.elapsed();

    serving.with_context(|| runtime_log(&paths.gateway_supervisor_log()))?;
    ensure!(
        stopped_within < STOP_GRACE_PERIOD,
        "Gateway ignored SIGTERM for {stopped_within:?}"
    );

    Ok(())
}

/// PV's PHP worker lifecycle: validate the rendered worker root config with its imported project
/// site, start it through the supervisor, wait for the admin API and the worker port, move the
/// project to a new port by reloading the unchanged root config, and stop within the grace period.
async fn worker_contract(paths: &PvPaths, artifact_root: &Utf8Path, php_track: &str) -> Result<()> {
    let [port, moved_port] = available_ports()?;
    let runtime_key = php_track.to_owned();
    let project_root = paths.home().join("contract-project");
    state::fs::ensure_user_dir(&project_root)?;
    let project = PhpWorkerProject {
        primary_hostname: "contract.test".to_owned(),
        project_root: project_root.clone(),
        root: project_root.clone(),
    };
    let projects_dir = paths.worker_projects_config_dir(&runtime_key);
    let config = render_php_worker_config(&PhpWorkerConfigInput {
        php_track: php_track.to_owned(),
        port,
        admin_socket_path: paths.worker_admin_socket(&runtime_key),
        projects_config_glob: projects_dir.join("*.Caddyfile"),
        projects: vec![project.clone()],
    })?;
    let config_path = paths.worker_root_config(&runtime_key);
    let fragment_path = projects_dir.join("contract.Caddyfile");
    state::fs::write_sensitive_file(&config_path, &config)?;
    state::fs::write_sensitive_file(
        &fragment_path,
        &render_php_worker_project_config(&project, port)?,
    )?;
    resources::ensure_php_track_defaults(paths, php_track)?;
    let executable = frankenphp_adapter()?.executable_path(artifact_root);
    let command = CaddyCliCommand::frankenphp(&executable);
    let worker = PhpWorkerRuntimePlan {
        php_track: php_track.to_owned(),
        runtime_key: runtime_key.clone(),
        loaded_modules: Vec::new(),
        port,
        admin_socket_path: paths.worker_admin_socket(&runtime_key),
        projects: vec![RuntimeProject {
            id: "contract".to_owned(),
            render_config: true,
            primary_hostname: project.primary_hostname.clone(),
            project_root: project_root.clone(),
            root: project_root,
        }],
    };
    let endpoint = CaddyAdminEndpoint::new(paths.worker_admin_socket(&runtime_key));

    assert_validation_contract(paths, &executable, &command, &config_path)?;
    let process = ProcessSupervisor::new(paths.clone())
        .start(worker_process_spec(
            paths,
            &worker,
            &command,
            artifact_root,
        )?)
        .await?;
    let serving = async {
        CaddyAdminClient::new()
            .wait_until_ready(&endpoint, READINESS_TIMEOUT)
            .await?;
        wait_for_readiness(
            ReadinessCheck::Tcp {
                host: Ipv4Addr::LOCALHOST.to_string(),
                port,
            },
            READINESS_TIMEOUT,
        )
        .await?;

        // Caddy reads imported fragments on every load, so an unchanged root config still moves
        // the project to its fragment's new port.
        state::fs::write_sensitive_file(
            &fragment_path,
            &render_php_worker_project_config(&project, moved_port)?,
        )?;
        CaddyAdminClient::new()
            .load_caddyfile(&endpoint, config.as_bytes())
            .await?;
        ensure_ports_closed(&[port])?;
        wait_for_readiness(
            ReadinessCheck::Tcp {
                host: Ipv4Addr::LOCALHOST.to_string(),
                port: moved_port,
            },
            RELOAD_TIMEOUT,
        )
        .await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let stop_started = Instant::now();
    process.stop(STOP_GRACE_PERIOD).await?;
    let stopped_within = stop_started.elapsed();

    serving.with_context(|| runtime_log(&paths.worker_log(&runtime_key)))?;
    ensure!(
        stopped_within < STOP_GRACE_PERIOD,
        "PHP worker ignored SIGTERM for {stopped_within:?}"
    );

    Ok(())
}

/// `validate` accepts the rendered config and rejects a missing one with Caddy's message.
fn assert_validation_contract(
    paths: &PvPaths,
    executable: &Utf8Path,
    command: &CaddyCliCommand,
    config_path: &Utf8Path,
) -> Result<()> {
    let valid = run_runtime_cli(executable, &command.validate_arguments(config_path))?;
    ensure!(
        valid.status.success(),
        "validate rejected the rendered config: {}",
        String::from_utf8_lossy(&valid.stderr)
    );

    let missing_path = paths.config().join("missing.Caddyfile");
    let missing = run_runtime_cli(executable, &command.validate_arguments(&missing_path))?;
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(missing.stderr)?.lines().last(),
        Some(
            format!(
                "Error: reading config from file: open {missing_path}: no such file or directory"
            )
            .as_str()
        )
    );

    Ok(())
}

fn gateway_config(paths: &PvPaths, http_port: u16, https_port: u16) -> Result<String> {
    with_skip_install_trust(&render_gateway_config(&GatewayConfigInput {
        http_port,
        https_port,
        admin_socket_path: paths.gateway_admin_socket(),
        ca_certificate_path: paths.ca_certificate(),
        ca_private_key_path: paths.ca_private_key(),
        storage_path: paths.root().join("certificates/caddy"),
        access_log_path: paths.gateway_access_log(),
        error_log_path: paths.gateway_error_log(),
        projects_config_glob: paths.gateway_projects_config_dir().join("*.Caddyfile"),
        import_project_configs: true,
    })?)
}

fn gateway_identity(paths: &PvPaths, http_port: u16, https_port: u16) -> ReadinessCheck {
    ReadinessCheck::GatewayIdentity {
        http_host: Ipv4Addr::LOCALHOST.to_string(),
        http_port,
        https_host: Ipv4Addr::LOCALHOST.to_string(),
        https_port,
        server_name: GATEWAY_SERVER_NAME.to_owned(),
        path: "/__pv/health".to_owned(),
        expected_body: format!("pv-gateway-health-v1:{http_port}:{https_port}"),
        ca_certificate_path: paths.ca_certificate(),
    }
}

fn ensure_ports_closed(ports: &[u16]) -> Result<()> {
    for port in ports {
        ensure!(
            TcpStream::connect((Ipv4Addr::LOCALHOST, *port)).is_err(),
            "port {port} still accepts connections after the reload"
        );
    }

    Ok(())
}

fn contract_paths() -> Result<(Utf8TempDir, PvPaths)> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;

    Ok((tempdir, paths))
}

fn seed_local_ca(paths: &PvPaths) -> Result<()> {
    let local_ca = platform::generate_local_ca()?;
    state::fs::write_sensitive_file(&paths.ca_certificate(), &local_ca.certificate_pem)?;
    state::fs::write_sensitive_file(&paths.ca_private_key(), &local_ca.private_key_pem)?;

    Ok(())
}

/// Real Caddy tries to add a new local CA to the system trust store, which prompts on a
/// developer machine. The fakes ignore the option.
fn with_skip_install_trust(config: &str) -> Result<String> {
    let Some(global_options) = config.strip_prefix("{\n") else {
        bail!("rendered Caddyfile does not start with a global options block");
    };

    Ok(format!("{{\n    skip_install_trust\n{global_options}"))
}

fn available_ports<const COUNT: usize>() -> Result<[u16; COUNT]> {
    let listeners = (0..COUNT)
        .map(|_index| TcpListener::bind((Ipv4Addr::LOCALHOST, 0)))
        .collect::<Result<Vec<_>, _>>()?;
    let ports = listeners
        .iter()
        .map(|listener| Ok(listener.local_addr()?.port()))
        .collect::<Result<Vec<_>>>()?;

    ports
        .try_into()
        .map_err(|_ports| anyhow!("expected {COUNT} ports"))
}

fn run_runtime_cli(executable: &Utf8Path, arguments: &[String]) -> Result<Output> {
    Ok(RuntimeCommand::new(executable).args(arguments).output()?)
}

fn runtime_log(path: &Utf8Path) -> String {
    let log = state::fs::read_to_string(path).unwrap_or_default();
    format!("runtime log {path}:\n{log}")
}

fn target_platform() -> TargetPlatform {
    if cfg!(target_arch = "aarch64") {
        TargetPlatform::DarwinArm64
    } else {
        TargetPlatform::DarwinAmd64
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "ignored real-artifact contracts use environment variables as an explicit opt-in gate"
)]
fn real_artifact_manifest_url() -> Result<Option<String>> {
    if std::env::var("PV_E2E_REAL_ARTIFACTS").as_deref() != Ok("1") {
        return Ok(None);
    }

    match std::env::var("PV_E2E_ARTIFACT_MANIFEST_URL") {
        Ok(url) => Ok(Some(url)),
        Err(error) => bail!("PV_E2E_ARTIFACT_MANIFEST_URL is required: {error}"),
    }
}
