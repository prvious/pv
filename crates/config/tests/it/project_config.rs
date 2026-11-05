use std::io;

use anyhow::{Result, anyhow};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use config::{
    ConfigError, ProjectConfig, ProjectConfigFile, normalize_primary_hostname,
    write_project_php_track,
};
use insta::{assert_debug_snapshot, assert_snapshot};

#[test]
fn project_config_rejects_invalid_hooks() -> Result<()> {
    let mut errors = Vec::new();
    for (source, expected_field) in [
        ("hooks: null", "hooks"),
        ("hooks: []", "hooks"),
        ("hooks: install", "hooks"),
        ("hooks: { prepare: install }", "hooks.prepare"),
        ("hooks: { setup: null }", "hooks.setup"),
        ("hooks: { setup: [1] }", "hooks.setup"),
        ("hooks: { setup: [false] }", "hooks.setup"),
        ("hooks: { setup: [{}] }", "hooks.setup"),
        ("hooks: { prepare: [null] }", "hooks.prepare"),
    ] {
        let error = ProjectConfig::parse(source)
            .err()
            .ok_or_else(|| anyhow!("accepted {source}"))?;
        assert!(matches!(
            &error,
            ConfigError::InvalidFieldType { field, .. } if field == expected_field
        ));
        errors.push(error.to_string());
    }
    for source in ["hooks: { prepare: [''] }", "hooks: { setup: ['   '] }"] {
        let error = ProjectConfig::parse(source)
            .err()
            .ok_or_else(|| anyhow!("accepted {source}"))?;
        assert!(matches!(&error, ConfigError::EmptyField { .. }));
        errors.push(error.to_string());
    }
    let error = ProjectConfig::parse("hooks: { ready: [] }")
        .err()
        .ok_or_else(|| anyhow!("accepted unknown hook"))?;
    assert!(matches!(&error, ConfigError::UnknownHookKey { key } if key == "ready"));
    errors.push(error.to_string());
    assert_debug_snapshot!(errors);
    Ok(())
}

#[test]
fn project_config_writer_preserves_hooks_when_updating_php() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    let source = r#"
php: 8.3
hooks:
  prepare:
    - cp "$PV_SOURCE_PATH/auth.json" auth.json
  setup:
    - composer install
    - pnpm build
"#;
    write_file(&project.join("pv.yml"), source)?;
    let original = ProjectConfig::parse(source)?;
    let updated = write_project_php_track(&project, "8.4")?;
    let reloaded = ProjectConfigFile::read_from_root(&project)?;
    assert_eq!(updated.config.hooks, original.hooks);
    assert_eq!(reloaded.config, updated.config);
    assert_snapshot!(read_file(&project.join("pv.yml"))?);
    Ok(())
}

