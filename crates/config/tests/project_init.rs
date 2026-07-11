use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use config::{
    AllocationEnvContext, ConfigError, ProjectEnvContext, ProjectInitResourceName,
    ResourceEnvContext, default_project_init_selection, detect_project_init, render_project_env,
    render_project_init_config, validate_project_env_shape,
};
use insta::{assert_debug_snapshot, assert_snapshot};

#[derive(Clone, Copy, Debug)]
enum ExpectedDocumentRootError {
    Absolute,
    Escaping,
    Missing,
}

#[test]
fn project_init_detects_laravel_vite_and_common_resources() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("acme");
    create_laravel_fixture(&project)?;

    let detection = detect_project_init(&project)?;
    let selection = default_project_init_selection(&detection);
    let config = render_project_init_config(&detection, &selection)?;
    let yaml = yaml_serde::to_string(&config)?;

    assert_debug_snapshot!((&detection.signals, &selection.resources));
    assert_snapshot!(yaml);

    Ok(())
}

#[test]
fn project_init_keeps_ambiguous_database_selection_unselected() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(
        &project.join(".env.example"),
        "DB_HOST=127.0.0.1\nPOSTGRES_HOST=127.0.0.1\nREDIS_HOST=127.0.0.1\n",
    )?;

    let detection = detect_project_init(&project)?;
    let mysql = detection
        .resources
        .get(&ProjectInitResourceName::Mysql)
        .ok_or_else(|| anyhow!("missing mysql detection"))?;
    let postgres = detection
        .resources
        .get(&ProjectInitResourceName::Postgres)
        .ok_or_else(|| anyhow!("missing postgres detection"))?;

    assert!(!mysql.selected);
    assert!(!postgres.selected);

    Ok(())
}

#[test]
fn project_init_maps_only_clear_composer_php_constraints() -> Result<()> {
    let tempdir = tempdir()?;
    let cases = [
        ("exact", "8.3", "8.3"),
        ("caret", "^8.4", "8.4"),
        ("tilde", "~8.5.1", "8.5"),
        ("wildcard", "8.4.*", "8.4"),
        ("range", ">=8.2", "latest"),
        ("union", "^8.3 || ^8.4", "latest"),
        ("unsupported", "^8.2", "latest"),
    ];
    let mut suggestions = Vec::new();

    for (name, constraint, expected) in cases {
        let project = tempdir.path().join(name);
        create_dir(&project)?;
        write_file(
            &project.join("composer.json"),
            &format!(r#"{{"require":{{"php":"{constraint}"}}}}"#),
        )?;

        let detection = detect_project_init(&project)?;
        suggestions.push((constraint, detection.suggested_php.clone()));
        assert_eq!(detection.suggested_php, expected);
    }

    assert_debug_snapshot!(suggestions);

    Ok(())
}

#[test]
fn project_init_generates_resource_defaults_with_prefixed_secondary_allocations() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("acme");
    create_dir(&project)?;

    let detection = detect_project_init(&project)?;
    let mut selection = default_project_init_selection(&detection);
    select_resource(
        &mut selection,
        ProjectInitResourceName::Mysql,
        &["app", "analytics"],
    )?;
    select_resource(&mut selection, ProjectInitResourceName::Postgres, &["app"])?;
    select_resource(
        &mut selection,
        ProjectInitResourceName::Redis,
        &["cache", "sessions"],
    )?;
    select_resource(&mut selection, ProjectInitResourceName::Mailpit, &[])?;
    select_resource(
        &mut selection,
        ProjectInitResourceName::Rustfs,
        &["uploads", "backups"],
    )?;

    let config = render_project_init_config(&detection, &selection)?;
    assert_snapshot!(yaml_serde::to_string(&config)?);

    Ok(())
}

