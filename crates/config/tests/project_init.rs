use anyhow::{Result, anyhow};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use config::{
    ConfigError, ProjectInitResourceName, default_project_init_selection, detect_project_init,
    render_project_init_config,
};
use insta::{assert_debug_snapshot, assert_snapshot};

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
