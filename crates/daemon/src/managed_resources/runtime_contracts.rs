//! Contracts for the Managed Resource runtimes, driven through each runtime adapter the way
//! reconciliation does: the adapter's environment and process spec, PV's start and readiness wait
//! (which also rejects a runtime that exits right after its readiness check), and a stop within
//! the grace period. Each contract runs against a `pv-fake` persona, and against the real artifact
//! when `PV_E2E_REAL_ARTIFACTS=1`, so the fakes can't drift from what PV depends on.

use std::collections::BTreeMap;
use std::future;
use std::net::{Ipv4Addr, TcpListener};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::{Utf8TempDir, tempdir};
use pv_fake::Persona;
use resources::{
    ManagedResourceCommands, ManagedResourceInstall, ResourceAdapter, TargetPlatform,
    TrackSelector, mailpit_adapter, redis_adapter,
};
use state::PvPaths;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

use super::tests::ManagedResourceFixtureGuard;
use super::{
    ManagedResourceRuntimeAdapter, ManagedResourceRuntimeContext, adapter_readiness_timeout,
    start_or_adopt_runtime,
};
use crate::ProcessSupervisor;

const STOP_GRACE_PERIOD: Duration = Duration::from_secs(10);
const SMTP_GREETING_TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::test]
async fn fake_redis_satisfies_the_runtime_contract() -> Result<()> {
    let (_tempdir, paths) = contract_paths()?;
    let artifact_path = fake_artifact(
        &paths,
        "redis",
        "8.8",
        "bin/redis-server",
        Persona::RedisServer,
    )?;

    redis_contract(&paths, &artifact_path, "8.8").await
}

#[tokio::test]
#[ignore = "requires PV_E2E_REAL_ARTIFACTS=1 and PV_E2E_ARTIFACT_MANIFEST_URL"]
async fn real_redis_satisfies_the_runtime_contract() -> Result<()> {
    let Some(manifest_url) = real_artifact_manifest_url()? else {
        return Ok(());
    };
    let (_tempdir, paths) = contract_paths()?;
    let redis = install_real_artifact(&paths, manifest_url, &redis_adapter()?)?;

    redis_contract(
        &paths,
        redis.current_artifact_path(),
        redis.track().as_str(),
    )
    .await
}

#[tokio::test]
async fn fake_mailpit_satisfies_the_runtime_contract() -> Result<()> {
    let (_tempdir, paths) = contract_paths()?;
    let artifact_path = fake_artifact(&paths, "mailpit", "1", "bin/mailpit", Persona::Mailpit)?;

    mailpit_contract(
        &paths,
        &super::mailpit::MailpitRuntimeAdapter::new(),
        &artifact_path,
        "1",
    )
    .await
}

#[tokio::test]
#[ignore = "requires PV_E2E_REAL_ARTIFACTS=1 and PV_E2E_ARTIFACT_MANIFEST_URL"]
async fn real_mailpit_satisfies_the_runtime_contract() -> Result<()> {
    let Some(manifest_url) = real_artifact_manifest_url()? else {
        return Ok(());
    };
    let (_tempdir, paths) = contract_paths()?;
    let mailpit = install_real_artifact(&paths, manifest_url, &mailpit_adapter()?)?;

    mailpit_contract(
        &paths,
        &super::mailpit::MailpitRuntimeAdapter::new(),
        mailpit.current_artifact_path(),
        mailpit.track().as_str(),
    )
    .await
}

/// PV's test-only fake Mailpit adapter has no real binary, so its contract runs against the fake
/// alone.
#[tokio::test]
async fn pv_fake_mailpit_satisfies_the_fake_adapter_runtime_contract() -> Result<()> {
    let (_tempdir, paths) = contract_paths()?;
    let artifact_path = fake_artifact(
        &paths,
        "mailpit",
        "1.0",
        "bin/pv-fake-mailpit",
        Persona::PvFakeMailpit,
    )?;

    mailpit_contract(
        &paths,
        &super::fake::FakeMailpitRuntimeAdapter::new()?,
        &artifact_path,
        "1.0",
    )
    .await
}

/// PV's Redis runtime: start it from the adapter's rendered config and wait for its readiness
/// check, which runs the `redis` crate's connection handshake and `PING`, then stop it.
async fn redis_contract(paths: &PvPaths, artifact_path: &Utf8Path, track: &str) -> Result<()> {
    let [port] = available_ports()?;
    let adapter = super::redis::RedisRuntimeAdapter::new();

    run_contract(
        paths,
        &adapter,
        runtime_context(paths, "redis", track, artifact_path, [("redis", port)]),
        future::ready(Ok(())),
    )
    .await
}

/// PV's Mailpit runtime: once the adapter's dashboard readiness check passes, the SMTP port PV
/// hands to apps greets like an SMTP server.
async fn mailpit_contract(
    paths: &PvPaths,
    adapter: &dyn ManagedResourceRuntimeAdapter,
    artifact_path: &Utf8Path,
    track: &str,
) -> Result<()> {
    let [smtp_port, dashboard_port] = available_ports()?;

    run_contract(
        paths,
        adapter,
        runtime_context(
            paths,
            "mailpit",
            track,
            artifact_path,
            [("smtp", smtp_port), ("dashboard", dashboard_port)],
        ),
        smtp_greets(smtp_port),
    )
    .await
}

