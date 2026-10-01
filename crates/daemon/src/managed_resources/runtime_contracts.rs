//! Contracts for the Managed Resource runtimes, driven through each runtime adapter the way
//! reconciliation does: the adapter's environment, process spec and readiness check, PV's
//! supervisor, and a stop within the grace period. Each contract runs against a `pv-fake`
//! persona, and against the real artifact when `PV_E2E_REAL_ARTIFACTS=1`, so the fakes can't drift
//! from what PV depends on.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, TcpListener};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use camino::Utf8Path;
use camino_tempfile::{Utf8TempDir, tempdir};
use pv_fake::Persona;
use resources::{ManagedResourceCommands, TargetPlatform, TrackSelector, redis_adapter};
use state::PvPaths;

use super::{
    ManagedResourceRuntimeAdapter, ManagedResourceRuntimeContext, adapter_readiness_timeout,
    wait_for_managed_resource_readiness,
};
use crate::ProcessSupervisor;

const STOP_GRACE_PERIOD: Duration = Duration::from_secs(10);

#[tokio::test]
async fn fake_redis_satisfies_the_runtime_contract() -> Result<()> {
    let (_tempdir, paths) = contract_paths()?;
    let artifact_path = paths.home().join("redis-release");
    pv_fake::install(
        &artifact_path.join("bin/redis-server"),
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
    let commands = ManagedResourceCommands::new(paths.clone(), manifest_url, target_platform());
    let redis = commands.install(
        &redis_adapter()?,
        TrackSelector::Latest,
        &resources::UreqResourceHttpClient::new(),
    )?;

    redis_contract(
        &paths,
        redis.current_artifact_path(),
        redis.track().as_str(),
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
    )
    .await
}

/// Starts the runtime the way reconciliation does, waits for the adapter's readiness check, and
/// stops it within the grace period.
async fn run_contract(
    paths: &PvPaths,
    adapter: &dyn ManagedResourceRuntimeAdapter,
    context: ManagedResourceRuntimeContext,
) -> Result<()> {
    let env = adapter.resource_env(&context)?;
    let context = ManagedResourceRuntimeContext { env, ..context };
    let spec = adapter.build_process_spec(paths, &context)?;
    let log_path = spec.log_path.clone();
    adapter.prepare_runtime(paths, &context).await?;
    let readiness = adapter.readiness(&context)?;
    let process = ProcessSupervisor::new(paths.clone()).start(spec).await?;
    let ready =
        wait_for_managed_resource_readiness(&readiness, adapter_readiness_timeout(adapter)).await;
    let stop_started = Instant::now();
    process.stop(STOP_GRACE_PERIOD).await?;
    let stopped_within = stop_started.elapsed();

    ready.with_context(|| runtime_log(&log_path))?;
    ensure!(
        stopped_within < STOP_GRACE_PERIOD,
        "{} ignored SIGTERM for {stopped_within:?}",
        context.resource_name
    );

    Ok(())
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
