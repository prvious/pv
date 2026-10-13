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
use aws_sdk_s3::error::{ProvideErrorMetadata, SdkError};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::{Utf8TempDir, tempdir};
use pv_fake::Persona;
use resources::{
    ManagedResourceCommands, ManagedResourceInstall, ResourceAdapter, TargetPlatform,
    TrackSelector, mailpit_adapter, mysql_adapter, postgres_adapter, redis_adapter, rustfs_adapter,
};
use sqlx::mysql::{MySqlConnectOptions, MySqlPool};
use sqlx::postgres::PgPool;
use state::{EnvContextValues, PvPaths};
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

use super::sql::{self, SqlAdminContext, SqlEngine};
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

#[tokio::test]
async fn fake_rustfs_satisfies_the_runtime_contract() -> Result<()> {
    let (_tempdir, paths) = contract_paths()?;
    let artifact_path = fake_artifact(&paths, "rustfs", "1", "bin/rustfs", Persona::Rustfs)?;

    rustfs_contract(&paths, &artifact_path, "1").await
}

#[tokio::test]
#[ignore = "requires PV_E2E_REAL_ARTIFACTS=1 and PV_E2E_ARTIFACT_MANIFEST_URL"]
async fn real_rustfs_satisfies_the_runtime_contract() -> Result<()> {
    let Some(manifest_url) = real_artifact_manifest_url()? else {
        return Ok(());
    };
    let (_tempdir, paths) = contract_paths()?;
    let rustfs = install_real_artifact(&paths, manifest_url, &rustfs_adapter()?)?;

    rustfs_contract(
        &paths,
        rustfs.current_artifact_path(),
        rustfs.track().as_str(),
    )
    .await
}

#[tokio::test]
async fn fake_postgres_satisfies_the_runtime_contract() -> Result<()> {
    let (_tempdir, paths) = contract_paths()?;
    let artifact_path = fake_artifact(&paths, "postgres", "18", "bin/postgres", Persona::Postgres)?;
    pv_fake::install(&artifact_path.join("bin/initdb"), Persona::Initdb)?;

    postgres_contract(&paths, &artifact_path, "18").await
}

#[tokio::test]
#[ignore = "requires PV_E2E_REAL_ARTIFACTS=1 and PV_E2E_ARTIFACT_MANIFEST_URL"]
async fn real_postgres_satisfies_the_runtime_contract() -> Result<()> {
    let Some(manifest_url) = real_artifact_manifest_url()? else {
        return Ok(());
    };
    let (_tempdir, paths) = contract_paths()?;
    let postgres = install_real_artifact(&paths, manifest_url, &postgres_adapter()?)?;

    postgres_contract(
        &paths,
        postgres.current_artifact_path(),
        postgres.track().as_str(),
    )
    .await
}