#[test]
fn project_init_preserves_existing_config_values_when_merging_defaults() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("acme");
    create_dir(&project.join("web"))?;
    write_file(
        &project.join("pv.yml"),
        r#"php:
  version: "8.3"
  extensions:
    - intl
document_root: web
hostnames:
  - admin.acme.test
env:
  APP_URL: "https://custom.test"
  CUSTOM_VALUE: preserved
mysql:
  version: "8.4"
  env:
    DB_HOST: custom-host
  allocations:
    app:
      env:
        DB_DATABASE: custom_database
redis:
  version: "7.4"
  env:
    CUSTOM_REDIS: preserved
"#,
    )?;
    write_file(
        &project.join(".env.example"),
        "APP_URL=http://localhost\nDB_CONNECTION=mysql\nMAIL_HOST=127.0.0.1\n",
    )?;
    write_file(
        &project.join("package.json"),
        r#"{"devDependencies":{"vite":"^7.0.0"}}"#,
    )?;

    let detection = detect_project_init(&project)?;
    let selection = default_project_init_selection(&detection);
    let config = render_project_init_config(&detection, &selection)?;

    assert_snapshot!(yaml_serde::to_string(&config)?);

    Ok(())
}

#[test]
fn project_init_avoids_collisions_across_the_final_sql_allocation_set() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(
        &project.join("pv.yml"),
        r#"mysql:
  version: "8.4"
  allocations:
    reporting: {}
"#,
    )?;

    let detection = detect_project_init(&project)?;
    let mut selection = default_project_init_selection(&detection);
    let mysql = selection
        .resources
        .get(&ProjectInitResourceName::Mysql)
        .ok_or_else(|| anyhow!("missing MySQL selection"))?;
    assert_eq!(mysql.allocations, ["reporting"]);
    select_resource(
        &mut selection,
        ProjectInitResourceName::Mysql,
        &["reporting", "analytics"],
    )?;
    select_resource(
        &mut selection,
        ProjectInitResourceName::Postgres,
        &["app", "warehouse"],
    )?;

    let config = render_project_init_config(&detection, &selection)?;
    validate_project_env_shape(&config)?;

    let mysql = config
        .resources
        .get("mysql")
        .ok_or_else(|| anyhow!("missing generated MySQL config"))?;
    let postgres = config
        .resources
        .get("postgres")
        .ok_or_else(|| anyhow!("missing generated Postgres config"))?;
    assert!(mysql.allocations.contains_key("reporting"));
    assert!(mysql.allocations.contains_key("analytics"));
    assert!(postgres.allocations.contains_key("app"));
    assert!(postgres.allocations.contains_key("warehouse"));
    assert_snapshot!(yaml_serde::to_string(&config)?);

    Ok(())
}

#[test]
fn project_init_generated_defaults_preserve_effective_ancestor_env_values() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(
        &project.join("pv.yml"),
        r#"env:
  DB_CONNECTION: custom-root
mysql:
  version: "8.4"
  env:
    DB_HOST: custom-host
    DB_PORT: custom-port
  allocations:
    app: {}
"#,
    )?;

    let detection = detect_project_init(&project)?;
    let mut selection = default_project_init_selection(&detection);
    select_resource(&mut selection, ProjectInitResourceName::Mysql, &["app"])?;
    let config = render_project_init_config(&detection, &selection)?;
    validate_project_env_shape(&config)?;

    let app_env = &config
        .resources
        .get("mysql")
        .and_then(|resource| resource.allocations.get("app"))
        .ok_or_else(|| anyhow!("missing MySQL app allocation"))?
        .env;
    assert!(!app_env.contains_key("DB_CONNECTION"));
    assert!(!app_env.contains_key("DB_HOST"));
    assert!(!app_env.contains_key("DB_PORT"));

    let rendered = render_project_env(&config, &mysql_project_env_context())?;
    assert_eq!(
        rendered.values.get("DB_CONNECTION").map(String::as_str),
        Some("custom-root")
    );
    assert_eq!(
        rendered.values.get("DB_HOST").map(String::as_str),
        Some("custom-host")
    );
    assert_eq!(
        rendered.values.get("DB_PORT").map(String::as_str),
        Some("custom-port")
    );

    Ok(())
}

