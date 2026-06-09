use std::ffi::OsString;
use std::sync::{Mutex, MutexGuard};

use anyhow::{Result, anyhow};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::{Settings, assert_debug_snapshot};
use state::{
    Database, LinkProjectInput, ProjectManagedResourceInput, ProjectRecord, PvPaths,
    ResourceAllocationInput,
};

use daemon::DaemonError;

// The SQL module is daemon-local; include it here so tests do not make it public.
#[path = "../src/managed_resources/sql.rs"]
mod sql;

use sql::{
    RecordingSqlAdmin, SqlAdminContext, SqlAllocationContext, SqlAllocationRequest, SqlEngine,
    ensure_database_allocation_for_test, postgres_options, sql_allocation_env, sql_resource_env,
};

static PG_ENV_LOCK: Mutex<()> = Mutex::new(());

#[tokio::test]
async fn sql_foundation_creates_database_and_marks_allocation_ready() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"mysql:
  version: "8.0"
  allocations:
    app-db: {}
"#,
    )?;
    seed_desired_sql_allocation(
        &paths,
        &project,
        "mysql",
        "8.0",
        "app-db",
        "acme_test_app_db",
    )?;
    let mut database = Database::open(&paths)?;
    let mut admin = RecordingSqlAdmin::default();
    let context = sql_admin_context();

    ensure_database_allocation_for_test(
        &mut database,
        &mut admin,
        SqlAllocationRequest {
            project_id: &project.id,
            resource_name: "mysql",
            track: "8.0",
            allocation_name: "app-db",
            engine: SqlEngine::Mysql,
            context: &context,
        },
    )
    .await?;

    assert_sql_snapshot(
        "sql_foundation_creates_database_and_marks_allocation_ready",
        (
            sql_resource_env(&context, SqlEngine::Mysql),
            admin.operations(),
            database.resource_allocations(&project.id, "mysql")?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn sql_foundation_creates_postgres_database_and_marks_allocation_ready() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "api.acme.test",
        r#"postgres:
  version: "16"
  allocations:
    analytics: {}
"#,
    )?;
    seed_desired_sql_allocation(
        &paths,
        &project,
        "postgres",
        "16",
        "analytics",
        "api_acme_test_analytics",
    )?;
    let mut database = Database::open(&paths)?;
    let mut admin = RecordingSqlAdmin::default();
    let context = postgres_admin_context();

    ensure_database_allocation_for_test(
        &mut database,
        &mut admin,
        SqlAllocationRequest {
            project_id: &project.id,
            resource_name: "postgres",
            track: "16",
            allocation_name: "analytics",
            engine: SqlEngine::Postgres,
            context: &context,
        },
    )
    .await?;

    assert_sql_snapshot(
        "sql_foundation_creates_postgres_database_and_marks_allocation_ready",
        (
            sql_resource_env(&context, SqlEngine::Postgres),
            admin.operations(),
            database.resource_allocations(&project.id, "postgres")?,
        ),
    )?;

    Ok(())
}

#[tokio::test]
async fn sql_foundation_verifies_already_ready_allocation_without_rewriting_state() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"mysql:
  version: "8.0"
  allocations:
    app-db: {}
"#,
    )?;
    seed_desired_sql_allocation(
        &paths,
        &project,
        "mysql",
        "8.0",
        "app-db",
        "acme_test_app_db",
    )?;
    let mut database = Database::open(&paths)?;
    let context = sql_admin_context();
    database.mark_resource_allocation_ready(
        &project.id,
        "mysql",
        "8.0",
        "app-db",
        &sql_allocation_env(
            &allocation_context(&context, "acme_test_app_db"),
            SqlEngine::Mysql,
        ),
    )?;
    let before = database.resource_allocations(&project.id, "mysql")?;
    let mut admin = RecordingSqlAdmin::default();

    ensure_database_allocation_for_test(
        &mut database,
        &mut admin,
        SqlAllocationRequest {
            project_id: &project.id,
            resource_name: "mysql",
            track: "8.0",
            allocation_name: "app-db",
            engine: SqlEngine::Mysql,
            context: &context,
        },
    )
    .await?;

    let after = database.resource_allocations(&project.id, "mysql")?;
    if before != after {
        return Err(anyhow!(
            "ready allocation state changed: before={before:#?} after={after:#?}"
        ));
    }
    assert_sql_snapshot(
        "sql_foundation_verifies_already_ready_allocation_without_rewriting_state",
        (admin.operations(), after),
    )?;

    Ok(())
}

