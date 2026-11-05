use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use config::{
    AllocationEnvContext, ConfigError, ProjectEnvContext, ProjectInitResourceName,
    ResourceEnvContext, default_project_init_selection, detect_project_init, render_project_env,
    render_project_init_config, validate_project_env_shape,
};
use insta::{assert_debug_snapshot, assert_snapshot};

#[test]
fn project_init_selects_a_database_only_from_a_named_connection() -> Result<()> {
    let tempdir = tempdir()?;
    let mut detections = Vec::new();

    for (case, env_example) in [
        (
            "ambiguous",
            "DB_HOST=127.0.0.1\nPOSTGRES_HOST=127.0.0.1\nREDIS_HOST=127.0.0.1\n",
        ),
        ("postgres", "DB_CONNECTION=postgres\n"),
        ("pgsql", "DB_CONNECTION=pgsql\n"),
    ] {
        let project = tempdir.path().join(case);
        create_dir(&project)?;
        write_file(&project.join(".env.example"), env_example)?;

        let detection = detect_project_init(&project)?;
        let database = |name| {
            detection
                .resources
                .get(&name)
                .map(|resource| (resource.selected, resource.reason.clone()))
        };
        detections.push((
            case,
            database(ProjectInitResourceName::Mysql),
            database(ProjectInitResourceName::Postgres),
        ));
    }

    assert_debug_snapshot!(detections);

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
    type Selections<'a> = &'a [(ProjectInitResourceName, &'a [&'a str])];
    let cases: [(&str, Option<&str>, Selections<'_>); 2] = [
        (
            "project_init_generates_resource_defaults_with_prefixed_secondary_allocations",
            None,
            &[
                (ProjectInitResourceName::Mysql, &["app", "analytics"]),
                (ProjectInitResourceName::Postgres, &["app"]),
                (ProjectInitResourceName::Redis, &["cache", "sessions"]),
                (ProjectInitResourceName::Mailpit, &[]),
                (ProjectInitResourceName::Rustfs, &["uploads", "backups"]),
            ],
        ),
        (
            "project_init_avoids_collisions_across_the_final_sql_allocation_set",
            Some("mysql:\n  version: \"8.4\"\n  allocations:\n    reporting: {}\n"),
            &[
                (ProjectInitResourceName::Mysql, &["reporting", "analytics"]),
                (ProjectInitResourceName::Postgres, &["app", "warehouse"]),
            ],
        ),
    ];

    for (name, existing_config, selections) in cases {
        let tempdir = tempdir()?;
        let project = tempdir.path().join("acme");
        create_dir(&project)?;
        if let Some(existing_config) = existing_config {
            write_file(&project.join("pv.yml"), existing_config)?;
        }

        let detection = detect_project_init(&project)?;
        let mut selection = default_project_init_selection(&detection);
        for (resource, allocations) in selections {
            select_resource(&mut selection, *resource, allocations)?;
        }
        let config = render_project_init_config(&detection, &selection)?;
        validate_project_env_shape(&config)?;

        assert_snapshot!(name, yaml_serde::to_string(&config)?);
    }

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
root: web
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
