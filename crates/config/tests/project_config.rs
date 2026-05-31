use std::io;

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
fn project_config_accepts_registered_resource_aliases() -> Result<()> {
    let config = ProjectConfig::parse(
        r#"
pg:
  version: latest
mail:
  env:
    MAIL_HOST: "${smtp_host}"
s3:
  allocations:
    app:
      env:
        S3_BUCKET: "${bucket}"
"#,
    )?;

    assert_debug_snapshot!(config.resources);

    Ok(())
}

#[test]
fn project_config_rejects_invalid_scalar_shapes() -> Result<()> {
    assert!(matches!(
        ProjectConfig::parse("php: false\n"),
        Err(ConfigError::InvalidFieldType { field, .. }) if field == "php"
    ));
    assert!(matches!(
        ProjectConfig::parse("mysql:\n  version: true\n"),
        Err(ConfigError::InvalidFieldType { field, .. }) if field == "mysql.version"
    ));
    assert!(matches!(
        ProjectConfig::parse("document_root: true\n"),
        Err(ConfigError::InvalidFieldType { field, .. }) if field == "document_root"
    ));

    let config = ProjectConfig::parse("env:\n  FEATURE_ENABLED: true\n")?;
    assert_eq!(
        config.env.get("FEATURE_ENABLED").map(String::as_str),
        Some("true")
    );

    Ok(())
}

#[test]
fn project_config_validates_php_and_resource_tracks() -> Result<()> {
    assert_eq!(
        ProjectConfig::parse("php: latest\n")?.php.as_deref(),
        Some("latest")
    );
    assert_eq!(
        ProjectConfig::parse("mysql:\n  version: 8.4\n")?
            .resources
            .get("mysql")
            .and_then(|resource| resource.track.as_deref()),
        Some("8.4")
    );
    assert!(matches!(
        ProjectConfig::parse("php: ../8.4\n"),
        Err(ConfigError::InvalidPhpTrack { track, .. }) if track == "../8.4"
    ));
    assert!(matches!(
        ProjectConfig::parse("mysql:\n  version: ../8.4\n"),
        Err(ConfigError::InvalidResourceTrack {
            resource,
            track,
            ..
        }) if resource == "mysql" && track == "../8.4"
    ));

    Ok(())
}

#[test]
fn project_config_rejects_invalid_env_placeholders() -> Result<()> {
    assert!(matches!(
        ProjectConfig::parse("env:\n  APP_URL: \"${missing_value}\"\n"),
        Err(ConfigError::UnknownEnvPlaceholder { placeholder, .. })
            if placeholder == "missing_value"
    ));
    assert!(matches!(
        ProjectConfig::parse("env:\n  APP_URL: \"${ProjectUrl}\"\n"),
        Err(ConfigError::InvalidEnvPlaceholder { placeholder, .. })
            if placeholder == "ProjectUrl"
    ));
    assert!(ProjectConfig::parse("env:\n  APP_URL: \"$${missing_value}\"\n").is_ok());
    assert!(ProjectConfig::parse("rustfs:\n  env:\n    PUBLIC_URL: \"${url}\"\n").is_ok());

    Ok(())
}

