use anyhow::{Result, anyhow};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::{Settings, assert_debug_snapshot};
use rusqlite::{Connection, params};
use state::testing::Migration;
use state::{Database, PvPaths, StateError};

#[test]
fn paths_are_derived_from_an_injected_home() -> Result<()> {
    let paths = PvPaths::for_home(Utf8Path::new("/tmp/pv-test-home"));

    assert_debug_snapshot!(paths.summary());

    Ok(())
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

    assert_debug_snapshot!(backup_table_counts);

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
fn resource_allocations_reject_duplicate_generated_names_per_resource_track() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    state::testing::transaction(&mut database, |transaction| {
        transaction.execute(
            "INSERT INTO projects (id, path, primary_hostname, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                "project_1",
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
fn primary_project_hostname_rows_must_match_the_project() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let mut database = Database::open(&paths)?;

    state::testing::transaction(&mut database, |transaction| {
        transaction.execute(
            "INSERT INTO projects (id, path, primary_hostname, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
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

    let mismatched_project_update = state::testing::transaction(&mut database, |transaction| {
        transaction.execute(
            "UPDATE projects SET primary_hostname = ?1 WHERE id = ?2",
            params!["renamed.test", "project_1"],
        )?;

        Ok(())
    });

    assert!(mismatched_project_update.is_err());

    let delete_primary = state::testing::transaction(&mut database, |transaction| {
        transaction.execute(
            "DELETE FROM project_hostnames WHERE hostname = ?1",
            params!["acme.test"],
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
