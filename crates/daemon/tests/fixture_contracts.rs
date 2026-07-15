use std::os::unix::fs::PermissionsExt;

use anyhow::{Result, bail};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::{Settings, assert_debug_snapshot};

const MYSQL_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/mysql.py"
));
const FAKE_MAILPIT_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/fake-mailpit.py"
));
const POSTGRES_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/postgres.py"
));
const MAILPIT_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/mailpit.py"
));
const RUSTFS_FIXTURE_TEMPLATE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/managed-resources/rustfs.py.in"
));
const RUSTFS_REJECT_S3_SENTINEL: &str = "__PV_REJECT_S3__";

#[expect(
    clippy::disallowed_types,
    reason = "daemon fixture contract tests execute materialized test programs"
)]
type FixtureCommand = std::process::Command;

#[derive(Debug)]
struct FixtureOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

#[test]
fn mysql_fixture_cli_preserves_shell_contract() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("mysqld");
    let rejected_data_dir = tempdir.path().join("rejected-data");
    let first_data_dir = tempdir.path().join("first-data");
    let selected_data_dir = tempdir.path().join("selected-data");

    materialize_fixture(&fixture, MYSQL_FIXTURE)?;

    let first_argument_failure = run_fixture(
        &fixture,
        &[
            "--initialize-insecure",
            "--no-defaults",
            "--datadir",
            rejected_data_dir.as_str(),
        ],
        tempdir.path(),
    )?;
    let successful_initialization = run_fixture(
        &fixture,
        &[
            "--no-defaults",
            "--bind-address=127.0.0.1",
            "--future-option",
            "--initialize-insecure",
            "--datadir",
            first_data_dir.as_str(),
            "--datadir",
            selected_data_dir.as_str(),
            "--basedir",
            tempdir.path().as_str(),
        ],
        tempdir.path(),
    )?;

    assert_fixture_snapshot(
        tempdir.path(),
        "mysql_fixture_cli_preserves_shell_contract",
        (
            first_argument_failure,
            successful_initialization,
            path_exists(&rejected_data_dir.join("mysql"))?,
            path_exists(&first_data_dir.join("mysql"))?,
            path_exists(&selected_data_dir.join("mysql"))?,
        ),
    )
}

#[test]
fn fake_mailpit_fixture_cli_ignores_extra_arguments() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("fake-mailpit");

    materialize_fixture(&fixture, FAKE_MAILPIT_FIXTURE)?;
    let output = run_fixture(
        &fixture,
        &["not-a-port", "also-not-a-port", "ignored-extra"],
        tempdir.path(),
    )?;

    assert_fixture_snapshot(
        tempdir.path(),
        "fake_mailpit_fixture_cli_ignores_extra_arguments",
        (
            output.code,
            output.stdout,
            output.stderr.contains("ValueError"),
            output.stderr.contains("ignored-extra"),
        ),
    )
}

#[test]
fn postgres_fixture_cli_preserves_shell_contract() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("postgres");
    let initialized_data_dir = tempdir.path().join("initialized-postgres");
    let selected_missing_data_dir = tempdir.path().join("selected-missing-postgres");

    materialize_fixture(&fixture, POSTGRES_FIXTURE)?;
    state::fs::write_sensitive_file(&initialized_data_dir.join("PG_VERSION"), "16\n")?;

    let unknown_argument = run_fixture(&fixture, &["--unexpected"], tempdir.path())?;
    let last_data_dir_wins = run_fixture(
        &fixture,
        &[
            "-D",
            initialized_data_dir.as_str(),
            "-D",
            selected_missing_data_dir.as_str(),
            "-h",
            "127.0.0.1",
            "-p",
            "5432",
        ],
        tempdir.path(),
    )?;

    assert_fixture_snapshot(
        tempdir.path(),
        "postgres_fixture_cli_preserves_shell_contract",
        (unknown_argument, last_data_dir_wins),
    )
}

