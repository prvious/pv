use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;

use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use config::{
    AllocationEnvContext, ConfigError, ProjectConfig, ProjectEnvContext, RenderedProjectEnv,
    ResourceEnvContext, format_project_env, render_project_env, transform_managed_env_block,
    write_project_env_file,
};
use insta::assert_debug_snapshot;

#[test]
fn project_env_renderer_resolves_project_resource_and_allocation_contexts() -> Result<()> {
    let config = ProjectConfig::parse(
        r#"
env:
  APP_URL: "${url}"
  SHARED_VALUE: root
  ROOT_TLS_KEY: "${tls.key}"
  ROOT_TLS_CERT: "${tls.cert}"
  ROOT_TLS_CA: "${tls.ca}"
mysql:
  env:
    DB_HOST: "${host}"
    DB_PORT: "${port}"
    RESOURCE_URL: "${url}"
    SHARED_VALUE: resource
    RESOURCE_TLS_KEY: "${tls.key}"
    RESOURCE_TLS_CERT: "${tls.cert}"
    RESOURCE_TLS_CA: "${tls.ca}"
  allocations:
    app:
      env:
        BOOL_VALUE: true
        DATABASE_URL: "mysql://${username}:${password}@${host}:${port}/${database}"
        DB_DATABASE: "${database}"
        ESCAPED_VALUE: "$${database} $$${database} $$"
        NUMBER_VALUE: 42
        SHARED_VALUE: allocation
        ALLOCATION_TLS_KEY: "${tls.key}"
        ALLOCATION_TLS_CERT: "${tls.cert}"
        ALLOCATION_TLS_CA: "${tls.ca}"
rustfs:
  env:
    AWS_ENDPOINT: "${endpoint}"
    AWS_URL: "${url}"
  allocations:
    uploads:
      env:
        AWS_ACCESS_KEY_ID: "${access_key}"
        AWS_BUCKET: "${bucket}"
        AWS_SECRET_ACCESS_KEY: "${secret_key}"
"#,
    )?;
    let context = project_context(&[
        (
            "mysql",
            ResourceEnvContext {
                track: "8.4".to_string(),
                // TLS paths always come from the Project, never a resource or allocation.
                values: values(&[
                    ("host", "127.0.0.1"),
                    ("password", "secret"),
                    ("port", "3306"),
                    ("url", "mysql://root:secret@127.0.0.1:3306"),
                    ("username", "root"),
                    ("tls.key", "/unexpected/resource.key"),
                    ("tls.cert", "/unexpected/resource.crt"),
                    ("tls.ca", "/unexpected/resource-ca.pem"),
                ]),
                allocations: allocations(&[(
                    "app",
                    AllocationEnvContext {
                        generated_name: "acme_test_app".to_string(),
                        values: values(&[
                            ("database", "acme_test_app"),
                            ("tls.key", "/unexpected/allocation.key"),
                            ("tls.cert", "/unexpected/allocation.crt"),
                            ("tls.ca", "/unexpected/allocation-ca.pem"),
                        ]),
                    },
                )]),
            },
        ),
        (
            "rustfs",
            ResourceEnvContext {
                track: "2026.1".to_string(),
                values: values(&[
                    ("access_key", "pv-access"),
                    ("endpoint", "http://127.0.0.1:9000"),
                    ("secret_key", "pv-secret"),
                    ("url", "http://127.0.0.1:9001"),
                ]),
                allocations: allocations(&[(
                    "uploads",
                    AllocationEnvContext {
                        generated_name: "acme-test-uploads".to_string(),
                        values: values(&[("bucket", "acme-test-uploads")]),
                    },
                )]),
            },
        ),
    ]);

    let rendered = render_project_env(&config, &context)?;

    assert_debug_snapshot!((&rendered, format_project_env(&rendered)));

    Ok(())
}

#[test]
fn resource_only_env_omits_serving_placeholders_across_scopes() -> Result<()> {
    let config = ProjectConfig::parse(
        r#"
serve: false
env:
  APP_ENV: local
  APP_URL: "${url}"
  TLS_KEY: "${tls.key}"
mysql:
  env:
    DB_HOST: "${host}"
    DATABASE_URL: "${url}"
    TLS_CA: "${tls.ca}"
  allocations:
    app:
      env:
        DB_DATABASE: "${database}"
        ALLOCATION_URL: "${url}"
        TLS_CERT: "${tls.cert}"
"#,
    )?;
    let context = ProjectEnvContext {
        primary_hostname: String::new(),
        tls_ca_path: String::new(),
        tls_cert_path: String::new(),
        tls_key_path: String::new(),
        resources: BTreeMap::from([(
            "mysql".to_string(),
            ResourceEnvContext {
                track: "8.4".to_string(),
                values: values(&[
                    ("host", "127.0.0.1"),
                    ("url", "mysql://root@127.0.0.1:3306"),
                ]),
                allocations: allocations(&[(
                    "app",
                    AllocationEnvContext {
                        generated_name: "acme_app".to_string(),
                        values: values(&[
                            ("database", "acme_app"),
                            ("url", "mysql://root@127.0.0.1:3306/acme_app"),
                        ]),
                    },
                )]),
            },
        )]),
    };

    let rendered = render_project_env(&config, &context)?;

    assert_debug_snapshot!((&rendered, format_project_env(&rendered)));

    Ok(())
}