#[test]
fn project_config_validates_url_placeholder_scopes() {
    let cases = vec![
        (
            "project-url-project-env",
            ProjectConfig::parse(
                r#"
env:
  APP_URL: "${project_url}"
"#,
            ),
        ),
        (
            "legacy-url-project-env",
            ProjectConfig::parse(
                r#"
env:
  APP_URL: "${url}"
"#,
            ),
        ),
        (
            "unknown-project-env",
            ProjectConfig::parse(
                r#"
env:
  APP_URL: "${missing_url}"
"#,
            ),
        ),
        (
            "resource-project-url",
            ProjectConfig::parse(
                r#"
mysql:
  env:
    APP_URL: "${project_url}"
"#,
            ),
        ),
        (
            "resource-scoped-url",
            ProjectConfig::parse(
                r#"
mysql:
  env:
    DATABASE_URL: "${url}"
"#,
            ),
        ),
        (
            "resource-url-not-exposed",
            ProjectConfig::parse(
                r#"
mailpit:
  env:
    MAIL_URL: "${url}"
"#,
            ),
        ),
        (
            "allocation-project-url",
            ProjectConfig::parse(
                r#"
mysql:
  allocations:
    app:
      env:
        APP_URL: "${project_url}"
"#,
            ),
        ),
        (
            "allocation-scoped-url",
            ProjectConfig::parse(
                r#"
mysql:
  allocations:
    app:
      env:
        DATABASE_URL: "${url}"
"#,
            ),
        ),
        (
            "allocation-unknown-url-like-placeholder",
            ProjectConfig::parse(
                r#"
mysql:
  allocations:
    app:
      env:
        DATABASE_URL: "${database_url}"
"#,
            ),
        ),
    ];

    assert_debug_snapshot!(cases);
}

#[test]
fn project_config_rejects_env_placeholders_outside_scope() -> Result<()> {
    assert!(matches!(
        ProjectConfig::parse("env:\n  DB_DATABASE: \"${database}\"\n"),
        Err(ConfigError::UnknownEnvPlaceholder { field, placeholder })
            if field == "env.DB_DATABASE" && placeholder == "database"
    ));
    assert!(matches!(
        ProjectConfig::parse("mysql:\n  env:\n    DB_DATABASE: \"${database}\"\n"),
        Err(ConfigError::UnknownEnvPlaceholder { field, placeholder })
            if field == "mysql.env.DB_DATABASE" && placeholder == "database"
    ));
    assert!(matches!(
        ProjectConfig::parse("mysql:\n  env:\n    MAIL_HOST: \"${smtp_host}\"\n"),
        Err(ConfigError::UnknownEnvPlaceholder { field, placeholder })
            if field == "mysql.env.MAIL_HOST" && placeholder == "smtp_host"
    ));
    assert!(matches!(
        ProjectConfig::parse(
            r#"
redis:
  allocations:
    app:
      env:
        S3_BUCKET: "${bucket}"
"#
        ),
        Err(ConfigError::UnknownEnvPlaceholder { field, placeholder })
            if field == "redis.allocations.app.env.S3_BUCKET" && placeholder == "bucket"
    ));

    assert!(ProjectConfig::parse("env:\n  APP_URL: \"${project_url}\"\n").is_ok());
    assert!(ProjectConfig::parse("mysql:\n  env:\n    APP_URL: \"${project_url}\"\n").is_ok());
    assert!(ProjectConfig::parse("mysql:\n  env:\n    DB_HOST: \"${host}\"\n").is_ok());
    assert!(
        ProjectConfig::parse(
            r#"
mysql:
  allocations:
    app:
      env:
        APP_URL: "${project_url}"
"#
        )
        .is_ok()
    );
    assert!(
        ProjectConfig::parse(
            r#"
rustfs:
  env:
    AWS_ENDPOINT: "${endpoint}"
    AWS_URL: "${url}"
"#
        )
        .is_ok()
    );
    assert!(
        ProjectConfig::parse(
            r#"
mysql:
  allocations:
    app:
      env:
        DB_HOST: "${host}"
        DB_DATABASE: "${database}"
        DB_USERNAME: "${username}"
        DB_PASSWORD: "${password}"
        DB_PORT: "${port}"
"#
        )
        .is_ok()
    );

    Ok(())
}

