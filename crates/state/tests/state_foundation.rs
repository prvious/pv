use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::{Settings, assert_debug_snapshot};
use rusqlite::{Connection, params};
use state::testing::Migration;
use state::{
    Database, EnvContextValues, GATEWAY_HTTP_PREFERRED_PORT, GATEWAY_HTTPS_PREFERRED_PORT,
    GatewayPort, JobStatus, ManagedResourceDesiredState, ManagedResourceTrackInstallInput,
    ManagedResourceTrackRemovalInput, PortOwner, PortRequest, ProjectEnvObservedStatus,
    ProjectEnvObservedWarningInput, ProjectManagedResourceInput, ProjectRecord, PvPaths,
    RUNTIME_PORT_FALLBACK_END, RUNTIME_PORT_FALLBACK_START, ResourceAllocationInput,
    RuntimeObservedStatus, RuntimeSubject, StateError,
};

#[test]
fn paths_are_derived_from_an_injected_home() -> Result<()> {
    let paths = PvPaths::for_home(Utf8Path::new("/tmp/pv-test-home"));

    assert_debug_snapshot!(paths.summary());

    Ok(())
}

#[test]
fn ca_paths_are_derived_from_an_injected_home() {
    let paths = PvPaths::for_home(Utf8Path::new("/tmp/pv-test-home"));

    assert_eq!(
        paths.ca_certificate().as_str(),
        "/tmp/pv-test-home/.pv/certificates/ca.pem"
    );
    assert_eq!(
        paths.ca_private_key().as_str(),
        "/tmp/pv-test-home/.pv/certificates/ca-key.pem"
    );
}

#[test]
fn pv_paths_include_prepared_pf_artifacts() {
    let paths = PvPaths::for_home("/Users/alice");

    assert_eq!(
        paths.pf_anchor_config().as_str(),
        "/Users/alice/.pv/config/pf/com.prvious.pv"
    );
    assert_eq!(
        paths.pf_conf_reference_config().as_str(),
        "/Users/alice/.pv/config/pf/pf.conf"
    );
}

#[test]
fn pv_paths_include_gateway_and_worker_runtime_artifacts() {
    let paths = PvPaths::for_home("/Users/alice");

    assert_eq!(
        paths.gateway_root_config().as_str(),
        "/Users/alice/.pv/config/gateway/Caddyfile"
    );
    assert_eq!(
        paths.gateway_projects_config_dir().as_str(),
        "/Users/alice/.pv/config/gateway/projects"
    );
    assert_eq!(
        paths.worker_root_config("8.4").as_str(),
        "/Users/alice/.pv/config/workers/php-8.4/Caddyfile"
    );
    assert_eq!(
        paths.worker_projects_config_dir("8.4").as_str(),
        "/Users/alice/.pv/config/workers/php-8.4/projects"
    );
    assert_eq!(
        paths.gateway_log().as_str(),
        "/Users/alice/.pv/logs/gateway/gateway.log"
    );
    assert_eq!(
        paths.worker_log("8.4").as_str(),
        "/Users/alice/.pv/logs/workers/php-8.4.log"
    );
    assert_eq!(
        paths.gateway_pid().as_str(),
        "/Users/alice/.pv/run/gateway.pid"
    );
    assert_eq!(
        paths.gateway_runtime_metadata().as_str(),
        "/Users/alice/.pv/run/gateway.json"
    );
    assert_eq!(
        paths.worker_pid("8.4").as_str(),
        "/Users/alice/.pv/run/workers/php-8.4.pid"
    );
    assert_eq!(
        paths.worker_runtime_metadata("8.4").as_str(),
        "/Users/alice/.pv/run/workers/php-8.4.json"
    );
    assert_eq!(
        paths.resource_runtime_config("mailpit", "1.0").as_str(),
        "/Users/alice/.pv/config/resources/mailpit-1.0.json"
    );
    assert_eq!(
        paths.resource_log("mailpit", "1.0").as_str(),
        "/Users/alice/.pv/logs/resources/mailpit-1.0.log"
    );
    assert_eq!(
        paths.resource_pid("mailpit", "1.0").as_str(),
        "/Users/alice/.pv/run/resources/mailpit-1.0.pid"
    );
    assert_eq!(
        paths.resource_runtime_metadata("mailpit", "1.0").as_str(),
        "/Users/alice/.pv/run/resources/mailpit-1.0.json"
    );
    assert_eq!(
        paths.resource_data_dir("mailpit", "1.0").as_str(),
        "/Users/alice/.pv/resources/mailpit/1.0/data"
    );
}

#[test]
fn layout_creates_expected_directories_with_user_only_modes() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));

    state::fs::ensure_layout(&paths)?;

    assert_debug_snapshot!(state::fs::inspect_layout(&paths)?);

    Ok(())
}

#[test]
fn database_runs_migrations_and_exposes_core_schema() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;

    let database = Database::open(&paths)?;

    assert_debug_snapshot!(database.inspect()?);

    Ok(())
}

#[test]
fn database_files_are_restricted_to_the_current_user() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;

    let _database = Database::open(&paths)?;

    assert_debug_snapshot!(state::fs::inspect_database_files(&paths)?);

    Ok(())
}

#[test]
fn migration_backups_are_created_only_when_pending_migrations_exist() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;

    let first_migration = [Migration::new(
        1,
        "first",
        "CREATE TABLE first_table (id TEXT PRIMARY KEY);",
    )];
    state::testing::open_with_migrations(&paths, &first_migration)?;
    with_normalized_backup_names(|| {
        assert_debug_snapshot!("after_first_open", state::fs::migration_backups(&paths)?);
        Ok::<(), anyhow::Error>(())
    })?;

    let second_migration = [
        Migration::new(
            1,
            "first",
            "CREATE TABLE first_table (id TEXT PRIMARY KEY);",
        ),
        Migration::new(
            2,
            "second",
            "CREATE TABLE second_table (id TEXT PRIMARY KEY);",
        ),
    ];
    state::testing::open_with_migrations(&paths, &second_migration)?;

    with_normalized_backup_names(|| {
        assert_debug_snapshot!("after_second_open", state::fs::migration_backups(&paths)?);
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn migration_backup_includes_committed_wal_pages() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    let first_migration = [Migration::new(
        1,
        "first",
        "CREATE TABLE first_table (id TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )];
    let mut database = state::testing::open_with_migrations(&paths, &first_migration)?;
    state::testing::transaction(&mut database, |transaction| {
        transaction.execute(
            "INSERT INTO first_table (id, value) VALUES (?1, ?2)",
            params!["row_1", "from_wal"],
        )?;

        Ok(())
    })?;

    let second_migration = [
        Migration::new(
            1,
            "first",
            "CREATE TABLE first_table (id TEXT PRIMARY KEY, value TEXT NOT NULL);",
        ),
        Migration::new(
            2,
            "second",
            "CREATE TABLE second_table (id TEXT PRIMARY KEY);",
        ),
    ];
    state::testing::open_with_migrations(&paths, &second_migration)?;

    let backup_names = state::fs::migration_backups(&paths)?;
    let backup_name = backup_names
        .first()
        .ok_or_else(|| anyhow!("missing migration backup"))?;
    let backup_connection = Connection::open(paths.root().join(backup_name))?;
    let backed_up_rows = backup_connection.query_row(
        "SELECT COUNT(*) FROM first_table WHERE value = ?1",
        params!["from_wal"],
        |row| row.get::<_, i64>(0),
    )?;

    assert_debug_snapshot!(backed_up_rows);

    Ok(())
}

#[test]
fn migration_backup_retention_keeps_the_latest_five_backups() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    let manual_backup_path = paths.root().join("pv.db.00000000-manual.bak");

    {
        let _manual_backup = Connection::open(&manual_backup_path)?;
    }

    let migrations = [
        Migration::new(1, "m1", "CREATE TABLE m1 (id TEXT PRIMARY KEY);"),
        Migration::new(2, "m2", "CREATE TABLE m2 (id TEXT PRIMARY KEY);"),
        Migration::new(3, "m3", "CREATE TABLE m3 (id TEXT PRIMARY KEY);"),
        Migration::new(4, "m4", "CREATE TABLE m4 (id TEXT PRIMARY KEY);"),
        Migration::new(5, "m5", "CREATE TABLE m5 (id TEXT PRIMARY KEY);"),
        Migration::new(6, "m6", "CREATE TABLE m6 (id TEXT PRIMARY KEY);"),
        Migration::new(7, "m7", "CREATE TABLE m7 (id TEXT PRIMARY KEY);"),
        Migration::new(8, "m8", "CREATE TABLE m8 (id TEXT PRIMARY KEY);"),
    ];

    for migration_count in 1..=migrations.len() {
        state::testing::open_with_migrations(&paths, &migrations[..migration_count])?;
    }

    let backup_table_counts = migration_backup_table_counts(&paths)?;

    assert!(manual_backup_path.exists());
    assert_debug_snapshot!(backup_table_counts);

    Ok(())
}

#[test]
fn migration_backups_ignore_non_pv_backup_files() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    let manual_backup_names = [
        "pv.db.manual.bak",
        "pv.db.00000000-manual.bak",
        "pv.db.not-a-timestamp.bak",
    ];

    for manual_backup_name in manual_backup_names {
        let manual_backup_path = paths.root().join(manual_backup_name);
        let _manual_backup = Connection::open(&manual_backup_path)?;
    }

    let backups = state::fs::migration_backups(&paths)?;

    for manual_backup_name in manual_backup_names {
        assert!(!backups.contains(&manual_backup_name.to_string()));
    }

    Ok(())
}

fn with_normalized_backup_names(assertion: impl FnOnce() -> Result<()>) -> Result<()> {
    let mut settings = Settings::clone_current();
    settings.add_filter(
        r"pv\.db\.\d{8}-\d{6}(?:-\d+)?\.bak",
        "pv.db.<timestamp>.bak",
    );

    settings.bind(assertion)
}

fn with_normalized_timestamps(assertion: impl FnOnce() -> Result<()>) -> Result<()> {
    let mut settings = Settings::clone_current();
    settings.add_filter(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", "<timestamp>");
    settings.add_filter(r#"id: "[a-z0-9]{10}""#, r#"id: "<project_id>""#);
    settings.add_filter(
        r#"project_id: "[a-z0-9]{10}""#,
        r#"project_id: "<project_id>""#,
    );

    settings.bind(assertion)
}

#[test]
fn pending_migrations_roll_back_as_one_batch_when_later_migration_fails() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    let first_migration = [Migration::new(
        1,
        "first",
        "CREATE TABLE first_table (id TEXT PRIMARY KEY);",
    )];
    state::testing::open_with_migrations(&paths, &first_migration)?;

    let failing_migrations = [
        Migration::new(
            1,
            "first",
            "CREATE TABLE first_table (id TEXT PRIMARY KEY);",
        ),
        Migration::new(
            2,
            "second",
            "CREATE TABLE second_table (id TEXT PRIMARY KEY);",
        ),
        Migration::new(3, "third", "CREATE TABLE broken_sql ("),
    ];
    let result = state::testing::open_with_migrations(&paths, &failing_migrations);

    assert!(matches!(
        result,
        Err(StateError::MigrationFailed {
            version: 3,
            name: "third",
            ..
        })
    ));

    let connection = Connection::open(paths.db())?;
    assert_debug_snapshot!((
        table_exists(&connection, "second_table")?,
        applied_migration_count(&connection)?
    ));

    Ok(())
}

#[test]
fn applied_migration_name_mismatches_are_reported() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    let first_migration = [Migration::new(
        1,
        "first",
        "CREATE TABLE first_table (id TEXT PRIMARY KEY);",
    )];
    state::testing::open_with_migrations(&paths, &first_migration)?;

    let renamed_migration = [Migration::new(
        1,
        "renamed",
        "CREATE TABLE first_table (id TEXT PRIMARY KEY);",
    )];
    let result = state::testing::open_with_migrations(&paths, &renamed_migration);

    assert!(matches!(
        result,
        Err(StateError::MigrationNameMismatch {
            version: 1,
            expected: "renamed",
            actual,
        }) if actual == "first"
    ));

    Ok(())
}

#[test]
fn resource_allocations_reject_duplicate_generated_names_per_resource_track() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    state::testing::transaction(&mut database, |transaction| {
        transaction.execute(
            "INSERT INTO projects (id, path, original_path, primary_hostname, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                "project_1",
                "/tmp/acme",
                "/tmp/acme",
                "acme.test",
                "2026-05-23T00:00:00Z",
                "2026-05-23T00:00:00Z",
            ],
        )?;
        transaction.execute(
            "INSERT INTO resource_allocations (id, project_id, resource_name, track, allocation_name, generated_name, status, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                "allocation_1",
                "project_1",
                "rustfs",
                "latest",
                "uploads",
                "acme-test-uploads",
                "desired",
                "2026-05-23T00:00:00Z",
                "2026-05-23T00:00:00Z",
            ],
        )?;

        Ok(())
    })?;

    let result = state::testing::transaction(&mut database, |transaction| {
        transaction.execute(
            "INSERT INTO resource_allocations (id, project_id, resource_name, track, allocation_name, generated_name, status, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                "allocation_2",
                "project_1",
                "rustfs",
                "latest",
                "upload_assets",
                "acme-test-uploads",
                "desired",
                "2026-05-23T00:00:00Z",
                "2026-05-23T00:00:00Z",
            ],
        )?;

        Ok(())
    });

    assert!(result.is_err());
    assert_debug_snapshot!(state::testing::query_i64(
        &database,
        "SELECT COUNT(*) FROM resource_allocations"
    )?);

    Ok(())
}