#[test]
fn mailpit_fixture_cli_preserves_shell_contract() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("mailpit");
    let missing_database = tempdir.path().join("missing/mailpit.db");

    materialize_fixture(&fixture, MAILPIT_FIXTURE)?;

    let unknown_argument = run_fixture(&fixture, &["--unexpected"], tempdir.path())?;
    let missing_required_arguments =
        run_fixture(&fixture, &["--disable-version-check"], tempdir.path())?;
    let missing_version_check = run_fixture(
        &fixture,
        &[
            "--smtp",
            "127.0.0.1:1025",
            "--listen",
            "127.0.0.1:8025",
            "--database",
            missing_database.as_str(),
        ],
        tempdir.path(),
    )?;
    let invalid_database_path = run_fixture(
        &fixture,
        &[
            "--smtp",
            "127.0.0.1:1025",
            "--listen",
            "127.0.0.1:8025",
            "--database",
            "mailpit.db",
            "--disable-version-check",
        ],
        tempdir.path(),
    )?;
    let missing_database_directory = run_fixture(
        &fixture,
        &[
            "--smtp",
            "127.0.0.1:1025",
            "--listen",
            "127.0.0.1:8025",
            "--database",
            missing_database.as_str(),
            "--disable-version-check",
        ],
        tempdir.path(),
    )?;
    let duplicate_database_last_wins = run_fixture(
        &fixture,
        &[
            "--smtp",
            "127.0.0.1:1025",
            "--listen",
            "127.0.0.1:8025",
            "--database",
            "mailpit.db",
            "--database",
            missing_database.as_str(),
            "--disable-version-check",
        ],
        tempdir.path(),
    )?;

    assert_fixture_snapshot(
        tempdir.path(),
        "mailpit_fixture_cli_preserves_shell_contract",
        (
            unknown_argument,
            missing_required_arguments,
            missing_version_check,
            invalid_database_path,
            missing_database_directory,
            duplicate_database_last_wins,
        ),
    )
}

#[test]
fn rustfs_fixture_cli_preserves_shell_contract() -> Result<()> {
    let tempdir = tempdir()?;
    let fixture = tempdir.path().join("rustfs");
    let first_data_dir = tempdir.path().join("first-rustfs-data");
    let selected_data_dir = tempdir.path().join("selected-rustfs-data");
    let rendered = render_rustfs_fixture(false)?;

    materialize_fixture(&fixture, &rendered)?;
    let output = run_fixture(
        &fixture,
        &[
            first_data_dir.as_str(),
            "--future-option",
            selected_data_dir.as_str(),
            "--address",
            "invalid-api-address",
            "--console-address",
            "invalid-console-address",
        ],
        tempdir.path(),
    )?;

    assert_fixture_snapshot(
        tempdir.path(),
        "rustfs_fixture_cli_preserves_shell_contract",
        (
            output.code,
            output.stdout,
            output.stderr.contains("ValueError"),
            path_exists(&first_data_dir)?,
            path_exists(&selected_data_dir.join("buckets"))?,
            path_exists(&selected_data_dir.join("process-env"))?,
            path_exists(&tempdir.path().join("invalid-api-address"))?,
            path_exists(&tempdir.path().join("invalid-console-address"))?,
            rendered.contains(RUSTFS_REJECT_S3_SENTINEL),
        ),
    )
}

fn render_rustfs_fixture(reject_s3: bool) -> Result<String> {
    let occurrence_count = RUSTFS_FIXTURE_TEMPLATE
        .matches(RUSTFS_REJECT_S3_SENTINEL)
        .count();
    if occurrence_count != 1 {
        bail!(
            "RustFS fixture must contain exactly one {RUSTFS_REJECT_S3_SENTINEL} sentinel; found {occurrence_count}"
        );
    }

    let replacement = if reject_s3 { "True" } else { "False" };
    let rendered = RUSTFS_FIXTURE_TEMPLATE.replacen(RUSTFS_REJECT_S3_SENTINEL, replacement, 1);
    if rendered.contains(RUSTFS_REJECT_S3_SENTINEL) {
        bail!("RustFS fixture still contains {RUSTFS_REJECT_S3_SENTINEL} after rendering");
    }

    Ok(rendered)
}

fn materialize_fixture(path: &Utf8Path, source: &str) -> Result<()> {
    state::fs::write_sensitive_file(path, source)?;
    set_executable(path)
}

fn run_fixture(
    path: &Utf8Path,
    arguments: &[&str],
    current_dir: &Utf8Path,
) -> Result<FixtureOutput> {
    let output = FixtureCommand::new(path.as_std_path())
        .args(arguments)
        .current_dir(current_dir)
        .output()?;

    Ok(FixtureOutput {
        code: output.status.code(),
        stdout: String::from_utf8(output.stdout)?,
        stderr: String::from_utf8(output.stderr)?,
    })
}

fn assert_fixture_snapshot(
    tempdir: &Utf8Path,
    name: &'static str,
    snapshot: impl std::fmt::Debug,
) -> Result<()> {
    let mut settings = Settings::clone_current();
    settings.add_filter(&regex_literal(tempdir.as_str()), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!(name, snapshot);
        Ok::<(), anyhow::Error>(())
    })
}

#[expect(
    clippy::disallowed_methods,
    reason = "daemon fixture contract tests inspect fixture filesystem effects directly"
)]
fn path_exists(path: &Utf8Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "daemon fixture contract tests set materialized fixture executable bits directly"
)]
fn set_executable(path: &Utf8Path) -> Result<()> {
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)?;

    Ok(())
}

fn regex_literal(value: &str) -> String {
    let mut literal = String::new();

    for character in value.chars() {
        if matches!(
            character,
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$'
        ) {
            literal.push('\\');
        }
        literal.push(character);
    }

    literal
}
