#![cfg(target_os = "linux")]

use anyhow::Result;
use camino_tempfile::tempdir;
use insta::assert_debug_snapshot;
use platform::{DaemonRegistrationConfig, inspect_daemon_registration_file};
use state::{PvPaths, fs};

#[test]
fn daemon_unit_inspection_reports_owned_drift_foreign_and_unreadable_files() -> Result<()> {
    let tempdir = tempdir()?;
    let config = DaemonRegistrationConfig::for_paths(&PvPaths::for_home(tempdir.path()));
    let rendered = config.render()?;
    let current = tempdir.path().join("current.service");
    let stale = tempdir.path().join("stale.service");
    let foreign = tempdir.path().join("foreign.service");
    let unreadable = tempdir.path().join("directory.service");
    fs::write_sensitive_file(&current, &rendered)?;
    fs::write_sensitive_file(&stale, &rendered.replace("RestartSec=1", "RestartSec=2"))?;
    fs::write_sensitive_file(&foreign, &rendered.replace("# Managed by PV\n", ""))?;
    fs::write_sensitive_file(&unreadable.join("child"), "child\n")?;

    let states = [
        tempdir.path().join("missing.service"),
        current,
        stale,
        foreign,
        unreadable,
    ]
    .map(|path| inspect_daemon_registration_file(&path, Some(&config)))
    .map(|state| {
        let state = format!("{state:?}").replace(tempdir.path().as_str(), "<tempdir>");
        if let Some((prefix, _message)) = state.split_once(", message: ")
            && prefix.starts_with("Unreadable ")
        {
            return format!("{prefix}, message: \"<read error>\" }}");
        }
        state
    });
    assert_debug_snapshot!(states, @r#"
    [
        "Missing { path: \"<tempdir>/missing.service\" }",
        "Current { path: \"<tempdir>/current.service\", value: DaemonRegistrationConfig }",
        "Stale { path: \"<tempdir>/stale.service\", expected: Some(DaemonRegistrationConfig), actual: None }",
        "Conflict { path: \"<tempdir>/foreign.service\" }",
        "Unreadable { path: \"<tempdir>/directory.service\", message: \"<read error>\" }",
    ]
    "#);
    Ok(())
}
