use std::sync::Arc;
use std::time::Duration;

use config::ProjectConfigFile;
use futures_util::StreamExt;
use state::{Database, ProjectMode, PvPaths};
use tokio::io::AsyncRead;
use tokio::sync::oneshot;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{MissedTickBehavior, sleep, timeout};

use crate::DaemonError;
use crate::ipc::{LocalListener, LocalStream};
use crate::jobs::{
    record_background_reconciliation_error, run_background_reconciliation_job, run_job,
};
use crate::managed_resources::ManagedResourceRuntimeCatalog;
use crate::project_env::{project_tls_artifact_exists, project_tls_files_are_current};
use crate::reconciliation::{ReconciliationQueue, ReconciliationScope};
use crate::watcher::ProjectConfigWatcher;
use protocol::{
    DaemonCommand, DaemonRequest, DaemonResponse, DaemonTransport, PROTOCOL_VERSION, write_line,
};

const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(50);
const PROJECT_CONFIG_DEBOUNCE: Duration = Duration::from_millis(50);
const PROJECT_CONFIG_WATCH_INTERVAL: Duration = Duration::from_millis(100);
const REQUEST_LINE_TIMEOUT: Duration = Duration::from_secs(30);
const TLS_HEALTH_INTERVAL: Duration = Duration::from_secs(30);

pub(crate) async fn serve(
    paths: PvPaths,
    listener: LocalListener,
    mut shutdown: oneshot::Receiver<()>,
    runtime_catalog: Option<Arc<ManagedResourceRuntimeCatalog>>,
) -> Result<(), DaemonError> {
    let mut connections = JoinSet::new();
    let queue = ReconciliationQueue::new();
    let background_paths = paths.clone();
    let background_queue = queue.clone();
    let background_runtime_catalog = runtime_catalog.clone();
    let debouncer = crate::reconciliation::ReconciliationDebouncer::new(
        PROJECT_CONFIG_DEBOUNCE,
        move |scope| {
            let paths = background_paths.clone();
            let queue = background_queue.clone();
            let runtime_catalog = background_runtime_catalog.clone();
            let _task = tokio::spawn(async move {
                let scope_text = scope.to_string();
                if let Err(error) = run_background_reconciliation_job(
                    paths.clone(),
                    queue,
                    scope,
                    runtime_catalog.as_deref(),
                )
                .await
                {
                    let _result =
                        record_background_reconciliation_error(&paths, &scope_text, &error);
                }
            });
        },
    );
    let watcher = ProjectConfigWatcher::new(
        paths.clone(),
        debouncer.clone(),
        PROJECT_CONFIG_WATCH_INTERVAL,
    );
    let mut watcher_task = tokio::spawn(watcher.run());
    let mut tls_health_task: Option<JoinHandle<Result<Vec<ReconciliationScope>, DaemonError>>> =
        None;
    let mut tls_health_interval = tokio::time::interval(TLS_HEALTH_INTERVAL);
    tls_health_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                watcher_task.abort();
                let _join_result = watcher_task.await;
                if let Some(task) = tls_health_task.take() {
                    task.abort();
                }
                connections.abort_all();
                while connections.join_next().await.is_some() {}

                return Ok(());
            }
            watcher_result = &mut watcher_task => {
                match watcher_result {
                    Ok(Ok(())) => return Ok(()),
                    Ok(Err(error)) => return Err(error),
                    Err(error) if error.is_panic() => return Err(error.into()),
                    Err(_error) => return Ok(()),
                }
            }
            tls_health_result = async {
                match tls_health_task.as_mut() {
                    Some(task) => Some(task.await),
                    None => None,
                }
            }, if tls_health_task.is_some() => {
                tls_health_task = None;
                if let Some(tls_health_result) = tls_health_result {
                    match tls_health_result {
                        Ok(Ok(scopes)) => {
                            for scope in scopes {
                                debouncer.request(scope).await;
                            }
                        }
                        Ok(Err(_error)) => {}
                        Err(error) if error.is_panic() => return Err(error.into()),
                        Err(_error) => {}
                    }
                }
            }
            _ = tls_health_interval.tick() => {
                if tls_health_task.is_none() {
                    let health_paths = paths.clone();
                    tls_health_task = Some(tokio::task::spawn_blocking(move || {
                        collect_project_tls_health_scopes(&health_paths)
                    }));
                }
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _address)) => {
                        let connection_paths = paths.clone();
                        let connection_queue = queue.clone();
                        let connection_runtime_catalog = runtime_catalog.clone();

                        connections.spawn(async move {
                            handle_connection(
                                connection_paths,
                                connection_queue,
                                stream,
                                connection_runtime_catalog,
                            )
                            .await
                        });
                    }
                    Err(_error) => {
                        sleep(ACCEPT_ERROR_BACKOFF).await;
                    }
                }
            }
            joined = connections.join_next(), if !connections.is_empty() => {
                match joined {
                    Some(Ok(Ok(()))) | None => {}
                    Some(Ok(Err(_error))) => {}
                    Some(Err(error)) if error.is_panic() => return Err(error.into()),
                    Some(Err(_error)) => {}
                }
            }
        }
    }
}

