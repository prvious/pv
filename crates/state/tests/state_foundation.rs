use anyhow::{Result, anyhow};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::{Settings, assert_debug_snapshot};
use rusqlite::{Connection, params};
use state::{Database, Migration, PvPaths};

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
    Database::open_with_migrations(&paths, &first_migration)?;
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
    Database::open_with_migrations(&paths, &second_migration)?;

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
    let mut database = Database::open_with_migrations(&paths, &first_migration)?;
    database.transaction(|transaction| {
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
    Database::open_with_migrations(&paths, &second_migration)?;

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
        Database::open_with_migrations(&paths, &migrations[..migration_count])?;
    }

    assert_debug_snapshot!(state::fs::migration_backups(&paths)?.len());

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
fn transactions_roll_back_when_the_operation_fails() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;

    let mut database = Database::open(&paths)?;
    let result = database.transaction(|transaction| {
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
    assert_debug_snapshot!(database.query_i64("SELECT COUNT(*) FROM jobs")?);

    Ok(())
}
