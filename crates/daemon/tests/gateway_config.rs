use anyhow::Result;
use camino::Utf8PathBuf;
use daemon::DaemonError;
use daemon::gateway_config::{
    GatewayConfigInput, GatewayProjectRoute, PhpWorkerConfigInput, PhpWorkerProject,
    render_gateway_config, render_gateway_project_config, render_php_worker_config,
    render_php_worker_project_config,
};
use insta::assert_snapshot;

#[test]
fn gateway_config_renderer_outputs_gateway_caddyfile() -> Result<()> {
    let input = GatewayConfigInput {
        http_port: 48080,
        https_port: 48443,
        ca_certificate_path: Utf8PathBuf::from("/Users/alice/.pv/certificates/ca.pem"),
        ca_private_key_path: Utf8PathBuf::from("/Users/alice/.pv/certificates/ca-key.pem"),
        storage_path: Utf8PathBuf::from("/Users/alice/.pv/certificates/caddy"),
        projects_config_glob: Utf8PathBuf::from(
            "/Users/alice/.pv/config/gateway/projects/*.Caddyfile",
        ),
        import_project_configs: true,
    };

    assert_snapshot!(render_gateway_config(&input)?);

    Ok(())
}

#[test]
fn worker_config_renderer_outputs_track_caddyfile() -> Result<()> {
    let input = PhpWorkerConfigInput {
        php_track: "8.4".to_owned(),
        port: 45001,
        projects_config_glob: Utf8PathBuf::from(
            "/Users/alice/.pv/config/workers/php-8.4/projects/*.Caddyfile",
        ),
        projects: vec![PhpWorkerProject {
            primary_hostname: "acme.test".to_owned(),
            hostnames: vec!["api.acme.test".to_owned()],
            project_root: Utf8PathBuf::from("/Users/alice/Code/acme"),
            document_root: Utf8PathBuf::from("/Users/alice/Code/acme/public"),
        }],
    };

    let rendered = render_php_worker_config(&input)?;

    assert!(!rendered.contains("php_ini"));
    assert_snapshot!(rendered);

    Ok(())
}

#[test]
fn config_renderers_quote_path_tokens_with_spaces() -> Result<()> {
    let gateway = render_gateway_config(&GatewayConfigInput {
        http_port: 48080,
        https_port: 48443,
        ca_certificate_path: Utf8PathBuf::from("/Users/Alice Smith/.pv/certificates/ca.pem"),
        ca_private_key_path: Utf8PathBuf::from("/Users/Alice Smith/.pv/certificates/ca-key.pem"),
        storage_path: Utf8PathBuf::from("/Users/Alice Smith/.pv/certificates/caddy"),
        projects_config_glob: Utf8PathBuf::from(
            "/Users/Alice Smith/.pv/config/gateway/projects/*.Caddyfile",
        ),
        import_project_configs: true,
    })?;
    let worker = render_php_worker_config(&PhpWorkerConfigInput {
        php_track: "8.4".to_owned(),
        port: 45001,
        projects_config_glob: Utf8PathBuf::from(
            "/Users/Alice Smith/.pv/config/workers/php-8.4/projects/*.Caddyfile",
        ),
        projects: vec![PhpWorkerProject {
            primary_hostname: "acme.test".to_owned(),
            hostnames: vec![],
            project_root: Utf8PathBuf::from("/Users/Alice Smith/Code/acme"),
            document_root: Utf8PathBuf::from("/Users/Alice Smith/Code/acme/public"),
        }],
    })?;

    assert_snapshot!(format!("Gateway:\n{gateway}\nWorker:\n{worker}"));

    Ok(())
}

#[test]
fn config_renderers_reject_control_characters_in_path_tokens() {
    let result = render_gateway_config(&GatewayConfigInput {
        http_port: 48080,
        https_port: 48443,
        ca_certificate_path: Utf8PathBuf::from("/Users/alice/.pv/certificates/ca\n.pem"),
        ca_private_key_path: Utf8PathBuf::from("/Users/alice/.pv/certificates/ca-key.pem"),
        storage_path: Utf8PathBuf::from("/Users/alice/.pv/certificates/caddy"),
        projects_config_glob: Utf8PathBuf::from(
            "/Users/alice/.pv/config/gateway/projects/*.Caddyfile",
        ),
        import_project_configs: true,
    });

    assert!(matches!(
        result,
        Err(DaemonError::UnexpectedProtocolResponse { reason })
            if reason.contains("control character")
    ));
}

#[test]
fn gateway_config_renderer_outputs_empty_gateway_listener() -> Result<()> {
    let input = GatewayConfigInput {
        http_port: 48080,
        https_port: 48443,
        ca_certificate_path: Utf8PathBuf::from("/Users/alice/.pv/certificates/ca.pem"),
        ca_private_key_path: Utf8PathBuf::from("/Users/alice/.pv/certificates/ca-key.pem"),
        storage_path: Utf8PathBuf::from("/Users/alice/.pv/certificates/caddy"),
        projects_config_glob: Utf8PathBuf::from(
            "/Users/alice/.pv/config/gateway/projects/*.Caddyfile",
        ),
        import_project_configs: false,
    };

    let rendered = render_gateway_config(&input)?;

    assert!(
        rendered
            .lines()
            .any(|line| line == "    bind 127.0.0.1 ::1"),
        "empty Gateway fallback must bind only loopback interfaces"
    );
    assert_snapshot!(rendered);

    Ok(())
}

#[test]
fn gateway_config_renderer_imports_project_configs_when_requested() -> Result<()> {
    let input = GatewayConfigInput {
        http_port: 48080,
        https_port: 48443,
        ca_certificate_path: Utf8PathBuf::from("/Users/alice/.pv/certificates/ca.pem"),
        ca_private_key_path: Utf8PathBuf::from("/Users/alice/.pv/certificates/ca-key.pem"),
        storage_path: Utf8PathBuf::from("/Users/alice/.pv/certificates/caddy"),
        projects_config_glob: Utf8PathBuf::from(
            "/Users/alice/.pv/config/gateway/projects/*.Caddyfile",
        ),
        import_project_configs: true,
    };

    assert_snapshot!(render_gateway_config(&input)?);

    Ok(())
}

#[test]
fn gateway_project_config_renderer_outputs_project_caddyfile() -> Result<()> {
    let route = GatewayProjectRoute {
        id: "project_acme".to_owned(),
        render_config: true,
        primary_hostname: "acme.test".to_owned(),
        hostnames: vec!["api.acme.test".to_owned()],
        worker_port: 45001,
    };

    assert_snapshot!(render_gateway_project_config(&route)?);

    Ok(())
}

#[test]
fn worker_project_config_renderer_outputs_project_caddyfile() -> Result<()> {
    let project = PhpWorkerProject {
        primary_hostname: "acme.test".to_owned(),
        hostnames: vec!["api.acme.test".to_owned()],
        project_root: Utf8PathBuf::from("/Users/alice/Code/acme"),
        document_root: Utf8PathBuf::from("/Users/alice/Code/acme/public"),
    };

    assert_snapshot!(render_php_worker_project_config(&project, 45001)?);

    Ok(())
}