#[tokio::test]
async fn sql_foundation_refreshes_ready_allocation_env_when_context_changes() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project = link_project(
        &paths,
        &tempdir.path().join("project"),
        "acme.test",
        r#"mysql:
  version: "8.0"
  allocations:
    app-db: {}
"#,
    )?;
    seed_desired_sql_allocation(
        &paths,
        &project,
        "mysql",
        "8.0",
        "app-db",
        "acme_test_app_db",
    )?;
    let mut database = Database::open(&paths)?;
    let old_context = SqlAdminContext {
        port: 19_001,
        ..sql_admin_context()
    };
    let new_context = SqlAdminContext {
        port: 19_002,
        ..sql_admin_context()
    };
    database.mark_resource_allocation_ready(
        &project.id,
        "mysql",
        "8.0",
        "app-db",
        &sql_allocation_env(
            &allocation_context(&old_context, "acme_test_app_db"),
            SqlEngine::Mysql,
        ),
    )?;
    let mut admin = RecordingSqlAdmin::default();

    ensure_database_allocation_for_test(
        &mut database,
        &mut admin,
        SqlAllocationRequest {
            project_id: &project.id,
            resource_name: "mysql",
            track: "8.0",
            allocation_name: "app-db",
            engine: SqlEngine::Mysql,
            context: &new_context,
        },
    )
    .await?;

    let allocations = database.resource_allocations(&project.id, "mysql")?;
    let expected_env = sql_allocation_env(
        &allocation_context(&new_context, "acme_test_app_db"),
        SqlEngine::Mysql,
    );
    assert_eq!(
        allocations[0].env, expected_env,
        "expected Ready allocation env to refresh after SQL runtime context changed"
    );
    assert_sql_snapshot(
        "sql_foundation_refreshes_ready_allocation_env_when_context_changes",
        (admin.operations(), allocations),
    )?;

    Ok(())
}

#[test]
fn sql_foundation_postgres_admin_options_ignore_ambient_ssl_env() -> Result<()> {
    let _env = ScopedPgEnv::with([
        ("PGSSLMODE", "require"),
        ("PGSSLROOTCERT", "/tmp/hostile-root.pem"),
        ("PGSSLCERT", "/tmp/hostile-client.pem"),
        ("PGSSLKEY", "/tmp/hostile-client.key"),
        ("PGAPPNAME", "hostile-app"),
    ]);
    let context = postgres_admin_context();

    let options = postgres_options(&context);

    assert_eq!(
        format!("{:?}", options.get_ssl_mode()),
        "Disable",
        "expected PV Postgres admin options to force local non-TLS connections"
    );
    assert_eq!(
        options.get_application_name(),
        Some("pv"),
        "expected PV Postgres admin options to use a PV-owned application name"
    );

    Ok(())
}

#[tokio::test]
async fn sql_foundation_rejects_unsafe_database_identifier_before_connecting() -> Result<()> {
    let result =
        sql::create_database_if_missing(&sql_admin_context(), SqlEngine::Mysql, "bad-name").await;

    assert_sql_snapshot(
        "sql_foundation_rejects_unsafe_database_identifier_before_connecting",
        format!("{result:#?}"),
    )?;

    Ok(())
}

#[test]
fn sql_foundation_percent_encodes_url_userinfo() -> Result<()> {
    let context = SqlAdminContext {
        host: "127.0.0.1".to_string(),
        port: 15432,
        username: "admin@local:dev/%#".to_string(),
        password: "pa:ss/wo@rd%#".to_string(),
    };
    let allocation = allocation_context(&context, "acme_test_app_db");

    assert_sql_snapshot(
        "sql_foundation_percent_encodes_url_userinfo",
        (
            sql_resource_env(&context, SqlEngine::Mysql),
            sql_allocation_env(&allocation, SqlEngine::Mysql),
            sql_resource_env(&context, SqlEngine::Postgres),
            sql_allocation_env(&allocation, SqlEngine::Postgres),
        ),
    )?;

    Ok(())
}