#[test]
fn project_init_seeds_existing_structured_values_and_applies_edits() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("acme");
    create_dir(&project.join("web"))?;
    create_dir(&project.join("edited"))?;
    write_file(
        &project.join("pv.yml"),
        r#"document_root: web
mysql:
  version: "8.4"
  allocations:
    analytics: {}
redis:
  version: "7.4"
  allocations:
    sessions: {}
"#,
    )?;

    let detection = detect_project_init(&project)?;
    let mut selection = default_project_init_selection(&detection);
    assert_eq!(
        selection.document_root.as_deref(),
        Some(Utf8Path::new("web"))
    );
    let mysql = selection
        .resources
        .get(&ProjectInitResourceName::Mysql)
        .ok_or_else(|| anyhow!("missing MySQL selection"))?;
    assert!(mysql.selected);
    assert_eq!(mysql.track, "8.4");
    assert_eq!(mysql.allocations, ["analytics"]);
    let redis = selection
        .resources
        .get(&ProjectInitResourceName::Redis)
        .ok_or_else(|| anyhow!("missing Redis selection"))?;
    assert!(redis.selected);
    assert_eq!(redis.track, "7.4");
    assert_eq!(redis.allocations, ["sessions"]);

    selection.document_root = Some(Utf8PathBuf::from("edited"));
    selection
        .resources
        .get_mut(&ProjectInitResourceName::Mysql)
        .ok_or_else(|| anyhow!("missing MySQL selection"))?
        .track = "9.1".to_string();
    selection
        .resources
        .get_mut(&ProjectInitResourceName::Redis)
        .ok_or_else(|| anyhow!("missing Redis selection"))?
        .track = "8.0".to_string();

    let config = render_project_init_config(&detection, &selection)?;
    assert_eq!(
        config.document_root.as_deref(),
        Some(Utf8Path::new("edited"))
    );
    assert_eq!(
        config
            .resources
            .get("mysql")
            .and_then(|resource| resource.track.as_deref()),
        Some("9.1")
    );
    assert_eq!(
        config
            .resources
            .get("redis")
            .and_then(|resource| resource.track.as_deref()),
        Some("8.0")
    );

    Ok(())
}

#[test]
fn project_init_validates_edited_document_roots_before_returning() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    create_dir(&project.join("public"))?;
    create_dir(&tempdir.path().join("outside"))?;

    let detection = detect_project_init(&project)?;
    let selection = default_project_init_selection(&detection);
    let cases = [
        (project.join("public"), ExpectedDocumentRootError::Absolute),
        (
            Utf8PathBuf::from("../outside"),
            ExpectedDocumentRootError::Escaping,
        ),
        (
            Utf8PathBuf::from("missing"),
            ExpectedDocumentRootError::Missing,
        ),
    ];

    for (document_root, expected) in cases {
        let mut selection = selection.clone();
        selection.document_root = Some(document_root);
        let Err(error) = render_project_init_config(&detection, &selection) else {
            return Err(anyhow!(
                "expected {expected:?} document root to reject the proposal"
            ));
        };
        let matches_expected = match expected {
            ExpectedDocumentRootError::Absolute => {
                matches!(error, ConfigError::AbsoluteDocumentRoot { .. })
            }
            ExpectedDocumentRootError::Escaping => {
                matches!(error, ConfigError::DocumentRootEscapesProject { .. })
            }
            ExpectedDocumentRootError::Missing => {
                matches!(error, ConfigError::DocumentRootNotDirectory { .. })
            }
        };
        assert!(matches_expected, "unexpected {expected:?} error: {error:?}");
    }

    Ok(())
}

#[test]
fn project_init_validates_existing_env_shape_before_returning() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(
        &project.join("pv.yml"),
        r#"mysql:
  allocations:
    app:
      env:
        SHARED_KEY: mysql
postgres:
  allocations:
    app:
      env:
        SHARED_KEY: postgres
"#,
    )?;

    let detection = detect_project_init(&project)?;
    let selection = default_project_init_selection(&detection);
    let result = render_project_init_config(&detection, &selection);

    assert!(matches!(
        result,
        Err(ConfigError::DuplicateRenderedEnvKey { key, .. }) if key == "SHARED_KEY"
    ));

    Ok(())
}

