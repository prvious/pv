use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use camino::Utf8Path;
use config::ProjectConfigFile;
use futures_util::{StreamExt, stream};
use state::{
    Database, GatewayPort, ManagedResourceDesiredState, PortOwner, ProjectMode, PvPaths,
    RuntimeSubject,
};
use tokio::time::{Instant, timeout};

use crate::DaemonError;
use crate::gateway::persisted_gateway_is_healthy;
use crate::managed_resources::{ManagedResourceReadiness, ManagedResourceRuntimeCatalog};
use crate::project_env::{project_tls_artifact_exists, project_tls_files_are_current};
use crate::reconciliation::ReconciliationScope;
use crate::supervisor::{ProcessSupervisor, ReadinessCheck, probe_readiness_once};

pub(crate) const RUNTIME_HEALTH_INTERVAL: Duration = Duration::from_secs(30);
const HEALTHY_RESET_INTERVAL: Duration = Duration::from_secs(60);
const RUNTIME_HEALTH_PROBE_TIMEOUT: Duration = Duration::from_secs(1);
const RUNTIME_HEALTH_PROBE_CONCURRENCY: usize = 4;
const RUNTIME_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(5),
    Duration::from_secs(15),
];

pub(crate) struct RuntimeHealthScan {
    observations: Vec<RuntimeHealthObservation>,
    pub(crate) maintenance_scopes: Vec<ReconciliationScope>,
    pub(crate) errors: Vec<RuntimeHealthScanError>,
}

struct RuntimeHealthObservation {
    subject: RuntimeSubject,
    scopes: BTreeSet<ReconciliationScope>,
    healthy: bool,
}

pub(crate) struct RuntimeHealthScanError {
    pub(crate) subject: String,
    pub(crate) scope: String,
    pub(crate) error: String,
}

struct DesiredRuntimeProbe {
    subject: RuntimeSubject,
    scopes: BTreeSet<ReconciliationScope>,
    current: bool,
    error: Option<String>,
    readiness: Option<RuntimeReadinessProbe>,
}

enum RuntimeReadinessProbe {
    Gateway { http_port: u16, https_port: u16 },
    Worker(Box<ReadinessCheck>),
    Resource(Box<ManagedResourceReadiness>),
}

/// Schedules three quick recovery attempts before returning to the periodic scan cadence.
#[derive(Default)]
pub(crate) struct RuntimeRecoveryBackoff {
    entries: BTreeMap<RuntimeSubject, RuntimeRecoveryEntry>,
}

struct RuntimeRecoveryEntry {
    attempts: usize,
    next_attempt: Option<Instant>,
    healthy_since: Option<Instant>,
}

impl RuntimeRecoveryBackoff {
    pub(crate) fn scopes_to_reconcile(
        &mut self,
        now: Instant,
        scan: &RuntimeHealthScan,
    ) -> BTreeSet<ReconciliationScope> {
        let desired_subjects = scan
            .observations
            .iter()
            .map(|observation| observation.subject.clone())
            .collect::<BTreeSet<_>>();
        self.entries
            .retain(|subject, _entry| desired_subjects.contains(subject));

        let mut reset_subjects = Vec::new();
        let mut scopes = BTreeSet::new();
        for observation in &scan.observations {
            if observation.healthy {
                if let Some(entry) = self.entries.get_mut(&observation.subject) {
                    let healthy_since = entry.healthy_since.get_or_insert(now);
                    entry.next_attempt = None;
                    if now.duration_since(*healthy_since) >= HEALTHY_RESET_INTERVAL {
                        reset_subjects.push(observation.subject.clone());
                    }
                }
                continue;
            }

            let entry = self
                .entries
                .entry(observation.subject.clone())
                .or_insert_with(|| RuntimeRecoveryEntry::after_failure(now, 0));
            if entry.healthy_since.take().is_some() {
                let attempts = if entry.attempts >= RUNTIME_RETRY_DELAYS.len() {
                    0
                } else {
                    entry.attempts
                };
                *entry = RuntimeRecoveryEntry::after_failure(now, attempts);
            }
            let Some(next_attempt) = entry.next_attempt else {
                *entry = RuntimeRecoveryEntry::after_failure(now, 0);
                continue;
            };
            if now < next_attempt {
                continue;
            }

            scopes.extend(observation.scopes.iter().cloned());
            entry.attempts += 1;
            entry.next_attempt = RUNTIME_RETRY_DELAYS
                .get(entry.attempts)
                .map(|delay| now + *delay);
        }
        for subject in reset_subjects {
            self.entries.remove(&subject);
        }

        scopes
    }

