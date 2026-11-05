use std::process::ExitCode;

use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::{Settings, assert_debug_snapshot};
use state::Database;
use support::resource_cli::{
    ResourceCliSpec, ScriptedClient, TestEnvironment, create_dir, fixture_artifact,
    managed_resource_records, prepare_existing_release, pv_paths, resource_manifest,
    resource_record_snapshots,
};

use crate::support;
use crate::support::run_pv;

const POSTGRES_SUPPORT_FILES: &[(&str, &str)] = &[
    ("bin/initdb", "fixture initdb\n"),
    ("share/postgres.bki", "fixture bki\n"),
    ("lib/libcrypto.3.dylib", "fixture libcrypto\n"),
    ("lib/libssl.3.dylib", "fixture libssl\n"),
    ("lib/postgresql/pg_trgm.dylib", "fixture pg_trgm\n"),
    ("lib/postgresql/pgcrypto.dylib", "fixture pgcrypto\n"),
    ("lib/postgresql/sslinfo.dylib", "fixture sslinfo\n"),
    (
        "share/extension/pg_trgm.control",
        "fixture pg_trgm control\n",
    ),
    (
        "share/extension/pgcrypto.control",
        "fixture pgcrypto control\n",
    ),
    (
        "share/extension/sslinfo.control",
        "fixture sslinfo control\n",
    ),
];
const RESOURCE: ResourceCliSpec = ResourceCliSpec {
    resource_name: "postgres",
    executable_path: "bin/postgres",
    support_files: POSTGRES_SUPPORT_FILES,
};
const DEFAULT_TRACK: &str = "16";
const NEW_VERSION: &str = "16.4-pv1";

#[test]
fn pg_alias_install_records_canonical_postgres_resource() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let artifact = fixture_artifact(NEW_VERSION);
    prepare_existing_release(&home, DEFAULT_TRACK, &artifact, RESOURCE)?;
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        ScriptedClient::new().with_text(&resource_manifest(DEFAULT_TRACK, &[&artifact], RESOURCE)),
    );

    let output = run_pv(&["pg:install", DEFAULT_TRACK], &environment)?;
    let database = Database::open(&pv_paths(&home))?;
    let records = managed_resource_records(&database, RESOURCE)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(!output.stderr.contains("error:"));
    assert_resource_snapshot(
        "pg_alias_install_records_canonical_postgres_resource",
        tempdir.path(),
        &(output, resource_record_snapshots(&records, tempdir.path())?),
    );

    Ok(())
}

fn assert_resource_snapshot(
    name: &'static str,
    tempdir: &Utf8Path,
    snapshot: &impl std::fmt::Debug,
) {
    let mut settings = Settings::clone_current();
    settings.add_filter(tempdir.as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!(name, snapshot);
    });
}
