use anyhow::{Result, anyhow, bail, ensure};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
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
    pv_fake::install_monitor(&paths)?;
    let commands = ManagedResourceCommands::new(paths.clone(), manifest_url, target_platform());
    let client = resources::UreqResourceHttpClient::new();

    commands.install(&caddy_adapter()?, TrackSelector::Latest, &client)?;
    let php_install = commands.install(&php_adapter()?, TrackSelector::Latest, &client)?;
    let frankenphp_install = commands.install(
        &frankenphp_adapter()?,
        TrackSelector::Track(php_install.track().clone()),
        &client,
    )?;
    seed_local_ca(&paths)?;

    let parent = create_php_project(tempdir.path(), "parent", "root: public\n")?;
    let child = create_php_project(tempdir.path(), "child", "root: public\n")?;
    // The extension gives this Project its own worker, so only the Gateway can pick it.
    let admin = create_php_project(
        tempdir.path(),
        "admin",
        "root: public\nphp:\n  extensions: [apcu]\n",
    )?;
    let mut database = Database::open(&paths)?;
    link_project(&mut database, &parent, "laravel.test")?;
    link_project(&mut database, &child, "api.laravel.test")?;
    link_project(&mut database, &admin, "admin.laravel.test")?;
    drop(database);

    let php_track = php_install.track().as_str();
    let worker_runtime_keys = [
        php_track.to_owned(),
        state::php_runtime_key(php_track, &["apcu".to_owned()])?,
    ];
    preserve_gateway_request_result(
        verify_wildcard_routing(&paths, &parent, &child, &worker_runtime_keys[1]).await,
        daemon::stop_recorded_runtimes(paths.clone())
            .await
            .map_err(Into::into),
        || real_artifact_diagnostics(&paths, &worker_runtime_keys),
    )?;

    assert_eq!(frankenphp_install.track(), php_install.track());

    Ok(())
}

async fn verify_wildcard_routing(
    paths: &PvPaths,
    parent: &Utf8Path,
    child: &Utf8Path,
    admin_runtime_key: &str,
) -> Result<()> {
    // Fragment files are named by Project ID, so swapping the two hostnames flips which
    // Project's fragment Caddy imports first.
    for (laravel, api) in [("parent", "child"), ("child", "parent")] {
        if laravel == "child" {
            let mut database = Database::open(paths)?;
            link_project(&mut database, parent, "swap.test")?;
            link_project(&mut database, child, "laravel.test")?;
            link_project(&mut database, parent, "api.laravel.test")?;
        }
        daemon::gateway::reconcile_gateway_runtimes(paths).await?;
        ensure!(
            paths.worker_runtime_metadata(admin_runtime_key).exists(),
            "admin.laravel.test is not on its own `{admin_runtime_key}` worker"
        );

        for (hostname, project) in [
            ("laravel.test", laravel),
            ("tenant.laravel.test", laravel),
            ("api.laravel.test", api),
            ("x.api.laravel.test", api),
            ("admin.laravel.test", "admin"),
        ] {
            let response = request_gateway_https_with_curl(
                paths,
                hostname,
                &["--retry", "10", "--retry-delay", "1", "--retry-all-errors"],
            )?;
            ensure!(
                response == format!("{project}|{hostname}|https"),
                "{hostname} returned {response:?}"
            );
        }

        let deeper = request_gateway_https_with_curl(
            paths,
            "laravel.test",
            &["--header", "Host: v1.foo.laravel.test"],
        );
        ensure!(
            !matches!(&deeper, Ok(body) if body.contains('|')),
            "v1.foo.laravel.test reached a Project: {deeper:?}"
        );
        ensure!(
            request_gateway_https_with_curl(paths, "v1.foo.laravel.test", &[]).is_err(),
            "v1.foo.laravel.test completed a trusted TLS handshake"
        );
    }

    Ok(())
}

fn create_php_project(root: &Utf8Path, name: &str, config: &str) -> Result<Utf8PathBuf> {
    let project_root = root.join(name);
    state::fs::write_sensitive_file(
        &project_root.join("public/index.php"),
        &format!(
            "<?php echo '{name}|', $_SERVER['HTTP_HOST'] ?? '', '|', $_SERVER['HTTP_X_FORWARDED_PROTO'] ?? '';"
        ),
    )?;
    state::fs::write_sensitive_file(&project_root.join("pv.yml"), config)?;

    Ok(project_root)
}

fn link_project(database: &mut Database, project_root: &Utf8Path, hostname: &str) -> Result<()> {
    database.link_project(LinkProjectInput {
        path: project_root.to_path_buf(),
        original_path: project_root.to_path_buf(),
        primary_hostname: hostname.to_owned(),
        config_path: project_root.join("pv.yml"),
        desired_php_track: None,
    })?;

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

#[test]
fn real_artifact_diagnostics_includes_gateway_supervisor_log() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::write_sensitive_file(&paths.gateway_supervisor_log(), "supervisor failure\n")?;

    let diagnostics = real_artifact_diagnostics(&paths, &["8.4".to_owned()]);

    assert!(diagnostics.contains(&format!(
        "--- {} ---\nsupervisor failure\n",
        paths.gateway_supervisor_log()
    )));

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

fn real_artifact_diagnostics(paths: &PvPaths, worker_runtime_keys: &[String]) -> String {
    let mut diagnostics = format!("PV real-artifact diagnostics root: {}\n", paths.root());
    let worker_paths = worker_runtime_keys.iter().flat_map(|runtime_key| {
        [
            paths.worker_root_config(runtime_key),
            paths.worker_log(runtime_key),
            paths.worker_runtime_metadata(runtime_key),
        ]
    });
    for path in [
        paths.gateway_root_config(),
        paths.gateway_log(),
        paths.gateway_access_log(),
        paths.gateway_error_log(),
        paths.gateway_supervisor_log(),
        paths.gateway_runtime_metadata(),
    ]
    .into_iter()
    .chain(worker_paths)
    {
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

#[expect(
    clippy::disallowed_types,
    reason = "ignored real-artifact E2E shells out to curl to verify TLS with PV's CA"
)]
fn request_gateway_https_with_curl(
    paths: &PvPaths,
    hostname: &str,
    extra_arguments: &[&str],
) -> Result<String> {
    let mut database = Database::open(paths)?;
    let gateway_ports = database.assign_gateway_ports(|_port| true)?;
    let ca_certificate = paths.ca_certificate().to_string();
    let resolve = format!("{hostname}:{}:127.0.0.1", gateway_ports.https.port);
    let url = format!("https://{hostname}:{}/", gateway_ports.https.port);
    let output = std::process::Command::new("/usr/bin/curl")
        .args([
            "--silent",
            "--show-error",
            "--fail",
            "--connect-timeout",
            "5",
            "--max-time",
            "30",
            "--cacert",
            &ca_certificate,
            "--resolve",
            &resolve,
        ])
        .args(extra_arguments)
        .arg(url)
        .output()?;

    if !output.status.success() {
        bail!(
            "curl failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
