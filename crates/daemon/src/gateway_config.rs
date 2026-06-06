use camino::Utf8PathBuf;

use crate::DaemonError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayConfigInput {
    pub http_port: u16,
    pub https_port: u16,
    pub ca_certificate_path: Utf8PathBuf,
    pub ca_private_key_path: Utf8PathBuf,
    pub routes: Vec<GatewayProjectRoute>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayProjectRoute {
    pub primary_hostname: String,
    pub hostnames: Vec<String>,
    pub worker_port: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhpWorkerConfigInput {
    pub php_track: String,
    pub port: u16,
    pub projects: Vec<PhpWorkerProject>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhpWorkerProject {
    pub primary_hostname: String,
    pub hostnames: Vec<String>,
    pub project_root: Utf8PathBuf,
    pub document_root: Utf8PathBuf,
}

pub fn render_gateway_config(input: &GatewayConfigInput) -> Result<String, DaemonError> {
    let mut output = String::new();
    output.push_str("{\n");
    output.push_str("    admin off\n");
    output.push_str(&format!("    http_port {}\n", input.http_port));
    output.push_str(&format!("    https_port {}\n", input.https_port));
    output.push_str("    pki {\n");
    output.push_str("        ca local {\n");
    output.push_str("            root {\n");
    output.push_str("                format pem_file\n");
    output.push_str(&format!(
        "                cert {}\n",
        quoted_caddyfile_token(input.ca_certificate_path.as_str())
    ));
    output.push_str(&format!(
        "                key {}\n",
        quoted_caddyfile_token(input.ca_private_key_path.as_str())
    ));
    output.push_str("            }\n");
    output.push_str("        }\n");
    output.push_str("    }\n");
    output.push_str("}\n");

    let mut routes: Vec<&GatewayProjectRoute> = input.routes.iter().collect();
    routes.sort_by(|left, right| left.primary_hostname.cmp(&right.primary_hostname));

    for route in routes {
        output.push('\n');
        output.push_str(&format!(
            "{} {{\n",
            comma_separated_hostnames(&route.primary_hostname, &route.hostnames)?
        ));
        output.push_str("    bind 127.0.0.1 ::1\n");
        output.push_str("    tls {\n");
        output.push_str("        issuer internal {\n");
        output.push_str("            ca local\n");
        output.push_str("        }\n");
        output.push_str("    }\n");
        output.push_str(&format!(
            "    reverse_proxy 127.0.0.1:{} {{\n",
            route.worker_port
        ));
        output.push_str("        header_up Host {host}\n");
        output.push_str("        header_up X-Forwarded-Host {host}\n");
        output.push_str("        header_up X-Forwarded-Proto {scheme}\n");
        output.push_str("        header_up X-Forwarded-For {remote_host}\n");
        output.push_str("    }\n");
        output.push_str("}\n");
    }

    Ok(output)
}

pub fn render_php_worker_config(input: &PhpWorkerConfigInput) -> Result<String, DaemonError> {
    let mut output = String::new();
    let mut projects: Vec<&PhpWorkerProject> = input.projects.iter().collect();
    projects.sort_by(|left, right| left.primary_hostname.cmp(&right.primary_hostname));

    for (index, project) in projects.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }

        output.push_str(&format!(
            "{} {{\n",
            comma_separated_worker_sites(
                &project.primary_hostname,
                &project.hostnames,
                input.port
            )?
        ));
        output.push_str("    bind 127.0.0.1 ::1\n");
        output.push_str(&format!(
            "    root * {}\n",
            quoted_caddyfile_token(project.document_root.as_str())
        ));
        output.push_str("    php_server\n");
        output.push_str("    file_server\n");
        output.push_str("}\n");
    }

    Ok(output)
}

fn comma_separated_hostnames(
    primary_hostname: &str,
    hostnames: &[String],
) -> Result<String, DaemonError> {
    let hostnames = sorted_hostnames(primary_hostname, hostnames)?;

    Ok(hostnames.join(", "))
}

fn comma_separated_worker_sites(
    primary_hostname: &str,
    hostnames: &[String],
    port: u16,
) -> Result<String, DaemonError> {
    let hostnames = sorted_hostnames(primary_hostname, hostnames)?;
    let sites = hostnames
        .into_iter()
        .map(|hostname| format!("http://{hostname}:{port}"))
        .collect::<Vec<_>>();

    Ok(sites.join(", "))
}

fn sorted_hostnames<'input>(
    primary_hostname: &'input str,
    hostnames: &'input [String],
) -> Result<Vec<&'input str>, DaemonError> {
    if primary_hostname.is_empty() {
        return Err(DaemonError::UnexpectedProtocolResponse {
            reason: "gateway config requires a primary hostname per route".to_owned(),
        });
    }

    let mut hostnames = hostnames.iter().map(String::as_str).collect::<Vec<_>>();
    hostnames.push(primary_hostname);
    hostnames.sort_unstable();
    hostnames.dedup();

    Ok(hostnames)
}

fn quoted_caddyfile_token(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");

    format!("\"{escaped}\"")
}
