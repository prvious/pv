use std::fmt::Debug;

use anyhow::Result;
use camino_tempfile::tempdir;
use insta::assert_debug_snapshot;
use macos::{ResolverConfig, inspect_resolver_file};
use state::fs;

#[test]
fn resolver_config_renders_pv_owned_test_resolver_file() {
    let config = ResolverConfig::new(35353);
    let rendered = config.render();

    assert_debug_snapshot!(&rendered);
    assert_eq!(ResolverConfig::parse(&rendered), Some(config));
}

#[test]
fn resolver_file_inspection_reports_missing_current_stale_conflict_and_unreadable() -> Result<()> {
    let tempdir = tempdir()?;
    let current_path = tempdir.path().join("current");
    let stale_path = tempdir.path().join("stale");
    let wrong_nameserver_path = tempdir.path().join("wrong-nameserver");
    let mixed_nameservers_path = tempdir.path().join("mixed-nameservers");
    let conflict_path = tempdir.path().join("conflict");
    let unreadable_path = tempdir.path().join("directory");
    let expected = ResolverConfig::new(35353);

    fs::write_sensitive_file(&current_path, &expected.render())?;
    fs::write_sensitive_file(&stale_path, &ResolverConfig::new(45000).render())?;
    fs::write_sensitive_file(
        &wrong_nameserver_path,
        "# Managed by PV\n# Source: PV prepared resolver config for /etc/resolver/test\nnameserver 10.0.0.1\nport 35353\n",
    )?;
    fs::write_sensitive_file(
        &mixed_nameservers_path,
        "# Managed by PV\n# Source: PV prepared resolver config for /etc/resolver/test\nnameserver 127.0.0.1\nnameserver 10.0.0.1\nport 35353\n",
    )?;
    fs::write_sensitive_file(&conflict_path, "nameserver 127.0.0.1\nport 35353\n")?;
    fs::write_sensitive_file(&unreadable_path.join("child"), "child\n")?;

    let states = vec![
        inspect_resolver_file(&tempdir.path().join("missing"), Some(&expected)),
        inspect_resolver_file(&current_path, Some(&expected)),
        inspect_resolver_file(&stale_path, Some(&expected)),
        inspect_resolver_file(&wrong_nameserver_path, Some(&expected)),
        inspect_resolver_file(&mixed_nameservers_path, Some(&expected)),
        inspect_resolver_file(&conflict_path, Some(&expected)),
        inspect_resolver_file(&unreadable_path, Some(&expected)),
    ];

    let normalized_states = states
        .into_iter()
        .map(|state| normalize_state_debug(&state, tempdir.path().as_str()))
        .collect::<Vec<_>>();

    assert_debug_snapshot!(normalized_states);

    Ok(())
}

fn normalize_state_debug(state: &impl Debug, tempdir_path: &str) -> String {
    let state_debug = format!("{state:?}").replace(tempdir_path, "[tempdir]");

    if let Some((prefix, _message)) = state_debug.split_once(", message: ")
        && prefix.starts_with("Unreadable ")
    {
        return format!("{prefix}, message: \"<read error>\" }}");
    }

    state_debug
}