fn link_project(
    paths: &PvPaths,
    project_path: &Utf8Path,
    primary_hostname: &str,
    config_source: &str,
) -> Result<ProjectRecord> {
    let config_path = project_path.join("pv.yml");

    state::fs::write_sensitive_file(&config_path, config_source)?;

    let mut database = Database::open(paths)?;
    let result = database.link_project(LinkProjectInput {
        path: project_path.to_path_buf(),
        original_path: project_path.to_path_buf(),
        primary_hostname: primary_hostname.to_string(),
        config_path,
        desired_php_track: None,
        additional_hostnames: Vec::new(),
    })?;

    Ok(result.project)
}

fn seed_desired_sql_allocation(
    paths: &PvPaths,
    project: &ProjectRecord,
    resource_name: &str,
    track: &str,
    allocation_name: &str,
    generated_name: &str,
) -> Result<()> {
    let mut database = Database::open(paths)?;

    database.replace_project_managed_resources(
        &project.id,
        &[ProjectManagedResourceInput {
            resource_name: resource_name.to_string(),
            track: track.to_string(),
        }],
    )?;
    database.replace_project_resource_allocations(
        &project.id,
        resource_name,
        track,
        &[ResourceAllocationInput {
            allocation_name: allocation_name.to_string(),
            generated_name: generated_name.to_string(),
        }],
    )?;

    Ok(())
}

fn sql_admin_context() -> SqlAdminContext {
    SqlAdminContext {
        host: "127.0.0.1".to_string(),
        port: 3306,
        username: "root".to_string(),
        password: "secret".to_string(),
    }
}

fn postgres_admin_context() -> SqlAdminContext {
    SqlAdminContext {
        host: "127.0.0.1".to_string(),
        port: 5432,
        username: "postgres".to_string(),
        password: "pg-secret".to_string(),
    }
}

fn allocation_context(context: &SqlAdminContext, database: &str) -> SqlAllocationContext {
    SqlAllocationContext {
        database: database.to_string(),
        host: context.host.clone(),
        port: context.port,
        username: context.username.clone(),
        password: context.password.clone(),
    }
}

struct ScopedPgEnv {
    _guard: MutexGuard<'static, ()>,
    saved: Vec<(&'static str, Option<OsString>)>,
}

impl ScopedPgEnv {
    #[expect(
        clippy::disallowed_methods,
        reason = "SQL foundation regression tests explicitly control PG* env inputs"
    )]
    fn with<const N: usize>(vars: [(&'static str, &'static str); N]) -> Self {
        let guard = pg_env_guard();
        let keys = [
            "PGSSLMODE",
            "PGSSLROOTCERT",
            "PGSSLCERT",
            "PGSSLKEY",
            "PGAPPNAME",
        ];
        let saved = keys
            .into_iter()
            .map(|key| (key, std::env::var_os(key)))
            .collect::<Vec<_>>();

        for key in keys {
            // SAFETY: This test helper serializes all PG* environment mutations in this
            // test binary with PG_ENV_LOCK and restores the previous values before the
            // guard is released.
            unsafe {
                std::env::remove_var(key);
            }
        }
        for (key, value) in vars {
            // SAFETY: This test helper serializes all PG* environment mutations in this
            // test binary with PG_ENV_LOCK and restores the previous values before the
            // guard is released.
            unsafe {
                std::env::set_var(key, value);
            }
        }

        Self {
            _guard: guard,
            saved,
        }
    }
}

impl Drop for ScopedPgEnv {
    #[expect(
        clippy::disallowed_methods,
        reason = "SQL foundation regression tests explicitly restore PG* env inputs"
    )]
    fn drop(&mut self) {
        for (key, value) in &self.saved {
            match value {
                Some(value) => {
                    // SAFETY: ScopedPgEnv holds PG_ENV_LOCK while restoring the values
                    // that were saved before this test's PG* environment mutation.
                    unsafe {
                        std::env::set_var(key, value);
                    }
                }
                None => {
                    // SAFETY: ScopedPgEnv holds PG_ENV_LOCK while restoring the values
                    // that were saved before this test's PG* environment mutation.
                    unsafe {
                        std::env::remove_var(key);
                    }
                }
            }
        }
    }
}

fn pg_env_guard() -> MutexGuard<'static, ()> {
    match PG_ENV_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn assert_sql_snapshot(name: &'static str, snapshot: impl std::fmt::Debug) -> Result<()> {
    let mut settings = Settings::clone_current();

    settings.add_filter(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", "<timestamp>");
    settings.add_filter(
        r#"project_id: "[a-z0-9]{10}""#,
        r#"project_id: "<project_id>""#,
    );
    settings.bind(|| {
        assert_debug_snapshot!(name, snapshot);
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}