#[tokio::test]
#[ignore = "requires PV_E2E_REAL_ARTIFACTS=1 and PV_E2E_ARTIFACT_MANIFEST_URL"]
async fn real_postgres_forced_exit_keeps_recovery_records() -> Result<()> {
    let Some(manifest_url) = real_artifact_manifest_url()? else {
        return Ok(());
    };
    let (_tempdir, paths) = contract_paths()?;
    let postgres = install_real_artifact(&paths, manifest_url, &postgres_adapter()?)?;
    let track = postgres.track().as_str();
    let [port] = available_ports()?;
    let adapter = super::postgres::PostgresRuntimeAdapter::new();
    let mut context = runtime_context(
        &paths,
        "postgres",
        track,
        postgres.current_artifact_path(),
        [("postgres", port)],
    );
    context.env = adapter.resource_env(&context)?;
    let admin = SqlAdminContext {
        host: Ipv4Addr::LOCALHOST.to_string(),
        port,
        username: context.env.get("username").cloned().unwrap_or_default(),
        password: context.env.get("password").cloned().unwrap_or_default(),
    };
    let mut guard = ManagedResourceFixtureGuard::new(&paths)?;
    let outcome = async {
        adapter.prepare_runtime(&paths, &context).await?;
        let supervisor = ProcessSupervisor::new(paths.clone());
        let spec = adapter.build_process_spec(&paths, &context)?;
        let pending = start_or_adopt_runtime(&supervisor, spec.clone(), adapter.stop_signal(),
            adapter.readiness(&context)?, adapter_readiness_timeout(&adapter), None).await?
            .ok_or_else(|| anyhow!("Postgres was not started"))?;
        pending.wait().await?;
        let pool = PgPool::connect_with(sql::postgres_options(&admin)).await?;
        let mut connection = pool.acquire().await?;
        let backend_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *connection).await?;
        let backend_pid = u32::try_from(backend_pid)?;
        let backend_identity = platform::inspect_process_start_identity(backend_pid)?
            .ok_or_else(|| anyhow!("connected backend had no process identity"))?;
        let postmaster = supervisor.adopt_recorded( &spec.metadata_path)?
            .ok_or_else(|| anyhow!("Postgres ownership was not verified"))?;
        postmaster.kill_and_wait_for_test(Duration::from_secs(2))?;
        let stopped = crate::stop_recorded_runtimes(paths.clone()).await;
        ensure!(matches!(stopped, Err(crate::DaemonError::RuntimeStopFailures { ref failures })
            if failures.iter().any(|failure| matches!(failure.error(), crate::DaemonError::RuntimeCleanupUnproven { .. }))),
            "forced Postgres exit did not block cleanup: {stopped:?}");
        ensure!(!crate::recorded_monitor_subjects(&paths)?.is_empty());
        ensure!(state::fs::path_entry_exists(&spec.metadata_path)?);
        ensure!(state::fs::path_entry_exists(&spec.command)?);
        let query = tokio::time::timeout(Duration::from_secs(3),
            sqlx::query("SELECT 1").execute(&mut *connection)).await?;
        ensure!(query.is_err(), "backend connection survived postmaster death");
        // PostgreSQL's own parent-death handling must finish this connected
        // backend. Root-group disappearance alone is not our oracle.
        tokio::time::timeout(Duration::from_secs(5), async {
            while platform::inspect_process_start_identity(backend_pid)? == Some(backend_identity) {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Ok::<_, platform::PlatformError>(())
        }).await??;
        // The backend is gone too, so the monitor's held exit can be released by hand.
        crate::release_monitor(&paths, &format!("resources/postgres/{track}")).await?;
        Ok::<_, anyhow::Error>(())
    }.await;
    match (outcome, guard.cleanup().await) {
        (Ok(()), cleanup) => cleanup,
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(cleanup)) => Err(anyhow!(
            "{error:#}; fixture cleanup also failed: {cleanup:#}"
        )),
    }
}

#[tokio::test]
async fn fake_mysql_satisfies_the_runtime_contract() -> Result<()> {
    let (_tempdir, paths) = contract_paths()?;
    let artifact_path = fake_artifact(&paths, "mysql", "8.4", "bin/mysqld", Persona::Mysqld)?;

    mysql_contract(&paths, &artifact_path, "8.4").await
}

#[tokio::test]
#[ignore = "requires PV_E2E_REAL_ARTIFACTS=1 and PV_E2E_ARTIFACT_MANIFEST_URL"]
async fn real_mysql_satisfies_the_runtime_contract() -> Result<()> {
    let Some(manifest_url) = real_artifact_manifest_url()? else {
        return Ok(());
    };
    let (_tempdir, paths) = contract_paths()?;
    let mysql = install_real_artifact(&paths, manifest_url, &mysql_adapter()?)?;

    mysql_contract(
        &paths,
        mysql.current_artifact_path(),
        mysql.track().as_str(),
    )
    .await?;
    real_mysql_deletes_allocations(
        &paths,
        mysql.current_artifact_path(),
        mysql.track().as_str(),
    )
    .await
}

