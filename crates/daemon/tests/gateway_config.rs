use anyhow::Result;
use camino::Utf8PathBuf;
use daemon::gateway_config::{
    GatewayConfigInput, GatewayProjectRoute, PhpWorkerConfigInput, PhpWorkerProject,
    render_gateway_config, render_php_worker_config,
};
use insta::assert_snapshot;

#[test]
fn gateway_config_renderer_outputs_gateway_caddyfile() -> Result<()> {
    let input = GatewayConfigInput {
        http_port: 48080,
        https_port: 48443,
        ca_certificate_path: Utf8PathBuf::from("/Users/alice/.pv/certificates/ca.pem"),
        ca_private_key_path: Utf8PathBuf::from("/Users/alice/.pv/certificates/ca-key.pem"),
        routes: vec![GatewayProjectRoute {
            primary_hostname: "acme.test".to_owned(),
            hostnames: vec!["api.acme.test".to_owned()],
            worker_port: 45001,
        }],
    };

    assert_snapshot!(render_gateway_config(&input)?);

    Ok(())
}

#[test]
fn worker_config_renderer_outputs_track_caddyfile() -> Result<()> {
    let input = PhpWorkerConfigInput {
        php_track: "8.4".to_owned(),
        port: 45001,
        projects: vec![PhpWorkerProject {
            primary_hostname: "acme.test".to_owned(),
            hostnames: vec!["api.acme.test".to_owned()],
            project_root: Utf8PathBuf::from("/Users/alice/Code/acme"),
            document_root: Utf8PathBuf::from("/Users/alice/Code/acme/public"),
        }],
    };

    assert_snapshot!(render_php_worker_config(&input)?);

    Ok(())
}

#[test]
fn config_renderers_quote_path_tokens_with_spaces() -> Result<()> {
    let gateway = render_gateway_config(&GatewayConfigInput {
        http_port: 48080,
        https_port: 48443,
        ca_certificate_path: Utf8PathBuf::from("/Users/Alice Smith/.pv/certificates/ca.pem"),
        ca_private_key_path: Utf8PathBuf::from("/Users/Alice Smith/.pv/certificates/ca-key.pem"),
        routes: vec![GatewayProjectRoute {
            primary_hostname: "acme.test".to_owned(),
            hostnames: vec![],
            worker_port: 45001,
        }],
    })?;
    let worker = render_php_worker_config(&PhpWorkerConfigInput {
        php_track: "8.4".to_owned(),
        port: 45001,
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
