use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use daemon::ProcessSupervisor;
use daemon::gateway::{
    CaddyCliCommand, PhpWorkerRuntimePlan, gateway_process_spec, worker_process_spec,
};
use resources::{
    ManagedResourceCommands, TargetPlatform, TrackSelector, caddy_adapter, frankenphp_adapter,
    php_adapter,
};
use state::{Database, LinkProjectInput, PvPaths};

#[tokio::test]
#[ignore = "requires PV_E2E_REAL_ARTIFACTS=1 and PV_E2E_ARTIFACT_MANIFEST_URL"]
#[expect(
    clippy::disallowed_methods,
    reason = "ignored real-artifact E2E uses environment variables as an explicit opt-in gate"
)]
async fn real_artifact_gateway_e2e_serves_tiny_php_project() -> Result<()> {
    if std::env::var("PV_E2E_REAL_ARTIFACTS").as_deref() != Ok("1") {
        return Ok(());
    }
    let manifest_url = match std::env::var("PV_E2E_ARTIFACT_MANIFEST_URL") {
        Ok(url) => url,
        Err(error) => bail!("PV_E2E_ARTIFACT_MANIFEST_URL is required: {error}"),
    };

    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let commands = ManagedResourceCommands::new(paths.clone(), manifest_url, target_platform());
    let client = resources::UreqResourceHttpClient::new();

    let caddy_install = commands.install(&caddy_adapter()?, TrackSelector::Latest, &client)?;
    let php_install = commands.install(&php_adapter()?, TrackSelector::Latest, &client)?;
    let frankenphp_install = commands.install(
        &frankenphp_adapter()?,
        TrackSelector::Track(php_install.track().clone()),
        &client,
    )?;
    seed_local_ca(&paths)?;

    let project_root = tempdir.path().join("project");
    state::fs::write_sensitive_file(
        &project_root.join("public/index.php"),
        "<?php echo 'pv-real-artifact-ok';",
    )?;
    state::fs::write_sensitive_file(&project_root.join("pv.yml"), "document_root: public\n")?;
    let mut database = Database::open(&paths)?;
    database.link_project(LinkProjectInput {
        path: project_root.clone(),
        original_path: project_root.clone(),
        primary_hostname: "real-artifact.test".to_string(),
        config_path: project_root.join("pv.yml"),
        desired_php_track: None,
        additional_hostnames: vec![],
    })?;
    drop(database);

    let caddy_command =
        CaddyCliCommand::caddy(caddy_install.current_artifact_path().join("bin/caddy"));
    let frankenphp_command = CaddyCliCommand::frankenphp(
        frankenphp_install
            .current_artifact_path()
            .join("bin/frankenphp"),
    );
    let response = preserve_gateway_request_result(
        request_real_artifact_project(&paths).await,
        stop_gateway_runtimes(
            &paths,
            php_install.track().as_str(),
            &caddy_command,
            &frankenphp_command,
            frankenphp_install.current_artifact_path(),
        )
        .await,
        || real_artifact_diagnostics(&paths, php_install.track().as_str()),
    )?;

    assert_eq!(response, "pv-real-artifact-ok");
    assert_eq!(frankenphp_install.track(), php_install.track());

    Ok(())
}

#[test]
fn gateway_cleanup_error_is_reported_without_masking_request_error() -> Result<()> {
    let result: Result<()> = preserve_gateway_request_result(
        Err(anyhow!("request failed")),
        Err(anyhow!("cleanup failed")),
        || "diagnostics root".to_string(),
    );
    let Err(error) = result else {
        bail!("expected request failure");
    };
    let rendered = format!("{error:#}");

    assert!(rendered.contains("diagnostics root"));
    assert!(rendered.contains("request failed"));
    assert!(rendered.contains("gateway runtime cleanup also failed"));
    assert!(rendered.contains("cleanup failed"));

    Ok(())
}

fn preserve_gateway_request_result<T>(
    request: Result<T>,
    cleanup: Result<()>,
    diagnostics: impl FnOnce() -> String,
) -> Result<T> {
    match (request, cleanup) {
        (Ok(response), Ok(())) => Ok(response),
        (Ok(_response), Err(cleanup_error)) => {
            Err(cleanup_error.context("gateway runtime cleanup failed"))
        }
        (Err(error), Ok(())) => Err(error.context(diagnostics())),
        (Err(error), Err(cleanup_error)) => Err(error.context(format!(
            "{}\ngateway runtime cleanup also failed: {cleanup_error:#}",
            diagnostics()
        ))),
    }
}