#[test]
fn project_init_does_not_add_app_url_for_unrelated_existing_root_env() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(&project.join("pv.yml"), "env:\n  CUSTOM_VALUE: preserved\n")?;

    let detection = detect_project_init(&project)?;
    let selection = default_project_init_selection(&detection);
    assert!(!selection.include_app_url);

    let config = render_project_init_config(&detection, &selection)?;
    assert_eq!(
        config.env,
        BTreeMap::from([("CUSTOM_VALUE".to_string(), "preserved".to_string())])
    );

    Ok(())
}

#[test]
fn project_init_returns_typed_error_for_invalid_json() -> Result<()> {
    let tempdir = tempdir()?;
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(&project.join("composer.json"), "{invalid")?;

    let result = detect_project_init(&project);
    let Err(ConfigError::InvalidInitJson { path, reason }) = result else {
        return Err(anyhow!("expected InvalidInitJson, got {result:?}"));
    };

    assert_eq!(path.file_name(), Some("composer.json"));
    assert!(path.is_absolute());
    assert!(!reason.is_empty());

    Ok(())
}

fn create_laravel_fixture(project: &Utf8Path) -> Result<()> {
    create_dir(&project.join("bootstrap"))?;
    create_dir(&project.join("config"))?;
    create_dir(&project.join("public"))?;
    write_file(&project.join("artisan"), "")?;
    write_file(&project.join("bootstrap/app.php"), "<?php\n")?;
    write_file(&project.join("config/app.php"), "<?php\n")?;
    write_file(&project.join("public/index.php"), "<?php\n")?;
    write_file(
        &project.join("composer.json"),
        r#"{"require":{"php":"^8.4","laravel/framework":"^12.0"}}"#,
    )?;
    write_file(
        &project.join("package.json"),
        r#"{"devDependencies":{"vite":"^7.0.0","laravel-vite-plugin":"^2.0.0"}}"#,
    )?;
    write_file(
        &project.join(".env.example"),
        r#"APP_URL=http://localhost
DB_CONNECTION=mysql
REDIS_HOST=127.0.0.1
CACHE_STORE=redis
MAIL_MAILER=smtp
AWS_ACCESS_KEY_ID=
AWS_SECRET_ACCESS_KEY=
"#,
    )?;

    Ok(())
}

fn select_resource(
    selection: &mut config::ProjectInitSelection,
    name: ProjectInitResourceName,
    allocations: &[&str],
) -> Result<()> {
    let resource = selection
        .resources
        .get_mut(&name)
        .ok_or_else(|| anyhow!("missing {name:?} selection"))?;
    resource.selected = true;
    resource.allocations = allocations
        .iter()
        .map(|allocation| (*allocation).to_string())
        .collect();

    Ok(())
}

fn mysql_project_env_context() -> ProjectEnvContext {
    ProjectEnvContext {
        primary_hostname: "acme.test".to_string(),
        tls_ca_path: "/tmp/ca.pem".to_string(),
        tls_cert_path: "/tmp/acme.crt".to_string(),
        tls_key_path: "/tmp/acme.key".to_string(),
        resources: BTreeMap::from([(
            "mysql".to_string(),
            ResourceEnvContext {
                track: "8.4".to_string(),
                values: BTreeMap::from([
                    ("host".to_string(), "generated-host".to_string()),
                    ("password".to_string(), "generated-password".to_string()),
                    ("port".to_string(), "3306".to_string()),
                    ("username".to_string(), "generated-user".to_string()),
                ]),
                allocations: BTreeMap::from([(
                    "app".to_string(),
                    AllocationEnvContext {
                        generated_name: "acme_test_app".to_string(),
                        values: BTreeMap::from([(
                            "database".to_string(),
                            "acme_test_app".to_string(),
                        )]),
                    },
                )]),
            },
        )]),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "Project init tests create fixture directories"
)]
fn create_dir(path: &Utf8Path) -> Result<()> {
    std::fs::create_dir_all(path)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "Project init tests write fixture files"
)]
fn write_file(path: &Utf8Path, contents: &str) -> Result<()> {
    std::fs::write(path, contents)?;

    Ok(())
}
