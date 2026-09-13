#[cfg(target_os = "macos")]
use std::io::{BufRead, BufReader, Write, copy, sink};
#[cfg(target_os = "macos")]
use std::net::Shutdown;
#[cfg(target_os = "macos")]
use std::os::unix::net::UnixListener;
use std::process::ExitCode;
#[cfg(target_os = "macos")]
use std::thread;
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::{Settings, assert_debug_snapshot};
use state::{Database, JobsLock};
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

const DIRECT_ARTIFACT_MUTATIONS: &[&[&str]] = &[
    &["mysql:install"],
    &["mysql:update"],
    &["composer:install"],
    &["composer:update"],
    &["php:install"],
    &["php:update"],
    &["php:use", "8.4", "--global"],
];

#[test]
fn direct_artifact_mutations_reject_active_daemon_jobs_lock() -> anyhow::Result<()> {
    let mut outputs = Vec::new();

    for command in DIRECT_ARTIFACT_MUTATIONS {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let current_dir = tempdir.path().join("outside");
        create_dir(&current_dir)?;
        let paths = pv_paths(&home);
        let _jobs_lock = JobsLock::acquire(&paths)?;
        let environment = TestEnvironment::new(&home, &current_dir, ScriptedClient::new());

        let output = run_pv(command, &environment)?;

        outputs.push((
            command.join(" "),
            format!("{output:?}").replace(tempdir.path().as_str(), "<tempdir>"),
        ));
        assert_eq!(environment.text_request_count(), 0, "{command:?}");
        assert_eq!(environment.byte_request_count(), 0, "{command:?}");
        let database = Database::open(&paths)?;
        assert!(
            database.managed_resource_tracks()?.is_empty(),
            "{command:?}"
        );
        assert_eq!(database.global_php_default_track()?, None, "{command:?}");
    }

    assert_debug_snapshot!(outputs);

    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "CLI test runs a bounded fake Unix socket daemon"
)]
fn artifact_mutation_releases_jobs_lock_before_daemon_submission() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let current_dir = tempdir.path().join("outside");
    create_dir(&current_dir)?;
    let paths = pv_paths(&home);
    let artifact = fixture_artifact(NEW_VERSION);
    prepare_existing_release(&home, DEFAULT_TRACK, &artifact, RESOURCE)?;
    state::fs::ensure_layout(&paths)?;
    let listener = UnixListener::bind(paths.daemon_socket().as_std_path())?;
    listener.set_nonblocking(true)?;
    let daemon_paths = paths.clone();
    let daemon_thread = thread::spawn(move || -> anyhow::Result<serde_json::Value> {
        let deadline = Instant::now() + Duration::from_secs(3);
        let (mut stream, _address) = loop {
            match listener.accept() {
                Ok(accepted) => break accepted,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error.into()),
            }
        };
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
        let mut request = String::new();
        BufReader::new(stream.try_clone()?).read_line(&mut request)?;
        let _jobs_lock = JobsLock::acquire(&daemon_paths)?;
        let mut response = format!(
            r#"{{"type":"response","protocol_version":{},"status":"accepted","message":"job accepted","job_id":"job_lock_1"}}"#,
            daemon::PROTOCOL_VERSION
        );
        response.push('\n');
        stream.write_all(response.as_bytes())?;
        stream.shutdown(Shutdown::Write)?;
        copy(&mut stream, &mut sink())?;

        Ok(serde_json::from_str(request.trim_end())?)
    });
    let environment = TestEnvironment::new(
        &home,
        &current_dir,
        ScriptedClient::new().with_text(&resource_manifest(DEFAULT_TRACK, &[&artifact], RESOURCE)),
    );

    let output = run_pv(&["mysql:install"], &environment)?;
    let request = daemon_thread
        .join()
        .map_err(|_error| anyhow::anyhow!("fake daemon thread panicked"))??;
    let records = managed_resource_records(&Database::open(&paths)?, RESOURCE)?;

    assert_resource_snapshot(
        "artifact_mutation_releases_jobs_lock_before_daemon_submission",
        tempdir.path(),
        &(
            output,
            request,
            resource_record_snapshots(&records, tempdir.path())?,
        ),
    );

    Ok(())
}

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
