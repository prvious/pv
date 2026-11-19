use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use platform::clock::real;
use resources::{
    ManagedResourceCommands, TargetPlatform, TrackSelector, caddy_adapter, frankenphp_adapter,
    php_adapter,
};
use state::{Database, LinkProjectInput, PortOwner, PvPaths};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

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
    let commands =
        ManagedResourceCommands::new(paths.clone(), manifest_url, TargetPlatform::current()?);
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
    let gateway_listeners = crate::host::test_gateway_listeners()?;
    preserve_gateway_request_result(
        verify_wildcard_routing(
            &paths,
            &gateway_listeners,
            &parent,
            &child,
            &worker_runtime_keys[1],
        )
        .await,
        daemon::stop_recorded_runtimes(paths.clone())
            .await
            .map_err(Into::into),
        || real_artifact_diagnostics(&paths, &worker_runtime_keys),
    )?;

    assert_eq!(frankenphp_install.track(), php_install.track());

    Ok(())
}

#[tokio::test]
#[ignore = "requires PV_E2E_REAL_ARTIFACTS=1 and PV_E2E_ARTIFACT_MANIFEST_URL"]
#[expect(
    clippy::disallowed_methods,
    reason = "ignored real-artifact E2E uses environment variables as an explicit opt-in gate"
)]
async fn real_artifact_gateway_e2e_proxies_local_project() -> Result<()> {
    if std::env::var("PV_E2E_REAL_ARTIFACTS").as_deref() != Ok("1") {
        return Ok(());
    }
    let manifest_url = std::env::var("PV_E2E_ARTIFACT_MANIFEST_URL")?;
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    pv_fake::install_monitor(&paths)?;
    pv_fake::seed_dns_port(&paths)?;
    let commands =
        ManagedResourceCommands::new(paths.clone(), manifest_url, TargetPlatform::current()?);
    commands.install(
        &caddy_adapter()?,
        TrackSelector::Latest,
        &resources::UreqResourceHttpClient::new(),
    )?;
    seed_local_ca(&paths)?;

    let upstream_port = pv_fake::port()?;
    let project_root = tempdir.path().join("api");
    state::fs::write_sensitive_file(
        &project_root.join("pv.yml"),
        &format!("proxy: {upstream_port}\n"),
    )?;
    let hostname = "api.laravel.test";
    link_project(&mut Database::open(&paths)?, &project_root, hostname)?;
    let project = Database::open(&paths)?
        .project_by_path(&project_root)?
        .context("proxy Project was not linked")?;
    let scope = format!("project:{}", project.id);

    let host = crate::host::test_host()?;
    let (_, https_port) = host.gateway_listeners.ports();
    let daemon =
        daemon::RunningDaemon::start_without_managed_resource_adapters(paths.clone(), host).await?;
    let request = async {
        reconcile(&paths, "system").await?;
        let unavailable = request_gateway_https_with_curl(
            &paths, https_port, hostname,
            &["--no-fail", "--write-out", "\n%{http_code}"],
        )?;
        ensure!(
            unavailable == format!(
                "PV could not reach http://127.0.0.1:{upstream_port}. Start your app and check that it listens on this address.\n502"
            ),
            "an offline upstream returned {unavailable:?}"
        );

        state::fs::write_sensitive_file(&project.config_path, "proxy: 0\n")?;
        ensure!(reconcile(&paths, &scope).await.is_err(), "an invalid proxy port was applied");

        let upstream = TcpListener::bind(("127.0.0.1", upstream_port)).await?;
        let request_paths = paths.clone();
        let request = tokio::task::spawn_blocking(move || {
            request_gateway_https_with_curl(
                &request_paths, https_port, hostname,
                &[
                    "--header", "X-Forwarded-Host: bogus.test",
                    "--header", "X-Forwarded-Proto: http",
                    "--header", "X-Forwarded-For: 192.0.2.10",
                ],
            )
        });
        let echo = async {
            real::timeout(Duration::from_secs(30), echo_proxy_request(upstream))
                .await
                .context("the Gateway did not forward the request to the upstream")?
        };
        let ((), response) = tokio::try_join!(echo, async { request.await? })?;
        let authority = format!("{hostname}:{https_port}");
        ensure!(
            response == format!("{authority}|{authority}|https|127.0.0.1"),
            "the upstream received {response:?}"
        );
        ensure!(
            request_gateway_https_with_curl(&paths, https_port, "x.api.laravel.test", &["--no-fail"]).is_err(),
            "x.api.laravel.test reached the proxy Project"
        );
        let database = Database::open(&paths)?;
        ensure!(database.managed_resource_tracks()?.iter().all(|track| track.resource_name == "caddy"));
        ensure!(database.assigned_ports()?.iter().all(|port| !matches!(port.owner, PortOwner::PhpWorker { .. })));
        drop(database);

        state::fs::write_sensitive_file(&project.config_path, &format!("serve: false\nproxy: {upstream_port}\n"))?;
        let stopped = reconcile(&paths, &scope).await?;
        let removed = request_gateway_https_with_curl(&paths, https_port, hostname, &["--no-fail"]);
        ensure!(removed.is_err(), "{hostname} still answered after serving was disabled: {removed:?}");
        ensure!(!paths.gateway_projects_config_dir().join(format!("{}.Caddyfile", project.id)).exists());
        let targeted_gateway = state::fs::read_to_string(&paths.daemon_log())?
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .any(|record| record["job_id"] == stopped.id && record["phase"] == "gateway"
                && record["subject"] == "target_project");
        ensure!(targeted_gateway, "proxy removal should reconcile only the target Project");
        Ok(())
    }.await;
    let shutdown = daemon.shutdown().await;
    let stop = daemon::stop_recorded_runtimes(paths.clone()).await;
    preserve_gateway_request_result(request, shutdown.and(stop).map_err(Into::into), || {
        real_artifact_diagnostics(&paths, &[])
    })
}

