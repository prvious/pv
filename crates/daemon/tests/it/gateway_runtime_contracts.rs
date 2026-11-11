//! Contracts for the Gateway and PHP worker runtimes, written with PV's own config renderers,
//! supervisor, admin client, and readiness checks. Each contract runs against the `pv-fake` Caddy
//! and FrankenPHP personas, and against the real artifacts when `PV_E2E_REAL_ARTIFACTS=1`, so the
//! fakes can't drift from what PV actually depends on.
#![cfg(unix)]

use std::net::{Ipv4Addr, TcpStream};
use std::process::Output;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use camino::Utf8Path;
use camino_tempfile::{Utf8TempDir, tempdir};
use daemon::gateway::{
    CaddyCliCommand, PhpWorkerRuntimePlan, RuntimeProject, gateway_process_spec,
    gateway_readiness_check, worker_process_spec,
};
use daemon::gateway_config::{
    GatewayConfigInput, GatewayProjectRoute, PhpWorkerConfigInput, PhpWorkerProject,
    render_gateway_config, render_gateway_project_config, render_php_worker_config,
    render_php_worker_project_config,
};
use daemon::{
    CaddyAdminClient, CaddyAdminEndpoint, ProcessSupervisor, ReadinessCheck, wait_for_readiness,
};
use platform::GatewayListenerSource;
use platform::clock::{Limit, real};
use pv_fake::Persona;
use resources::{
    ManagedResourceCommands, TargetPlatform, TrackSelector, caddy_adapter, frankenphp_adapter,
    php_adapter,
};
use serde_json::Value;
use state::PvPaths;

#[expect(
    clippy::disallowed_types,
    reason = "runtime contracts run the runtime CLI the way PV's validation step does"
)]
type RuntimeCommand = std::process::Command;

const READINESS: Limit = Limit::new("contract readiness", Duration::from_secs(30));
/// Caddy switches listeners before `/load` responds; this only absorbs scheduling delays.
const RELOAD: Limit = Limit::new("contract reload readiness", Duration::from_secs(5));
const STOP_GRACE_PERIOD: Duration = Duration::from_secs(10);
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
    let commands =
        ManagedResourceCommands::new(paths.clone(), manifest_url, TargetPlatform::current()?);
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
    let commands =
        ManagedResourceCommands::new(paths.clone(), manifest_url, TargetPlatform::current()?);
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

