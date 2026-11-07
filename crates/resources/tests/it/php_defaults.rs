use std::collections::BTreeMap;
use std::result::Result as StdResult;

use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::assert_debug_snapshot;
use resources::{
    PHP_TRACK_DEFAULT_INI, PhpTrackDefaults, ensure_php_track_defaults, php_track_defaults,
    php_track_environment, php_track_exec_environment,
};
use state::{PvPaths, StateError, fs};

#[test]
fn php_track_defaults_seed_stripped_sample_once() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let defaults = ensure_php_track_defaults(&paths, "8.4")?;

    assert_eq!(
        fs::read_to_string(defaults.php_ini())?,
        PHP_TRACK_DEFAULT_INI
    );

    fs::write_sensitive_file(defaults.php_ini(), "memory_limit = 768M\n")?;
    let seeded_again = ensure_php_track_defaults(&paths, "8.4")?;

    assert_eq!(seeded_again, defaults);
    assert_eq!(
        fs::read_to_string(defaults.php_ini())?,
        "memory_limit = 768M\n"
    );

    Ok(())
}

#[test]
fn php_track_defaults_reject_unsupported_tracks() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));

    assert_invalid_track(ensure_php_track_defaults(&paths, "8.2"), "8.2")?;
    assert_invalid_track(php_track_defaults(&paths, "8.2"), "8.2")?;
    assert_invalid_track(php_track_environment(&paths, "8.2"), "8.2")?;
    assert_invalid_track(php_track_exec_environment(&paths, "8.2"), "8.2")?;

    Ok(())
}

#[test]
fn php_track_defaults_reject_blocking_paths() -> Result<()> {
    let tempdir = tempdir()?;
    let symlink_root = tempdir.path().join("php_ini_is_a_symlink");

    let errors = [
        (
            "conf_d_is_a_file",
            blocked_defaults_error(&tempdir.path().join("conf_d_is_a_file"), |defaults| {
                fs::write_sensitive_file(defaults.conf_dir(), "not a directory\n")?;
                Ok(())
            })?,
        ),
        (
            "php_ini_is_a_directory",
            blocked_defaults_error(&tempdir.path().join("php_ini_is_a_directory"), |defaults| {
                fs::ensure_user_dir(defaults.php_ini())?;
                Ok(())
            })?,
        ),
        (
            "php_ini_is_a_symlink",
            blocked_defaults_error(&symlink_root, |defaults| {
                let target = symlink_root.join("linked-php.ini");
                fs::write_sensitive_file(&target, "memory_limit = 768M\n")?;
                fs::symlink_file(&target, defaults.php_ini())?;
                Ok(())
            })?,
        ),
    ];

    assert_debug_snapshot!(errors);

    Ok(())
}

/// Blocks one PHP track defaults path with `block`, then returns where and why seeding failed.
fn blocked_defaults_error(
    root: &Utf8Path,
    block: impl FnOnce(&PhpTrackDefaults) -> Result<()>,
) -> Result<(String, String)> {
    let paths = PvPaths::for_home(root.join("home"));
    let defaults = php_track_defaults(&paths, "8.5")?;
    fs::ensure_user_dir(defaults.etc_dir())?;
    block(&defaults)?;

    match ensure_php_track_defaults(&paths, "8.5") {
        Err(StateError::Filesystem { path, source }) => {
            Ok((path.strip_prefix(root)?.to_string(), source.to_string()))
        }
        other => anyhow::bail!("expected blocked PHP track defaults to fail, got {other:?}"),
    }
}

#[test]
fn php_track_defaults_env_helpers_point_at_track_etc() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));

    assert_eq!(
        php_track_environment(&paths, "8.3")?,
        BTreeMap::from([
            (
                "PHPRC".to_owned(),
                paths.resources().join("php/8.3/etc").to_string(),
            ),
            (
                "PHP_INI_SCAN_DIR".to_owned(),
                paths.resources().join("php/8.3/etc/conf.d").to_string(),
            ),
        ])
    );

    let exec_env = php_track_exec_environment(&paths, "8.3")?
        .into_iter()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.to_string_lossy().into_owned(),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        exec_env,
        vec![
            (
                "PHPRC".to_owned(),
                paths.resources().join("php/8.3/etc").to_string(),
            ),
            (
                "PHP_INI_SCAN_DIR".to_owned(),
                paths.resources().join("php/8.3/etc/conf.d").to_string(),
            ),
        ]
    );

    Ok(())
}

fn assert_invalid_track<T>(result: StdResult<T, StateError>, track: &str) -> Result<()> {
    match result {
        Err(StateError::InvalidProjectTrack {
            track: invalid_track,
        }) if invalid_track == track => Ok(()),
        Err(error) => anyhow::bail!("expected invalid track error, got {error}"),
        Ok(_) => anyhow::bail!("expected unsupported PHP track to fail"),
    }
}
