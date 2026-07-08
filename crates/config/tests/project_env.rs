use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;

use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use config::{
    AllocationEnvContext, ConfigError, ProjectConfig, ProjectEnvContext, RenderedProjectEnv,
    ResourceEnvContext, format_project_env, render_project_env, transform_managed_env_block,
    validate_project_env_shape, write_project_env_file,
};
use insta::assert_debug_snapshot;

#[test]
fn project_env_renderer_returns_empty_output_for_no_mappings() -> Result<()> {
    let config = ProjectConfig::parse(
        r#"
mysql:
  version: "8.4"
"#,
    )?;
    let context = ProjectEnvContext {
        primary_hostname: "acme.test".to_string(),
        tls_ca_path: "/Users/alice/.pv/certificates/ca.pem".to_string(),
        tls_cert_path: "/Users/alice/.pv/certificates/projects/project123/tls.crt".to_string(),
        tls_key_path: "/Users/alice/.pv/certificates/projects/project123/tls.key".to_string(),
        resources: BTreeMap::new(),
    };

    let rendered = render_project_env(&config, &context)?;
    let transformed = transform_managed_env_block(
        Some(
            r#"USER_VALUE=1
# >>> PV MANAGED
OLD_VALUE=stays
# <<< PV MANAGED
"#,
        ),
        &rendered,
    )?;

    assert_debug_snapshot!((
        &rendered,
        format_project_env(&RenderedProjectEnv::default()),
        transformed
    ));

    Ok(())
}