async fn reconcile(paths: &PvPaths, scope: &str) -> Result<daemon::CompletedJob> {
    let paths = paths.clone();
    let scope = scope.to_owned();
    Ok(
        tokio::task::spawn_blocking(move || daemon::run_job_blocking(paths, "reconcile", &scope))
            .await??,
    )
}

async fn echo_proxy_request(upstream: TcpListener) -> Result<()> {
    let (stream, _) = upstream.accept().await?;
    let mut stream = BufReader::new(stream);
    let mut headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        if stream.read_line(&mut line).await? == 0 || line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.trim_end().split_once(':') {
            headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let body = [
        "host",
        "x-forwarded-host",
        "x-forwarded-proto",
        "x-forwarded-for",
    ]
    .map(|name| headers.get(name).map_or("", String::as_str))
    .join("|");
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.get_mut().write_all(response.as_bytes()).await?;
    Ok(())
}

async fn verify_wildcard_routing(
    paths: &PvPaths,
    gateway_listeners: &daemon::GatewayListenerHolder,
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
        daemon::gateway::reconcile_gateway_runtimes(paths, gateway_listeners).await?;
        let (_http_port, https_port) = gateway_listeners.source().ports();
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
                https_port,
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
            https_port,
            "laravel.test",
            &["--header", "Host: v1.foo.laravel.test"],
        );
        ensure!(
            !matches!(&deeper, Ok(body) if body.contains('|')),
            "v1.foo.laravel.test reached a Project: {deeper:?}"
        );
        ensure!(
            request_gateway_https_with_curl(paths, https_port, "v1.foo.laravel.test", &[]).is_err(),
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
    https_port: u16,
    hostname: &str,
    extra_arguments: &[&str],
) -> Result<String> {
    let ca_certificate = paths.ca_certificate().to_string();
    let resolve = format!("{hostname}:{https_port}:127.0.0.1");
    let url = format!("https://{hostname}:{https_port}/");
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
