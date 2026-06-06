use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use daemon::gateway::build_runtime_plan;
use insta::{Settings, assert_debug_snapshot};
use serde_json::json;
use state::{Database, LinkProjectInput, PvPaths};

#[test]
fn runtime_plan_groups_linked_projects_by_php_track() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let acme = tempdir.path().join("acme");
    let other = tempdir.path().join("other/api");

    create_project(
        &acme,
        r#"php: "8.4"
document_root: public
hostnames:
  - api.acme.test
"#,
    )?;
    create_project(
        &other,
        r#"php: "8.3"
document_root: public
"#,
    )?;

    let mut database = Database::open(&paths)?;
    database.link_project(LinkProjectInput {
        path: acme.clone(),
        original_path: acme.clone(),
        primary_hostname: "acme.test".to_owned(),
        config_path: acme.join("pv.yml"),
        desired_php_track: None,
        additional_hostnames: vec!["api.acme.test".to_owned()],
    })?;
    database.link_project(LinkProjectInput {
        path: other.clone(),
        original_path: other.clone(),
        primary_hostname: "other.test".to_owned(),
        config_path: other.join("pv.yml"),
        desired_php_track: None,
        additional_hostnames: Vec::new(),
    })?;
    drop(database);

    let plan = build_runtime_plan(&paths)?;

    assert_runtime_plan_snapshot("runtime_plan_groups_linked_projects_by_php_track", plan);

    Ok(())
}

#[test]
fn runtime_plan_resolves_latest_php_track_from_cached_manifest() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let project_root = tempdir.path().join("latest-project");
    seed_php_manifest(&paths, "8.4")?;
    create_project(
        &project_root,
        r#"php: latest
document_root: public
"#,
    )?;

    let mut database = Database::open(&paths)?;
    database.link_project(LinkProjectInput {
        path: project_root.clone(),
        original_path: project_root.clone(),
        primary_hostname: "latest.test".to_owned(),
        config_path: project_root.join("pv.yml"),
        desired_php_track: None,
        additional_hostnames: Vec::new(),
    })?;
    drop(database);

    let plan = build_runtime_plan(&paths)?;

    assert_runtime_plan_snapshot(
        "runtime_plan_resolves_latest_php_track_from_cached_manifest",
        plan,
    );

    Ok(())
}

fn create_project(project_root: &Utf8Path, config_source: &str) -> Result<()> {
    state::fs::write_sensitive_file(&project_root.join("public/index.php"), "<?php\n")?;
    state::fs::write_sensitive_file(&project_root.join("pv.yml"), config_source)?;

    Ok(())
}

fn seed_php_manifest(paths: &PvPaths, default_track: &str) -> Result<()> {
    state::fs::write_sensitive_file(
        &paths.downloads().join("manifest.json"),
        &json!({
            "schema_version": 1,
            "minimum_pv_version": "0.1.0",
            "resources": [
                {
                    "name": "php",
                    "default_track": default_track,
                    "tracks": [
                        {
                            "name": "8.3",
                            "artifacts": [
                                {
                                    "artifact_version": "8.3.21-pv1",
                                    "upstream_version": "8.3.21",
                                    "pv_build_revision": "pv1",
                                    "platform": "darwin-arm64",
                                    "url": "https://artifacts.example.test/php-8.3.21-pv1-darwin-arm64.tar.gz",
                                    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                                    "size": 12345,
                                    "published_at": "2026-05-26T14:30:00Z"
                                }
                            ]
                        },
                        {
                            "name": "8.4",
                            "artifacts": [
                                {
                                    "artifact_version": "8.4.8-pv1",
                                    "upstream_version": "8.4.8",
                                    "pv_build_revision": "pv1",
                                    "platform": "darwin-arm64",
                                    "url": "https://artifacts.example.test/php-8.4.8-pv1-darwin-arm64.tar.gz",
                                    "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                                    "size": 12345,
                                    "published_at": "2026-05-27T14:30:00Z"
                                }
                            ]
                        }
                    ]
                }
            ]
        })
        .to_string(),
    )?;

    Ok(())
}

fn assert_runtime_plan_snapshot(name: &str, plan: daemon::gateway::RuntimePlan) {
    let mut settings = Settings::clone_current();
    settings.add_filter(r#"/[^"]*/\.tmp[A-Za-z0-9._-]+"#, "<tempdir>");
    settings.add_filter(r#"id: "[a-z0-9]{10}""#, r#"id: "<project_id>""#);
    settings.bind(|| {
        assert_debug_snapshot!(name, plan);
    });
}