#[test]
fn managed_resource_tracks_record_desired_and_installed_state() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    database.record_managed_resource_track_desired(
        "redis",
        "7.2",
        ManagedResourceDesiredState::Installed,
    )?;
    database.record_managed_resource_track_installed(
        "redis",
        "7.2",
        "7.2.5-pv1",
        Utf8Path::new("/Users/example/.pv/resources/redis/7.2/releases/7.2.5-pv1"),
    )?;

    with_normalized_timestamps(|| {
        assert_debug_snapshot!(database.managed_resource_tracks()?);
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn managed_resource_tracks_record_desired_and_installed_batch_atomically() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let php_path = Utf8Path::new("/Users/example/.pv/resources/php/8.4/releases/8.4.8-pv1");
    let frankenphp_path =
        Utf8Path::new("/Users/example/.pv/resources/frankenphp/8.4/releases/8.4.8-pv1");

    database.record_managed_resource_tracks_desired_and_installed(&[
        ManagedResourceTrackInstallInput {
            resource_name: "php",
            track: "8.4",
            installed_version: "8.4.8-pv1",
            current_artifact_path: php_path,
        },
        ManagedResourceTrackInstallInput {
            resource_name: "frankenphp",
            track: "8.4",
            installed_version: "8.4.8-pv1",
            current_artifact_path: frankenphp_path,
        },
    ])?;
    let installed_tracks = database.managed_resource_tracks()?;

    let result = database.record_managed_resource_tracks_desired_and_installed(&[
        ManagedResourceTrackInstallInput {
            resource_name: "redis",
            track: "7.2",
            installed_version: "7.2.5-pv1",
            current_artifact_path: Utf8Path::new(
                "/Users/example/.pv/resources/redis/7.2/releases/7.2.5-pv1",
            ),
        },
        ManagedResourceTrackInstallInput {
            resource_name: "mysql",
            track: "latest",
            installed_version: "8.4.0-pv1",
            current_artifact_path: Utf8Path::new(
                "/Users/example/.pv/resources/mysql/latest/releases/8.4.0-pv1",
            ),
        },
    ]);
    let tracks_after_invalid_batch = database.managed_resource_tracks()?;

    assert!(matches!(
        result,
        Err(StateError::ReservedConcreteTrack { track }) if track == "latest"
    ));
    with_normalized_timestamps(|| {
        assert_debug_snapshot!((installed_tracks, tracks_after_invalid_batch));
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn managed_resource_installed_update_preserves_removed_desired_state() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    database.record_managed_resource_track_desired(
        "redis",
        "7.2",
        ManagedResourceDesiredState::Removed,
    )?;
    let record = database.record_managed_resource_track_installed(
        "redis",
        "7.2",
        "7.2.5-pv1",
        Utf8Path::new("/Users/example/.pv/resources/redis/7.2/releases/7.2.5-pv1"),
    )?;

    assert_eq!(record.desired_state, ManagedResourceDesiredState::Removed);
    assert_eq!(record.installed_version.as_deref(), Some("7.2.5-pv1"));

    Ok(())
}

#[test]
fn managed_resource_tracks_record_removal_intent_options() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    database.record_managed_resource_track_installed(
        "redis",
        "7.2",
        "7.2.5-pv1",
        Utf8Path::new("/Users/example/.pv/resources/redis/7.2/releases/7.2.5-pv1"),
    )?;
    database.record_managed_resource_track_removal_intent("redis", "7.2", true, true)?;

    with_normalized_timestamps(|| {
        assert_debug_snapshot!(database.managed_resource_tracks()?);
        Ok::<(), anyhow::Error>(())
    })?;

    database.record_managed_resource_track_desired(
        "redis",
        "7.2",
        ManagedResourceDesiredState::Installed,
    )?;

    with_normalized_timestamps(|| {
        assert_debug_snapshot!(
            "after_reinstall_intent",
            database.managed_resource_tracks()?
        );
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn managed_resource_tracks_record_removal_intents_batch_atomically() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    database.record_managed_resource_tracks_desired_and_installed(&[
        ManagedResourceTrackInstallInput {
            resource_name: "php",
            track: "8.4",
            installed_version: "8.4.8-pv1",
            current_artifact_path: Utf8Path::new(
                "/Users/example/.pv/resources/php/8.4/releases/8.4.8-pv1",
            ),
        },
        ManagedResourceTrackInstallInput {
            resource_name: "frankenphp",
            track: "8.4",
            installed_version: "8.4.8-pv1",
            current_artifact_path: Utf8Path::new(
                "/Users/example/.pv/resources/frankenphp/8.4/releases/8.4.8-pv1",
            ),
        },
    ])?;
    let tracks_before_invalid_batch = database.managed_resource_tracks()?;

    let result = database.record_managed_resource_tracks_removal_intent(&[
        ManagedResourceTrackRemovalInput {
            resource_name: "php",
            track: "8.4",
            prune: true,
            force: true,
        },
        ManagedResourceTrackRemovalInput {
            resource_name: "frankenphp",
            track: "latest",
            prune: true,
            force: true,
        },
    ]);
    let tracks_after_invalid_batch = database.managed_resource_tracks()?;

    assert!(matches!(
        result,
        Err(StateError::ReservedConcreteTrack { track }) if track == "latest"
    ));
    with_normalized_timestamps(|| {
        assert_debug_snapshot!((tracks_before_invalid_batch, tracks_after_invalid_batch));
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn global_php_default_track_round_trips() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    assert_eq!(database.global_php_default_track()?, None);

    database.record_global_php_default_track("8.3")?;
    assert_eq!(database.global_php_default_track()?.as_deref(), Some("8.3"));

    database.record_global_php_default_track("8.4")?;
    assert_eq!(database.global_php_default_track()?.as_deref(), Some("8.4"));

    Ok(())
}

#[test]
fn global_php_default_rejects_latest_and_invalid_tracks() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    assert!(matches!(
        database.record_global_php_default_track("latest"),
        Err(StateError::ReservedConcreteTrack { track }) if track == "latest"
    ));
    assert!(matches!(
        database.record_global_php_default_track("../8.4"),
        Err(StateError::InvalidManagedResourceIdentity { kind: "track", value })
            if value == "../8.4"
    ));
    assert_eq!(database.global_php_default_track()?, None);

    Ok(())
}

#[test]
fn project_resource_requirements_migration_backfills_env_context() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let old_migrations = [
        Migration::new(
            1,
            "core_state_schema",
            include_str!("../src/sql/001_core_state_schema.sql"),
        ),
        Migration::new(
            2,
            "managed_resource_removal_intent",
            include_str!("../src/sql/002_managed_resource_removal_intent.sql"),
        ),
        Migration::new(
            3,
            "project_primary_hostname_updates",
            include_str!("../src/sql/003_project_primary_hostname_updates.sql"),
        ),
        Migration::new(
            4,
            "project_original_path",
            include_str!("../src/sql/004_project_original_path.sql"),
        ),
    ];
    let mut old_database = state::testing::open_with_migrations(&paths, &old_migrations)?;
    state::testing::transaction(&mut old_database, |transaction| {
        transaction.execute(
            "INSERT INTO managed_resource_tracks (
                resource_name,
                track,
                desired_state,
                installed_version,
                current_artifact_path,
                usage_count,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                "mysql",
                "8.0",
                "installed",
                "8.0.36-pv1",
                "/Users/example/.pv/resources/mysql/8.0/releases/8.0.36-pv1",
                2,
                "2026-05-23T00:00:00Z",
            ],
        )?;

        Ok(())
    })?;
    drop(old_database);

    let upgraded_migrations = [
        old_migrations[0],
        old_migrations[1],
        old_migrations[2],
        old_migrations[3],
        Migration::new(
            5,
            "project_resource_requirements",
            include_str!("../src/sql/005_project_resource_requirements.sql"),
        ),
    ];
    let database = state::testing::open_with_migrations(&paths, &upgraded_migrations)?;

    assert_debug_snapshot!(database.managed_resource_tracks()?);

    Ok(())
}

#[test]
fn migrated_project_resource_state_round_trips_through_public_apis() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let old_migrations = [
        Migration::new(
            1,
            "core_state_schema",
            include_str!("../src/sql/001_core_state_schema.sql"),
        ),
        Migration::new(
            2,
            "managed_resource_removal_intent",
            include_str!("../src/sql/002_managed_resource_removal_intent.sql"),
        ),
        Migration::new(
            3,
            "project_primary_hostname_updates",
            include_str!("../src/sql/003_project_primary_hostname_updates.sql"),
        ),
        Migration::new(
            4,
            "project_original_path",
            include_str!("../src/sql/004_project_original_path.sql"),
        ),
    ];
    let mut old_database = state::testing::open_with_migrations(&paths, &old_migrations)?;
    let linked = old_database.link_project(state::LinkProjectInput {
        path: tempdir.path().join("acme"),
        original_path: tempdir.path().join("acme"),
        primary_hostname: "acme.test".to_string(),
        config_path: tempdir.path().join("acme/pv.yml"),
        desired_php_track: Some("8.4".to_string()),
        additional_hostnames: vec!["api.acme.test".to_string()],
    })?;
    state::testing::transaction(&mut old_database, |transaction| {
        transaction.execute(
            "INSERT INTO managed_resource_tracks (
                resource_name,
                track,
                desired_state,
                installed_version,
                current_artifact_path,
                usage_count,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                "mysql",
                "8.0",
                "installed",
                "8.0.36-pv1",
                "/Users/example/.pv/resources/mysql/8.0/releases/8.0.36-pv1",
                0,
                "2026-05-23T00:00:00Z",
            ],
        )?;

        Ok(())
    })?;
    drop(old_database);

    let upgraded_migrations = [
        old_migrations[0],
        old_migrations[1],
        old_migrations[2],
        old_migrations[3],
        Migration::new(
            5,
            "project_resource_requirements",
            include_str!("../src/sql/005_project_resource_requirements.sql"),
        ),
    ];
    let mut database = state::testing::open_with_migrations(&paths, &upgraded_migrations)?;
    let project_id = linked.project.id;
    let before = (
        database.inspect()?,
        database.projects()?,
        database.managed_resource_tracks()?,
    );

    database.replace_project_managed_resources(
        &project_id,
        &[
            ProjectManagedResourceInput {
                resource_name: "mysql".to_string(),
                track: "8.0".to_string(),
            },
            ProjectManagedResourceInput {
                resource_name: "redis".to_string(),
                track: "7.2".to_string(),
            },
        ],
    )?;
    database.record_managed_resource_track_env_context(
        "mysql",
        "8.0",
        &env_context(&[
            ("host", "127.0.0.1"),
            ("password", "secret"),
            ("port", "3306"),
            ("username", "root"),
        ]),
    )?;
    database.record_managed_resource_track_env_context(
        "redis",
        "7.2",
        &env_context(&[("host", "127.0.0.1"), ("port", "6379")]),
    )?;
    let desired_allocations = database.replace_project_resource_allocations(
        &project_id,
        "mysql",
        "8.0",
        &[ResourceAllocationInput {
            allocation_name: "app-db".to_string(),
            generated_name: "acme_test_app_db".to_string(),
        }],
    )?;
    let ready_allocation = database.mark_resource_allocation_ready(
        &project_id,
        "mysql",
        "8.0",
        "app-db",
        &env_context(&[("database", "acme_test_app_db")]),
    )?;
    let env_state = database.project_env_context(&project_id)?;
    let observed_state = database.record_project_env_observed_snapshot(
        &project_id,
        ProjectEnvObservedStatus::Warning,
        Some("rendered with warnings after migration"),
        &[ProjectEnvObservedWarningInput {
            kind: "duplicate_key".to_string(),
            message: "APP_URL already exists outside the PV block".to_string(),
        }],
    )?;
    let after = (
        database.projects()?,
        database.project_by_hostname("api.acme.test")?,
        database.project_managed_resources(&project_id)?,
        database.managed_resource_tracks()?,
        desired_allocations,
        ready_allocation,
        database.resource_allocations(&project_id, "mysql")?,
        env_state,
        observed_state,
        database.project_env_observed_warnings(&project_id)?,
    );

    let mut settings = Settings::clone_current();
    settings.add_filter(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", "<timestamp>");
    settings.add_filter(r#"id: "[a-z0-9]{10}""#, r#"id: "<project_id>""#);
    settings.add_filter(
        r#"project_id: "[a-z0-9]{10}""#,
        r#"project_id: "<project_id>""#,
    );
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((before, after));
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn project_managed_resources_recalculate_usage_counts() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let acme = link_test_project(&mut database, tempdir.path(), "acme", "acme.test")?;
    let other = link_test_project(&mut database, tempdir.path(), "other", "other.test")?;

    database.replace_project_managed_resources(
        &acme.id,
        &[
            ProjectManagedResourceInput {
                resource_name: "mysql".to_string(),
                track: "8.0".to_string(),
            },
            ProjectManagedResourceInput {
                resource_name: "redis".to_string(),
                track: "7.2".to_string(),
            },
        ],
    )?;
    database.replace_project_managed_resources(
        &other.id,
        &[ProjectManagedResourceInput {
            resource_name: "mysql".to_string(),
            track: "8.0".to_string(),
        }],
    )?;
    let after_initial = (
        database.project_managed_resources(&acme.id)?,
        database.managed_resource_tracks()?,
    );

    database.replace_project_managed_resources(
        &acme.id,
        &[ProjectManagedResourceInput {
            resource_name: "mysql".to_string(),
            track: "8.4".to_string(),
        }],
    )?;
    database.unlink_project(&other.id)?;
    let after_switch_and_unlink = (
        database.project_managed_resources(&acme.id)?,
        database.managed_resource_tracks()?,
    );

    with_normalized_timestamps(|| {
        assert_debug_snapshot!((after_initial, after_switch_and_unlink));
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn resource_allocations_preserve_generated_names_and_env_context() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let project = link_test_project(&mut database, tempdir.path(), "acme", "acme.test")?;

    database.replace_project_managed_resources(
        &project.id,
        &[ProjectManagedResourceInput {
            resource_name: "mysql".to_string(),
            track: "8.0".to_string(),
        }],
    )?;
    database.record_managed_resource_track_env_context(
        "mysql",
        "8.0",
        &env_context(&[
            ("host", "127.0.0.1"),
            ("password", "secret"),
            ("port", "3306"),
            ("username", "root"),
        ]),
    )?;
    let created = database.replace_project_resource_allocations(
        &project.id,
        "mysql",
        "8.0",
        &[ResourceAllocationInput {
            allocation_name: "app-db".to_string(),
            generated_name: "acme_test_app_db".to_string(),
        }],
    )?;
    let ready = database.mark_resource_allocation_ready(
        &project.id,
        "mysql",
        "8.0",
        "app-db",
        &env_context(&[("database", "acme_test_app_db")]),
    )?;
    let context = database.project_env_context(&project.id)?;

    let removed =
        database.replace_project_resource_allocations(&project.id, "mysql", "8.0", &[])?;
    let after_removal = database.resource_allocations(&project.id, "mysql")?;
    let readded = database.replace_project_resource_allocations(
        &project.id,
        "mysql",
        "8.0",
        &[ResourceAllocationInput {
            allocation_name: "app-db".to_string(),
            generated_name: "renamed_test_app_db".to_string(),
        }],
    )?;

    with_normalized_timestamps(|| {
        assert_debug_snapshot!((created, ready, context, removed, after_removal, readded));
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn resource_allocations_reject_generated_name_collision_with_inactive_allocation() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let project = link_test_project(&mut database, tempdir.path(), "acme", "acme.test")?;

    database.replace_project_resource_allocations(
        &project.id,
        "mysql",
        "8.0",
        &[ResourceAllocationInput {
            allocation_name: "app-db".to_string(),
            generated_name: "acme_test_app_db".to_string(),
        }],
    )?;
    let removed =
        database.replace_project_resource_allocations(&project.id, "mysql", "8.0", &[])?;
    let after_removal = database.resource_allocations(&project.id, "mysql")?;
    let collision = database.replace_project_resource_allocations(
        &project.id,
        "mysql",
        "8.0",
        &[ResourceAllocationInput {
            allocation_name: "app_db".to_string(),
            generated_name: "acme_test_app_db".to_string(),
        }],
    );
    let collision_summary = match collision {
        Err(StateError::ResourceAllocationGeneratedNameCollision {
            resource,
            track,
            generated,
        }) => (resource, track, generated),
        result => return Err(anyhow!("expected generated-name collision, got {result:?}")),
    };
    let after_collision = database.resource_allocations(&project.id, "mysql")?;

    assert_eq!(after_removal, after_collision);
    with_normalized_timestamps(|| {
        assert_debug_snapshot!((removed, after_collision, collision_summary));
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn generated_env_context_escapes_round_trip_through_state() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let project = link_test_project(&mut database, tempdir.path(), "acme", "acme.test")?;

    database.replace_project_managed_resources(
        &project.id,
        &[ProjectManagedResourceInput {
            resource_name: "mysql".to_string(),
            track: "8.0".to_string(),
        }],
    )?;
    database.record_managed_resource_track_env_context(
        "mysql",
        "8.0",
        &env_context(&[
            ("backslash", r"C:\pv\mysql"),
            ("literal_newline", r"line\nvalue"),
            ("multiline", "line one\nline two"),
            ("quote", r#"root "local""#),
            ("unicode", "caf\u{00e9}-\u{03bb}"),
        ]),
    )?;
    database.replace_project_resource_allocations(
        &project.id,
        "mysql",
        "8.0",
        &[ResourceAllocationInput {
            allocation_name: "app-db".to_string(),
            generated_name: "acme_test_app_db".to_string(),
        }],
    )?;
    database.mark_resource_allocation_ready(
        &project.id,
        "mysql",
        "8.0",
        "app-db",
        &env_context(&[
            ("database", "acme_test_app_db"),
            ("dsn", "mysql://root:pa\\ss@127.0.0.1/acme_test_app_db"),
            ("literal_newline", r"row\nnext"),
            ("multiline", "row one\nrow two"),
            ("password", r#"pa"ss\word"#),
            ("unicode", "schema-\u{03b4}"),
        ]),
    )?;

    let tracks = database.managed_resource_tracks()?;
    let allocations = database.resource_allocations(&project.id, "mysql")?;
    let context = database.project_env_context(&project.id)?;

    with_normalized_timestamps(|| {
        assert_debug_snapshot!((tracks, allocations, context));
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn resource_allocation_ready_requires_desired_track() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let project = link_test_project(&mut database, tempdir.path(), "acme", "acme.test")?;

    let desired = database.replace_project_resource_allocations(
        &project.id,
        "mysql",
        "8.0",
        &[ResourceAllocationInput {
            allocation_name: "app-db".to_string(),
            generated_name: "acme_test_app_db".to_string(),
        }],
    )?;
    let wrong_track = database.mark_resource_allocation_ready(
        &project.id,
        "mysql",
        "8.4",
        "app-db",
        &env_context(&[("database", "acme_test_app_db")]),
    );
    let ready = database.mark_resource_allocation_ready(
        &project.id,
        "mysql",
        "8.0",
        "app-db",
        &env_context(&[("database", "acme_test_app_db")]),
    )?;
    let already_ready = database.mark_resource_allocation_ready(
        &project.id,
        "mysql",
        "8.0",
        "app-db",
        &env_context(&[("database", "acme_test_app_db")]),
    );
    let switched = database.replace_project_resource_allocations(
        &project.id,
        "mysql",
        "8.4",
        &[ResourceAllocationInput {
            allocation_name: "app-db".to_string(),
            generated_name: "acme_test_app_db".to_string(),
        }],
    )?;
    let stale_track = database.mark_resource_allocation_ready(
        &project.id,
        "mysql",
        "8.0",
        "app-db",
        &env_context(&[("database", "acme_test_app_db")]),
    );
    let removed =
        database.replace_project_resource_allocations(&project.id, "mysql", "8.4", &[])?;
    let inactive = database.mark_resource_allocation_ready(
        &project.id,
        "mysql",
        "8.4",
        "app-db",
        &env_context(&[("database", "acme_test_app_db")]),
    );

    assert!(matches!(
        wrong_track,
        Err(StateError::ResourceAllocationNotDesired { track, .. }) if track == "8.4"
    ));
    assert!(matches!(
        already_ready,
        Err(StateError::ResourceAllocationNotDesired { track, .. }) if track == "8.0"
    ));
    assert!(matches!(
        stale_track,
        Err(StateError::ResourceAllocationNotDesired { track, .. }) if track == "8.0"
    ));
    assert!(matches!(
        inactive,
        Err(StateError::ResourceAllocationNotDesired { track, .. }) if track == "8.4"
    ));

    with_normalized_timestamps(|| {
        assert_debug_snapshot!((desired, ready, switched, removed));
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn project_env_context_uses_ready_allocations_from_required_track_only() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let project = link_test_project(&mut database, tempdir.path(), "acme", "acme.test")?;

    database.replace_project_managed_resources(
        &project.id,
        &[ProjectManagedResourceInput {
            resource_name: "mysql".to_string(),
            track: "8.0".to_string(),
        }],
    )?;
    database.record_managed_resource_track_env_context(
        "mysql",
        "8.0",
        &env_context(&[("host", "127.0.0.1"), ("port", "3306")]),
    )?;
    database.replace_project_resource_allocations(
        &project.id,
        "mysql",
        "8.0",
        &[ResourceAllocationInput {
            allocation_name: "app-db".to_string(),
            generated_name: "acme_test_app_db".to_string(),
        }],
    )?;
    database.mark_resource_allocation_ready(
        &project.id,
        "mysql",
        "8.0",
        "app-db",
        &env_context(&[("database", "acme_test_app_db")]),
    )?;
    let old_track_context = database.project_env_context(&project.id)?;
    set_resource_allocation_env_json(&mut database, &project.id, "mysql", "app-db", "{")?;

    database.replace_project_managed_resources(
        &project.id,
        &[ProjectManagedResourceInput {
            resource_name: "mysql".to_string(),
            track: "8.4".to_string(),
        }],
    )?;
    database.record_managed_resource_track_env_context(
        "mysql",
        "8.4",
        &env_context(&[("host", "127.0.0.1"), ("port", "3406")]),
    )?;
    let switched_track_context = database.project_env_context(&project.id)?;

    with_normalized_timestamps(|| {
        assert_debug_snapshot!((old_track_context, switched_track_context));
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn resource_allocations_reject_invalid_allocation_names_at_state_boundary() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let project = link_test_project(&mut database, tempdir.path(), "acme", "acme.test")?;

    let invalid_allocations = [
        ("App", "acme_test_uppercase"),
        ("app.db", "acme_test_dotted"),
        ("1app", "acme_test_digit_start"),
        ("", "acme_test_empty"),
    ];

    for (allocation_name, generated_name) in invalid_allocations {
        let result = database.replace_project_resource_allocations(
            &project.id,
            "mysql",
            "8.0",
            &[ResourceAllocationInput {
                allocation_name: allocation_name.to_string(),
                generated_name: generated_name.to_string(),
            }],
        );

        assert!(matches!(
            result,
            Err(StateError::InvalidResourceAllocationIdentity { kind: "allocation", value })
                if value == allocation_name
        ));
    }

    assert_eq!(
        state::testing::query_i64(&database, "SELECT COUNT(*) FROM resource_allocations")?,
        0
    );

    Ok(())
}

#[test]
fn resource_env_context_validation_reports_typed_errors() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    database.record_managed_resource_track_desired(
        "redis",
        "7.2",
        ManagedResourceDesiredState::Installed,
    )?;
    set_managed_resource_env_json(&mut database, "{")?;
    assert!(matches!(
        database.managed_resource_tracks(),
        Err(StateError::InvalidEnvJson { .. })
    ));

    set_managed_resource_env_json(&mut database, "[]")?;
    assert!(matches!(
        database.managed_resource_tracks(),
        Err(StateError::InvalidEnvJson { .. })
    ));

    set_managed_resource_env_json(&mut database, r#"{"port":3306}"#)?;
    assert!(matches!(
        database.managed_resource_tracks(),
        Err(StateError::InvalidEnvJson { .. })
    ));

    let invalid_context = BTreeMap::from([(String::new(), "value".to_string())]);
    assert!(matches!(
        database.record_managed_resource_track_env_context("redis", "7.2", &invalid_context),
        Err(StateError::InvalidEnvContext { .. })
    ));

    set_managed_resource_env_json(&mut database, "{}")?;
    let project = link_test_project(&mut database, tempdir.path(), "acme", "acme.test")?;
    state::testing::transaction(&mut database, |transaction| {
        transaction.execute(
            "INSERT INTO resource_allocations (
                id,
                project_id,
                resource_name,
                track,
                allocation_name,
                generated_name,
                env_json,
                status,
                created_at,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            params![
                "allocation_000001",
                project.id,
                "redis",
                "7.2",
                "cache",
                "acme-test-cache-",
                "{}",
                "mystery",
                "2026-05-23T00:00:00Z",
            ],
        )?;

        Ok(())
    })?;

    assert!(matches!(
        database.resource_allocations(&project.id, "redis"),
        Err(StateError::UnknownResourceAllocationStatus { status }) if status == "mystery"
    ));

    Ok(())
}

#[test]
fn resource_state_apis_reject_latest_tracks() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let project = link_test_project(&mut database, tempdir.path(), "acme", "acme.test")?;

    assert!(matches!(
        database.record_managed_resource_track_desired(
            "mysql",
            "latest",
            ManagedResourceDesiredState::Installed,
        ),
        Err(StateError::ReservedConcreteTrack { track }) if track == "latest"
    ));
    assert!(matches!(
        database.replace_project_managed_resources(
            &project.id,
            &[ProjectManagedResourceInput {
                resource_name: "mysql".to_string(),
                track: "latest".to_string(),
            }],
        ),
        Err(StateError::ReservedConcreteTrack { track }) if track == "latest"
    ));
    assert!(matches!(
        database.replace_project_resource_allocations(&project.id, "mysql", "latest", &[]),
        Err(StateError::ReservedConcreteTrack { track }) if track == "latest"
    ));
    assert!(matches!(
        database.record_managed_resource_track_env_context("mysql", "latest", &BTreeMap::new()),
        Err(StateError::ReservedConcreteTrack { track }) if track == "latest"
    ));

    Ok(())
}

#[test]
fn project_env_observed_status_and_warnings_round_trip() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let project = link_test_project(&mut database, tempdir.path(), "acme", "acme.test")?;

    let pending = database.record_project_env_observed_snapshot(
        &project.id,
        ProjectEnvObservedStatus::Pending,
        None,
        &[],
    )?;
    let warning_state = database.record_project_env_observed_snapshot(
        &project.id,
        ProjectEnvObservedStatus::Warning,
        Some("rendered with warnings"),
        &[
            ProjectEnvObservedWarningInput {
                kind: "duplicate_key".to_string(),
                message: "APP_URL already exists outside the PV block".to_string(),
            },
            ProjectEnvObservedWarningInput {
                kind: "duplicate_key".to_string(),
                message: "DATABASE_URL already exists outside the PV block".to_string(),
            },
        ],
    )?;
    let failed = database.record_project_env_observed_snapshot(
        &project.id,
        ProjectEnvObservedStatus::Failed,
        Some("render failed"),
        &[],
    )?;
    let invalid_warning = database.record_project_env_observed_snapshot(
        &project.id,
        ProjectEnvObservedStatus::Warning,
        Some("invalid warning"),
        &[ProjectEnvObservedWarningInput {
            kind: String::new(),
            message: "invalid warning should not change stored state".to_string(),
        }],
    );
    let after_invalid_warning = database.project_env_observed_state(&project.id)?;

    assert!(matches!(
        invalid_warning,
        Err(StateError::InvalidProjectEnvObservedWarning { kind: "kind", .. })
    ));

    with_normalized_timestamps(|| {
        assert_debug_snapshot!((pending, warning_state, failed, after_invalid_warning));
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn unlink_project_clears_project_env_observed_state() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let project = link_test_project(&mut database, tempdir.path(), "acme", "acme.test")?;

    database.record_project_env_observed_snapshot(
        &project.id,
        ProjectEnvObservedStatus::Warning,
        Some("rendered with warnings"),
        &[ProjectEnvObservedWarningInput {
            kind: "duplicate_key".to_string(),
            message: "APP_URL already exists outside the PV block".to_string(),
        }],
    )?;

    let before_unlink = database.project_env_observed_state(&project.id)?;
    let unlinked = database.unlink_project(&project.id)?;
    let after_unlink = database.project_env_observed_state(&project.id)?;

    assert_eq!(unlinked.id, project.id);
    with_normalized_timestamps(|| {
        assert_debug_snapshot!((before_unlink, after_unlink));
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn runtime_observed_state_round_trips_through_observed_states() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    database.record_runtime_observed_snapshot(
        RuntimeSubject::Gateway,
        RuntimeObservedStatus::Running,
        Some("gateway is ready"),
    )?;
    database.record_runtime_observed_snapshot(
        RuntimeSubject::PhpWorker {
            php_track: "8.4".to_string(),
        },
        RuntimeObservedStatus::Failed,
        Some("readiness timed out"),
    )?;
    database.record_runtime_observed_snapshot(
        RuntimeSubject::Resource {
            name: "mailpit".to_string(),
            track: "1.0".to_string(),
        },
        RuntimeObservedStatus::Running,
        Some("mailpit is ready"),
    )?;
    let updated_gateway = database.record_runtime_observed_snapshot(
        RuntimeSubject::Gateway,
        RuntimeObservedStatus::Degraded,
        Some("gateway config reload failed"),
    )?;
    let invalid = database.record_runtime_observed_snapshot(
        RuntimeSubject::PhpWorker {
            php_track: String::new(),
        },
        RuntimeObservedStatus::Pending,
        None,
    );
    let reserved = database.record_runtime_observed_snapshot(
        RuntimeSubject::PhpWorker {
            php_track: "latest".to_string(),
        },
        RuntimeObservedStatus::Pending,
        None,
    );

    assert_eq!(updated_gateway.status, RuntimeObservedStatus::Degraded);
    assert!(matches!(
        invalid,
        Err(StateError::InvalidRuntimeSubject { kind: "php_track", value }) if value.is_empty()
    ));
    assert!(matches!(
        reserved,
        Err(StateError::InvalidRuntimeSubject { kind: "php_track", value }) if value == "latest"
    ));

    with_normalized_timestamps(|| {
        assert_debug_snapshot!(database.runtime_observed_states()?);
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn managed_resource_removal_intent_migration_backfills_existing_tracks() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let old_schema = r#"
        CREATE TABLE managed_resource_tracks (
            resource_name TEXT NOT NULL,
            track TEXT NOT NULL,
            desired_state TEXT NOT NULL,
            installed_version TEXT,
            current_artifact_path TEXT,
            usage_count INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (resource_name, track)
        );

        INSERT INTO managed_resource_tracks (
            resource_name,
            track,
            desired_state,
            installed_version,
            current_artifact_path,
            usage_count,
            updated_at
        )
        VALUES (
            'redis',
            '7.2',
            'installed',
            '7.2.5-pv1',
            '/Users/example/.pv/resources/redis/7.2/releases/7.2.5-pv1',
            3,
            '2026-05-23T00:00:00Z'
        );
    "#;
    let old_migration = [Migration::new(1, "core_state_schema", old_schema)];
    let old_database = state::testing::open_with_migrations(&paths, &old_migration)?;
    drop(old_database);

    let upgraded_migrations = [
        Migration::new(1, "core_state_schema", old_schema),
        Migration::new(
            2,
            "managed_resource_removal_intent",
            include_str!("../src/sql/002_managed_resource_removal_intent.sql"),
        ),
        Migration::new(
            5,
            "project_resource_requirements",
            include_str!("../src/sql/005_project_resource_requirements.sql"),
        ),
    ];
    let database = state::testing::open_with_migrations(&paths, &upgraded_migrations)?;

    assert_debug_snapshot!(database.managed_resource_tracks()?);

    Ok(())
}

#[test]
fn project_original_path_migration_backfills_existing_paths() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let old_schema = include_str!("../src/sql/001_core_state_schema.sql");
    let old_migration = [Migration::new(1, "core_state_schema", old_schema)];
    let mut old_database = state::testing::open_with_migrations(&paths, &old_migration)?;
    state::testing::transaction(&mut old_database, |transaction| {
        transaction.execute(
            "INSERT INTO projects (id, path, primary_hostname, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                "project_1",
                "/tmp/acme",
                "acme.test",
                "2026-05-23T00:00:00Z",
                "2026-05-23T00:00:00Z",
            ],
        )?;

        Ok(())
    })?;
    drop(old_database);

    let upgraded_migrations = [
        Migration::new(1, "core_state_schema", old_schema),
        Migration::new(
            2,
            "managed_resource_removal_intent",
            include_str!("../src/sql/002_managed_resource_removal_intent.sql"),
        ),
        Migration::new(
            3,
            "project_primary_hostname_updates",
            include_str!("../src/sql/003_project_primary_hostname_updates.sql"),
        ),
        Migration::new(
            4,
            "project_original_path",
            include_str!("../src/sql/004_project_original_path.sql"),
        ),
    ];
    let database = state::testing::open_with_migrations(&paths, &upgraded_migrations)?;
    let project = database
        .project_by_path(Utf8Path::new("/tmp/acme"))?
        .ok_or_else(|| anyhow!("missing migrated project"))?;

    assert_eq!(project.original_path.as_str(), "/tmp/acme");

    Ok(())
}

#[test]
fn managed_resource_tracks_reject_unknown_desired_state_values() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    state::testing::transaction(&mut database, |transaction| {
        transaction.execute(
            "INSERT INTO managed_resource_tracks (resource_name, track, desired_state, updated_at)
            VALUES (?1, ?2, ?3, ?4)",
            params!["redis", "7.2", "mystery", "2026-05-23T00:00:00Z"],
        )?;

        Ok(())
    })?;
    let result = database.managed_resource_tracks();

    assert!(matches!(
        result,
        Err(StateError::UnknownManagedResourceDesiredState { desired_state })
            if desired_state == "mystery"
    ));

    Ok(())
}

#[test]
fn managed_resource_track_writes_reject_invalid_identities() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    let invalid_resource = database.record_managed_resource_track_desired(
        ".",
        "7.2",
        ManagedResourceDesiredState::Installed,
    );
    assert!(matches!(
        invalid_resource,
        Err(StateError::InvalidManagedResourceIdentity { kind: "name", value })
            if value == "."
    ));

    let invalid_track = database.record_managed_resource_track_desired(
        "redis",
        "..",
        ManagedResourceDesiredState::Installed,
    );
    assert!(matches!(
        invalid_track,
        Err(StateError::InvalidManagedResourceIdentity { kind: "track", value })
            if value == ".."
    ));

    let invalid_version = database.record_managed_resource_track_installed(
        "redis",
        "7.2",
        "..",
        Utf8Path::new("/Users/example/.pv/resources/redis/7.2/releases/7.2.5-pv1"),
    );
    assert!(matches!(
        invalid_version,
        Err(StateError::InvalidManagedResourceIdentity {
            kind: "artifact version",
            value
        }) if value == ".."
    ));

    Ok(())
}

#[test]
fn managed_resource_tracks_reject_invalid_persisted_identities() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    state::testing::transaction(&mut database, |transaction| {
        transaction.execute(
            "INSERT INTO managed_resource_tracks (resource_name, track, desired_state, installed_version, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5)",
            params![".", "7.2", "installed", "7.2.5-pv1", "2026-05-23T00:00:00Z"],
        )?;

        Ok(())
    })?;
    let result = database.managed_resource_tracks();

    assert!(matches!(
        result,
        Err(StateError::InvalidManagedResourceIdentity { kind: "name", value })
            if value == "."
    ));

    Ok(())
}

#[test]
fn primary_project_hostname_rows_must_match_the_project() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    state::testing::transaction(&mut database, |transaction| {
        transaction.execute(
            "INSERT INTO projects (id, path, original_path, primary_hostname, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                "project_1",
                "/tmp/acme",
                "/tmp/acme",
                "acme.test",
                "2026-05-23T00:00:00Z",
                "2026-05-23T00:00:00Z",
            ],
        )?;

        Ok(())
    })?;

    let mismatched_primary = state::testing::transaction(&mut database, |transaction| {
        transaction.execute(
            "INSERT INTO project_hostnames (hostname, project_id, is_primary, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                "other.test",
                "project_1",
                1,
                "2026-05-23T00:00:00Z",
            ],
        )?;

        Ok(())
    });

    assert!(mismatched_primary.is_err());

    state::testing::transaction(&mut database, |transaction| {
        transaction.execute(
            "INSERT INTO project_hostnames (hostname, project_id, is_primary, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                "acme.test",
                "project_1",
                1,
                "2026-05-23T00:00:00Z",
            ],
        )?;

        Ok(())
    })?;

    let duplicate_primary = state::testing::transaction(&mut database, |transaction| {
        transaction.execute(
            "INSERT INTO project_hostnames (hostname, project_id, is_primary, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                "acme-alias.test",
                "project_1",
                1,
                "2026-05-23T00:00:00Z",
            ],
        )?;

        Ok(())
    });

    assert!(duplicate_primary.is_err());

    state::testing::transaction(&mut database, |transaction| {
        transaction.execute(
            "UPDATE projects SET primary_hostname = ?1 WHERE id = ?2",
            params!["renamed.test", "project_1"],
        )?;

        Ok(())
    })?;
    let renamed_primary = state::testing::query_i64(
        &database,
        "SELECT COUNT(*) FROM project_hostnames WHERE hostname = 'renamed.test' AND is_primary = 1",
    )?;

    assert_eq!(renamed_primary, 1);

    let delete_primary = state::testing::transaction(&mut database, |transaction| {
        transaction.execute(
            "DELETE FROM project_hostnames WHERE hostname = ?1",
            params!["renamed.test"],
        )?;

        Ok(())
    });

    assert!(delete_primary.is_err());

    state::testing::transaction(&mut database, |transaction| {
        transaction.execute("DELETE FROM projects WHERE id = ?1", params!["project_1"])?;

        Ok(())
    })?;

    Ok(())
}

#[test]
fn linked_projects_preserve_ids_and_refresh_hostnames() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let project_path = tempdir.path().join("acme");
    let config_path = project_path.join("pv.yml");

    let created = database.link_project(state::LinkProjectInput {
        path: project_path.clone(),
        original_path: project_path.clone(),
        primary_hostname: "acme.test".to_string(),
        config_path: config_path.clone(),
        desired_php_track: Some("8.4".to_string()),
        additional_hostnames: vec!["api.acme.test".to_string()],
    })?;
    let updated = database.link_project(state::LinkProjectInput {
        path: project_path.clone(),
        original_path: project_path,
        primary_hostname: "store.test".to_string(),
        config_path,
        desired_php_track: Some("8.3".to_string()),
        additional_hostnames: vec!["admin.store.test".to_string()],
    })?;

    assert_eq!(created.project.id, updated.project.id);
    let mut settings = Settings::clone_current();
    settings.add_filter(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", "<timestamp>");
    settings.add_filter(r#"id: "[a-z0-9]{10}""#, r#"id: "<project_id>""#);
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((created.status, updated.status, database.projects()?));
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn linked_projects_refresh_desired_php_track_independently() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let created = database.link_project(state::LinkProjectInput {
        path: tempdir.path().join("acme"),
        original_path: tempdir.path().join("acme"),
        primary_hostname: "acme.test".to_string(),
        config_path: tempdir.path().join("acme/pv.yml"),
        desired_php_track: Some("8.4".to_string()),
        additional_hostnames: vec!["api.acme.test".to_string()],
    })?;

    let updated = database.replace_project_desired_php_track(&created.project.id, Some("8.3"))?;

    assert_eq!(updated.desired_php_track.as_deref(), Some("8.3"));
    assert_eq!(
        updated.additional_hostnames,
        vec!["api.acme.test".to_string()]
    );

    Ok(())
}

#[test]
fn linked_projects_store_original_and_canonical_paths() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let original_path = tempdir.path().join("linked-acme");
    let canonical_path = tempdir.path().join("real-acme");

    let created = database.link_project(state::LinkProjectInput {
        path: canonical_path.clone(),
        original_path: original_path.clone(),
        primary_hostname: "acme.test".to_string(),
        config_path: canonical_path.join("pv.yml"),
        desired_php_track: None,
        additional_hostnames: Vec::new(),
    })?;
    let resolved = database
        .project_by_path(&canonical_path)?
        .ok_or_else(|| anyhow!("missing linked project"))?;

    assert_eq!(created.project.path, canonical_path);
    assert_eq!(created.project.original_path, original_path);
    assert_eq!(resolved.original_path, original_path);

    Ok(())
}

#[test]
fn linked_projects_allow_noncanonical_original_paths() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let project_path = tempdir.path().join("acme");
    let original_path = tempdir.path().join("work").join("..").join("acme");

    let created = database.link_project(state::LinkProjectInput {
        path: project_path.clone(),
        original_path: original_path.clone(),
        primary_hostname: "acme.test".to_string(),
        config_path: project_path.join("pv.yml"),
        desired_php_track: None,
        additional_hostnames: Vec::new(),
    })?;

    assert_eq!(created.project.path, project_path);
    assert_eq!(created.project.original_path, original_path);

    Ok(())
}

#[test]
fn linked_projects_can_promote_additional_hostname_to_primary() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let project_path = tempdir.path().join("acme");
    let config_path = project_path.join("pv.yml");

    let created = database.link_project(state::LinkProjectInput {
        path: project_path.clone(),
        original_path: project_path.clone(),
        primary_hostname: "acme.test".to_string(),
        config_path: config_path.clone(),
        desired_php_track: None,
        additional_hostnames: vec!["api.acme.test".to_string()],
    })?;
    let updated = database.link_project(state::LinkProjectInput {
        path: project_path.clone(),
        original_path: project_path,
        primary_hostname: "api.acme.test".to_string(),
        config_path,
        desired_php_track: None,
        additional_hostnames: vec!["acme.test".to_string()],
    })?;

    assert_eq!(created.project.id, updated.project.id);
    assert_eq!(updated.project.primary_hostname, "api.acme.test");
    assert_eq!(updated.project.additional_hostnames, vec!["acme.test"]);

    Ok(())
}

#[test]
fn link_project_rejects_invalid_input_shapes() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    assert!(matches!(
        database.link_project(state::LinkProjectInput {
            path: "relative".into(),
            original_path: tempdir.path().join("acme"),
            primary_hostname: "acme.test".to_string(),
            config_path: tempdir.path().join("acme/pv.yml"),
            desired_php_track: None,
            additional_hostnames: Vec::new(),
        }),
        Err(state::StateError::InvalidProjectPath { kind: "path", .. })
    ));
    assert!(matches!(
        database.link_project(state::LinkProjectInput {
            path: tempdir.path().join("acme"),
            original_path: "relative".into(),
            primary_hostname: "acme.test".to_string(),
            config_path: tempdir.path().join("acme/pv.yml"),
            desired_php_track: None,
            additional_hostnames: Vec::new(),
        }),
        Err(state::StateError::InvalidProjectPath {
            kind: "original path",
            ..
        })
    ));
    assert!(matches!(
        database.link_project(state::LinkProjectInput {
            path: tempdir.path().join("acme"),
            original_path: tempdir.path().join("acme"),
            primary_hostname: "Acme.test".to_string(),
            config_path: tempdir.path().join("acme/pv.yml"),
            desired_php_track: None,
            additional_hostnames: Vec::new(),
        }),
        Err(state::StateError::InvalidProjectHostname { hostname, .. }) if hostname == "Acme.test"
    ));
    assert!(matches!(
        database.link_project(state::LinkProjectInput {
            path: tempdir.path().join("acme"),
            original_path: tempdir.path().join("acme"),
            primary_hostname: "acme.test".to_string(),
            config_path: tempdir.path().join("acme/pv.yml"),
            desired_php_track: Some(String::new()),
            additional_hostnames: Vec::new(),
        }),
        Err(state::StateError::InvalidProjectTrack { track }) if track.is_empty()
    ));
    assert!(matches!(
        database.link_project(state::LinkProjectInput {
            path: tempdir.path().join("acme"),
            original_path: tempdir.path().join("acme"),
            primary_hostname: "acme.test".to_string(),
            config_path: tempdir.path().join("acme/pv.yml"),
            desired_php_track: Some("latest".to_string()),
            additional_hostnames: Vec::new(),
        }),
        Err(state::StateError::InvalidProjectTrack { track }) if track == "latest"
    ));

    Ok(())
}

#[test]
fn linked_project_hostname_collisions_are_rejected() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    let first = database.link_project(state::LinkProjectInput {
        path: tempdir.path().join("acme"),
        original_path: tempdir.path().join("acme"),
        primary_hostname: "acme.test".to_string(),
        config_path: tempdir.path().join("acme/pv.yml"),
        desired_php_track: None,
        additional_hostnames: vec!["api.acme.test".to_string()],
    })?;
    let collision = database.link_project(state::LinkProjectInput {
        path: tempdir.path().join("other"),
        original_path: tempdir.path().join("other"),
        primary_hostname: "api.acme.test".to_string(),
        config_path: tempdir.path().join("other/pv.yml"),
        desired_php_track: None,
        additional_hostnames: Vec::new(),
    });

    assert!(matches!(
        collision,
        Err(StateError::ProjectHostnameCollision {
            hostname,
            project_id,
        }) if hostname == "api.acme.test" && project_id == first.project.id
    ));

    Ok(())
}

#[test]
fn linked_project_hostname_validation_checks_pending_config_hostnames() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    let first = database.link_project(state::LinkProjectInput {
        path: tempdir.path().join("acme"),
        original_path: tempdir.path().join("acme"),
        primary_hostname: "acme.test".to_string(),
        config_path: tempdir.path().join("acme/pv.yml"),
        desired_php_track: None,
        additional_hostnames: Vec::new(),
    })?;
    let second = database.link_project(state::LinkProjectInput {
        path: tempdir.path().join("other"),
        original_path: tempdir.path().join("other"),
        primary_hostname: "other.test".to_string(),
        config_path: tempdir.path().join("other/pv.yml"),
        desired_php_track: None,
        additional_hostnames: Vec::new(),
    })?;

    let duplicate_primary = database.validate_project_hostnames(
        &first.project.id,
        &first.project.primary_hostname,
        std::slice::from_ref(&first.project.primary_hostname),
    );
    assert!(matches!(
        duplicate_primary,
        Err(StateError::DuplicateProjectHostname { hostname }) if hostname == "acme.test"
    ));

    let collision = database.validate_project_hostnames(
        &second.project.id,
        &second.project.primary_hostname,
        std::slice::from_ref(&first.project.primary_hostname),
    );
    assert!(matches!(
        collision,
        Err(StateError::ProjectHostnameCollision {
            hostname,
            project_id,
        }) if hostname == "acme.test" && project_id == first.project.id
    ));

    Ok(())
}

#[test]
fn nearest_project_resolution_prefers_nested_projects() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let parent_path = tempdir.path().join("acme");
    let nested_path = parent_path.join("packages/admin");
    let nested_child_path = nested_path.join("src");

    database.link_project(state::LinkProjectInput {
        path: parent_path.clone(),
        original_path: parent_path,
        primary_hostname: "acme.test".to_string(),
        config_path: tempdir.path().join("acme/pv.yml"),
        desired_php_track: None,
        additional_hostnames: Vec::new(),
    })?;
    let nested = database.link_project(state::LinkProjectInput {
        path: nested_path.clone(),
        original_path: nested_path,
        primary_hostname: "admin.acme.test".to_string(),
        config_path: tempdir.path().join("acme/packages/admin/pv.yml"),
        desired_php_track: None,
        additional_hostnames: Vec::new(),
    })?;

    let resolved = database
        .nearest_project_for_path(&nested_child_path)?
        .ok_or_else(|| anyhow!("missing nearest project"))?;

    assert_eq!(resolved.id, nested.project.id);

    Ok(())
}

#[test]
fn recent_jobs_returns_the_latest_one_hundred_jobs() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    for job_number in 1..=105 {
        let job = database.start_job("test", &format!("scope_{job_number}"))?;
        database.complete_job(&job.id, "done")?;
    }

    let jobs = database.recent_jobs()?;
    let stored_job_count = state::testing::query_i64(&database, "SELECT COUNT(*) FROM jobs")?;

    assert_debug_snapshot!((
        jobs.len(),
        jobs.first().map(|job| job.id.as_str()),
        jobs.last().map(|job| job.id.as_str()),
        stored_job_count,
    ));

    Ok(())
}

#[test]
fn port_allocator_persists_reuses_avoids_collisions_and_releases_assignments() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let mysql = PortRequest::resource("mysql", "8.4", 3306, 45000, 45009);
    let redis = PortRequest::resource("redis", "8.6", 45000, 45000, 45009);

    let assigned_mysql = database.assign_port(mysql.clone(), |port| port != 3306)?;
    let reused_mysql = database.assign_port(mysql.clone(), |port| port == assigned_mysql.port)?;
    let assigned_redis = database.assign_port(redis, |_port| true)?;
    let released_mysql = database.release_port(PortOwner::Resource {
        name: "mysql".to_string(),
        track: "8.4".to_string(),
        port: "default".to_string(),
    })?;
    let reassigned_mysql = database.assign_port(mysql, |port| port != 3306)?;

    with_normalized_timestamps(|| {
        assert_debug_snapshot!((
            assigned_mysql,
            reused_mysql,
            assigned_redis,
            released_mysql,
            reassigned_mysql,
            database.assigned_ports()?,
        ));
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn resource_port_allocator_distinguishes_named_ports_for_one_track() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let smtp = PortRequest::resource_port("mailpit", "1.0", "smtp", 1025, 45000, 45009);
    let dashboard = PortRequest::resource_port("mailpit", "1.0", "dashboard", 8025, 45000, 45009);

    let assigned_smtp = database.assign_port(smtp.clone(), |_port| true)?;
    let assigned_dashboard = database.assign_port(dashboard.clone(), |_port| true)?;
    let reused_smtp = database.assign_port(smtp, |_port| true)?;
    let released_dashboard = database.release_port(PortOwner::Resource {
        name: "mailpit".to_string(),
        track: "1.0".to_string(),
        port: "dashboard".to_string(),
    })?;

    with_normalized_timestamps(|| {
        assert_debug_snapshot!((
            assigned_smtp,
            assigned_dashboard,
            reused_smtp,
            released_dashboard,
            database.assigned_ports()?,
        ));
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn legacy_port_insert_contract_survives_named_resource_port_migration() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    state::testing::transaction(&mut database, |transaction| {
        transaction.execute(
            "INSERT INTO ports (owner_kind, owner_id, owner_track, port, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(owner_kind, owner_id, owner_track) DO UPDATE SET
                port = excluded.port,
                updated_at = excluded.updated_at",
            params!["resource", "mysql", "8.0", 3306, "2026-06-08T00:00:00Z"],
        )?;
        transaction.execute(
            "INSERT INTO ports (owner_kind, owner_id, owner_track, port, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(owner_kind, owner_id, owner_track) DO UPDATE SET
                port = excluded.port,
                updated_at = excluded.updated_at",
            params!["resource", "mysql", "8.0", 3307, "2026-06-08T00:00:01Z"],
        )?;

        Ok(())
    })?;

    let smtp = PortRequest::resource_port("mailpit", "1.0", "smtp", 1025, 45000, 45009);
    let dashboard = PortRequest::resource_port("mailpit", "1.0", "dashboard", 8025, 45000, 45009);
    let assigned_smtp = database.assign_port(smtp, |_port| true)?;
    let assigned_dashboard = database.assign_port(dashboard, |_port| true)?;
    let legacy_mysql_rows = state::testing::query_i64(
        &database,
        "SELECT COUNT(*)
        FROM ports
        WHERE owner_kind = 'resource'
        AND owner_id = 'mysql'
        AND owner_track = '8.0'
        AND port = 3307",
    )?;

    with_normalized_timestamps(|| {
        assert_debug_snapshot!((
            legacy_mysql_rows,
            assigned_smtp,
            assigned_dashboard,
            database.assigned_ports()?,
        ));
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn dns_port_allocator_persists_and_reuses_preferred_assignment() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    let assigned_dns = database.assign_port(PortRequest::pv_dns(), |port| port == 35353)?;
    let reused_dns =
        database.assign_port(PortRequest::pv_dns(), |port| port == assigned_dns.port)?;
    let fallback_dns = {
        database.release_port(PortOwner::Dns)?;
        database.assign_port(PortRequest::pv_dns(), |port| port != 35353)?
    };

    with_normalized_timestamps(|| {
        assert_debug_snapshot!((
            assigned_dns,
            reused_dns,
            fallback_dns,
            database.assigned_ports()?,
        ));
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn gateway_port_allocator_persists_distinct_http_and_https_assignments() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    let assigned = database.assign_gateway_ports(|port| {
        port == GATEWAY_HTTP_PREFERRED_PORT || port == GATEWAY_HTTPS_PREFERRED_PORT
    })?;
    let reused = database.assign_gateway_ports(|_port| false)?;

    assert_eq!(assigned.http.port, GATEWAY_HTTP_PREFERRED_PORT);
    assert_eq!(assigned.https.port, GATEWAY_HTTPS_PREFERRED_PORT);
    assert_eq!(assigned.http.owner, PortOwner::Gateway(GatewayPort::Http));
    assert_eq!(assigned.https.owner, PortOwner::Gateway(GatewayPort::Https));
    assert_eq!(reused.http.port, assigned.http.port);
    assert_eq!(reused.https.port, assigned.https.port);

    with_normalized_timestamps(|| {
        assert_debug_snapshot!((assigned, reused, database.assigned_ports()?));
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn gateway_port_allocator_uses_fallbacks_when_preferred_ports_are_unavailable() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    let assigned = database.assign_gateway_ports(|port| {
        port != GATEWAY_HTTP_PREFERRED_PORT && port != GATEWAY_HTTPS_PREFERRED_PORT
    })?;

    assert_eq!(assigned.http.port, RUNTIME_PORT_FALLBACK_START);
    assert_eq!(assigned.https.port, RUNTIME_PORT_FALLBACK_START + 1);

    Ok(())
}

#[test]
fn php_worker_port_allocator_persists_one_port_per_track() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let php84 = PortRequest::php_worker("8.4", 45000, 45000, 45009);
    let php83 = PortRequest::php_worker("8.3", 45000, 45000, 45009);

    let assigned_php84 = database.assign_port(php84.clone(), |port| port == 45000)?;
    let reused_php84 = database.assign_port(php84, |_port| false)?;
    let assigned_php83 = database.assign_port(php83, |_port| true)?;
    let reserved_track = database.assign_port(
        PortRequest::php_worker("latest", 45000, 45000, 45009),
        |_port| true,
    );
    let invalid_track = database.assign_port(
        PortRequest::php_worker("../8.4", 45000, 45000, 45009),
        |_port| true,
    );

    assert_eq!(
        assigned_php84.owner,
        PortOwner::PhpWorker {
            php_track: "8.4".to_string()
        }
    );
    assert_eq!(assigned_php84.port, 45000);
    assert_eq!(reused_php84.port, assigned_php84.port);
    assert_eq!(
        assigned_php83.owner,
        PortOwner::PhpWorker {
            php_track: "8.3".to_string()
        }
    );
    assert_eq!(assigned_php83.port, 45001);
    assert!(matches!(
        reserved_track,
        Err(StateError::ReservedConcreteTrack { track }) if track == "latest"
    ));
    assert!(matches!(
        invalid_track,
        Err(StateError::InvalidManagedResourceIdentity { kind: "track", value })
            if value == "../8.4"
    ));

    with_normalized_timestamps(|| {
        assert_debug_snapshot!((
            assigned_php84,
            reused_php84,
            assigned_php83,
            database.assigned_ports()?
        ));
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn gateway_port_allocator_rolls_back_when_https_cannot_be_assigned() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    let error = match database.assign_gateway_ports(|port| port == GATEWAY_HTTP_PREFERRED_PORT) {
        Ok(assignments) => {
            return Err(anyhow!(
                "HTTPS assignment should fail, but assigned {assignments:?}"
            ));
        }
        Err(error) => error,
    };

    assert!(matches!(
        error,
        StateError::NoAvailablePort {
            name,
            preferred_port: GATEWAY_HTTPS_PREFERRED_PORT,
            fallback_start: RUNTIME_PORT_FALLBACK_START,
            fallback_end: RUNTIME_PORT_FALLBACK_END,
            ..
        } if name == "gateway https"
    ));
    assert_eq!(database.assigned_ports()?, Vec::new());

    Ok(())
}

#[test]
fn port_allocator_keeps_owner_components_structured() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let mysql_debug = PortRequest::resource_port("mysql", "8.4", "debug", 45000, 45000, 45009);
    let mysql_default =
        PortRequest::resource_port("mysql", "8.4-debug", "default", 45000, 45000, 45009);

    let assigned_mysql_debug = database.assign_port(mysql_debug, |_port| true)?;
    let assigned_mysql_default = database.assign_port(mysql_default, |_port| true)?;
    let released_mysql_debug = database.release_port(PortOwner::Resource {
        name: "mysql".to_string(),
        track: "8.4".to_string(),
        port: "debug".to_string(),
    })?;

    assert_eq!(
        assigned_mysql_debug.owner,
        PortOwner::Resource {
            name: "mysql".to_string(),
            track: "8.4".to_string(),
            port: "debug".to_string(),
        }
    );
    assert_eq!(assigned_mysql_debug.port, 45000);
    assert_eq!(
        assigned_mysql_default.owner,
        PortOwner::Resource {
            name: "mysql".to_string(),
            track: "8.4-debug".to_string(),
            port: "default".to_string(),
        }
    );
    assert_eq!(assigned_mysql_default.port, 45001);
    assert!(released_mysql_debug);
    assert_eq!(database.assigned_ports()?, vec![assigned_mysql_default]);

    Ok(())
}

#[test]
fn port_allocator_scans_the_full_documented_dns_fallback_range() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    let assigned_dns = database.assign_port(PortRequest::pv_dns(), |port| {
        port == RUNTIME_PORT_FALLBACK_END
    })?;

    assert_eq!(assigned_dns.port, RUNTIME_PORT_FALLBACK_END);

    Ok(())
}

#[test]
fn job_records_expose_typed_statuses() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;
    let job = database.start_job("test", "system")?;

    database.complete_job(&job.id, "done")?;

    let jobs = database.recent_jobs()?;

    assert!(matches!(
        jobs.first().map(|job| job.status),
        Some(JobStatus::Succeeded)
    ));

    Ok(())
}

#[test]
fn completing_unknown_job_returns_typed_error() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    let result = database.complete_job("missing_job", "done");

    assert!(matches!(
        result,
        Err(StateError::JobNotFound { id }) if id == "missing_job"
    ));

    Ok(())
}

#[test]
fn failing_unknown_job_returns_typed_error() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    let result = database.fail_job("missing_job", "failed");

    assert!(matches!(
        result,
        Err(StateError::JobNotFound { id }) if id == "missing_job"
    ));

    Ok(())
}

#[test]
fn recent_jobs_rejects_unknown_status_values() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    state::testing::transaction(&mut database, |transaction| {
        transaction.execute(
            "INSERT INTO jobs (id, kind, scope, status, started_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["job_1", "test", "system", "mystery", "2026-05-24T00:00:00Z"],
        )?;

        Ok(())
    })?;

    let result = database.recent_jobs();

    assert!(matches!(
        result,
        Err(StateError::UnknownJobStatus { status }) if status == "mystery"
    ));

    Ok(())
}

#[test]
fn transactions_roll_back_when_the_operation_fails() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;

    let mut database = Database::open(&paths)?;
    let result = state::testing::transaction(&mut database, |transaction| {
        transaction.execute(
            "INSERT INTO jobs (id, kind, scope, status, started_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["job_1", "test", "system", "running", "2026-05-23T00:00:00Z"],
        )?;
        transaction.execute(
            "INSERT INTO missing_table (id) VALUES (?1)",
            params!["boom"],
        )?;

        Ok(())
    });

    assert!(result.is_err());
    assert_debug_snapshot!(state::testing::query_i64(
        &database,
        "SELECT COUNT(*) FROM jobs"
    )?);

    Ok(())
}

fn link_test_project(
    database: &mut Database,
    root: &Utf8Path,
    directory_name: &str,
    primary_hostname: &str,
) -> Result<ProjectRecord> {
    let path = root.join(directory_name);

    Ok(database
        .link_project(state::LinkProjectInput {
            path: path.clone(),
            original_path: path.clone(),
            primary_hostname: primary_hostname.to_string(),
            config_path: path.join("pv.yml"),
            desired_php_track: None,
            additional_hostnames: Vec::new(),
        })?
        .project)
}

fn env_context(values: &[(&str, &str)]) -> EnvContextValues {
    values
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect()
}

fn set_managed_resource_env_json(database: &mut Database, env_json: &str) -> Result<()> {
    state::testing::transaction(database, |transaction| {
        transaction.execute(
            "UPDATE managed_resource_tracks
            SET env_json = ?1
            WHERE resource_name = ?2
            AND track = ?3",
            params![env_json, "redis", "7.2"],
        )?;

        Ok(())
    })?;

    Ok(())
}

fn set_resource_allocation_env_json(
    database: &mut Database,
    project_id: &str,
    resource_name: &str,
    allocation_name: &str,
    env_json: &str,
) -> Result<()> {
    state::testing::transaction(database, |transaction| {
        transaction.execute(
            "UPDATE resource_allocations
            SET env_json = ?1
            WHERE project_id = ?2
            AND resource_name = ?3
            AND allocation_name = ?4",
            params![env_json, project_id, resource_name, allocation_name],
        )?;

        Ok(())
    })?;

    Ok(())
}

fn migration_backup_table_counts(paths: &PvPaths) -> Result<Vec<i64>> {
    let mut table_counts = Vec::new();

    for backup_name in state::fs::migration_backups(paths)? {
        let connection = Connection::open(paths.root().join(backup_name))?;
        table_counts.push(connection.query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name GLOB 'm[0-9]*'",
            [],
            |row| row.get(0),
        )?);
    }

    Ok(table_counts)
}

fn table_exists(connection: &Connection, table: &str) -> Result<bool> {
    let count = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = ?1",
        params![table],
        |row| row.get::<_, i64>(0),
    )?;

    Ok(count > 0)
}

fn applied_migration_count(connection: &Connection) -> Result<i64> {
    Ok(connection.query_row("SELECT COUNT(*) FROM pv_migrations", [], |row| row.get(0))?)
}