fn collect_project_tls_health_scopes(
    paths: &PvPaths,
) -> Result<Vec<ReconciliationScope>, DaemonError> {
    let Some(database) = Database::open_read_only(paths)? else {
        return Ok(Vec::new());
    };
    let ca_certificate_pem = state::fs::read_to_string(&paths.ca_certificate())?;
    let _ca_private_key_pem = state::fs::read_to_string(&paths.ca_private_key())?;
    let projects = database.projects()?;
    let mut scopes = Vec::new();

    for project in projects {
        if project.mode == ProjectMode::ResourceOnly {
            continue;
        }
        let should_assess = match ProjectConfigFile::read_from_root(&project.path) {
            Ok(config_file) => config_file.config.uses_tls_placeholders(),
            Err(_error) => project_tls_artifact_exists(paths, &project).unwrap_or(false),
        };
        if !should_assess {
            continue;
        }
        let Ok(is_current) = project_tls_files_are_current(paths, &project, &ca_certificate_pem)
        else {
            continue;
        };
        if !is_current && let Ok(scope) = ReconciliationScope::project(project.id) {
            scopes.push(scope);
        }
    }

    scopes.sort();
    Ok(scopes)
}

async fn handle_connection(
    paths: PvPaths,
    queue: ReconciliationQueue,
    stream: LocalStream,
    runtime_catalog: Option<Arc<ManagedResourceRuntimeCatalog>>,
) -> Result<(), DaemonError> {
    let mut transport = protocol::transport(stream);
    let Some(line) = read_request_line(&mut transport, REQUEST_LINE_TIMEOUT).await? else {
        return Ok(());
    };
    let request = serde_json::from_str::<DaemonRequest>(&line)?;

    if request.protocol_version != PROTOCOL_VERSION {
        write_line(
            &mut transport,
            &DaemonResponse::error("daemon protocol mismatch; run `pv daemon:restart`"),
        )
        .await?;

        return Ok(());
    }

    match request.command {
        DaemonCommand::Health => {
            write_line(&mut transport, &DaemonResponse::ok("daemon healthy")).await?;

            Ok(())
        }
        DaemonCommand::RunJob { kind, scope } => {
            run_job(
                paths,
                queue,
                transport,
                &kind,
                &scope,
                runtime_catalog.as_deref(),
            )
            .await
        }
        DaemonCommand::ManagedResourceUpdateCheck => {
            let update_paths = paths.clone();
            let update_catalog = runtime_catalog.clone();
            let update_check_result = tokio::task::spawn_blocking(move || {
                crate::managed_resources::update_check(update_paths, update_catalog.as_deref())
            })
            .await?;
            match update_check_result {
                Ok(update_check) => {
                    write_line(
                        &mut transport,
                        &DaemonResponse::ok_update_check(
                            "Managed Resource update check completed",
                            update_check,
                        ),
                    )
                    .await?;
                }
                Err(error) => {
                    write_line(&mut transport, &DaemonResponse::error(error.to_string())).await?;
                }
            }

            Ok(())
        }
    }
}