/// PV's Gateway lifecycle: validate the rendered root config, start it through the supervisor with
/// two inherited listeners, wait for the admin API and the public identity route over HTTP and
/// HTTPS, keep serving through two reloads, the second adding a project site, and stop within the
/// grace period. Caddy's own default ports point at two more free ports, where any listener Caddy
/// opened by itself would show.
async fn gateway_contract(paths: &PvPaths, executable: &Utf8Path) -> Result<()> {
    seed_local_ca(paths)?;
    let [
        http_port,
        https_port,
        default_http_port,
        default_https_port,
        worker_port,
    ] = pv_fake::ports()?;
    let listeners = GatewayListenerSource::Bind {
        http_port,
        https_port,
    }
    .open()?;
    state::fs::ensure_user_dir(&paths.gateway_projects_config_dir())?;
    let config = gateway_config(paths, default_http_port, default_https_port)?;
    let config_path = paths.gateway_root_config();
    state::fs::write_sensitive_file(&config_path, &config)?;
    let command = CaddyCliCommand::caddy(executable);
    let endpoint = CaddyAdminEndpoint::new(paths.gateway_admin_socket());
    let identity = gateway_readiness_check(http_port, https_port, &paths.ca_certificate());

    assert_validation_contract(paths, executable, &command, &config_path)?;
    let process = ProcessSupervisor::new(paths.clone())
        .start_gateway(gateway_process_spec(paths, &command), &listeners)
        .await?;
    let serving = async {
        CaddyAdminClient::new()
            .wait_until_ready(&endpoint, &READINESS)
            .await?;
        wait_for_readiness(identity.clone(), &READINESS).await?;

        CaddyAdminClient::new()
            .load_caddyfile(&endpoint, config.as_bytes())
            .await?;
        wait_for_readiness(identity.clone(), &RELOAD).await?;

        state::fs::write_sensitive_file(
            &paths
                .gateway_projects_config_dir()
                .join("contract.Caddyfile"),
            &render_gateway_project_config(&GatewayProjectRoute {
                id: "contract".to_owned(),
                render_config: true,
                primary_hostname: "contract.test".to_owned(),
                worker_port,
                access_log_path: paths.gateway_access_log(),
            })?,
        )?;
        CaddyAdminClient::new()
            .load_caddyfile(&endpoint, config.as_bytes())
            .await?;
        wait_for_readiness(identity.clone(), &RELOAD).await?;
        ensure_ports_closed(&[default_http_port, default_https_port])?;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let stop_started = real::now();
    process.stop(STOP_GRACE_PERIOD).await?;
    let stopped_within = real::elapsed(stop_started);

    serving.with_context(|| runtime_log(&paths.gateway_supervisor_log()))?;
    ensure!(
        stopped_within < STOP_GRACE_PERIOD,
        "Gateway ignored SIGTERM for {stopped_within:?}"
    );

    Ok(())
}

/// PV's PHP worker lifecycle: validate the rendered worker root config with its imported project
/// site, start it through the supervisor, wait for the admin API and the worker's health route,
/// move the worker to a new port with a reload, and stop within the grace period.
async fn worker_contract(paths: &PvPaths, artifact_root: &Utf8Path, php_track: &str) -> Result<()> {
    let [port, moved_port] = pv_fake::ports()?;
    let runtime_key = php_track.to_owned();
    let project_root = paths.home().join("contract-project");
    state::fs::ensure_user_dir(&project_root)?;
    let project = PhpWorkerProject {
        primary_hostname: "contract.test".to_owned(),
        project_root: project_root.clone(),
        root: project_root.clone(),
    };
    let projects_dir = paths.worker_projects_config_dir(&runtime_key);
    let worker_config = |port| {
        render_php_worker_config(&PhpWorkerConfigInput {
            php_track: php_track.to_owned(),
            port,
            admin_socket_path: paths.worker_admin_socket(&runtime_key),
            projects_config_glob: projects_dir.join("*.Caddyfile"),
            projects: vec![project.clone()],
        })
    };
    let config = worker_config(port)?;
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
            .wait_until_ready(&endpoint, &READINESS)
            .await?;
        wait_for_readiness(worker_identity(port), &READINESS).await?;

        // Caddy has closed the old port and serves the new health route by the time `/load`
        // returns.
        state::fs::write_sensitive_file(
            &fragment_path,
            &render_php_worker_project_config(&project, moved_port)?,
        )?;
        CaddyAdminClient::new()
            .load_caddyfile(&endpoint, worker_config(moved_port)?.as_bytes())
            .await?;
        ensure_ports_closed(&[port])?;
        wait_for_readiness(worker_identity(moved_port), &RELOAD).await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let stop_started = real::now();
    process.stop(STOP_GRACE_PERIOD).await?;
    let stopped_within = real::elapsed(stop_started);

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
    let stderr = String::from_utf8(missing.stderr)?;
    let error = stderr
        .lines()
        .last()
        .context("missing config error was not reported")?;
    let expected =
        format!("reading config from file: open {missing_path}: no such file or directory");
    if let Ok(log) = serde_json::from_str::<Value>(error) {
        assert_eq!(log["level"], "error");
        assert_eq!(log["msg"], expected);
    } else {
        assert_eq!(error, format!("Error: {expected}"));
    }

    Ok(())
}

/// The production root config, with Caddy's default ports moved to `default_http_port` and
/// `default_https_port`.
fn gateway_config(
    paths: &PvPaths,
    default_http_port: u16,
    default_https_port: u16,
) -> Result<String> {
    with_test_global_options(
        &render_gateway_config(&GatewayConfigInput {
            admin_socket_path: paths.gateway_admin_socket(),
            ca_certificate_path: paths.ca_certificate(),
            ca_private_key_path: paths.ca_private_key(),
            storage_path: paths.root().join("certificates/caddy"),
            access_log_path: paths.gateway_access_log(),
            error_log_path: paths.gateway_error_log(),
            projects_config_glob: paths.gateway_projects_config_dir().join("*.Caddyfile"),
            import_project_configs: true,
        })?,
        &[
            "skip_install_trust".to_owned(),
            format!("http_port {default_http_port}"),
            format!("https_port {default_https_port}"),
        ],
    )
}

fn worker_identity(port: u16) -> ReadinessCheck {
    ReadinessCheck::WorkerIdentity {
        host: Ipv4Addr::LOCALHOST.to_string(),
        port,
        server_name: "pv-worker.localhost".to_owned(),
        path: "/__pv/health".to_owned(),
        expected_body: format!("pv-worker-health-v1:{port}"),
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
    pv_fake::install_monitor(&paths)?;

    Ok((tempdir, paths))
}

fn seed_local_ca(paths: &PvPaths) -> Result<()> {
    let local_ca = platform::generate_local_ca()?;
    state::fs::write_sensitive_file(&paths.ca_certificate(), &local_ca.certificate_pem)?;
    state::fs::write_sensitive_file(&paths.ca_private_key(), &local_ca.private_key_pem)?;

    Ok(())
}

/// Adds `options` to the rendered global options block. Real Caddy tries to add a new local CA to
/// the system trust store, which prompts on a developer machine, so the Gateway's config skips it;
/// the fakes ignore that option.
fn with_test_global_options(config: &str, options: &[String]) -> Result<String> {
    let Some(global_options) = config.strip_prefix("{\n") else {
        bail!("rendered Caddyfile does not start with a global options block");
    };
    let options = options
        .iter()
        .map(|option| format!("    {option}\n"))
        .collect::<String>();

    Ok(format!("{{\n{options}{global_options}"))
}

fn run_runtime_cli(executable: &Utf8Path, arguments: &[String]) -> Result<Output> {
    Ok(RuntimeCommand::new(executable).args(arguments).output()?)
}

fn runtime_log(path: &Utf8Path) -> String {
    let log = state::fs::read_to_string(path).unwrap_or_default();
    format!("runtime log {path}:\n{log}")
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
