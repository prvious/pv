use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use config::{ConfigError, ProjectConfig, ProjectConfigFile};
use insta::assert_debug_snapshot;

#[test]
fn project_config_parses_strict_resource_env_shape() -> Result<()> {
    let config = ProjectConfig::parse(
        r#"
php: 8.4
document_root: public
hostnames:
  - Api.Acme.test.
env:
  APP_URL: "${project_url}"
mysql:
  version: 8.0
  env:
    DB_HOST: "${host}"
  allocations:
    app-db:
      env:
        DB_DATABASE: "${database}"
postgresql:
  version: latest
"#,
    )?;

    assert_debug_snapshot!(config);

    Ok(())
}

#[test]
fn project_config_rejects_anchors_unknown_keys_and_invalid_hostnames() -> Result<()> {
    let anchored = ProjectConfig::parse("php: &php 8.4\nother: *php\n");
    let unknown = ProjectConfig::parse("php: 8.4\nunexpected: true\n");
    let invalid_hostname = ProjectConfig::parse("hostnames:\n  - api.example.com\n");

    assert!(matches!(anchored, Err(ConfigError::AnchorsUnsupported)));
    assert!(matches!(
        unknown,
        Err(ConfigError::UnknownTopLevelKey { key }) if key == "unexpected"
    ));
    assert!(matches!(
        invalid_hostname,
        Err(ConfigError::InvalidHostname { hostname, .. }) if hostname == "api.example.com"
    ));

    Ok(())
}

#[test]
fn project_config_discovery_validates_paths_and_conflicts() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("acme");
    let public = project.join("public");
    create_dir(&public)?;
    write_file(
        &project.join("pv.yml"),
        "document_root: public\nhostnames:\n  - admin.acme.test\n",
    )?;

    let config_file = ProjectConfigFile::read_from_root(&project)?;

    assert_debug_snapshot!((
        config_file.path.file_name(),
        config_file.exists,
        config_file.config,
    ));

    write_file(&project.join("pv.yaml"), "php: 8.3\n")?;
    let conflict = ProjectConfigFile::read_from_root(&project);
    assert!(matches!(
        conflict,
        Err(ConfigError::ConfigFileConflict { .. })
    ));

    Ok(())
}

#[test]
fn project_config_rejects_document_roots_that_escape_project() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(&project.join("pv.yml"), "document_root: ../outside\n")?;
    create_dir(&tempdir.path().join("outside"))?;

    let result = ProjectConfigFile::read_from_root(&project);

    assert!(matches!(
        result,
        Err(ConfigError::DocumentRootEscapesProject { document_root }) if document_root.as_str() == "../outside"
    ));

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "Project config tests create fixture directories"
)]
fn create_dir(path: &Utf8Path) -> Result<()> {
    std::fs::create_dir_all(path)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "Project config tests write fixture files"
)]
fn write_file(path: &Utf8Path, contents: &str) -> Result<()> {
    std::fs::write(path, contents)?;

    Ok(())
}