/// PV's Redis runtime: start it from the adapter's rendered config and wait for its readiness
/// check, which runs the `redis` crate's connection handshake and `PING`. Then PV's allocation
/// cleanup deletes one prefix's keys and nothing else, and PV stops it.
async fn redis_contract(paths: &PvPaths, artifact_path: &Utf8Path, track: &str) -> Result<()> {
    let [port] = available_ports()?;
    let adapter = super::redis::RedisRuntimeAdapter::new();
    let context = runtime_context(paths, "redis", track, artifact_path, [("redis", port)]);

    run_contract(
        paths,
        &adapter,
        context.clone(),
        redis_deletes_allocations(&adapter, &context, port),
    )
    .await
}

async fn redis_deletes_allocations(
    adapter: &dyn ManagedResourceRuntimeAdapter,
    context: &ManagedResourceRuntimeContext,
    port: u16,
) -> Result<()> {
    // More keys than one `SCAN` call looks at, so cleanup has to follow the cursor.
    let deleted = (0..1500)
        .map(|index| format!("acme.cache.{index}"))
        .collect::<Vec<_>>();
    // Another allocation, another Project, and the old prefix format all survive.
    let kept = [
        "acme.cache-jobs.1",
        "acme-feat.cache.1",
        "acme-cache-1",
        "acme.cachex",
    ];
    // Laravel keeps its cache in database 1, so cleanup covers every database.
    for database in [0, 1] {
        let mut connection = redis_connection(port, database).await?;
        let mut pipeline = redis::pipe();
        for key in deleted.iter().map(String::as_str).chain(kept) {
            pipeline.cmd("SET").arg(key).arg("1").ignore();
        }
        pipeline.query_async::<()>(&mut connection).await?;
    }

    // Twice, as a retried cleanup runs again after the keys are gone.
    for _attempt in 0..2 {
        adapter
            .delete_allocation(context, &context.env, "acme.cache.")
            .await?;
    }

    for database in [0, 1] {
        let mut connection = redis_connection(port, database).await?;
        let deleted_left: usize = redis::cmd("EXISTS")
            .arg(&deleted)
            .query_async(&mut connection)
            .await?;
        let kept_left: usize = redis::cmd("EXISTS")
            .arg(&kept)
            .query_async(&mut connection)
            .await?;
        ensure!(
            (deleted_left, kept_left) == (0, kept.len()),
            "database {database} kept {deleted_left} deleted and {kept_left} kept keys"
        );
    }

    Ok(())
}

