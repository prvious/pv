use std::future::Future;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use futures_util::StreamExt;
use protocol::{
    DaemonCommand, DaemonRequest, DaemonResponse, PROTOCOL_VERSION, ResponseStatus, write_line,
};
use resources::{
    ArtifactManifestCache, ManagedResourceCommands, ResourceAdapter, RuntimeArtifactAdapter,
    TargetPlatform, TrackName, TrackSelector, composer_adapter, frankenphp_adapter,
    mailpit_adapter, mysql_adapter, php_adapter, postgres_adapter, redis_adapter, rustfs_adapter,
};
use serde_json::Value;
use state::{
    Database, LinkProjectInput, ProjectRecord, PvPaths, ResourceAllocationStatus,
    RuntimeObservedStatus, RuntimeSubject,
};
use tokio::net::UnixStream;
use tokio::time::timeout;

const TEST_TIMEOUT: Duration = Duration::from_secs(600);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
const EVENT_TIMEOUT: Duration = Duration::from_secs(180);
const SETUP_DEFAULTS: &[SetupDefault] = &[
    SetupDefault {
        resource: "composer",
        track: "2",
        adapter: composer_adapter,
        darwin_amd64_required: true,
    },
    SetupDefault {
        resource: "frankenphp",
        track: "8.5",
        adapter: frankenphp_adapter,
        darwin_amd64_required: false,
    },
    SetupDefault {
        resource: "mailpit",
        track: "1",
        adapter: mailpit_adapter,
        darwin_amd64_required: true,
    },
    SetupDefault {
        resource: "mysql",
        track: "8.4",
        adapter: mysql_adapter,
        darwin_amd64_required: true,
    },
    SetupDefault {
        resource: "php",
        track: "8.5",
        adapter: php_adapter,
        darwin_amd64_required: false,
    },
    SetupDefault {
        resource: "postgres",
        track: "18",
        adapter: postgres_adapter,
        darwin_amd64_required: true,
    },
    SetupDefault {
        resource: "redis",
        track: "8.8",
        adapter: redis_adapter,
        darwin_amd64_required: true,
    },
    SetupDefault {
        resource: "rustfs",
        track: "1",
        adapter: rustfs_adapter,
        darwin_amd64_required: true,
    },
];

struct SetupDefault {
    resource: &'static str,
    track: &'static str,
    adapter: fn() -> resources::Result<RuntimeArtifactAdapter>,
    darwin_amd64_required: bool,
}

#[tokio::test]
#[ignore = "requires PV_E2E_REAL_ARTIFACTS=1 and PV_E2E_ARTIFACT_MANIFEST_URL"]
async fn real_artifact_manifest_contains_setup_defaults_for_current_platform() -> Result<()> {
    let Some(manifest_url) = real_artifact_manifest_url()? else {
        return Ok(());
    };
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let client = resources::UreqResourceHttpClient::new();
    let manifest = ArtifactManifestCache::new(paths.downloads())
        .refresh_latest(&manifest_url, &client)?
        .into_manifest();
    let target = target_platform();

    for default in SETUP_DEFAULTS {
        if skip_unstable_darwin_amd64_default(default, target) {
            continue;
        }

        let adapter = (default.adapter)()?;
        let track = TrackName::new(default.track)?;
        manifest
            .select_latest(adapter.resource_name(), &track, target)
            .with_context(|| {
                format!(
                    "manifest is missing setup default {} track {} for {:?}",
                    default.resource, default.track, target
                )
            })?;
    }

    Ok(())
}