    pub(crate) fn next_scan_at(&self, now: Instant) -> Instant {
        let periodic = now + RUNTIME_HEALTH_INTERVAL;
        self.entries.values().fold(periodic, |next, entry| {
            let entry_next = entry.next_attempt.or(entry
                .healthy_since
                .map(|healthy| healthy + HEALTHY_RESET_INTERVAL));
            entry_next.map_or(next, |entry_next| next.min(entry_next))
        })
    }
}

impl RuntimeRecoveryEntry {
    fn after_failure(now: Instant, attempts: usize) -> Self {
        Self {
            attempts,
            next_attempt: RUNTIME_RETRY_DELAYS.get(attempts).map(|delay| now + *delay),
            healthy_since: None,
        }
    }
}

pub(crate) async fn scan_runtime_health(
    paths: PvPaths,
    runtime_catalog: Option<Arc<ManagedResourceRuntimeCatalog>>,
) -> Result<RuntimeHealthScan, DaemonError> {
    let runtime_catalog = match runtime_catalog {
        Some(runtime_catalog) => runtime_catalog,
        None => Arc::new(ManagedResourceRuntimeCatalog::production()?),
    };
    let collect_paths = paths.clone();
    let collect_catalog = Arc::clone(&runtime_catalog);
    let collected = tokio::task::spawn_blocking(move || {
        collect_runtime_health_probes(&collect_paths, &collect_catalog)
    })
    .await??;
    let inspected = stream::iter(collected.probes)
        .map(|probe| inspect_runtime_probe(paths.clone(), probe))
        .buffer_unordered(RUNTIME_HEALTH_PROBE_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
    let mut errors = collected.errors;
    let mut observations = Vec::with_capacity(inspected.len());
    for (observation, error) in inspected {
        if let Some(error) = error {
            errors.push(error);
        }
        observations.push(observation);
    }
    observations.sort_by(|left, right| left.subject.cmp(&right.subject));

    Ok(RuntimeHealthScan {
        observations,
        maintenance_scopes: collected.maintenance_scopes,
        errors,
    })
}

struct CollectedRuntimeHealth {
    probes: Vec<DesiredRuntimeProbe>,
    maintenance_scopes: Vec<ReconciliationScope>,
    errors: Vec<RuntimeHealthScanError>,
}

fn collect_runtime_health_probes(
    paths: &PvPaths,
    runtime_catalog: &ManagedResourceRuntimeCatalog,
) -> Result<CollectedRuntimeHealth, DaemonError> {
    let Some(database) = Database::open_read_only(paths)? else {
        return Ok(CollectedRuntimeHealth {
            probes: Vec::new(),
            maintenance_scopes: Vec::new(),
            errors: Vec::new(),
        });
    };
    let assignments = database.assigned_ports()?;
    let tracks = database.managed_resource_tracks()?;
    let supervisor = ProcessSupervisor::new(paths.clone());
    let mut probes = Vec::new();

    if let Some(caddy) = tracks.iter().find(|track| {
        track.resource_name == "caddy"
            && track.desired_state == ManagedResourceDesiredState::Installed
    }) {
        let http_port = assignments.iter().find_map(|assignment| {
            matches!(&assignment.owner, PortOwner::Gateway(GatewayPort::Http))
                .then_some(assignment.port)
        });
        let https_port = assignments.iter().find_map(|assignment| {
            matches!(&assignment.owner, PortOwner::Gateway(GatewayPort::Https))
                .then_some(assignment.port)
        });
        let readiness = http_port.zip(https_port).map(|(http_port, https_port)| {
            RuntimeReadinessProbe::Gateway {
                http_port,
                https_port,
            }
        });
        let scope = ReconciliationScope::resource("caddy", caddy.track.clone())?;
        let (current, error) = recorded_runtime_is_current(
            &supervisor,
            &paths.gateway_pid(),
            &paths.gateway_runtime_metadata(),
            caddy.current_artifact_path.as_deref(),
        );
        probes.push(DesiredRuntimeProbe {
            subject: RuntimeSubject::Gateway,
            scopes: [scope].into_iter().collect(),
            current,
            error,
            readiness,
        });
    }

    let mut projects_by_runtime = BTreeMap::<String, (String, BTreeSet<_>)>::new();
    for project in database.projects()? {
        if project.mode == ProjectMode::Served
            && let Some(track) = &project.php_runtime.track
        {
            let runtime_key =
                state::php_runtime_key(track, &project.php_runtime.loaded_extensions)?;
            projects_by_runtime
                .entry(runtime_key)
                .or_insert_with(|| (track.clone(), BTreeSet::new()))
                .1
                .insert(ReconciliationScope::project(project.id)?);
        }
    }
    for (runtime_key, (php_track, scopes)) in projects_by_runtime {
        let port = assignments
            .iter()
            .find_map(|assignment| match &assignment.owner {
                PortOwner::PhpWorker { php_runtime_key } if php_runtime_key == &runtime_key => {
                    Some(assignment.port)
                }
                _ => None,
            });
        let artifact_root = tracks
            .iter()
            .find(|track| {
                track.resource_name == "frankenphp"
                    && track.track == php_track
                    && track.desired_state == ManagedResourceDesiredState::Installed
            })
            .and_then(|track| track.current_artifact_path.as_deref());
        let (current, error) = recorded_runtime_is_current(
            &supervisor,
            &paths.worker_pid(&runtime_key),
            &paths.worker_runtime_metadata(&runtime_key),
            artifact_root,
        );
        probes.push(DesiredRuntimeProbe {
            subject: php_runtime_subject(&runtime_key),
            scopes,
            current,
            error,
            readiness: port.map(|port| {
                RuntimeReadinessProbe::Worker(Box::new(ReadinessCheck::Tcp {
                    host: "127.0.0.1".to_owned(),
                    port,
                }))
            }),
        });
    }

    for track in tracks {
        if track.desired_state != ManagedResourceDesiredState::Installed
            || track.usage_count <= 0
            || !runtime_catalog.has_runtime_adapter(&track.resource_name)
        {
            continue;
        }
        let health_probe =
            runtime_catalog.persisted_health_probe(paths, &database, &track, &assignments);
        let (readiness, readiness_error) = match health_probe {
            Ok(Some(health_probe)) => (
                Some(RuntimeReadinessProbe::Resource(Box::new(health_probe))),
                None,
            ),
            Ok(None) => (None, None),
            Err(error) => (None, Some(error.to_string())),
        };
        let (current, ownership_error) = recorded_runtime_is_current(
            &supervisor,
            &paths.resource_pid(&track.resource_name, &track.track),
            &paths.resource_runtime_metadata(&track.resource_name, &track.track),
            track.current_artifact_path.as_deref(),
        );
        let scope = ReconciliationScope::resource(&track.resource_name, &track.track)?;
        probes.push(DesiredRuntimeProbe {
            subject: RuntimeSubject::Resource {
                name: track.resource_name.clone(),
                track: track.track.clone(),
            },
            scopes: [scope].into_iter().collect(),
            current,
            error: ownership_error.or(readiness_error),
            readiness,
        });
    }
    probes.sort_by(|left, right| left.subject.cmp(&right.subject));

    let (maintenance_scopes, errors) = match collect_project_tls_health(paths, &database) {
        Ok(health) => (health.scopes, health.errors),
        Err(error) => (
            Vec::new(),
            vec![RuntimeHealthScanError {
                subject: "project_tls".to_owned(),
                scope: "projects".to_owned(),
                error: error.to_string(),
            }],
        ),
    };

    Ok(CollectedRuntimeHealth {
        probes,
        maintenance_scopes,
        errors,
    })
}

struct ProjectTlsHealth {
    scopes: Vec<ReconciliationScope>,
    errors: Vec<RuntimeHealthScanError>,
}

#[cfg(test)]
pub(crate) fn collect_project_tls_health_scopes(
    paths: &PvPaths,
    database: &Database,
) -> Result<Vec<ReconciliationScope>, DaemonError> {
    Ok(collect_project_tls_health(paths, database)?.scopes)
}

fn collect_project_tls_health(
    paths: &PvPaths,
    database: &Database,
) -> Result<ProjectTlsHealth, DaemonError> {
    let ca_certificate_pem = state::fs::read_to_string(&paths.ca_certificate())?;
    // Do not enqueue renewal work that cannot read the signing key.
    state::fs::read_to_string(&paths.ca_private_key())?;
    let mut scopes = Vec::new();
    let mut errors = Vec::new();

    for project in database.projects()? {
        if project.mode == ProjectMode::ResourceOnly {
            continue;
        }
        let scope = ReconciliationScope::project(&project.id)?;
        let assessment = match ProjectConfigFile::read_from_root(&project.path) {
            Ok(config_file) if !config_file.config.uses_tls_placeholders() => Ok(None),
            Ok(_config_file) => project_tls_files_are_current(paths, &project, &ca_certificate_pem)
                .map(|is_current| (!is_current).then_some(scope.clone())),
            Err(_error) => project_tls_artifact_exists(paths, &project).and_then(|exists| {
                if !exists {
                    return Ok(None);
                }
                project_tls_files_are_current(paths, &project, &ca_certificate_pem)
                    .map(|is_current| (!is_current).then_some(scope.clone()))
            }),
        };
        match assessment {
            Ok(Some(scope)) => scopes.push(scope),
            Ok(None) => {}
            Err(error) => errors.push(RuntimeHealthScanError {
                subject: format!("project_tls:{}", project.id),
                scope: scope.to_string(),
                error: error.to_string(),
            }),
        }
    }
    scopes.sort();
    scopes.dedup();

    Ok(ProjectTlsHealth { scopes, errors })
}

fn recorded_runtime_is_current(
    supervisor: &ProcessSupervisor,
    pid_path: &Utf8Path,
    metadata_path: &Utf8Path,
    artifact_root: Option<&Utf8Path>,
) -> (bool, Option<String>) {
    let Some(artifact_root) = artifact_root else {
        return (false, None);
    };
    match supervisor.adopt_recorded(pid_path, metadata_path) {
        Ok(Some(runtime)) => (runtime.uses_current_artifact(artifact_root), None),
        Ok(None) => (false, None),
        Err(error) => (false, Some(error.to_string())),
    }
}

async fn inspect_runtime_probe(
    paths: PvPaths,
    probe: DesiredRuntimeProbe,
) -> (RuntimeHealthObservation, Option<RuntimeHealthScanError>) {
    let (healthy, probe_error) = if !probe.current {
        (false, probe.error)
    } else {
        match probe.readiness {
            Some(RuntimeReadinessProbe::Gateway {
                http_port,
                https_port,
            }) => match persisted_gateway_is_healthy(&paths, http_port, https_port).await {
                Ok(healthy) => (healthy, probe.error),
                Err(error) => (false, Some(error.to_string())),
            },
            Some(RuntimeReadinessProbe::Worker(check)) => (
                matches!(
                    timeout(RUNTIME_HEALTH_PROBE_TIMEOUT, probe_readiness_once(&check)).await,
                    Ok(Ok(()))
                ),
                probe.error,
            ),
            Some(RuntimeReadinessProbe::Resource(readiness)) => (
                matches!(
                    timeout(RUNTIME_HEALTH_PROBE_TIMEOUT, readiness.probe_once()).await,
                    Ok(Ok(()))
                ),
                probe.error,
            ),
            None => (false, probe.error),
        }
    };
    let error = probe_error.map(|error| RuntimeHealthScanError {
        subject: format!("{:?}", probe.subject),
        scope: probe
            .scopes
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(","),
        error,
    });

    (
        RuntimeHealthObservation {
            subject: probe.subject,
            scopes: probe.scopes,
            healthy,
        },
        error,
    )
}

fn php_runtime_subject(runtime_key: &str) -> RuntimeSubject {
    if runtime_key.contains('+') {
        RuntimeSubject::PhpRuntimeWorker {
            php_runtime_key: runtime_key.to_owned(),
        }
    } else {
        RuntimeSubject::PhpWorker {
            php_track: runtime_key.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::net::TcpListener as StdTcpListener;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;

    use anyhow::anyhow;
    use camino::{Utf8Path, Utf8PathBuf};
    use camino_tempfile::tempdir;
    use state::{
        Database, LinkProjectInput, PortRequest, ProjectManagedResourceInput, PvPaths,
        RuntimeSubject,
    };
    use tokio::time::{Duration, Instant, advance};

    use super::{
        HEALTHY_RESET_INTERVAL, RUNTIME_HEALTH_INTERVAL, RuntimeHealthObservation,
        RuntimeHealthScan, RuntimeRecoveryBackoff, scan_runtime_health,
    };
    use crate::managed_resources::ManagedResourceRuntimeCatalog;
    use crate::{ProcessSpec, ProcessSupervisor, ReconciliationScope};

    fn scan(healthy: bool) -> anyhow::Result<RuntimeHealthScan> {
        Ok(RuntimeHealthScan {
            observations: vec![RuntimeHealthObservation {
                subject: RuntimeSubject::Resource {
                    name: "redis".to_owned(),
                    track: "8.2".to_owned(),
                },
                scopes: [ReconciliationScope::resource("redis", "8.2")?]
                    .into_iter()
                    .collect(),
                healthy,
            }],
            maintenance_scopes: Vec::new(),
            errors: Vec::new(),
        })
    }

    #[tokio::test(start_paused = true)]
    async fn backoff_retries_at_one_five_and_fifteen_seconds_then_waits_for_later_tick()
    -> anyhow::Result<()> {
        let mut backoff = RuntimeRecoveryBackoff::default();
        let mut now = Instant::now();
        assert!(backoff.scopes_to_reconcile(now, &scan(false)?).is_empty());
        assert_eq!(backoff.next_scan_at(now), now + Duration::from_secs(1));

        for delay in [1, 5, 15] {
            advance(Duration::from_secs(delay) - Duration::from_millis(1)).await;
            now = Instant::now();
            assert!(backoff.scopes_to_reconcile(now, &scan(false)?).is_empty());
            advance(Duration::from_millis(1)).await;
            now = Instant::now();
            assert_eq!(backoff.scopes_to_reconcile(now, &scan(false)?).len(), 1);
        }
        assert_eq!(backoff.next_scan_at(now), now + RUNTIME_HEALTH_INTERVAL);

        advance(RUNTIME_HEALTH_INTERVAL).await;
        now = Instant::now();
        assert!(backoff.scopes_to_reconcile(now, &scan(false)?).is_empty());
        assert_eq!(backoff.next_scan_at(now), now + Duration::from_secs(1));

        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn backoff_resets_after_sixty_continuously_healthy_seconds() -> anyhow::Result<()> {
        let mut backoff = RuntimeRecoveryBackoff::default();
        let mut now = Instant::now();
        backoff.scopes_to_reconcile(now, &scan(false)?);
        advance(Duration::from_secs(1)).await;
        now = Instant::now();
        backoff.scopes_to_reconcile(now, &scan(false)?);
        backoff.scopes_to_reconcile(now, &scan(true)?);

        advance(HEALTHY_RESET_INTERVAL).await;
        now = Instant::now();
        backoff.scopes_to_reconcile(now, &scan(true)?);
        assert!(backoff.scopes_to_reconcile(now, &scan(false)?).is_empty());
        assert_eq!(backoff.next_scan_at(now), now + Duration::from_secs(1));

        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn brief_healthy_period_preserves_the_retry_attempt() -> anyhow::Result<()> {
        let mut backoff = RuntimeRecoveryBackoff::default();
        let mut now = Instant::now();
        backoff.scopes_to_reconcile(now, &scan(false)?);
        advance(Duration::from_secs(1)).await;
        now = Instant::now();
        backoff.scopes_to_reconcile(now, &scan(false)?);
        backoff.scopes_to_reconcile(now, &scan(true)?);

        advance(HEALTHY_RESET_INTERVAL - Duration::from_secs(1)).await;
        now = Instant::now();
        assert!(backoff.scopes_to_reconcile(now, &scan(false)?).is_empty());
        assert_eq!(backoff.next_scan_at(now), now + Duration::from_secs(5));

        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn first_periodic_scan_is_not_immediate() {
        let now = Instant::now();
        assert_eq!(
            RuntimeRecoveryBackoff::default().next_scan_at(now),
            now + RUNTIME_HEALTH_INTERVAL
        );
    }

    #[tokio::test]
    async fn scanner_uses_persisted_runtime_state_without_writing_or_assigning_ports()
    -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        let project_path = tempdir.path().join("project");
        let config_path = project_path.join("pv.yml");
        state::fs::write_sensitive_file(&config_path, "php: \"8.4\"\n")?;
        state::fs::write_sensitive_file(&paths.ca_certificate(), "unused certificate")?;
        state::fs::write_sensitive_file(&paths.ca_private_key(), "unused private key")?;
        let mut database = Database::open(&paths)?;
        let project = database
            .link_project(LinkProjectInput {
                path: project_path.clone(),
                original_path: project_path,
                primary_hostname: "health.test".to_owned(),
                config_path,
                desired_php_track: Some("8.4".to_owned()),
                additional_hostnames: Vec::new(),
            })?
            .project;
        let projects_before = database.projects()?;
        let tracks_before = database.managed_resource_tracks()?;
        let observed_before = database.runtime_observed_states()?;
        let jobs_before = database.recent_jobs()?;
        drop(database);

        let scan = scan_runtime_health(
            paths.clone(),
            Some(Arc::new(ManagedResourceRuntimeCatalog::without_adapters()?)),
        )
        .await?;

        assert_eq!(scan.observations.len(), 1);
        assert_eq!(
            scan.observations[0].subject,
            RuntimeSubject::PhpWorker {
                php_track: "8.4".to_owned()
            }
        );
        assert_eq!(
            scan.observations[0].scopes,
            [ReconciliationScope::project(project.id)?]
                .into_iter()
                .collect()
        );
        assert!(!scan.observations[0].healthy);
        let database = Database::open(&paths)?;
        assert_eq!(database.projects()?, projects_before);
        assert_eq!(database.managed_resource_tracks()?, tracks_before);
        assert_eq!(database.runtime_observed_states()?, observed_before);
        assert_eq!(database.recent_jobs()?, jobs_before);
        assert!(database.assigned_ports()?.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn killed_worker_selects_all_and_only_its_project_scopes() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        state::fs::write_sensitive_file(&paths.ca_certificate(), "unused certificate")?;
        state::fs::write_sensitive_file(&paths.ca_private_key(), "unused private key")?;
        let mut database = Database::open(&paths)?;
        let killed_project_id = link_runtime_project(
            &mut database,
            tempdir.path(),
            "killed",
            "killed.test",
            "8.3",
        )?;
        let shared_project_path = tempdir.path().join("shared-invalid");
        let shared_project_id = link_runtime_project(
            &mut database,
            tempdir.path(),
            "shared-invalid",
            "shared-invalid.test",
            "8.3",
        )?;
        state::fs::write_sensitive_file(&shared_project_path.join("pv.yml"), "php: [\n")?;
        let healthy_project_id = link_runtime_project(
            &mut database,
            tempdir.path(),
            "healthy",
            "healthy.test",
            "8.4",
        )?;
        let killed_runtime = prepare_worker_artifact(&mut database, tempdir.path(), "8.3")?;
        let healthy_runtime = prepare_worker_artifact(&mut database, tempdir.path(), "8.4")?;
        let _killed_listener = assign_php_worker_listener(&mut database, "8.3")?;
        let _healthy_listener = assign_php_worker_listener(&mut database, "8.4")?;
        drop(database);
        let supervisor = ProcessSupervisor::new(paths.clone());
        let killed_process = supervisor
            .start(worker_process_spec(&paths, &killed_runtime, "8.3"))
            .await?;
        let healthy_process = supervisor
            .start(worker_process_spec(&paths, &healthy_runtime, "8.4"))
            .await?;
        killed_process.stop(Duration::from_secs(1)).await?;

        let scan_result = scan_runtime_health(
            paths.clone(),
            Some(Arc::new(ManagedResourceRuntimeCatalog::without_adapters()?)),
        )
        .await;
        let cleanup_result = healthy_process.stop(Duration::from_secs(1)).await;
        let scan = scan_result?;
        cleanup_result?;
        let mut backoff = RuntimeRecoveryBackoff::default();
        let detected_at = Instant::now();
        assert!(backoff.scopes_to_reconcile(detected_at, &scan).is_empty());
        let scopes = backoff.scopes_to_reconcile(detected_at + Duration::from_secs(1), &scan);

        assert_eq!(
            scopes,
            [
                ReconciliationScope::project(killed_project_id)?,
                ReconciliationScope::project(shared_project_id)?,
            ]
            .into_iter()
            .collect()
        );
        assert!(!scopes.contains(&ReconciliationScope::project(healthy_project_id)?));

        Ok(())
    }

    #[tokio::test]
    async fn scanner_rejects_replacement_required_and_stale_worker_artifacts() -> anyhow::Result<()>
    {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        state::fs::write_sensitive_file(&paths.ca_certificate(), "unused certificate")?;
        state::fs::write_sensitive_file(&paths.ca_private_key(), "unused private key")?;
        let mut database = Database::open(&paths)?;
        link_runtime_project(
            &mut database,
            tempdir.path(),
            "project",
            "project.test",
            "8.4",
        )?;
        let runtime = prepare_worker_artifact(&mut database, tempdir.path(), "8.4")?;
        let _listener = assign_php_worker_listener(&mut database, "8.4")?;
        drop(database);
        let supervisor = ProcessSupervisor::new(paths.clone());
        let spec = worker_process_spec(&paths, &runtime, "8.4");
        let process = supervisor.start(spec.clone()).await?;
        supervisor.mark_replacement_required(&spec)?;

        let replacement_scan = scan_runtime_health(
            paths.clone(),
            Some(Arc::new(ManagedResourceRuntimeCatalog::without_adapters()?)),
        )
        .await?;
        supervisor.clear_replacement_required(&spec)?;
        let mut database = Database::open(&paths)?;
        let new_artifact_root = tempdir.path().join("frankenphp-8.4-new");
        database.record_managed_resource_track_installed(
            "frankenphp",
            "8.4",
            "8.4.1-pv1",
            &new_artifact_root,
        )?;
        drop(database);
        let stale_scan_result = scan_runtime_health(
            paths.clone(),
            Some(Arc::new(ManagedResourceRuntimeCatalog::without_adapters()?)),
        )
        .await;
        let cleanup_result = process.stop(Duration::from_secs(1)).await;
        let stale_scan = stale_scan_result?;
        cleanup_result?;

        for scan in [replacement_scan, stale_scan] {
            assert_eq!(scan.observations.len(), 1);
            assert_eq!(
                scan.observations[0].subject,
                RuntimeSubject::PhpWorker {
                    php_track: "8.4".to_owned()
                }
            );
            assert!(!scan.observations[0].healthy);
        }

        Ok(())
    }

    #[tokio::test]
    async fn one_metadata_error_does_not_discard_other_runtime_observations() -> anyhow::Result<()>
    {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        state::fs::write_sensitive_file(&paths.ca_certificate(), "unused certificate")?;
        state::fs::write_sensitive_file(&paths.ca_private_key(), "unused private key")?;
        let mut database = Database::open(&paths)?;
        let broken_project_id = link_runtime_project(
            &mut database,
            tempdir.path(),
            "broken",
            "broken.test",
            "8.3",
        )?;
        link_runtime_project(
            &mut database,
            tempdir.path(),
            "healthy",
            "healthy.test",
            "8.4",
        )?;
        let broken_runtime = prepare_worker_artifact(&mut database, tempdir.path(), "8.3")?;
        let healthy_runtime = prepare_worker_artifact(&mut database, tempdir.path(), "8.4")?;
        let _broken_listener = assign_php_worker_listener(&mut database, "8.3")?;
        let _healthy_listener = assign_php_worker_listener(&mut database, "8.4")?;
        drop(database);
        let supervisor = ProcessSupervisor::new(paths.clone());
        let broken_process = supervisor
            .start(worker_process_spec(&paths, &broken_runtime, "8.3"))
            .await?;
        let healthy_process = supervisor
            .start(worker_process_spec(&paths, &healthy_runtime, "8.4"))
            .await?;
        state::fs::write_sensitive_file(&paths.worker_runtime_metadata("8.3"), "{")?;

        let scan_result = scan_runtime_health(
            paths.clone(),
            Some(Arc::new(ManagedResourceRuntimeCatalog::without_adapters()?)),
        )
        .await;
        let broken_cleanup = broken_process.stop(Duration::from_secs(1)).await;
        let healthy_cleanup = healthy_process.stop(Duration::from_secs(1)).await;
        let scan = scan_result?;
        broken_cleanup?;
        healthy_cleanup?;

        assert_eq!(scan.observations.len(), 2);
        assert!(scan.observations.iter().any(|observation| {
            observation.subject
                == RuntimeSubject::PhpWorker {
                    php_track: "8.3".to_owned(),
                }
                && !observation.healthy
        }));
        assert!(scan.observations.iter().any(|observation| {
            observation.subject
                == RuntimeSubject::PhpWorker {
                    php_track: "8.4".to_owned(),
                }
        }));
        assert_eq!(scan.errors.len(), 1);
        assert_eq!(
            scan.errors[0].scope,
            ReconciliationScope::project(broken_project_id)?.to_string()
        );

        Ok(())
    }

    #[tokio::test]
    async fn managed_resource_scan_does_not_prepare_or_write_runtime_files() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let paths = PvPaths::for_home(tempdir.path().join("home"));
        state::fs::write_sensitive_file(&paths.ca_certificate(), "unused certificate")?;
        state::fs::write_sensitive_file(&paths.ca_private_key(), "unused private key")?;
        let mut database = Database::open(&paths)?;
        let project_id = link_runtime_project(
            &mut database,
            tempdir.path(),
            "redis-project",
            "redis.test",
            "8.4",
        )?;
        database.replace_project_managed_resources(
            &project_id,
            &[ProjectManagedResourceInput {
                resource_name: "redis".to_owned(),
                track: "8.2".to_owned(),
            }],
        )?;
        database.record_managed_resource_track_installed(
            "redis",
            "8.2",
            "8.2.0-pv1",
            &paths.resources().join("redis/8.2/releases/8.2.0-pv1"),
        )?;
        database.assign_port(
            PortRequest::resource("redis", "8.2", 45_000, 45_000, 48_999),
            loopback_port_available,
        )?;
        let tracks_before = database.managed_resource_tracks()?;
        let assignments_before = database.assigned_ports()?;
        let observed_before = database.runtime_observed_states()?;
        let jobs_before = database.recent_jobs()?;
        drop(database);
        let config_path = paths.resource_runtime_config("redis", "8.2");
        let data_dir = paths.resource_data_dir("redis", "8.2");

        let scan = scan_runtime_health(paths.clone(), None).await?;

        assert!(scan.observations.iter().any(|observation| {
            observation.subject
                == RuntimeSubject::Resource {
                    name: "redis".to_owned(),
                    track: "8.2".to_owned(),
                }
                && !observation.healthy
        }));
        assert!(!state::fs::path_entry_exists(&config_path)?);
        assert!(!state::fs::path_entry_exists(&data_dir)?);
        let database = Database::open(&paths)?;
        assert_eq!(database.managed_resource_tracks()?, tracks_before);
        assert_eq!(database.assigned_ports()?, assignments_before);
        assert_eq!(database.runtime_observed_states()?, observed_before);
        assert_eq!(database.recent_jobs()?, jobs_before);

        Ok(())
    }

    fn link_runtime_project(
        database: &mut Database,
        root: &Utf8Path,
        directory: &str,
        hostname: &str,
        php_track: &str,
    ) -> anyhow::Result<String> {
        let project_path = root.join(directory);
        let config_path = project_path.join("pv.yml");
        state::fs::write_sensitive_file(&config_path, &format!("php: \"{php_track}\"\n"))?;
        let project = database
            .link_project(LinkProjectInput {
                path: project_path.clone(),
                original_path: project_path,
                primary_hostname: hostname.to_owned(),
                config_path,
                desired_php_track: Some(php_track.to_owned()),
                additional_hostnames: Vec::new(),
            })?
            .project;

        Ok(project.id)
    }

    fn loopback_port_available(port: u16) -> bool {
        StdTcpListener::bind(("127.0.0.1", port)).is_ok()
    }

    fn assign_php_worker_listener(
        database: &mut Database,
        php_track: &str,
    ) -> anyhow::Result<StdTcpListener> {
        let mut listener = None;
        database.assign_php_worker_port(php_track, |port| {
            listener = StdTcpListener::bind(("127.0.0.1", port)).ok();
            listener.is_some()
        })?;

        listener.ok_or_else(|| anyhow!("assigned PHP worker port was not reserved"))
    }

    fn prepare_worker_artifact(
        database: &mut Database,
        root: &Utf8Path,
        php_track: &str,
    ) -> anyhow::Result<Utf8PathBuf> {
        let artifact_root = root.join(format!("frankenphp-{php_track}"));
        let runtime = artifact_root.join("bin/frankenphp");
        state::fs::write_sensitive_file(&runtime, "#!/bin/sh\nwhile true; do sleep 1; done\n")?;
        set_executable(&runtime)?;
        database.record_managed_resource_track_installed(
            "frankenphp",
            php_track,
            &format!("{php_track}.0-pv1"),
            &artifact_root,
        )?;

        Ok(runtime)
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "runtime health tests set fixture executable bits directly"
    )]
    fn set_executable(path: &Utf8Path) -> anyhow::Result<()> {
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions)?;

        Ok(())
    }

    fn worker_process_spec(paths: &PvPaths, runtime: &Utf8Path, php_track: &str) -> ProcessSpec {
        ProcessSpec {
            name: format!("php-worker-{php_track}"),
            command: Utf8PathBuf::from(runtime),
            arguments: Vec::new(),
            private_environment: Default::default(),
            config_path: paths.worker_root_config(php_track),
            config_fingerprint: None,
            log_path: paths.worker_log(php_track),
            pid_path: paths.worker_pid(php_track),
            metadata_path: paths.worker_runtime_metadata(php_track),
            resource_name: "frankenphp".to_owned(),
            track: php_track.to_owned(),
        }
    }
}
