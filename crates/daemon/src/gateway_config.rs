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
    pub import_project_configs: bool,
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
        quoted_caddyfile_token(input.ca_certificate_path.as_str())?
    ));
    output.push_str(&format!(
        "                key {}\n",
        quoted_caddyfile_token(input.ca_private_key_path.as_str())?
    ));
    output.push_str("            }\n");
    output.push_str("        }\n");
    output.push_str("    }\n");
    output.push_str("}\n");

    if input.import_project_configs {
        output.push('\n');
        output.push_str(&format!(
            "import {}\n",
            quoted_caddyfile_token(input.projects_config_glob.as_str())?
        ));
    } else {
        output.push('\n');
        output.push_str(&format!(":{} {{\n", input.http_port));
        output.push_str("    bind 127.0.0.1 ::1\n");
        output.push_str("    respond \"PV Gateway is running\" 404\n");
        output.push_str("}\n");
    }

    Ok(output)
}

pub fn render_php_worker_config(input: &PhpWorkerConfigInput) -> Result<String, DaemonError> {
    let mut output = String::new();
    output.push_str(&format!("# PV_FAKE_PORT {}\n", input.port));
    if !input.projects.is_empty() {
        output.push_str(&format!(
            "import {}\n",
            quoted_caddyfile_token(input.projects_config_glob.as_str())?
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
        quoted_caddyfile_token(project.document_root.as_str())?
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
) -> Result<PromotedConfigTree, DaemonError>
where
    Validate: FnOnce(Utf8PathBuf) -> Validation,
    Validation: Future<Output = Result<(), DaemonError>>,
    Promote: FnOnce() -> Result<PromotedConfigDir, DaemonError>,
{
    let candidate_path = candidate_path_for(path);
    write_candidate_config(&candidate_path, candidate_content)?;

    if let Err(error) = validate(candidate_path.clone()).await {
        let _cleanup_result = remove_candidate_config(&candidate_path);

        return Err(error);
    }

    write_candidate_config(&candidate_path, active_content)?;
    let root = match promote_config_file(path, &candidate_path) {
        Ok(root) => root,
        Err(error) => {
            let _cleanup_result = remove_candidate_config(&candidate_path);

            return Err(error);
        }
    };
    let fragments = match promote_fragments() {
        Ok(fragments) => fragments,
        Err(error) => {
            if let Err(restore_error) = root.rollback() {
                return Err(rollback_failed_error(error, restore_error));
            }

            return Err(error);
        }
    };

    Ok(PromotedConfigTree { root, fragments })
}

#[derive(Debug)]
pub(crate) struct PromotedConfigTree {
    root: PromotedConfigFile,
    fragments: PromotedConfigDir,
}

impl PromotedConfigTree {
    pub(crate) fn commit(self) -> Result<(), DaemonError> {
        self.fragments.commit()?;
        self.root.commit()
    }

    pub(crate) fn rollback(self) -> Result<(), DaemonError> {
        let fragments = self.fragments.rollback();
        let root = self.root.rollback();

        match (fragments, root) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Err(error), Err(restore_error)) => Err(rollback_failed_error(error, restore_error)),
        }
    }
}

#[derive(Debug)]
struct PromotedConfigFile {
    active_path: Utf8PathBuf,
    backup_path: Utf8PathBuf,
    active_existed: bool,
}

impl PromotedConfigFile {
    fn commit(self) -> Result<(), DaemonError> {
        if self.active_existed {
            delete_optional_config(&self.backup_path)?;
        }

        Ok(())
    }

    fn rollback(self) -> Result<(), DaemonError> {
        delete_optional_config(&self.active_path)?;
        if self.active_existed {
            rename_candidate_config(&self.backup_path, &self.active_path)?;
        }

        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct PromotedConfigDir {
    active_dir: Utf8PathBuf,
    backup_dir: Utf8PathBuf,
    active_existed: bool,
}

impl PromotedConfigDir {
    fn commit(self) -> Result<(), DaemonError> {
        if self.active_existed {
            delete_optional_dir(&self.backup_dir)?;
        }

        Ok(())
    }

    fn rollback(self) -> Result<(), DaemonError> {
        delete_optional_dir(&self.active_dir)?;
        if self.active_existed {
            rename_config_dir(&self.backup_dir, &self.active_dir)?;
        }

        Ok(())
    }
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

fn quoted_caddyfile_token(value: &str) -> Result<String, DaemonError> {
    if value.chars().any(char::is_control) {
        return Err(DaemonError::UnexpectedProtocolResponse {
            reason: format!("Caddyfile token contains a control character: {value:?}"),
        });
    }

    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");

    Ok(format!("\"{escaped}\""))
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

fn promote_config_file(
    active_path: &Utf8Path,
    candidate_path: &Utf8Path,
) -> Result<PromotedConfigFile, DaemonError> {
    let backup_path = backup_path_for(active_path);
    let active_existed = active_path.exists();
    delete_optional_config(&backup_path)?;
    if active_existed {
        rename_candidate_config(active_path, &backup_path)?;
    }

    if let Err(error) = rename_candidate_config(candidate_path, active_path) {
        if active_existed
            && let Err(restore_error) = rename_candidate_config(&backup_path, active_path)
        {
            return Err(rollback_failed_error(error, restore_error));
        }

        return Err(error);
    }

    Ok(PromotedConfigFile {
        active_path: active_path.to_path_buf(),
        backup_path,
        active_existed,
    })
}

pub(crate) fn promote_config_dir(
    active_dir: &Utf8Path,
    candidate_dir: &Utf8Path,
) -> Result<PromotedConfigDir, DaemonError> {
    let backup_dir = backup_path_for(active_dir);
    let active_existed = active_dir.exists();
    delete_optional_dir(&backup_dir)?;
    if active_existed {
        rename_config_dir(active_dir, &backup_dir)?;
    }

    if let Err(error) = rename_config_dir(candidate_dir, active_dir) {
        if active_existed && let Err(restore_error) = rename_config_dir(&backup_dir, active_dir) {
            return Err(rollback_failed_error(error, restore_error));
        }

        return Err(error);
    }

    Ok(PromotedConfigDir {
        active_dir: active_dir.to_path_buf(),
        backup_dir,
        active_existed,
    })
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

#[expect(
    clippy::disallowed_methods,
    reason = "daemon gateway config promotion owns direct candidate directory replacement"
)]
fn rename_config_dir(from: &Utf8Path, to: &Utf8Path) -> Result<(), DaemonError> {
    std::fs::rename(from, to)?;

    Ok(())
}

fn delete_optional_dir(path: &Utf8Path) -> Result<(), DaemonError> {
    match fs::delete_dir_all(path) {
        Ok(()) => Ok(()),
        Err(state::StateError::Filesystem { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn rollback_failed_error(original: DaemonError, rollback: DaemonError) -> DaemonError {
    DaemonError::UnexpectedProtocolResponse {
        reason: format!("Gateway config promotion failed: {original}; rollback failed: {rollback}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use camino_tempfile::tempdir;

    #[tokio::test]
    async fn promotion_reports_restore_failure_when_fragment_promotion_rollback_fails()
    -> Result<(), Box<dyn std::error::Error>> {
        let tempdir = tempdir()?;
        let root_config = tempdir.path().join("Caddyfile");
        state::fs::write_sensitive_file(&root_config, "active\n")?;
        let root_config_for_fragment_promotion = root_config.clone();

        let result = promote_validated_config_tree_async(
            &root_config,
            "candidate\n",
            "active\n",
            |_candidate_path| async { Ok(()) },
            || {
                state::fs::delete_file(&root_config_for_fragment_promotion)?;
                create_dir(&root_config_for_fragment_promotion)?;

                Err(DaemonError::UnexpectedProtocolResponse {
                    reason: "fragment promotion failed".to_owned(),
                })
            },
        )
        .await;

        assert!(matches!(
            result,
            Err(DaemonError::UnexpectedProtocolResponse { reason })
                if reason.contains("fragment promotion failed")
                    && reason.contains("rollback failed")
        ));

        Ok(())
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "test fixture blocks root config rollback by replacing the file with a directory"
    )]
    fn create_dir(path: &Utf8Path) -> Result<(), DaemonError> {
        std::fs::create_dir(path)?;

        Ok(())
    }
}