#[tokio::test]
#[ignore = "requires PV_E2E_REAL_ARTIFACTS=1 and PV_E2E_ARTIFACT_MANIFEST_URL"]
async fn real_artifact_resource_matrix_smokes_backing_services_and_composer() -> Result<()> {
    let Some(manifest_url) = real_artifact_manifest_url()? else {
        return Ok(());
    };

    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let client = resources::UreqResourceHttpClient::new();
    let commands = ManagedResourceCommands::new(paths.clone(), manifest_url, target_platform());

    let mysql = commands.install(&mysql_adapter()?, setup_track_selector("mysql")?, &client)?;
    let postgres = commands.install(
        &postgres_adapter()?,
        setup_track_selector("postgres")?,
        &client,
    )?;
    let redis = commands.install(&redis_adapter()?, setup_track_selector("redis")?, &client)?;
    let mailpit = commands.install(
        &mailpit_adapter()?,
        setup_track_selector("mailpit")?,
        &client,
    )?;
    let rustfs = commands.install(&rustfs_adapter()?, setup_track_selector("rustfs")?, &client)?;
    let composer = if target_platform() == TargetPlatform::DarwinArm64 {
        Some(commands.install_composer_with_php_pair(setup_track_selector("php")?, &client)?)
    } else {
        None
    };

    let project = link_resource_matrix_project(
        &paths,
        &tempdir.path().join("project"),
        mysql.track().as_str(),
        postgres.track().as_str(),
        redis.track().as_str(),
        mailpit.track().as_str(),
        rustfs.track().as_str(),
    )?;
    seed_local_ca(&paths)?;
    let daemon = daemon::RunningDaemon::start(paths.clone()).await?;
    let result = timeout(TEST_TIMEOUT, async {
        run_reconciliation_job(&paths, &format!("project:{}", project.id)).await?;
        assert_resource_matrix_evidence(&paths, &project)?;
        if composer.is_some() {
            assert_composer_shim_reports_version(&paths).await?;
        }
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("real artifact resource matrix timed out")
    .and_then(|result| result);

    let demand_cleanup = async {
        write_project_config(
            &project,
            r#"env:
  APP_URL: "${project_url}"
"#,
        )?;
        await_cleanup_reconciliation(run_reconciliation_job(
            &paths,
            &format!("project:{}", project.id),
        ))
        .await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let daemon_cleanup = daemon.shutdown().await;

    preserve_primary_result_with_cleanup(
        result,
        [
            ("demand cleanup", demand_cleanup),
            (
                "daemon shutdown",
                daemon_cleanup.map_err(anyhow::Error::from),
            ),
        ],
    )?;

    if let Some(composer) = composer {
        assert_eq!(composer.composer().track().as_str(), "2");
    }

    Ok(())
}

#[test]
fn resource_matrix_cleanup_errors_are_reported_without_masking_primary_error() -> Result<()> {
    let result = preserve_primary_result_with_cleanup(
        Err(anyhow!("matrix failed")),
        [
            ("demand cleanup", Err(anyhow!("demand cleanup failed"))),
            ("daemon shutdown", Err(anyhow!("daemon shutdown failed"))),
        ],
    );
    let Err(error) = result else {
        bail!("expected matrix failure");
    };
    let rendered = format!("{error:#}");

    assert!(rendered.contains("matrix failed"));
    assert!(rendered.contains("cleanup also failed"));
    assert!(rendered.contains("demand cleanup failed"));
    assert!(rendered.contains("daemon shutdown failed"));

    Ok(())
}

#[tokio::test]
async fn cleanup_reconciliation_timeout_reports_bounded_cleanup() -> Result<()> {
    let result = await_cleanup_reconciliation_with_timeout(
        async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok::<String, anyhow::Error>("reconciled".to_string())
        },
        Duration::from_millis(1),
    )
    .await;
    let Err(error) = result else {
        bail!("expected cleanup reconciliation timeout");
    };
    let rendered = format!("{error:#}");

    assert!(rendered.contains("cleanup reconciliation timed out"));

    Ok(())
}

#[test]
fn resource_matrix_dotenv_assertion_redacts_contents() -> Result<()> {
    let result = assert_dotenv_contains("MYSQL_PASSWORD=secret\n", "POSTGRES_URL=postgres://");
    let Err(error) = result else {
        bail!("expected missing dotenv assertion");
    };
    let rendered = format!("{error:#}");

    assert!(rendered.contains("expected .env to contain `POSTGRES_URL=postgres://`"));
    assert!(rendered.contains("redacted"));
    assert!(!rendered.contains("secret"));

    Ok(())
}

fn preserve_primary_result_with_cleanup<const N: usize>(
    result: Result<()>,
    cleanup_results: [(&'static str, Result<()>); N],
) -> Result<()> {
    let cleanup_context = cleanup_failure_context(cleanup_results);
    match (result, cleanup_context) {
        (Ok(()), None) => Ok(()),
        (Ok(()), Some(cleanup_context)) => bail!("{cleanup_context}"),
        (Err(error), None) => Err(error),
        (Err(error), Some(cleanup_context)) => Err(error.context(cleanup_context)),
    }
}

fn cleanup_failure_context<const N: usize>(
    cleanup_results: [(&'static str, Result<()>); N],
) -> Option<String> {
    let failures = cleanup_results
        .into_iter()
        .filter_map(|(label, result)| result.err().map(|error| format!("{label}: {error:#}")))
        .collect::<Vec<_>>();
    if failures.is_empty() {
        None
    } else {
        Some(format!("cleanup also failed:\n{}", failures.join("\n")))
    }
}

async fn await_cleanup_reconciliation(
    reconciliation: impl Future<Output = Result<String>>,
) -> Result<()> {
    await_cleanup_reconciliation_with_timeout(reconciliation, TEST_TIMEOUT).await
}

async fn await_cleanup_reconciliation_with_timeout(
    reconciliation: impl Future<Output = Result<String>>,
    duration: Duration,
) -> Result<()> {
    timeout(duration, reconciliation)
        .await
        .context("cleanup reconciliation timed out")??;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "ignored real-artifact E2E uses environment variables as an explicit opt-in gate"
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

fn seed_local_ca(paths: &PvPaths) -> Result<()> {
    let local_ca = platform::generate_local_ca()?;
    state::fs::write_sensitive_file(&paths.ca_certificate(), &local_ca.certificate_pem)?;
    state::fs::write_sensitive_file(&paths.ca_private_key(), &local_ca.private_key_pem)?;

    Ok(())
}

fn target_platform() -> TargetPlatform {
    if cfg!(target_arch = "aarch64") {
        TargetPlatform::DarwinArm64
    } else {
        TargetPlatform::DarwinAmd64
    }
}

fn skip_unstable_darwin_amd64_default(default: &SetupDefault, target: TargetPlatform) -> bool {
    target == TargetPlatform::DarwinAmd64 && !default.darwin_amd64_required
}

fn link_resource_matrix_project(
    paths: &PvPaths,
    project_path: &Utf8Path,
    mysql_track: &str,
    postgres_track: &str,
    redis_track: &str,
    mailpit_track: &str,
    rustfs_track: &str,
) -> Result<ProjectRecord> {
    let config = format!(
        r#"env:
  APP_URL: "${{project_url}}"
mysql:
  version: "{mysql_track}"
  env:
    MYSQL_HOST: "${{host}}"
    MYSQL_PORT: "${{port}}"
    MYSQL_USERNAME: "${{username}}"
    MYSQL_PASSWORD: "${{password}}"
  allocations:
    app-db:
      env:
        MYSQL_DATABASE: "${{database}}"
        MYSQL_URL: "${{url}}"
postgres:
  version: "{postgres_track}"
  env:
    POSTGRES_HOST: "${{host}}"
    POSTGRES_PORT: "${{port}}"
    POSTGRES_USERNAME: "${{username}}"
    POSTGRES_PASSWORD: "${{password}}"
  allocations:
    app-db:
      env:
        POSTGRES_DATABASE: "${{database}}"
        POSTGRES_URL: "${{url}}"
redis:
  version: "{redis_track}"
  env:
    REDIS_HOST: "${{host}}"
    REDIS_PORT: "${{port}}"
    REDIS_URL: "${{url}}"
  allocations:
    cache:
      env:
        REDIS_PREFIX: "${{prefix}}"
mailpit:
  version: "{mailpit_track}"
  env:
    MAIL_HOST: "${{smtp_host}}"
    MAIL_PORT: "${{smtp_port}}"
    MAILPIT_DASHBOARD: "${{dashboard_url}}"
rustfs:
  version: "{rustfs_track}"
  env:
    S3_ENDPOINT: "${{endpoint}}"
    S3_ACCESS_KEY: "${{access_key}}"
    S3_SECRET_KEY: "${{secret_key}}"
  allocations:
    uploads:
      env:
        AWS_BUCKET: "${{bucket}}"
        AWS_ENDPOINT: "${{endpoint}}"
        AWS_ACCESS_KEY_ID: "${{access_key}}"
        AWS_SECRET_ACCESS_KEY: "${{secret_key}}"
        AWS_URL: "${{url}}"
"#
    );

    state::fs::write_sensitive_file(&project_path.join("pv.yml"), &config)?;
    let mut database = Database::open(paths)?;
    let result = database.link_project(LinkProjectInput {
        path: project_path.to_path_buf(),
        original_path: project_path.to_path_buf(),
        primary_hostname: "real-artifact-resources.test".to_string(),
        config_path: project_path.join("pv.yml"),
        desired_php_track: None,
        additional_hostnames: vec![],
    })?;

    Ok(result.project)
}

fn write_project_config(project: &ProjectRecord, config: &str) -> Result<()> {
    state::fs::write_sensitive_file(&project.config_path, config)?;

    Ok(())
}

fn assert_resource_matrix_evidence(paths: &PvPaths, project: &ProjectRecord) -> Result<()> {
    let database = Database::open(paths)?;
    let dotenv = state::fs::read_to_string(&project.path.join(".env"))?;
    for expected in [
        "MYSQL_URL=mysql://",
        "POSTGRES_URL=postgres://",
        "REDIS_URL=redis://",
        "REDIS_PREFIX=real-artifact-resources-test-cache-",
        "MAILPIT_DASHBOARD=http://127.0.0.1:",
        "AWS_BUCKET=real-artifact-resources-test-uploads",
        "AWS_ENDPOINT=http://127.0.0.1:",
    ] {
        assert_dotenv_contains(&dotenv, expected)?;
    }

    assert_ready_allocations(&database, &project.id, "mysql", 1)?;
    assert_ready_allocations(&database, &project.id, "postgres", 1)?;
    assert_ready_allocations(&database, &project.id, "redis", 1)?;
    assert_ready_allocations(&database, &project.id, "rustfs", 1)?;

    let runtime_states = database.runtime_observed_states()?;
    for resource in ["mailpit", "mysql", "postgres", "redis", "rustfs"] {
        let track = setup_track(resource)?;
        if !runtime_states.iter().any(|state| {
            state.subject
                == RuntimeSubject::Resource {
                    name: resource.to_string(),
                    track: track.to_string(),
                }
                && state.status == RuntimeObservedStatus::Running
        }) {
            bail!("expected running runtime status for `{resource}`; states: {runtime_states:#?}");
        }

        if !state::fs::path_exists(&paths.resource_runtime_metadata(resource, track)) {
            bail!("expected runtime metadata for `{resource}`");
        }
        if !state::fs::path_exists(&paths.resource_log(resource, track)) {
            bail!("expected runtime log for `{resource}`");
        }
    }

    Ok(())
}

fn assert_dotenv_contains(dotenv: &str, expected: &str) -> Result<()> {
    if dotenv.contains(expected) {
        return Ok(());
    }

    bail!("expected .env to contain `{expected}`; .env content redacted");
}

fn setup_track(resource: &str) -> Result<&'static str> {
    SETUP_DEFAULTS
        .iter()
        .find(|default| default.resource == resource)
        .map(|default| default.track)
        .with_context(|| format!("unknown setup resource `{resource}`"))
}

fn setup_track_selector(resource: &str) -> Result<TrackSelector> {
    Ok(TrackSelector::Track(TrackName::new(setup_track(
        resource,
    )?)?))
}

fn assert_ready_allocations(
    database: &Database,
    project_id: &str,
    resource: &str,
    expected_count: usize,
) -> Result<()> {
    let allocations = database.resource_allocations(project_id, resource)?;
    let ready_count = allocations
        .iter()
        .filter(|allocation| allocation.status == ResourceAllocationStatus::Ready)
        .count();

    if ready_count != expected_count {
        bail!(
            "expected {expected_count} ready `{resource}` allocation(s), got {ready_count}: {allocations:#?}"
        );
    }

    Ok(())
}

async fn run_reconciliation_job(paths: &PvPaths, scope: &str) -> Result<String> {
    let stream = UnixStream::connect(paths.daemon_socket()).await?;
    let mut transport = protocol::transport(stream);
    let request = DaemonRequest {
        protocol_version: PROTOCOL_VERSION,
        command: DaemonCommand::RunJob {
            kind: "reconcile".to_string(),
            scope: scope.to_string(),
        },
    };

    write_line(&mut transport, &request).await?;
    let response = read_response(&mut transport).await?;
    if response.status() != ResponseStatus::Accepted {
        bail!("daemon rejected reconciliation: {}", response.message());
    }
    let Some(job_id) = response.job_id() else {
        bail!("daemon accepted reconciliation without a job id");
    };
    let job_id = job_id.to_string();

    loop {
        let Some(line_result) = timeout(EVENT_TIMEOUT, transport.next()).await? else {
            bail!("daemon closed before completing job `{job_id}`");
        };
        let line = line_result?;
        if let Some(summary) = parse_job_event(&line, &job_id)? {
            return Ok(summary);
        }
    }
}

async fn read_response(
    transport: &mut protocol::DaemonTransport<UnixStream>,
) -> Result<DaemonResponse> {
    let Some(line_result) = timeout(RESPONSE_TIMEOUT, transport.next()).await? else {
        bail!("daemon closed before sending a response");
    };
    let line = line_result?;

    Ok(serde_json::from_str(&line)?)
}

fn parse_job_event(line: &str, expected_job_id: &str) -> Result<Option<String>> {
    let value = serde_json::from_str::<Value>(line)?;
    let Some(line_type) = value.get("type").and_then(Value::as_str) else {
        bail!("daemon sent event without a type: {line}");
    };
    let Some(job_id) = value.get("job_id").and_then(Value::as_str) else {
        bail!("daemon sent job event without a job_id: {line}");
    };
    if job_id != expected_job_id {
        bail!("daemon sent event for job `{job_id}` while waiting for `{expected_job_id}`");
    }

    match line_type {
        "job_started" | "progress" | "log" => Ok(None),
        "job_completed" => {
            let Some(summary) = value.get("summary").and_then(Value::as_str) else {
                bail!("daemon sent job_completed without a summary: {line}");
            };
            Ok(Some(summary.to_string()))
        }
        "job_failed" => {
            let Some(error) = value.get("error").and_then(Value::as_str) else {
                bail!("daemon sent job_failed without an error: {line}");
            };
            bail!("daemon reconciliation failed: {error}");
        }
        _ => bail!("daemon sent unexpected `{line_type}` line: {line}"),
    }
}

#[expect(
    clippy::disallowed_types,
    reason = "ignored real-artifact E2E shells out to PV's CLI to exercise the Composer shim"
)]
async fn assert_composer_shim_reports_version(paths: &PvPaths) -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = tokio::process::Command::new("cargo")
        .args([
            "run",
            "--quiet",
            "-p",
            "pv",
            "--",
            "shim:composer",
            "--version",
        ])
        .env("HOME", paths.home())
        .current_dir(workspace_root)
        .output()
        .await?;

    if !output.status.success() {
        bail!(
            "composer shim failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.contains("Composer version") {
        bail!("unexpected composer --version output: {stdout}");
    }

    Ok(())
}
