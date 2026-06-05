use anyhow::Result;
use camino_tempfile::tempdir;
use insta::assert_debug_snapshot;
use platform::{LaunchAgentConfig, inspect_launch_agent_file};
use state::fs;

#[test]
fn launch_agent_config_renders_pv_owned_plist() {
    let config = LaunchAgentConfig::new(
        "/Users/alice/.pv/bin/pv",
        "/Users/alice/.pv/logs/launchd.out.log",
        "/Users/alice/.pv/logs/launchd.err.log",
    );
    let rendered = config.render();

    assert_eq!(LaunchAgentConfig::parse(&rendered), Some(config));
    assert_debug_snapshot!(rendered);
}

#[test]
fn launch_agent_file_inspection_reports_missing_current_stale_conflict_and_unreadable() -> Result<()>
{
    let tempdir = tempdir()?;
    let current_path = tempdir.path().join("current.plist");
    let stale_path = tempdir.path().join("stale.plist");
    let conflict_path = tempdir.path().join("conflict.plist");
    let malformed_path = tempdir.path().join("malformed.plist");
    let unreadable_path = tempdir.path().join("directory.plist");
    let expected = LaunchAgentConfig::new(
        "/Users/alice/.pv/bin/pv",
        "/Users/alice/.pv/logs/launchd.out.log",
        "/Users/alice/.pv/logs/launchd.err.log",
    );
    let stale = LaunchAgentConfig::new(
        "/tmp/old-pv",
        "/Users/alice/.pv/logs/launchd.out.log",
        "/Users/alice/.pv/logs/launchd.err.log",
    );

    fs::write_sensitive_file(&current_path, &expected.render())?;
    fs::write_sensitive_file(&stale_path, &stale.render())?;
    fs::write_sensitive_file(
        &conflict_path,
        &expected.render().replace("<!-- Managed by PV -->\n", ""),
    )?;
    fs::write_sensitive_file(
        &malformed_path,
        "<!-- Managed by PV -->\n<plist><dict></dict></plist>\n",
    )?;
    fs::write_sensitive_file(&unreadable_path.join("child"), "child\n")?;

    let states = vec![
        inspect_launch_agent_file(&tempdir.path().join("missing.plist"), Some(&expected)),
        inspect_launch_agent_file(&current_path, Some(&expected)),
        inspect_launch_agent_file(&stale_path, Some(&expected)),
        inspect_launch_agent_file(&conflict_path, Some(&expected)),
        inspect_launch_agent_file(&malformed_path, Some(&expected)),
        inspect_launch_agent_file(&unreadable_path, Some(&expected)),
    ];

    let normalized_states = states
        .into_iter()
        .map(|state| {
            let state = format!("{state:?}").replace(tempdir.path().as_str(), "[tempdir]");
            if let Some((prefix, _message)) = state.split_once(", message: ")
                && prefix.starts_with("Unreadable ")
            {
                return format!("{prefix}, message: \"<read error>\" }}");
            }

            state
        })
        .collect::<Vec<_>>();

    assert_debug_snapshot!(normalized_states);

    Ok(())
}
