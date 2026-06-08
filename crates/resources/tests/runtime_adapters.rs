use anyhow::{Result, bail};
use camino_tempfile::tempdir;
use resources::{
    ResourceAdapter, ResourcesError, composer_adapter, frankenphp_adapter, mailpit_adapter,
    mysql_adapter, php_adapter, postgres_adapter, rustfs_adapter,
};
use state::fs::write_sensitive_file;

#[test]
fn php_adapter_validates_expected_executable_layout() -> Result<()> {
    let tempdir = tempdir()?;
    let release = tempdir.path();
    let adapter = php_adapter()?;
    let executable_path = release.join("bin/php");
    write_sensitive_file(&executable_path, "php executable")?;

    adapter.validate_installation(release)?;

    assert_eq!(adapter.executable_path(release), executable_path);

    Ok(())
}

#[test]
fn frankenphp_adapter_validates_expected_executable_layout() -> Result<()> {
    let tempdir = tempdir()?;
    let release = tempdir.path();
    let adapter = frankenphp_adapter()?;
    let executable_path = release.join("bin/frankenphp");
    write_sensitive_file(&executable_path, "frankenphp executable")?;

    adapter.validate_installation(release)?;

    assert_eq!(adapter.executable_path(release), executable_path);

    Ok(())
}

#[test]
fn composer_adapter_validates_expected_phar_layout() -> Result<()> {
    let tempdir = tempdir()?;
    let release = tempdir.path();
    let adapter = composer_adapter()?;
    let executable_path = release.join("composer.phar");
    write_sensitive_file(&executable_path, "composer phar")?;

    adapter.validate_installation(release)?;

    assert_eq!(adapter.executable_path(release), executable_path);

    Ok(())
}

#[test]
fn mailpit_adapter_validates_expected_executable_layout() -> Result<()> {
    let tempdir = tempdir()?;
    let release = tempdir.path();
    let adapter = mailpit_adapter()?;
    let executable_path = release.join("bin/mailpit");
    write_sensitive_file(&executable_path, "mailpit executable")?;

    adapter.validate_installation(release)?;

    assert_eq!(adapter.executable_path(release), executable_path);

    Ok(())
}

#[test]
fn redis_adapter_validates_expected_executable_layout() -> Result<()> {
    let tempdir = tempdir()?;
    let release = tempdir.path();
    let adapter = resources::redis_adapter()?;
    let executable_path = release.join("bin/redis-server");
    write_sensitive_file(&executable_path, "redis executable")?;

    adapter.validate_installation(release)?;

    assert_eq!(adapter.executable_path(release), executable_path);

    Ok(())
}

#[test]
fn rustfs_adapter_validates_expected_executable_layout() -> Result<()> {
    let tempdir = tempdir()?;
    let release = tempdir.path();
    let adapter = rustfs_adapter()?;
    let executable_path = release.join("bin/rustfs");
    write_sensitive_file(&executable_path, "rustfs executable")?;

    adapter.validate_installation(release)?;

    assert_eq!(adapter.executable_path(release), executable_path);

    Ok(())
}

#[test]
fn mysql_adapter_validates_expected_executable_layout() -> Result<()> {
    let tempdir = tempdir()?;
    let release = tempdir.path();
    let adapter = mysql_adapter()?;
    let executable_path = release.join("bin/mysqld");
    write_sensitive_file(&executable_path, "mysqld executable")?;

    adapter.validate_installation(release)?;

    assert_eq!(adapter.executable_path(release), executable_path);

    Ok(())
}

#[test]
fn postgres_adapter_validates_expected_executable_and_initdb_layout() -> Result<()> {
    let tempdir = tempdir()?;
    let release = tempdir.path();
    let adapter = postgres_adapter()?;
    let executable_path = release.join("bin/postgres");
    write_sensitive_file(&executable_path, "postgres executable")?;
    write_sensitive_file(&release.join("bin/initdb"), "postgres initdb")?;

    adapter.validate_installation(release)?;

    assert_eq!(adapter.executable_path(release), executable_path);

    Ok(())
}

#[test]
fn postgres_adapter_rejects_missing_initdb() -> Result<()> {
    let tempdir = tempdir()?;
    let release = tempdir.path();
    let adapter = postgres_adapter()?;
    write_sensitive_file(&release.join("bin/postgres"), "postgres executable")?;

    let Err(ResourcesError::InvalidArtifactLayout { resource, reason }) =
        adapter.validate_installation(release)
    else {
        bail!("expected InvalidArtifactLayout for missing initdb");
    };

    assert_eq!(resource, "postgres");
    assert_eq!(reason, "missing required file `bin/initdb`");

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
    assert_missing_executable_error(
        resources::redis_adapter()?.validate_installation(release),
        "redis",
    )?;
    assert_missing_executable_error(rustfs_adapter()?.validate_installation(release), "rustfs")?;
    assert_missing_executable_error(mysql_adapter()?.validate_installation(release), "mysql")?;
    assert_missing_executable_error(
        postgres_adapter()?.validate_installation(release),
        "postgres",
    )?;

    Ok(())
}

#[test]
fn runtime_adapters_reject_directory_executable_paths() -> Result<()> {
    let tempdir = tempdir()?;
    let release = tempdir.path();
    create_dir(&release.join("bin/php"))?;
    create_dir(&release.join("bin/frankenphp"))?;
    create_dir(&release.join("composer.phar"))?;
    create_dir(&release.join("bin/mailpit"))?;
    create_dir(&release.join("bin/redis-server"))?;
    create_dir(&release.join("bin/rustfs"))?;
    create_dir(&release.join("bin/mysqld"))?;
    create_dir(&release.join("bin/postgres"))?;

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
    assert_missing_executable_error(
        resources::redis_adapter()?.validate_installation(release),
        "redis",
    )?;
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

#[expect(
    clippy::disallowed_methods,
    reason = "test fixture creates a directory at the runtime executable path"
)]
fn create_dir(path: &camino::Utf8Path) -> Result<()> {
    std::fs::create_dir_all(path)?;

    Ok(())
}
