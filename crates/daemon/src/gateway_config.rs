use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};

use camino::{Utf8Path, Utf8PathBuf};
use state::fs;

use crate::DaemonError;

static CANDIDATE_CONFIG_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayConfigInput {
    pub http_port: u16,
    pub https_port: u16,
    pub ca_certificate_path: Utf8PathBuf,
    pub ca_private_key_path: Utf8PathBuf,
    pub projects_config_glob: Utf8PathBuf,
    pub routes: Vec<GatewayProjectRoute>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayProjectRoute {
    pub id: String,
    pub render_config: bool,
    pub primary_hostname: String,
    pub hostnames: Vec<String>,
    pub worker_port: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhpWorkerConfigInput {
    pub php_track: String,
    pub port: u16,
    pub projects_config_glob: Utf8PathBuf,
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
    output.push_str(&format!("# PV_FAKE_PORT {}\n", input.http_port));
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

    if routes.is_empty() {
        output.push('\n');
        output.push_str(&format!(":{} {{\n", input.http_port));
        output.push_str("    bind 127.0.0.1 ::1\n");
        output.push_str("    respond \"PV Gateway is running\" 404\n");
        output.push_str("}\n");
    } else {
        output.push('\n');
        output.push_str(&format!(
            "import {}\n",
            quoted_caddyfile_token(input.projects_config_glob.as_str())
        ));
    }

    Ok(output)
}

pub fn render_php_worker_config(input: &PhpWorkerConfigInput) -> Result<String, DaemonError> {
    let mut output = String::new();
    output.push_str(&format!("# PV_FAKE_PORT {}\n", input.port));
    if !input.projects.is_empty() {
        output.push_str(&format!(
            "import {}\n",
            quoted_caddyfile_token(input.projects_config_glob.as_str())
        ));
    }

    Ok(output)
}

pub fn render_gateway_project_config(route: &GatewayProjectRoute) -> Result<String, DaemonError> {
    let mut output = String::new();

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

    Ok(output)
}

pub fn render_php_worker_project_config(
    project: &PhpWorkerProject,
    port: u16,
) -> Result<String, DaemonError> {
    let mut output = String::new();

    output.push_str(&format!(
        "{} {{\n",
        comma_separated_worker_sites(&project.primary_hostname, &project.hostnames, port)?
    ));
    output.push_str("    bind 127.0.0.1 ::1\n");
    output.push_str(&format!(
        "    root * {}\n",
        quoted_caddyfile_token(project.document_root.as_str())
    ));
    output.push_str("    php_server\n");
    output.push_str("    file_server\n");
    output.push_str("}\n");

    Ok(output)
}

pub(crate) async fn promote_validated_config_tree_async<Validate, Validation, Promote>(
    path: &Utf8Path,
    candidate_content: &str,
    active_content: &str,
    validate: Validate,
    promote_fragments: Promote,
) -> Result<(), DaemonError>
where
    Validate: FnOnce(Utf8PathBuf) -> Validation,
    Validation: Future<Output = Result<(), DaemonError>>,
    Promote: FnOnce() -> Result<(), DaemonError>,
{
    let candidate_path = candidate_path_for(path);
    write_candidate_config(&candidate_path, candidate_content)?;

    if let Err(error) = validate(candidate_path.clone()).await {
        let _cleanup_result = remove_candidate_config(&candidate_path);

        return Err(error);
    }

    write_candidate_config(&candidate_path, active_content)?;
    let backup_path = backup_path_for(path);
    let active_existed = path.exists();
    if let Err(error) = delete_optional_config(&backup_path) {
        let _cleanup_result = remove_candidate_config(&candidate_path);

        return Err(error);
    }
    if active_existed && let Err(error) = rename_candidate_config(path, &backup_path) {
        let _cleanup_result = remove_candidate_config(&candidate_path);

        return Err(error);
    }
    if let Err(error) = rename_candidate_config(&candidate_path, path) {
        if active_existed {
            let _restore_result = rename_candidate_config(&backup_path, path);
        }
        let _cleanup_result = remove_candidate_config(&candidate_path);

        return Err(error);
    }
    if let Err(error) = promote_fragments() {
        let _active_cleanup_result = remove_candidate_config(path);
        if active_existed {
            let _restore_result = rename_candidate_config(&backup_path, path);
        }

        return Err(error);
    }
    if active_existed {
        let _cleanup_result = remove_candidate_config(&backup_path);
    }

    Ok(())
}

pub(crate) fn promote_validated_config(
    path: &Utf8Path,
    content: &str,
    validate: impl FnOnce(&Utf8Path) -> Result<(), DaemonError>,
) -> Result<(), DaemonError> {
    let candidate_path = candidate_path_for(path);
    write_candidate_config(&candidate_path, content)?;

    if let Err(error) = validate(&candidate_path) {
        let _cleanup_result = remove_candidate_config(&candidate_path);

        return Err(error);
    }

    rename_candidate_config(&candidate_path, path)?;

    Ok(())
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

fn candidate_path_for(path: &Utf8Path) -> Utf8PathBuf {
    let file_name = path.file_name().unwrap_or("config");
    let process_id = std::process::id();
    let counter = CANDIDATE_CONFIG_COUNTER.fetch_add(1, Ordering::Relaxed);

    path.with_file_name(format!("{file_name}.candidate.{process_id}.{counter}.tmp"))
}

fn backup_path_for(path: &Utf8Path) -> Utf8PathBuf {
    let file_name = path.file_name().unwrap_or("config");
    let process_id = std::process::id();
    let counter = CANDIDATE_CONFIG_COUNTER.fetch_add(1, Ordering::Relaxed);

    path.with_file_name(format!("{file_name}.previous.{process_id}.{counter}.tmp"))
}

fn write_candidate_config(path: &Utf8Path, content: &str) -> Result<(), DaemonError> {
    fs::write_sensitive_file(path, content)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "daemon gateway config promotion owns direct candidate file replacement"
)]
fn rename_candidate_config(from: &Utf8Path, to: &Utf8Path) -> Result<(), DaemonError> {
    std::fs::rename(from, to)?;

    Ok(())
}

fn remove_candidate_config(path: &Utf8Path) -> Result<(), DaemonError> {
    fs::delete_file(path)?;

    Ok(())
}

fn delete_optional_config(path: &Utf8Path) -> Result<(), DaemonError> {
    match fs::delete_file(path) {
        Ok(()) => Ok(()),
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}