#[test]
fn project_env_renderer_rejects_same_depth_duplicate_keys() -> Result<()> {
    let config = ProjectConfig::parse(
        r#"
mysql:
  allocations:
    app:
      env:
        DATABASE_URL: "mysql://app/${database}"
    analytics:
      env:
        DATABASE_URL: "mysql://analytics/${database}"
"#,
    )?;
    let context = project_context(&[(
        "mysql",
        ResourceEnvContext {
            track: "8.4".to_string(),
            values: values(&[
                ("host", "127.0.0.1"),
                ("password", "secret"),
                ("port", "3306"),
                ("username", "root"),
            ]),
            allocations: allocations(&[
                (
                    "analytics",
                    AllocationEnvContext {
                        generated_name: "acme_test_analytics".to_string(),
                        values: values(&[("database", "acme_test_analytics")]),
                    },
                ),
                (
                    "app",
                    AllocationEnvContext {
                        generated_name: "acme_test_app".to_string(),
                        values: values(&[("database", "acme_test_app")]),
                    },
                ),
            ]),
        },
    )]);

    let duplicate = render_project_env(&config, &context);

    assert_debug_snapshot!(&duplicate);
    assert!(matches!(
        duplicate,
        Err(ConfigError::DuplicateRenderedEnvKey { key, .. }) if key == "DATABASE_URL"
    ));

    Ok(())
}

