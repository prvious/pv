use std::collections::BTreeMap;

use anyhow::Result;
use camino_tempfile::tempdir;
use resources::{
    PHP_TRACK_DEFAULT_INI, ensure_php_track_defaults, php_track_defaults, php_track_environment,
    php_track_exec_environment,
};
use state::{PvPaths, fs};

#[test]
fn php_track_defaults_seed_stripped_sample_once() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let defaults = ensure_php_track_defaults(&paths, "8.4")?;
    let first_content = fs::read_to_string(defaults.php_ini())?;

    assert_eq!(defaults.etc_dir(), paths.resources().join("php/8.4/etc"));
    assert_eq!(
        defaults.conf_dir(),
        paths.resources().join("php/8.4/etc/conf.d")
    );
    assert_eq!(first_content, PHP_TRACK_DEFAULT_INI);
    assert!(first_content.starts_with("[PHP]\nengine = On\n"));
    assert!(first_content.contains("\n[Date]\n"));
    assert!(first_content.contains("\nunserialize_callback_func =\n"));
    assert!(!first_content.contains("; About php.ini"));

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
fn php_track_defaults_reject_blocking_paths() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let defaults = php_track_defaults(&paths, "8.5");
    fs::ensure_user_dir(defaults.etc_dir())?;
    fs::write_sensitive_file(defaults.conf_dir(), "not a directory\n")?;

    let error = match ensure_php_track_defaults(&paths, "8.5") {
        Ok(_) => anyhow::bail!("expected blocking conf.d path to fail"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("PHP track defaults conf.d path is not a directory")
    );

    Ok(())
}

#[test]
fn php_track_defaults_env_helpers_point_at_track_etc() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));

    assert_eq!(
        php_track_environment(&paths, "8.3"),
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

    let exec_env = php_track_exec_environment(&paths, "8.3")
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