#[test]
fn project_config_rejects_invalid_shapes() -> Result<()> {
    let rejected = [
        "serve: disabled\n",
        "env_file: false\n",
        "env_file: /tmp/acme.env\n",
        "env_file: config/../../acme.env\n",
        "serve: false\nroot: /tmp/public\n",
        "serve: false\nroot: public/../../outside\n",
        "root: ../outside\n",
        "root: true\n",
        "php: false\n",
        "php: ../8.4\n",
        "php:\n  extensions: redis\n",
        "php:\n  extensions: null\n",
        "php:\n  extensions:\n",
        "php:\n  extensions:\n    - true\n",
        "mysql:\n  version: true\n",
        "mysql:\n  version: ../8.4\n",
        "postgres:\n  version: \"16\"\npg:\n  version: latest\n",
    ]
    .map(|yaml| {
        (
            yaml,
            ProjectConfig::parse(yaml)
                .map(|_config| ())
                .map_err(|error| error.to_string()),
        )
    });
    assert_debug_snapshot!(rejected);

    // Values close to the rejected shapes that still parse.
    let config = ProjectConfig::parse("env:\n  FEATURE_ENABLED: true\n")?;
    assert_eq!(
        config.env.get("FEATURE_ENABLED").map(String::as_str),
        Some("true")
    );
    assert_eq!(
        ProjectConfig::parse("php: latest\n")?
            .php
            .as_ref()
            .and_then(|php| php.version_selector()),
        Some("latest")
    );
    assert_eq!(
        ProjectConfig::parse("mysql:\n  version: 8.4\n")?
            .resources
            .get("mysql")
            .and_then(|resource| resource.track.as_deref()),
        Some("8.4")
    );

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
fn project_config_checks_env_placeholders_and_their_scope() -> Result<()> {
    let rejected = [
        "env:\n  APP_URL: \"${missing_value}\"\n",
        "env:\n  APP_URL: \"${ProjectUrl}\"\n",
        "env:\n  TLS_PATH: \"${tls_ca}\"\n",
        "env:\n  TLS_PATH: \"${tls_cert}\"\n",
        "env:\n  TLS_PATH: \"${tls_key}\"\n",
        "env:\n  TLS_PATH: \"${tls.other}\"\n",
        "env:\n  DB_DATABASE: \"${database}\"\n",
        "mysql:\n  env:\n    DB_DATABASE: \"${database}\"\n",
        "mysql:\n  env:\n    MAIL_HOST: \"${smtp_host}\"\n",
        "redis:\n  allocations:\n    app:\n      env:\n        S3_BUCKET: \"${bucket}\"\n",
    ]
    .map(|yaml| {
        (
            yaml,
            ProjectConfig::parse(yaml)
                .map(|_config| ())
                .map_err(|error| error.to_string()),
        )
    });
    assert_debug_snapshot!(rejected);

    for accepted in [
        "env:\n  APP_URL: \"$${missing_value}\"\n",
        "env:\n  APP_URL: \"${url}\"\n",
        "env:\n  VITE_DEV_SERVER_KEY: \"${tls.key}\"\n  VITE_DEV_SERVER_CERT: \"${tls.cert}\"\n  PV_TLS_CA: \"${tls.ca}\"\n",
        "mysql:\n  env:\n    APP_URL: \"${url}\"\n",
        "mysql:\n  env:\n    DB_HOST: \"${host}\"\n",
        "mysql:\n  allocations:\n    app:\n      env:\n        APP_URL: \"${url}\"\n",
        "mysql:\n  allocations:\n    app:\n      env:\n        DB_HOST: \"${host}\"\n        DB_DATABASE: \"${database}\"\n        DB_USERNAME: \"${username}\"\n        DB_PASSWORD: \"${password}\"\n        DB_PORT: \"${port}\"\n",
        "rustfs:\n  env:\n    PUBLIC_URL: \"${url}\"\n",
        "rustfs:\n  env:\n    AWS_ENDPOINT: \"${endpoint}\"\n    AWS_URL: \"${url}\"\n",
    ] {
        assert!(ProjectConfig::parse(accepted).is_ok(), "{accepted}");
    }

    Ok(())
}

#[test]
fn project_config_validates_url_placeholder_scopes() {
    let cases = vec![
        (
            "project-url-root-env",
            ProjectConfig::parse(
                r#"
env:
  APP_URL: "${url}"
"#,
            ),
        ),
        (
            "tls-placeholders-project-env",
            ProjectConfig::parse(
                r#"
env:
  VITE_DEV_SERVER_KEY: "${tls.key}"
  VITE_DEV_SERVER_CERT: "${tls.cert}"
  PV_TLS_CA: "${tls.ca}"
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
            "resource-project-tls",
            ProjectConfig::parse(
                r#"
mysql:
  env:
    VITE_DEV_SERVER_KEY: "${tls.key}"
    VITE_DEV_SERVER_CERT: "${tls.cert}"
    PV_TLS_CA: "${tls.ca}"
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
            "allocation-project-tls",
            ProjectConfig::parse(
                r#"
mysql:
  allocations:
    app:
      env:
        VITE_DEV_SERVER_KEY: "${tls.key}"
        VITE_DEV_SERVER_CERT: "${tls.cert}"
        PV_TLS_CA: "${tls.ca}"
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
    let removed_hostnames = ProjectConfig::parse("hostnames:\n  - api.acme.test\n");
    let invalid_hostname = normalize_primary_hostname("api.example.com");
    let long_label = normalize_primary_hostname(&format!("{}.test", "a".repeat(64)));
    let wildcard = normalize_primary_hostname("*.acme.test");

    assert!(matches!(
        helper,
        Err(ConfigError::UnknownTopLevelKey { key }) if key == "defaults"
    ));
    assert!(matches!(
        unknown,
        Err(ConfigError::UnknownTopLevelKey { key }) if key == "unexpected"
    ));
    assert!(matches!(
        removed_hostnames,
        Err(ConfigError::UnknownTopLevelKey { key }) if key == "hostnames"
    ));
    assert!(matches!(
        wildcard,
        Err(ConfigError::InvalidHostname { hostname, .. }) if hostname == "*.acme.test"
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
fn project_config_writer_updates_php_in_discovered_file() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("acme");
    let public = project.join("public");
    create_dir(&public)?;
    write_file(
        &project.join("pv.yml"),
        r#"
php:
  version: 8.2
  extensions:
    - redis
    - xdebug
root: public
serve: false
env_file: .env.local
hooks:
  setup:
    - php artisan migrate
env:
  APP_URL: "${url}"
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

    let updated = write_project_php_track(&project, "8.4")?;
    let reloaded = ProjectConfigFile::read_from_root(&project)?;

    // Updating the track keeps extensions, controls, hooks and resources.
    assert_eq!(reloaded.config, updated.config);
    assert_debug_snapshot!((
        updated.path.file_name(),
        updated.exists,
        updated.config,
        reloaded.config,
    ));
    assert_snapshot!(read_file(&project.join("pv.yml"))?);

    Ok(())
}

#[test]
fn resource_only_config_defers_root_existence_validation() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(&project.join("pv.yml"), "serve: false\nroot: missing\n")?;

    let config_file = ProjectConfigFile::read_from_root(&project)?;

    assert!(!config_file.config.serve);
    assert_eq!(
        config_file.config.root.as_deref(),
        Some(Utf8Path::new("missing"))
    );

    Ok(())
}

#[test]
fn project_config_validates_env_file_parent_and_symlink_boundaries() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("acme");
    let outside = tempdir.path().join("outside");
    create_dir(&project)?;
    create_dir(&outside)?;

    write_file(&project.join("pv.yml"), "env_file: missing/.env\n")?;
    assert!(matches!(
        ProjectConfigFile::read_from_root(&project),
        Err(ConfigError::EnvFileParentNotDirectory { env_file })
            if env_file == "missing/.env"
    ));

    let outside_env = outside.join(".env");
    write_file(&outside_env, "")?;
    create_symlink(&outside_env, &project.join(".env.link"))?;
    write_file(&project.join("pv.yml"), "env_file: .env.link\n")?;
    assert!(matches!(
        ProjectConfigFile::read_from_root(&project),
        Err(ConfigError::EnvFileEscapesProject { env_file })
            if env_file == ".env.link"
    ));

    Ok(())
}

#[test]
fn project_config_distinguishes_missing_roots_from_filesystem_errors() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(&project.join("pv.yml"), "root: missing\n")?;

    let missing = ProjectConfigFile::read_from_root(&project);

    assert!(matches!(
        missing,
        Err(ConfigError::RootNotDirectory { root })
            if root.as_str() == "missing"
    ));

    write_file(&project.join("not-a-directory"), "")?;
    write_file(&project.join("pv.yml"), "root: not-a-directory/public\n")?;
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
    reason = "Project config tests read fixture files"
)]
fn read_file(path: &Utf8Path) -> Result<String> {
    Ok(std::fs::read_to_string(path)?)
}

#[expect(
    clippy::disallowed_methods,
    reason = "Project config tests create symlink fixtures"
)]
fn create_symlink(target: &Utf8Path, link: &Utf8Path) -> Result<()> {
    std::os::unix::fs::symlink(target, link)?;

    Ok(())
}
