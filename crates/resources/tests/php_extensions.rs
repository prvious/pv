use anyhow::Result;
use camino_tempfile::tempdir;
use resources::{
    PhpExtensionLoadKind, ResourcesError, ensure_php_runtime_overlay, php_runtime_environment,
    resolve_persisted_php_extension_modules, resolve_php_extension_request,
};
use state::{PvPaths, fs};

#[test]
fn resolves_available_and_ignored_php_extensions_from_artifact_metadata() -> Result<()> {
    let tempdir = tempdir()?;
    let artifact = tempdir.path().join("php");
    fs::write_sensitive_file(
        &artifact.join("share/pv/php-extensions.json"),
        r#"
[
  {"name":"redis","load_kind":"extension","path":"lib/php/extensions/redis.so"},
  {"name":"xdebug","load_kind":"zend_extension","path":"lib/php/extensions/xdebug.so"}
]
"#,
    )?;

    let resolution = resolve_php_extension_request(
        &artifact,
        &["xdebug".into(), "missing".into(), "redis".into()],
    )?;

    assert_eq!(resolution.requested, ["xdebug", "missing", "redis"]);
    assert_eq!(
        resolution
            .loaded
            .iter()
            .map(|module| module.name.as_str())
            .collect::<Vec<_>>(),
        ["redis", "xdebug"]
    );
    assert_eq!(
        resolution.loaded[1].load_kind,
        PhpExtensionLoadKind::ZendExtension
    );
    assert_eq!(resolution.ignored, ["missing"]);

    Ok(())
}

#[test]
fn persisted_php_extension_resolution_rejects_missing_metadata_names() -> Result<()> {
    let tempdir = tempdir()?;
    let artifact = tempdir.path().join("php");
    fs::write_sensitive_file(
        &artifact.join("share/pv/php-extensions.json"),
        r#"
[
  {"name":"redis","load_kind":"extension","path":"lib/php/extensions/redis.so"}
]
"#,
    )?;

    let result =
        resolve_persisted_php_extension_modules(&artifact, &["redis".into(), "xdebug".into()]);

    assert!(matches!(
        result,
        Err(ResourcesError::InvalidArtifactLayout { resource, reason })
            if resource == "php" && reason.contains("persisted PHP extension `xdebug`")
    ));

    Ok(())
}

#[test]
fn writes_runtime_overlay_for_loaded_php_extensions() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let artifact = tempdir.path().join("php");
    fs::write_sensitive_file(
        &artifact.join("share/pv/php-extensions.json"),
        r#"
[
  {"name":"redis","load_kind":"extension","path":"lib/php/extensions/redis.so"},
  {"name":"xdebug","load_kind":"zend_extension","path":"lib/php/extensions/xdebug.so"}
]
"#,
    )?;
    let resolution = resolve_php_extension_request(&artifact, &["redis".into(), "xdebug".into()])?;

    let overlay =
        ensure_php_runtime_overlay(&paths, "8.4+redis+xdebug", &artifact, &resolution.loaded)?;
    let redis_ini = fs::read_to_string(&overlay.join("10-redis.ini"))?;
    let xdebug_ini = fs::read_to_string(&overlay.join("20-xdebug.ini"))?;
    let env = php_runtime_environment(
        &paths,
        "8.4",
        "8.4+redis+xdebug",
        &artifact,
        &resolution.loaded,
    )?;

    assert!(redis_ini.contains("extension="));
    assert!(redis_ini.contains("redis.so"));
    assert!(xdebug_ini.contains("zend_extension="));
    assert!(xdebug_ini.contains("xdebug.so"));
    assert!(env["PHP_INI_SCAN_DIR"].contains("conf.d"));
    assert!(env["PHP_INI_SCAN_DIR"].contains("php-runtimes/8.4+redis+xdebug/conf.d"));

    Ok(())
}