async fn read_request_line<Stream>(
    transport: &mut DaemonTransport<Stream>,
    read_timeout: Duration,
) -> Result<Option<String>, DaemonError>
where
    Stream: AsyncRead + Unpin,
{
    match timeout(read_timeout, transport.next()).await {
        Ok(Some(line)) => Ok(Some(line?)),
        Ok(None) | Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use anyhow::{Result, anyhow};
    use camino::Utf8Path;
    use camino_tempfile::tempdir;
    use rcgen::{
        CertificateParams, DnType, ExtendedKeyUsagePurpose, Issuer, KeyPair, KeyUsagePurpose,
        PKCS_ECDSA_P256_SHA256,
    };
    use state::{Database, LinkProjectInput, ProjectRecord, PvPaths};
    use time::{Duration as CertificateDuration, OffsetDateTime};
    use tokio::io::duplex;

    use super::{collect_project_tls_health_scopes, read_request_line};
    use crate::reconciliation::ReconciliationScope;
    use protocol::transport;

    #[tokio::test]
    async fn request_line_read_times_out_for_idle_connection() -> Result<(), crate::DaemonError> {
        let (_client, server) = duplex(1024);
        let mut transport = transport(server);

        let line = read_request_line(&mut transport, Duration::from_millis(10)).await?;

        assert!(line.is_none());

        Ok(())
    }

    #[test]
    fn tls_health_poll_targets_only_expiring_tls_project() -> Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let local_ca = platform::generate_local_ca()?;
        state::fs::write_sensitive_file(&paths.ca_certificate(), &local_ca.certificate_pem)?;
        state::fs::write_sensitive_file(&paths.ca_private_key(), &local_ca.private_key_pem)?;

        let valid_project = link_health_project(
            &paths,
            &tempdir.path().join("valid"),
            "m-valid.test",
            "env:\n  CERT: \"${tls_cert}\"\n",
        )?;
        let non_tls_project = link_health_project(
            &paths,
            &tempdir.path().join("non-tls"),
            "n-non-tls.test",
            "php: \"8.4\"\n",
        )?;
        let invalid_project = link_health_project(
            &paths,
            &tempdir.path().join("invalid"),
            "a-invalid.test",
            "env: [\n",
        )?;
        assert!(config::ProjectConfigFile::read_from_root(&invalid_project.path).is_err());
        let malformed_cert_only_project = link_health_project(
            &paths,
            &tempdir.path().join("malformed-cert-only"),
            "b-malformed-cert-only.test",
            "env: [\n",
        )?;
        let malformed_key_only_project = link_health_project(
            &paths,
            &tempdir.path().join("malformed-key-only"),
            "c-malformed-key-only.test",
            "env: [\n",
        )?;
        let expiring_project = link_health_project(
            &paths,
            &tempdir.path().join("expiring"),
            "z-expiring.test",
            "env:\n  CERT: \"${tls_cert}\"\n",
        )?;
        write_project_certificate(&paths, &valid_project, &local_ca, 365)?;
        write_project_certificate(&paths, &expiring_project, &local_ca, 7)?;
        write_project_certificate(&paths, &invalid_project, &local_ca, 7)?;
        write_project_certificate(&paths, &malformed_cert_only_project, &local_ca, 7)?;
        write_project_certificate(&paths, &malformed_key_only_project, &local_ca, 7)?;
        state::fs::remove_file(&paths.project_tls_private_key(&malformed_cert_only_project.id))?;
        state::fs::remove_file(&paths.project_tls_certificate(&malformed_key_only_project.id))?;

        let scopes = collect_project_tls_health_scopes(&paths)?;
        let mut expected_scopes = vec![
            ReconciliationScope::project(expiring_project.id.clone())?,
            ReconciliationScope::project(invalid_project.id.clone())?,
            ReconciliationScope::project(malformed_cert_only_project.id.clone())?,
            ReconciliationScope::project(malformed_key_only_project.id.clone())?,
        ];
        expected_scopes.sort();

        assert_eq!(scopes, expected_scopes);
        assert!(state::fs::path_entry_exists(
            &paths.project_tls_certificate(&valid_project.id)
        )?);
        assert!(!state::fs::path_entry_exists(
            &paths.project_tls_certificate(&non_tls_project.id)
        )?);

        Ok(())
    }

    #[test]
    fn tls_health_collector_skips_malformed_project_without_existing_tls_artifacts() -> Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let local_ca = platform::generate_local_ca()?;
        state::fs::write_sensitive_file(&paths.ca_certificate(), &local_ca.certificate_pem)?;
        state::fs::write_sensitive_file(&paths.ca_private_key(), &local_ca.private_key_pem)?;

        let project = link_health_project(
            &paths,
            &tempdir.path().join("invalid"),
            "invalid.test",
            "env: [\n",
        )?;

        assert!(collect_project_tls_health_scopes(&paths)?.is_empty());
        assert!(!state::fs::path_entry_exists(
            &paths.project_tls_certificate(&project.id)
        )?);

        Ok(())
    }

    fn link_health_project(
        paths: &PvPaths,
        project_path: &Utf8Path,
        primary_hostname: &str,
        config_source: &str,
    ) -> Result<ProjectRecord> {
        let config_path = project_path.join("pv.yml");
        state::fs::write_sensitive_file(&config_path, config_source)?;
        let mut database = Database::open(paths)?;
        Ok(database
            .link_project(LinkProjectInput {
                path: project_path.to_path_buf(),
                original_path: project_path.to_path_buf(),
                primary_hostname: primary_hostname.to_owned(),
                config_path,
                desired_php_track: None,
                additional_hostnames: Vec::new(),
            })?
            .project)
    }

    fn write_project_certificate(
        paths: &PvPaths,
        project: &ProjectRecord,
        local_ca: &platform::GeneratedLocalCa,
        remaining_days: i64,
    ) -> Result<()> {
        let ca_key_pair = KeyPair::from_pem(&local_ca.private_key_pem)?;
        let issuer = Issuer::from_ca_cert_pem(&local_ca.certificate_pem, ca_key_pair)?;
        let primary_hostname = project
            .primary_hostname
            .as_deref()
            .ok_or_else(|| anyhow!("expected Project `{}` to have a hostname", project.slug))?;
        let mut params = CertificateParams::new(vec![primary_hostname.to_string()])?;
        let now = OffsetDateTime::now_utc();
        params.not_before = now - CertificateDuration::days(1);
        params.not_after = now + CertificateDuration::days(remaining_days);
        params
            .distinguished_name
            .push(DnType::CommonName, primary_hostname);
        params.use_authority_key_identifier_extension = true;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
        let certificate = params.signed_by(&key_pair, &issuer)?;
        state::fs::write_sensitive_file(
            &paths.project_tls_certificate(&project.id),
            &format!("{}{}", certificate.pem(), local_ca.certificate_pem),
        )?;
        state::fs::write_sensitive_file(
            &paths.project_tls_private_key(&project.id),
            &key_pair.serialize_pem(),
        )?;

        Ok(())
    }
}