#[test]
fn project_config_rejects_unsupported_and_colliding_allocations() -> Result<()> {
    assert!(matches!(
        ProjectConfig::parse("mailpit:\n  allocations:\n    inbox: {}\n"),
        Err(ConfigError::UnsupportedResourceAllocations { resource }) if resource == "mailpit"
    ));
    assert!(matches!(
        ProjectConfig::parse(
            r#"
mysql:
  allocations:
    app-db: {}
    app_db: {}
"#
        ),
        Err(ConfigError::DuplicateNormalizedAllocation {
            resource,
            normalized,
            ..
        }) if resource == "mysql" && normalized == "app_db"
    ));
    assert!(matches!(
        ProjectConfig::parse(
            r#"
rustfs:
  allocations:
    media_bucket: {}
    media-bucket: {}
"#
        ),
        Err(ConfigError::DuplicateNormalizedAllocation {
            resource,
            normalized,
            ..
        }) if resource == "rustfs" && normalized == "media-bucket"
    ));

    Ok(())
}

#[test]
fn project_config_rejects_duplicate_resource_aliases() -> Result<()> {
    let result = ProjectConfig::parse(
        r#"
postgres:
  version: "16"
pg:
  version: latest
"#,
    );

    assert!(matches!(
        result,
        Err(ConfigError::DuplicateResource { resource }) if resource == "postgres"
    ));

    Ok(())
}

#[test]
fn project_config_expands_yaml_aliases_and_merge_keys() -> Result<()> {
    let config = ProjectConfig::parse(
        r#"
env:
  MATH: 2 * 3
  TEAM: R&D & QA
postgres: &database
  version: 16
  env:
    DB_HOST: "${host}"
mysql:
  <<: *database
  version: 8.4
"#,
    )?;

    assert_debug_snapshot!(config);

    Ok(())
}

#[test]
fn project_config_rejects_helper_keys_unknown_keys_and_invalid_hostnames() -> Result<()> {
    let helper = ProjectConfig::parse(
        r#"
defaults: &database
  version: 16
postgres:
  <<: *database
"#,
    );
    let unknown = ProjectConfig::parse("php: 8.4\nunexpected: true\n");
    let invalid_hostname = ProjectConfig::parse("hostnames:\n  - api.example.com\n");
    let long_label = ProjectConfig::parse(&format!("hostnames:\n  - {}.test\n", "a".repeat(64)));

    assert!(matches!(
        helper,
        Err(ConfigError::UnknownTopLevelKey { key }) if key == "defaults"
    ));
    assert!(matches!(
        unknown,
        Err(ConfigError::UnknownTopLevelKey { key }) if key == "unexpected"
    ));
    assert!(matches!(
        invalid_hostname,
        Err(ConfigError::InvalidHostname { hostname, .. }) if hostname == "api.example.com"
    ));
    assert!(matches!(
        long_label,
        Err(ConfigError::InvalidHostname { hostname, .. }) if hostname.ends_with(".test")
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
fn project_config_discovery_reports_broken_config_symlinks() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    create_symlink(&project.join("missing.yml"), &project.join("pv.yml"))?;

    let result = ProjectConfigFile::read_from_root(&project);

    assert!(matches!(result, Err(ConfigError::Filesystem { .. })));

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

#[test]
fn project_config_distinguishes_missing_document_roots_from_filesystem_errors() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(&project.join("pv.yml"), "document_root: missing\n")?;

    let missing = ProjectConfigFile::read_from_root(&project);

    assert!(matches!(
        missing,
        Err(ConfigError::DocumentRootNotDirectory { document_root })
            if document_root.as_str() == "missing"
    ));

    write_file(&project.join("not-a-directory"), "")?;
    write_file(
        &project.join("pv.yml"),
        "document_root: not-a-directory/public\n",
    )?;
    let filesystem_error = ProjectConfigFile::read_from_root(&project);

    assert!(matches!(
        filesystem_error,
        Err(ConfigError::Filesystem { source, .. })
            if source.kind() == io::ErrorKind::NotADirectory
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

#[expect(
    clippy::disallowed_methods,
    reason = "Project config tests create symlink fixtures"
)]
fn create_symlink(target: &Utf8Path, link: &Utf8Path) -> Result<()> {
    std::os::unix::fs::symlink(target, link)?;

    Ok(())
}