#[test]
fn project_env_renderer_resolves_project_resource_and_allocation_contexts() -> Result<()> {
    let config = ProjectConfig::parse(
        r#"
env:
  APP_URL: "${project_url}"
  SHARED_VALUE: root
mysql:
  env:
    DB_HOST: "${host}"
    DB_PORT: "${port}"
    RESOURCE_URL: "${project_url}/mysql"
    SHARED_VALUE: resource
  allocations:
    app:
      env:
        BOOL_VALUE: true
        DATABASE_URL: "mysql://${username}:${password}@${host}:${port}/${database}"
        DB_DATABASE: "${database}"
        ESCAPED_VALUE: "$${database} $$${database} $$"
        NUMBER_VALUE: 42
        SHARED_VALUE: allocation
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
                values: values(&[
                    ("host", "127.0.0.1"),
                    ("password", "secret"),
                    ("port", "3306"),
                    ("username", "root"),
                ]),
                allocations: allocations(&[(
                    "app",
                    AllocationEnvContext {
                        generated_name: "acme_test_app".to_string(),
                        values: values(&[("database", "acme_test_app")]),
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
fn project_env_renderer_resolves_project_url_from_primary_hostname_across_scopes() -> Result<()> {
    let config = ProjectConfig::parse(
        r#"
env:
  ROOT_PROJECT_URL: "${project_url}"
mysql:
  env:
    RESOURCE_PROJECT_URL: "${project_url}"
    RESOURCE_SERVICE_URL: "${url}"
  allocations:
    app:
      env:
        ALLOCATION_PROJECT_URL: "${project_url}"
        ALLOCATION_SERVICE_URL: "${url}"
"#,
    )?;
    let context = project_context_with_primary(
        "primary.acme.test",
        &[(
            "mysql",
            ResourceEnvContext {
                track: "8.4".to_string(),
                values: values(&[
                    ("host", "127.0.0.1"),
                    ("password", "secret"),
                    ("port", "3306"),
                    ("project_url", "https://resource-context.test"),
                    ("url", "mysql://root:secret@127.0.0.1:3306"),
                    ("username", "root"),
                ]),
                allocations: allocations(&[(
                    "app",
                    AllocationEnvContext {
                        generated_name: "acme_test_app".to_string(),
                        values: values(&[
                            ("database", "acme_test_app"),
                            ("project_url", "https://allocation-context.test"),
                            ("url", "mysql://root:secret@127.0.0.1:3306/acme_test_app"),
                        ]),
                    },
                )]),
            },
        )],
    );

    let rendered = render_project_env(&config, &context)?;

    assert_eq!(
        rendered.values.get("ROOT_PROJECT_URL").map(String::as_str),
        Some("https://primary.acme.test")
    );
    assert_eq!(
        rendered
            .values
            .get("RESOURCE_PROJECT_URL")
            .map(String::as_str),
        Some("https://primary.acme.test")
    );
    assert_eq!(
        rendered
            .values
            .get("ALLOCATION_PROJECT_URL")
            .map(String::as_str),
        Some("https://primary.acme.test")
    );
    assert_debug_snapshot!((&rendered, format_project_env(&rendered)));

    Ok(())
}

#[test]
fn project_env_renderer_resolves_tls_paths_across_scopes() -> Result<()> {
    let config = ProjectConfig::parse(
        r#"
env:
  ROOT_TLS_KEY: "${tls_key}"
  ROOT_TLS_CERT: "${tls_cert}"
  ROOT_TLS_CA: "${tls_ca}"
mysql:
  env:
    RESOURCE_TLS_KEY: "${tls_key}"
    RESOURCE_TLS_CERT: "${tls_cert}"
    RESOURCE_TLS_CA: "${tls_ca}"
  allocations:
    app:
      env:
        ALLOCATION_TLS_KEY: "${tls_key}"
        ALLOCATION_TLS_CERT: "${tls_cert}"
        ALLOCATION_TLS_CA: "${tls_ca}"
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
                ("tls_key", "/unexpected/resource.key"),
                ("tls_cert", "/unexpected/resource.crt"),
                ("tls_ca", "/unexpected/resource-ca.pem"),
                ("username", "root"),
            ]),
            allocations: allocations(&[(
                "app",
                AllocationEnvContext {
                    generated_name: "acme_test_app".to_string(),
                    values: values(&[
                        ("database", "acme_test_app"),
                        ("tls_key", "/unexpected/allocation.key"),
                        ("tls_cert", "/unexpected/allocation.crt"),
                        ("tls_ca", "/unexpected/allocation-ca.pem"),
                    ]),
                },
            )]),
        },
    )]);

    let rendered = render_project_env(&config, &context)?;

    assert_debug_snapshot!((&rendered, format_project_env(&rendered)));

    Ok(())
}

#[test]
fn project_env_renderer_reports_missing_contexts() -> Result<()> {
    let resource_config = ProjectConfig::parse(
        r#"
mysql:
  env:
    DB_HOST: "${host}"
"#,
    )?;
    let missing_resource = render_project_env(&resource_config, &project_context(&[]));
    assert!(matches!(
        missing_resource,
        Err(ConfigError::MissingResourceEnvContext { resource }) if resource == "mysql"
    ));

    let missing_value = render_project_env(
        &resource_config,
        &project_context(&[("mysql", ResourceEnvContext::default())]),
    );
    assert!(matches!(
        missing_value,
        Err(ConfigError::MissingEnvContext { field, placeholder })
            if field == "mysql.env.DB_HOST" && placeholder == "host"
    ));

    let allocation_config = ProjectConfig::parse(
        r#"
mysql:
  allocations:
    app:
      env:
        DB_DATABASE: "${database}"
"#,
    )?;
    let missing_allocation = render_project_env(
        &allocation_config,
        &project_context(&[(
            "mysql",
            ResourceEnvContext {
                track: "8.4".to_string(),
                values: values(&[
                    ("host", "127.0.0.1"),
                    ("password", "secret"),
                    ("port", "3306"),
                    ("username", "root"),
                ]),
                allocations: BTreeMap::new(),
            },
        )]),
    );
    assert!(matches!(
        missing_allocation,
        Err(ConfigError::MissingAllocationEnvContext {
            resource,
            allocation,
        }) if resource == "mysql" && allocation == "app"
    ));

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
fn project_env_renderer_rejects_same_depth_resource_duplicate_keys() -> Result<()> {
    let config = ProjectConfig::parse(
        r#"
mysql:
  env:
    DB_HOST: "${host}"
redis:
  env:
    DB_HOST: "${host}"
"#,
    )?;
    let context = project_context(&[
        (
            "mysql",
            ResourceEnvContext {
                track: "8.4".to_string(),
                values: values(&[("host", "127.0.0.1")]),
                allocations: BTreeMap::new(),
            },
        ),
        (
            "redis",
            ResourceEnvContext {
                track: "7.2".to_string(),
                values: values(&[("host", "127.0.0.1")]),
                allocations: BTreeMap::new(),
            },
        ),
    ]);

    let validation = validate_project_env_shape(&config);
    let duplicate = render_project_env(&config, &context);

    assert_debug_snapshot!((&validation, &duplicate));
    assert!(matches!(
        validation,
        Err(ConfigError::DuplicateRenderedEnvKey { key, .. }) if key == "DB_HOST"
    ));
    assert!(matches!(
        duplicate,
        Err(ConfigError::DuplicateRenderedEnvKey { key, .. }) if key == "DB_HOST"
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
fn managed_env_block_transformer_replaces_appends_folds_and_warns() -> Result<()> {
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
            "append-with-duplicate-warning",
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
fn project_env_writer_creates_missing_env_with_private_permissions() -> Result<()> {
    let tempdir = tempdir()?;
    let env_path = tempdir.path().join(".env");
    let rendered = RenderedProjectEnv {
        values: values(&[
            ("APP_URL", "https://acme.test"),
            ("DB_PASSWORD", "secret value"),
        ]),
    };

    let transform = write_project_env_file(&env_path, &rendered)?;

    assert_debug_snapshot!((transform, read_file(&env_path)?, mode_string(&env_path)?));

    Ok(())
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

#[test]
fn project_env_writer_leaves_existing_env_unchanged_on_malformed_block() -> Result<()> {
    let tempdir = tempdir()?;
    let env_path = tempdir.path().join(".env");
    write_file(&env_path, "USER_ONLY=1\n# >>> PV MANAGED\nAPP_URL=old\n")?;
    set_file_mode(&env_path, 0o640)?;
    let before_content = read_file(&env_path)?;
    let before_mode = mode_string(&env_path)?;
    let rendered = RenderedProjectEnv {
        values: values(&[("APP_URL", "https://acme.test")]),
    };

    let result = write_project_env_file(&env_path, &rendered);
    let after_content = read_file(&env_path)?;
    let after_mode = mode_string(&env_path)?;

    assert_eq!(before_content, after_content);
    assert_eq!(before_mode, after_mode);
    assert_debug_snapshot!((result, after_content, after_mode));

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