async fn smtp_greets(port: u16) -> Result<()> {
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await?;
    let mut greeting = [0; 4];
    tokio::time::timeout(SMTP_GREETING_TIMEOUT, stream.read_exact(&mut greeting)).await??;
    ensure!(
        &greeting == b"220 ",
        "SMTP port {port} greeted with {greeting:?}"
    );

    Ok(())
}

/// Starts and waits for the runtime the way reconciliation does, awaits `check` once it's ready,
/// then stops it within the grace period. A fixture guard registered before startup stops it on
/// every other exit path, and a cleanup failure is reported alongside the contract's own failure.
async fn run_contract(
    paths: &PvPaths,
    adapter: &dyn ManagedResourceRuntimeAdapter,
    context: ManagedResourceRuntimeContext,
    check: impl Future<Output = Result<()>>,
) -> Result<()> {
    let env = adapter.resource_env(&context)?;
    let context = ManagedResourceRuntimeContext { env, ..context };
    let mut runtimes = ManagedResourceFixtureGuard::new(paths);
    runtimes.register(&context.resource_name, &context.track);

    let outcome = start_wait_and_stop(paths, adapter, &context, check).await;
    match (outcome, runtimes.cleanup().await) {
        (Ok(()), cleanup) => cleanup,
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(cleanup_error)) => Err(anyhow!(
            "{error:#}; fixture cleanup also failed: {cleanup_error:#}"
        )),
    }
}

async fn start_wait_and_stop(
    paths: &PvPaths,
    adapter: &dyn ManagedResourceRuntimeAdapter,
    context: &ManagedResourceRuntimeContext,
    check: impl Future<Output = Result<()>>,
) -> Result<()> {
    let spec = adapter.build_process_spec(paths, context)?;
    let (log_path, pid_path, metadata_path) = (
        spec.log_path.clone(),
        spec.pid_path.clone(),
        spec.metadata_path.clone(),
    );
    adapter.prepare_runtime(paths, context).await?;
    let supervisor = ProcessSupervisor::new(paths.clone());
    let Some(pending) = start_or_adopt_runtime(
        &supervisor,
        spec,
        adapter.readiness(context)?,
        adapter_readiness_timeout(adapter),
        None,
    )
    .await?
    else {
        bail!("{} was not started", context.resource_name);
    };
    let ready = pending.wait().await;
    let checked = match &ready {
        Ok(()) => check.await,
        Err(_error) => Ok(()),
    };
    // A runtime that failed its readiness wait has been stopped and its records removed already.
    let stop_started = Instant::now();
    let stopped = match supervisor.adopt_recorded(&pid_path, &metadata_path) {
        Ok(Some(process)) => process.stop(STOP_GRACE_PERIOD).await,
        Ok(None) => Ok(()),
        Err(error) => Err(error),
    };
    let stopped_within = stop_started.elapsed();

    ready.with_context(|| runtime_log(&log_path))?;
    checked.with_context(|| runtime_log(&log_path))?;
    stopped?;
    ensure!(
        stopped_within < STOP_GRACE_PERIOD,
        "{} ignored SIGTERM for {stopped_within:?}",
        context.resource_name
    );

    Ok(())
}

/// Installs a fake as an artifact in the resource's directory, where the fixture guard expects a
/// runtime's command.
fn fake_artifact(
    paths: &PvPaths,
    resource_name: &str,
    track: &str,
    executable: &str,
    persona: Persona,
) -> Result<Utf8PathBuf> {
    let artifact_path = paths
        .resources()
        .join(resource_name)
        .join(track)
        .join("releases/fake");
    pv_fake::install(&artifact_path.join(executable), persona)?;

    Ok(artifact_path)
}

fn runtime_context<const PORTS: usize>(
    paths: &PvPaths,
    resource_name: &str,
    track: &str,
    artifact_path: &Utf8Path,
    ports: [(&str, u16); PORTS],
) -> ManagedResourceRuntimeContext {
    ManagedResourceRuntimeContext {
        resource_name: resource_name.to_owned(),
        track: track.to_owned(),
        artifact_path: artifact_path.to_owned(),
        data_dir: paths.resource_data_dir(resource_name, track),
        ports: ports
            .into_iter()
            .map(|(name, port)| (name.to_owned(), port))
            .collect::<BTreeMap<_, _>>(),
        env: BTreeMap::new(),
        postgres_preload_libraries: Vec::new(),
    }
}

fn install_real_artifact(
    paths: &PvPaths,
    manifest_url: String,
    adapter: &impl ResourceAdapter,
) -> Result<ManagedResourceInstall> {
    let commands = ManagedResourceCommands::new(paths.clone(), manifest_url, target_platform());

    Ok(commands.install(
        adapter,
        TrackSelector::Latest,
        &resources::UreqResourceHttpClient::new(),
    )?)
}

fn contract_paths() -> Result<(Utf8TempDir, PvPaths)> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;

    Ok((tempdir, paths))
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
