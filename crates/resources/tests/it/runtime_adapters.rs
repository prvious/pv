use anyhow::{Result, bail};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use resources::{
    ResourceAdapter, ResourcesError, composer_adapter, frankenphp_adapter, mailpit_adapter,
    mysql_adapter, php_adapter, postgres_adapter, redis_adapter, rustfs_adapter,
};
use state::fs::write_sensitive_file;

#[test]
fn runtime_adapters_validate_expected_executable_layouts() -> Result<()> {
    let tempdir = tempdir()?;
    let cases = [
        (php_adapter()?, "bin/php"),
        (frankenphp_adapter()?, "bin/frankenphp"),
        (composer_adapter()?, "composer.phar"),
        (mailpit_adapter()?, "bin/mailpit"),
        (redis_adapter()?, "bin/redis-server"),
        (rustfs_adapter()?, "bin/rustfs"),
        (mysql_adapter()?, "bin/mysqld"),
    ];

    for (adapter, executable) in cases {
        let release = tempdir.path().join(adapter.resource_name().as_str());
        let executable_path = release.join(executable);
        write_sensitive_file(&executable_path, "executable")?;

        adapter.validate_installation(&release)?;

        assert_eq!(adapter.executable_path(&release), executable_path);
    }

    Ok(())
}

#[test]
fn postgres_adapter_validates_pv2_runtime_layout() -> Result<()> {
    let tempdir = tempdir()?;
    let release = tempdir.path();
    let adapter = postgres_adapter()?;
    let executable_path = release.join("bin/postgres");
    write_sensitive_file(&executable_path, "postgres executable")?;
    write_sensitive_file(&release.join("bin/initdb"), "postgres initdb")?;
    write_sensitive_file(&release.join("share/postgres.bki"), "postgres catalog")?;
    write_postgres_pv2_support_files(release)?;

    adapter.validate_installation(release)?;

    assert_eq!(adapter.executable_path(release), executable_path);

    Ok(())
}

fn write_postgres_pv2_support_files(release: &Utf8Path) -> Result<()> {
    for relative_path in [
        "lib/libcrypto.3.dylib",
        "lib/libssl.3.dylib",
        "lib/postgresql/pg_trgm.dylib",
        "lib/postgresql/pgcrypto.dylib",
        "lib/postgresql/sslinfo.dylib",
        "share/extension/pg_trgm.control",
        "share/extension/pgcrypto.control",
        "share/extension/sslinfo.control",
    ] {
        write_sensitive_file(&release.join(relative_path), "postgres pv2 support file")?;
    }

    Ok(())
}

#[test]
fn postgres_adapter_rejects_missing_initdb_and_support_files() -> Result<()> {
    let tempdir = tempdir()?;
    let release = tempdir.path();
    let adapter = postgres_adapter()?;
    write_sensitive_file(&release.join("bin/postgres"), "postgres executable")?;
    let missing_initdb = adapter.validate_installation(release);
    write_sensitive_file(&release.join("bin/initdb"), "postgres initdb")?;
    let missing_support_files = adapter.validate_installation(release);

    for (result, expected_reason) in [
        (missing_initdb, "missing required file `bin/initdb`"),
        (
            missing_support_files,
            "missing required file `share/postgres.bki`",
        ),
    ] {
        let Err(ResourcesError::InvalidArtifactLayout { resource, reason }) = result else {
            bail!("expected InvalidArtifactLayout, got {result:?}");
        };

        assert_eq!(resource, "postgres");
        assert_eq!(reason, expected_reason);
    }

    Ok(())
}

#[test]
fn runtime_adapters_reject_missing_executables() -> Result<()> {
    let tempdir = tempdir()?;
    let release = tempdir.path();

    assert_missing_executable_error(php_adapter()?.validate_installation(release), "php")?;
    assert_missing_executable_error(
        frankenphp_adapter()?.validate_installation(release),
        "frankenphp",
    )?;
    assert_missing_executable_error(
        composer_adapter()?.validate_installation(release),
        "composer",
    )?;
    assert_missing_executable_error(mailpit_adapter()?.validate_installation(release), "mailpit")?;
    assert_missing_executable_error(redis_adapter()?.validate_installation(release), "redis")?;
    assert_missing_executable_error(rustfs_adapter()?.validate_installation(release), "rustfs")?;
    assert_missing_executable_error(mysql_adapter()?.validate_installation(release), "mysql")?;
    assert_missing_executable_error(
        postgres_adapter()?.validate_installation(release),
        "postgres",
    )?;

    Ok(())
}

fn assert_missing_executable_error(
    result: resources::Result<()>,
    expected_resource: &str,
) -> Result<()> {
    let Err(ResourcesError::InvalidArtifactLayout { resource, .. }) = result else {
        bail!("expected InvalidArtifactLayout, got {result:?}");
    };

    assert_eq!(resource, expected_resource);

    Ok(())
}