async fn redis_connection(port: u16, database: u8) -> Result<redis::aio::MultiplexedConnection> {
    let client = redis::Client::open(format!("redis://127.0.0.1:{port}/{database}"))?;

    Ok(client.get_multiplexed_async_connection().await?)
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

/// PV's RustFS runtime, started with the keys PV generates: once `/health` answers, PV's allocation
/// step works, PV's allocation cleanup empties and deletes one bucket and nothing else, and the S3
/// operations the daemon tests use to inspect buckets behave as RustFS's do.
async fn rustfs_contract(paths: &PvPaths, artifact_path: &Utf8Path, track: &str) -> Result<()> {
    let [api_port, console_port] = available_ports()?;
    let adapter = super::rustfs::RustfsRuntimeAdapter;
    let mut context = runtime_context(
        paths,
        "rustfs",
        track,
        artifact_path,
        [("api", api_port), ("console", console_port)],
    );
    // Fix the generated keys up front, so the check signs with the keys the runtime gets.
    context.env = adapter.resource_env(&context)?;
    let env = context.env.clone();

    run_contract(paths, &adapter, context.clone(), async {
        rustfs_serves_allocations(env.clone()).await?;
        rustfs_deletes_allocations(&adapter, &context, &env).await
    })
    .await
}

async fn rustfs_deletes_allocations(
    adapter: &dyn ManagedResourceRuntimeAdapter,
    context: &ManagedResourceRuntimeContext,
    env: &EnvContextValues,
) -> Result<()> {
    let (deleted, kept) = ("acme-uploads", "acme-uploads-kept");
    let keys = ["a.txt", "nested/b.txt"];
    let client = super::rustfs::s3_client(env)?;
    for bucket in [deleted, kept] {
        super::rustfs::create_bucket(&client, bucket).await?;
        for key in keys {
            client
                .put_object()
                .bucket(bucket)
                .key(key)
                .body(key.as_bytes().to_vec().into())
                .send()
                .await?;
        }
    }

    // Twice, as a retried cleanup runs again after the bucket is gone.
    for _attempt in 0..2 {
        adapter.delete_allocation(context, env, deleted).await?;
    }

    let deleted_bucket = s3_error_code(client.head_bucket().bucket(deleted).send().await);
    let kept_objects = client
        .list_objects_v2()
        .bucket(kept)
        .send()
        .await?
        .contents()
        .iter()
        .filter_map(|object| object.key().map(str::to_owned))
        .collect::<Vec<_>>();
    ensure!(
        deleted_bucket.as_deref() == Some("NotFound"),
        "the deleted bucket answered {deleted_bucket:?}"
    );
    ensure!(
        kept_objects == keys,
        "the kept bucket holds {kept_objects:?}"
    );

    Ok(())
}

async fn rustfs_serves_allocations(env: EnvContextValues) -> Result<()> {
    let bucket = "pv-contract";
    let client = super::rustfs::s3_client(&env)?;
    // PV's allocation step, run twice as reconciliation does for a Ready allocation.
    for _attempt in 0..2 {
        super::rustfs::create_bucket(&client, bucket).await?;
        super::rustfs::verify_object_operations(&env, bucket).await?;
    }
    let probe = client
        .get_object()
        .bucket(bucket)
        .key(super::rustfs::PROBE_OBJECT)
        .send()
        .await?
        .body
        .collect()
        .await?
        .into_bytes();
    let outcomes = [
        s3_error_code(client.delete_bucket().bucket(bucket).send().await),
        s3_error_code(
            client
                .delete_object()
                .bucket(bucket)
                .key(super::rustfs::PROBE_OBJECT)
                .send()
                .await,
        ),
        s3_error_code(client.delete_bucket().bucket(bucket).send().await),
        s3_error_code(client.head_bucket().bucket(bucket).send().await),
    ];
    let mut wrong_keys = env.clone();
    wrong_keys.insert("secret_key".to_owned(), "not-the-secret-key".to_owned());
    let rejected = s3_error_code(
        super::rustfs::s3_client(&wrong_keys)?
            .create_bucket()
            .bucket("pv-rejected")
            .send()
            .await,
    );

    ensure!(
        probe == super::rustfs::PROBE_CONTENT.as_bytes(),
        "probe read back as {probe:?}"
    );
    ensure!(
        outcomes
            == [
                Some("BucketNotEmpty".to_owned()),
                None,
                None,
                Some("NotFound".to_owned())
            ],
        "bucket operations failed with {outcomes:?}"
    );
    ensure!(
        rejected.as_deref() == Some("SignatureDoesNotMatch"),
        "wrong keys failed with {rejected:?}"
    );

    Ok(())
}

/// PV's Postgres runtime: preparation runs `initdb` with PV's arguments, the readiness check signs
/// in with SCRAM, PV's allocation step creates a database and then finds it, PV's allocation
/// cleanup drops one database even while an app is connected to it, and PV's stop finishes within
/// the grace period while a client is still connected (#391).
async fn postgres_contract(paths: &PvPaths, artifact_path: &Utf8Path, track: &str) -> Result<()> {
    let [port] = available_ports()?;
    let adapter = super::postgres::PostgresRuntimeAdapter::new();
    let mut context = runtime_context(
        paths,
        "postgres",
        track,
        artifact_path,
        [("postgres", port)],
    );
    // Fix the generated password up front, so the check signs in with the one `initdb` gets.
    context.env = adapter.resource_env(&context)?;
    let admin = SqlAdminContext {
        host: Ipv4Addr::LOCALHOST.to_string(),
        port,
        username: context.env.get("username").cloned().unwrap_or_default(),
        password: context.env.get("password").cloned().unwrap_or_default(),
    };

    run_contract(
        paths,
        &adapter,
        context.clone(),
        postgres_serves_allocations(&adapter, &context, admin),
    )
    .await
}

/// Returns a connected client for the contract to hold through the stop.
async fn postgres_serves_allocations(
    adapter: &dyn ManagedResourceRuntimeAdapter,
    context: &ManagedResourceRuntimeContext,
    admin: SqlAdminContext,
) -> Result<PgPool> {
    let database = "pv_contract";
    // PV's allocation step, run twice as reconciliation does for a Ready allocation. The second
    // run creates nothing only if the first database is found.
    for _attempt in 0..2 {
        sql::create_database_if_missing(&admin, SqlEngine::Postgres, database).await?;
    }
    let pool = PgPool::connect_with(sql::postgres_options(&admin)).await?;
    let duplicate = sql_state(
        sqlx::query("CREATE DATABASE \"pv_contract\"")
            .execute(&pool)
            .await,
    );
    let wrong_password = SqlAdminContext {
        password: "not-the-password".to_owned(),
        ..admin.clone()
    };
    let refused = sql_state(PgPool::connect_with(sql::postgres_options(&wrong_password)).await);

    ensure!(
        duplicate.as_deref() == Some("42P04"),
        "creating {database} again failed with {duplicate:?}"
    );
    ensure!(
        refused.as_deref() == Some("28P01"),
        "a wrong password failed with {refused:?}"
    );

    let kept = "pv_contract_kept";
    sql::create_database_if_missing(&admin, SqlEngine::Postgres, kept).await?;
    // An app connected to the database doesn't stop cleanup.
    let app = PgPool::connect_with(sql::postgres_options(&admin).database(database)).await?;
    // Twice, as a retried cleanup runs again after the database is gone.
    for _attempt in 0..2 {
        adapter
            .delete_allocation(context, &context.env, database)
            .await?;
    }
    drop(app);
    let mut left = Vec::new();
    for name in [database, kept] {
        if sqlx::query("SELECT 1 FROM pg_database WHERE datname = $1")
            .bind(name)
            .fetch_optional(&pool)
            .await?
            .is_some()
        {
            left.push(name);
        }
    }
    ensure!(left == [kept], "cleanup left databases {left:?}");

    Ok(pool)
}

/// PV's MySQL runtime: preparation initializes the data directory, and the runtime starts with
/// PV's arguments and init file, accepts connections on its port, and stops on SIGTERM. The fake
/// speaks no MySQL protocol, so the readiness check is the daemon tests' TCP connect; PV's SQL
/// client meets real MySQL in `tests/real_artifact_resource_matrix.rs`.
async fn mysql_contract(paths: &PvPaths, artifact_path: &Utf8Path, track: &str) -> Result<()> {
    let [port] = available_ports()?;
    let adapter = super::mysql::MysqlRuntimeAdapter::with_recording_admin(
        super::mysql::RecordingMysqlAdmin::default(),
    )?;

    run_contract(
        paths,
        &adapter,
        runtime_context(paths, "mysql", track, artifact_path, [("mysql", port)]),
        future::ready(Ok(())),
    )
    .await
}

/// Real MySQL with PV's own SQL client, which the fake can't speak: PV's allocation cleanup drops
/// one database and leaves another.
async fn real_mysql_deletes_allocations(
    paths: &PvPaths,
    artifact_path: &Utf8Path,
    track: &str,
) -> Result<()> {
    let [port] = available_ports()?;
    let adapter = super::mysql::MysqlRuntimeAdapter::new();
    let mut context = runtime_context(paths, "mysql", track, artifact_path, [("mysql", port)]);
    // Fix the generated password up front, so the check signs in with the one MySQL gets.
    context.env = adapter.resource_env(&context)?;
    let admin = SqlAdminContext {
        host: Ipv4Addr::LOCALHOST.to_string(),
        port,
        username: context.env.get("username").cloned().unwrap_or_default(),
        password: context.env.get("password").cloned().unwrap_or_default(),
    };

    run_contract(paths, &adapter, context.clone(), async {
        let (deleted, kept) = ("pv_contract", "pv_contract_kept");
        for database in [deleted, kept] {
            sql::create_database_if_missing(&admin, SqlEngine::Mysql, database).await?;
        }
        // Twice, as a retried cleanup runs again after the database is gone.
        for _attempt in 0..2 {
            adapter
                .delete_allocation(&context, &context.env, deleted)
                .await?;
        }
        let pool = MySqlPool::connect_with(
            MySqlConnectOptions::new()
                .host(&admin.host)
                .port(admin.port)
                .username(&admin.username)
                .password(&admin.password),
        )
        .await?;
        let mut left = Vec::new();
        for database in [deleted, kept] {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM information_schema.schemata WHERE schema_name = ?",
            )
            .bind(database)
            .fetch_one(&pool)
            .await?;
            if count > 0 {
                left.push(database);
            }
        }
        ensure!(left == [kept], "cleanup left databases {left:?}");

        Ok(())
    })
    .await
}