#[test]
fn project_env_formatting_quotes_and_escapes_dotenv_values() {
    let rendered = RenderedProjectEnv {
        values: values(&[
            ("BACKSLASH", r"C:\tmp\pv"),
            ("DOLLAR", "pa$$word"),
            ("EMPTY", ""),
            ("HASH", "value#fragment"),
            ("MULTILINE", "first\nsecond"),
            ("QUOTE", r#"say "hi""#),
            ("SAFE", "https://acme.test/path"),
            ("SPACE", "hello world"),
        ]),
    };

    assert_debug_snapshot!(format_project_env(&rendered));
}

#[test]
fn managed_env_block_transformer_replaces_appends_and_folds() -> Result<()> {
    let rendered = RenderedProjectEnv {
        values: values(&[
            ("APP_URL", "https://acme.test"),
            ("DB_PASSWORD", "pa$$ word"),
        ]),
    };
    let cases = vec![
        (
            "missing-file",
            transform_managed_env_block(None, &rendered)?,
        ),
        (
            "empty-file",
            transform_managed_env_block(Some(""), &rendered)?,
        ),
        (
            "append-comments-out-conflict",
            transform_managed_env_block(
                Some(
                    r#"APP_URL=https://user.test
USER_ONLY=1
"#,
                ),
                &rendered,
            )?,
        ),
        (
            "replace-one-block",
            transform_managed_env_block(
                Some(
                    r#"USER_ONLY=1
# >>> PV MANAGED
OLD_VALUE=stale
# <<< PV MANAGED
TAIL=1
"#,
                ),
                &rendered,
            )?,
        ),
        (
            "fold-multiple-blocks",
            transform_managed_env_block(
                Some(
                    r#"BEFORE=1
# >>> PV MANAGED
OLD_ONE=stale
# <<< PV MANAGED
BETWEEN=1
# >>> PV MANAGED
OLD_TWO=stale
# <<< PV MANAGED
AFTER=1
"#,
                ),
                &rendered,
            )?,
        ),
    ];

    assert_debug_snapshot!(cases);

    Ok(())
}

#[test]
fn managed_env_block_transformer_comments_out_and_restores_overridden_assignments() -> Result<()> {
    let both_keys = RenderedProjectEnv {
        values: values(&[("APP_URL", "https://acme.test"), ("DB_HOST", "127.0.0.1")]),
    };
    let app_url_only = RenderedProjectEnv {
        values: values(&[("APP_URL", "https://acme.test")]),
    };
    let existing = r#"APP_URL=https://user.test
  export DB_HOST=localhost
# APP_URL=https://commented.test
# pv: keep this note
APP_URL=https://second.test
# >>> PV MANAGED
OLD_VALUE=stale
# <<< PV MANAGED
DB_HOST=between
# >>> PV MANAGED
# <<< PV MANAGED
USER_ONLY=1
"#;

    let commented = transform_managed_env_block(Some(existing), &both_keys)?;
    let repeated = transform_managed_env_block(Some(&commented.content), &both_keys)?;
    let key_removed = transform_managed_env_block(Some(&repeated.content), &app_url_only)?;
    let all_removed =
        transform_managed_env_block(Some(&key_removed.content), &RenderedProjectEnv::default())?;
    let block_deleted = transform_managed_env_block(
        Some("# pv: APP_URL=https://user.test\nUSER_ONLY=1\n"),
        &RenderedProjectEnv::default(),
    )?;

    assert_eq!(repeated.content, commented.content);
    assert!(!repeated.changed);
    assert_debug_snapshot!(vec![
        ("commented", commented),
        ("repeated", repeated),
        ("key-removed", key_removed),
        ("all-removed", all_removed),
        ("block-deleted", block_deleted),
    ]);

    Ok(())
}

#[test]
fn managed_env_block_transformer_moves_multiline_values_and_tab_exports_with_their_assignment()
-> Result<()> {
    let rendered = RenderedProjectEnv {
        values: values(&[("APP_KEY", "base64:key"), ("APP_URL", "https://acme.test")]),
    };
    let existing = concat!(
        r#"APP_KEY="first
second \" still quoted
last"
CERT="-----BEGIN-----
APP_URL=https://embedded.test
-----END-----"
"#,
        "export\tAPP_URL=https://user.test\n",
    );

    let commented = transform_managed_env_block(Some(existing), &rendered)?;
    let restored =
        transform_managed_env_block(Some(&commented.content), &RenderedProjectEnv::default())?;

    assert_eq!(
        restored.content,
        format!("{existing}# >>> PV MANAGED\n# <<< PV MANAGED\n")
    );
    assert_debug_snapshot!((commented, restored));

    Ok(())
}

#[test]
fn managed_env_block_transformer_rejects_malformed_markers() {
    let rendered = RenderedProjectEnv {
        values: values(&[("APP_URL", "https://acme.test")]),
    };
    let malformed = vec![
        (
            "start-without-end",
            transform_managed_env_block(Some("# >>> PV MANAGED\nAPP_URL=old\n"), &rendered),
        ),
        (
            "end-without-start",
            transform_managed_env_block(Some("APP_URL=old\n# <<< PV MANAGED\n"), &rendered),
        ),
        (
            "nested-start",
            transform_managed_env_block(
                Some("# >>> PV MANAGED\n# >>> PV MANAGED\n# <<< PV MANAGED\n"),
                &rendered,
            ),
        ),
    ];

    assert_debug_snapshot!(malformed);
}

#[test]
fn project_env_writer_updates_existing_env_preserving_permissions_and_normalizing_newlines()
-> Result<()> {
    let tempdir = tempdir()?;
    let env_path = tempdir.path().join(".env");
    write_file(
        &env_path,
        "USER_ONLY=1\r\n# >>> PV MANAGED\r\nOLD_VALUE=stale\r\n# <<< PV MANAGED\r\nTAIL=1",
    )?;
    set_file_mode(&env_path, 0o640)?;
    let rendered = RenderedProjectEnv {
        values: values(&[("APP_URL", "https://acme.test")]),
    };

    let first_transform = write_project_env_file(&env_path, &rendered)?;
    let second_transform = write_project_env_file(&env_path, &rendered)?;

    assert_debug_snapshot!((
        first_transform,
        read_file(&env_path)?,
        mode_string(&env_path)?,
        second_transform,
    ));

    Ok(())
}

fn project_context(resources: &[(&str, ResourceEnvContext)]) -> ProjectEnvContext {
    project_context_with_primary("acme.test", resources)
}

fn project_context_with_primary(
    primary_hostname: &str,
    resources: &[(&str, ResourceEnvContext)],
) -> ProjectEnvContext {
    ProjectEnvContext {
        primary_hostname: primary_hostname.to_string(),
        tls_ca_path: "/Users/alice/.pv/certificates/ca.pem".to_string(),
        tls_cert_path: "/Users/alice/.pv/certificates/projects/project123/tls.crt".to_string(),
        tls_key_path: "/Users/alice/.pv/certificates/projects/project123/tls.key".to_string(),
        resources: resources
            .iter()
            .map(|(resource, context)| ((*resource).to_string(), context.clone()))
            .collect(),
    }
}

fn allocations(
    allocations: &[(&str, AllocationEnvContext)],
) -> BTreeMap<String, AllocationEnvContext> {
    allocations
        .iter()
        .map(|(allocation, context)| ((*allocation).to_string(), context.clone()))
        .collect()
}

fn values(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect()
}

#[expect(
    clippy::disallowed_methods,
    reason = "Project env tests read fixture files directly"
)]
fn read_file(path: &Utf8Path) -> Result<String> {
    Ok(std::fs::read_to_string(path)?)
}

#[expect(
    clippy::disallowed_methods,
    reason = "Project env tests write fixture files directly"
)]
fn write_file(path: &Utf8Path, content: &str) -> Result<()> {
    std::fs::write(path, content)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "Project env tests set fixture permissions directly"
)]
fn set_file_mode(path: &Utf8Path, mode: u32) -> Result<()> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "Project env tests inspect fixture permissions directly"
)]
fn mode_string(path: &Utf8Path) -> Result<String> {
    let mode = std::fs::metadata(path)?.permissions().mode() & 0o777;

    Ok(format!("{mode:o}"))
}
