use std::process::ExitCode;

use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::{Settings, assert_debug_snapshot};
use state::Database;
use support::resource_cli::{
    ResourceCliSpec, ScriptedClient, TestEnvironment, create_dir, fixture_artifact,
    managed_resource_records, prepare_existing_release, pv_paths, record_installed_resource,
    resource_manifest, resource_record_snapshots, run_pv, seed_running_resource,
};

mod support;

const RESOURCE: ResourceCliSpec = ResourceCliSpec {
    resource_name: "mysql",
    executable_path: "bin/mysqld",
    support_files: &[],
};
const DEFAULT_TRACK: &str = "8.0";
const OLD_VERSION: &str = "8.0.35-pv1";
const NEW_VERSION: &str = "8.0.36-pv1";

#[test]
fn mysql_install_uses_manifest_default_and_installs_without_network_download() -> anyhow::Result<()>
{
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

    let output = run_pv(&["mysql:install"], &environment)?;
    let database = Database::open(&pv_paths(&home))?;
    let records = managed_resource_records(&database, RESOURCE)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_resource_snapshot(
        "mysql_install_uses_manifest_default_and_installs_without_network_download",
        tempdir.path(),
        &(
            output,
            resource_record_snapshots(&records, tempdir.path())?,
            environment.text_request_count(),
            environment.byte_request_count(),
        ),
    );

    Ok(())
}

#[test]
fn mysql_update_updates_installed_tracks() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let old_artifact = fixture_artifact(OLD_VERSION);
    let new_artifact = fixture_artifact(NEW_VERSION);
    record_installed_resource(&home, DEFAULT_TRACK, &old_artifact, RESOURCE)?;
    prepare_existing_release(&home, DEFAULT_TRACK, &new_artifact, RESOURCE)?;
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        ScriptedClient::new().with_text(&resource_manifest(
            DEFAULT_TRACK,
            &[&new_artifact],
            RESOURCE,
        )),
    );

    let output = run_pv(&["mysql:update"], &environment)?;
    let database = Database::open(&pv_paths(&home))?;
    let records = managed_resource_records(&database, RESOURCE)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_resource_snapshot(
        "mysql_update_updates_installed_tracks",
        tempdir.path(),
        &(
            output,
            resource_record_snapshots(&records, tempdir.path())?,
            environment.text_request_count(),
            environment.byte_request_count(),
        ),
    );

    Ok(())
}

#[test]
fn mysql_list_reports_running_state_ports_and_usage() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let paths = pv_paths(&home);
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let artifact = fixture_artifact(NEW_VERSION);
    record_installed_resource(&home, DEFAULT_TRACK, &artifact, RESOURCE)?;
    seed_running_resource(&paths, DEFAULT_TRACK, "tcp", 3306, RESOURCE)?;
    let environment = TestEnvironment::new(&home, &current_dir, ScriptedClient::new());

    let output = run_pv(&["mysql:list"], &environment)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_resource_snapshot(
        "mysql_list_reports_running_state_ports_and_usage",
        tempdir.path(),
        &output,
    );

    Ok(())
}

#[test]
fn mysql_uninstall_force_prune_queues_removal_intent() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let artifact = fixture_artifact(NEW_VERSION);
    record_installed_resource(&home, DEFAULT_TRACK, &artifact, RESOURCE)?;
    let environment = TestEnvironment::new(&home, &current_dir, ScriptedClient::new());

    let output = run_pv(
        &["mysql:uninstall", DEFAULT_TRACK, "--force", "--prune"],
        &environment,
    )?;
    let database = Database::open(&pv_paths(&home))?;
    let records = managed_resource_records(&database, RESOURCE)?;

    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stderr.is_empty());
    assert_resource_snapshot(
        "mysql_uninstall_force_prune_queues_removal_intent",
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