/// The SQLSTATE a statement failed with, or `None` if it succeeded.
fn sql_state<Output>(result: Result<Output, sqlx::Error>) -> Option<String> {
    result.err().map(|error| match error.as_database_error() {
        Some(database_error) => database_error
            .code()
            .map_or_else(|| "<no code>".to_owned(), |code| code.into_owned()),
        None => format!("<{error}>"),
    })
}

/// The S3 error code a request failed with, or `None` if it succeeded.
fn s3_error_code<Output, Error: ProvideErrorMetadata, Response>(
    result: Result<Output, SdkError<Error, Response>>,
) -> Option<String> {
    result
        .err()
        .map(|error| error.code().unwrap_or("<no code>").to_owned())
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
/// then stops it within the grace period with the adapter's stop signal. Whatever `check` returns,
/// such as an open client, stays alive through the stop. A fixture guard registered before startup
/// stops the runtime on every other exit path, and a cleanup failure is reported alongside the
/// contract's own failure.
async fn run_contract<Held>(
    paths: &PvPaths,
    adapter: &dyn ManagedResourceRuntimeAdapter,
    context: ManagedResourceRuntimeContext,
    check: impl Future<Output = Result<Held>>,
) -> Result<()> {
    let env = adapter.resource_env(&context)?;
    let context = ManagedResourceRuntimeContext { env, ..context };
    let mut runtimes = ManagedResourceFixtureGuard::new(paths)?;

    let outcome = start_wait_and_stop(paths, adapter, &context, check).await;
    match (outcome, runtimes.cleanup().await) {
        (Ok(()), cleanup) => cleanup,
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(cleanup_error)) => Err(anyhow!(
            "{error:#}; fixture cleanup also failed: {cleanup_error:#}"
        )),
    }
}