fn real_artifact_diagnostics(paths: &PvPaths, php_track: &str) -> String {
    let mut diagnostics = format!("PV real-artifact diagnostics root: {}\n", paths.root());
    for path in [
        paths.gateway_root_config(),
        paths.worker_root_config(php_track),
        paths.gateway_log(),
        paths.worker_log(php_track),
        paths.gateway_access_log(),
        paths.gateway_error_log(),
        paths.gateway_runtime_metadata(),
        paths.worker_runtime_metadata(php_track),
    ] {
        append_optional_file(&mut diagnostics, &path);
    }

    diagnostics
}

fn append_optional_file(diagnostics: &mut String, path: &Utf8Path) {
    match state::fs::read_to_string(path) {
        Ok(content) => diagnostics.push_str(&format!("--- {path} ---\n{content}\n")),
        Err(error) => diagnostics.push_str(&format!("--- {path} unavailable: {error} ---\n")),
    }
}

async fn request_real_artifact_project(paths: &PvPaths) -> Result<String> {
    daemon::gateway::reconcile_gateway_runtimes(paths).await?;
    request_gateway_https_with_curl(paths, "real-artifact.test")
}

fn target_platform() -> TargetPlatform {
    if cfg!(target_arch = "aarch64") {
        TargetPlatform::DarwinArm64
    } else {
        TargetPlatform::DarwinAmd64
    }
}

fn seed_local_ca(paths: &PvPaths) -> Result<()> {
    let local_ca = platform::generate_local_ca()?;
    state::fs::write_sensitive_file(&paths.ca_certificate(), &local_ca.certificate_pem)?;
    state::fs::write_sensitive_file(&paths.ca_private_key(), &local_ca.private_key_pem)?;

    Ok(())
}

async fn stop_gateway_runtimes(
    paths: &PvPaths,
    php_track: &str,
    gateway_command: &CaddyCliCommand,
    worker_command: &CaddyCliCommand,
    worker_artifact_root: &Utf8Path,
) -> Result<()> {
    let supervisor = ProcessSupervisor::new(paths.clone());

    if let Some(gateway) = supervisor.adopt(&gateway_process_spec(paths, gateway_command))? {
        gateway.stop(Duration::from_secs(1)).await?;
    }
    let worker_plan = default_worker_plan(php_track);
    if let Some(worker) = supervisor.adopt(&worker_process_spec(
        paths,
        &worker_plan,
        worker_command,
        worker_artifact_root,
    )?)? {
        worker.stop(Duration::from_secs(1)).await?;
    }

    Ok(())
}

fn default_worker_plan(php_track: &str) -> PhpWorkerRuntimePlan {
    PhpWorkerRuntimePlan {
        php_track: php_track.to_owned(),
        runtime_key: php_track.to_owned(),
        loaded_modules: Vec::new(),
        port: 0,
        admin_socket_path: Utf8PathBuf::from("/tmp/pv-worker-admin.sock"),
        projects: Vec::new(),
    }
}

#[expect(
    clippy::disallowed_types,
    reason = "ignored real-artifact E2E shells out to curl to verify TLS with PV's CA"
)]
fn request_gateway_https_with_curl(paths: &PvPaths, hostname: &str) -> Result<String> {
    let mut database = Database::open(paths)?;
    let gateway_ports = database.assign_gateway_ports(|_port| true)?;
    let ca_certificate = paths.ca_certificate().to_string();
    let resolve = format!("{hostname}:{}:127.0.0.1", gateway_ports.https.port);
    let url = format!("https://{hostname}:{}/", gateway_ports.https.port);
    let output = std::process::Command::new("/usr/bin/curl")
        .args(vec![
            "--silent".to_string(),
            "--show-error".to_string(),
            "--fail".to_string(),
            "--connect-timeout".to_string(),
            "5".to_string(),
            "--max-time".to_string(),
            "30".to_string(),
            "--cacert".to_string(),
            ca_certificate,
            "--resolve".to_string(),
            resolve,
            url,
        ])
        .output()?;

    if !output.status.success() {
        bail!(
            "curl failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