async fn start_wait_and_stop<Held>(
    paths: &PvPaths,
    adapter: &dyn ManagedResourceRuntimeAdapter,
    context: &ManagedResourceRuntimeContext,
    check: impl Future<Output = Result<Held>>,
) -> Result<()> {
    let spec = adapter.build_process_spec(paths, context)?;
    let (log_path, metadata_path) = (spec.log_path.clone(), spec.metadata_path.clone());
    adapter.prepare_runtime(paths, context).await?;
    let supervisor = ProcessSupervisor::new(paths.clone());
    let Some(pending) = start_or_adopt_runtime(
        &supervisor,
        spec,
        adapter.stop_signal(),
        adapter.readiness(context)?,
        adapter_readiness_timeout(adapter),
        None,
    )
    .await?
    else {
        bail!("{} was not started", context.resource_name);
    };
    let checked = match pending.wait().await {
        Ok(()) => check.await,
        Err(error) => Err(error.into()),
    };
    // A runtime that failed its readiness wait has been stopped and its records removed already.
    let stop_started = Instant::now();
    let stopped = crate::stop_recorded_runtimes(paths.clone()).await;
    let stopped_within = stop_started.elapsed();

    // Released only now, so whatever the check held stayed open through the stop.
    checked.with_context(|| runtime_log(&log_path))?;
    stopped?;
    ensure!(
        !state::fs::path_entry_exists(&metadata_path)?,
        "runtime metadata remains after stop"
    );
    ensure!(
        stopped_within < STOP_GRACE_PERIOD,
        "{} ignored its stop signal ({:?}) for {stopped_within:?}",
        context.resource_name,
        adapter.stop_signal()
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
    pv_fake::install_monitor(&paths)?;

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
