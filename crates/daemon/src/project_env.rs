use std::collections::{BTreeMap, BTreeSet};
use std::io;

use camino::Utf8PathBuf;
use config::{
    AllocationEnvContext, ProjectConfig, ProjectConfigFile, ProjectEnvContext, ProjectEnvWarning,
    ResourceEnvContext,
};
use resources::{
    ArtifactManifestCache, ConcreteTrackName, ResourceName, TrackSelector,
    generated_allocation_name,
};
use state::{
    Database, LinkProjectInput, ManagedResourceDesiredState, ProjectEnvObservedStatus,
    ProjectEnvObservedWarningInput, ProjectManagedResourceInput, ProjectMode,
    ProjectPhpRuntimeInput, ProjectReconciliationStateInput, ProjectRecord, PvPaths,
    ResourceAllocationInput, ResourceAllocationRecord, ResourceAllocationStatus, StateError,
};

use crate::DaemonError;
use crate::jobs::DaemonDownloadProgress;
use crate::managed_resources::ManagedResourceRuntimeCatalog;
use crate::structured_log;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectEnvReconciliationSummary {
    message: &'static str,
    requested_php_extensions: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectResourcePlan {
    pub(crate) resources: Vec<ProjectManagedResourceInput>,
    pub(crate) allocations: BTreeMap<String, ProjectResourceAllocationPlan>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectResourceAllocationPlan {
    pub(crate) allocations: Vec<ResourceAllocationInput>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ProjectDemand {
    pub(crate) resource_tracks: BTreeSet<DemandedResourceTrack>,
    resource_selections: BTreeMap<String, ProjectResourceTrackDemand>,
    pub(crate) php_track: Option<ProjectPhpTrackDemand>,
    pub(crate) used_persisted_state: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProjectResourceTrackDemand {
    version_selector: Option<String>,
    track: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectPhpTrackDemand {
    php_configured: bool,
    version_selector: Option<String>,
    serves_http: bool,
    track: String,
}

impl ProjectPhpTrackDemand {
    fn matches(&self, php: Option<&config::PhpConfig>, serves_http: bool) -> bool {
        self.php_configured == php.is_some()
            && self.version_selector.as_deref() == php.and_then(config::PhpConfig::version_selector)
            && self.serves_http == serves_http
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct DemandedResourceTrack {
    pub(crate) resource_name: String,
    pub(crate) track: String,
}

impl DemandedResourceTrack {
    pub(crate) fn new(resource_name: impl Into<String>, track: impl Into<String>) -> Self {
        Self {
            resource_name: resource_name.into(),
            track: track.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedPhpRuntime {
    pub(crate) track: String,
    pub(crate) runtime_key: String,
    pub(crate) requested_extensions: Vec<String>,
    pub(crate) loaded_extensions: Vec<String>,
    pub(crate) ignored_extensions: Vec<String>,
    pub(crate) loaded_modules: Vec<resources::PhpExtensionModule>,
}

impl ProjectEnvReconciliationSummary {
    pub(crate) fn as_str(&self) -> &'static str {
        self.message
    }

    pub(crate) fn requested_php_extensions(&self) -> bool {
        self.requested_php_extensions
    }
}

#[cfg(test)]
pub(crate) async fn reconcile_project_env(
    paths: &PvPaths,
    project_id: &str,
) -> Result<ProjectEnvReconciliationSummary, DaemonError> {
    reconcile_project_env_with_runtime_catalog_and_progress(
        paths,
        project_id,
        None,
        None,
        &BTreeSet::new(),
        DaemonDownloadProgress::disabled(),
    )
    .await
}

pub(crate) async fn reconcile_project_env_with_runtime_catalog_and_progress(
    paths: &PvPaths,
    project_id: &str,
    catalog: Option<&ManagedResourceRuntimeCatalog>,
    discovered_demand: Option<&ProjectDemand>,
    demanded_tracks: &BTreeSet<DemandedResourceTrack>,
    progress: DaemonDownloadProgress,
) -> Result<ProjectEnvReconciliationSummary, DaemonError> {
    let mut database = Database::open(paths)?;
    let project =
        database
            .project_by_id(project_id)?
            .ok_or_else(|| StateError::ProjectNotFound {
                target: project_id.to_string(),
            })?;

    match reconcile_loaded_project(
        paths,
        &mut database,
        &project,
        catalog,
        discovered_demand,
        demanded_tracks,
        progress,
    )
    .await
    {
        Ok(summary) => Ok(summary),
        Err(error) => {
            let message = error.to_string();
            record_project_env_failure(&mut database, &project.id, &message)?;

            Err(error)
        }
    }
}

#[cfg(test)]
pub(crate) async fn reconcile_project_env_with_catalog(
    paths: &PvPaths,
    database: &mut Database,
    project_id: &str,
    catalog: &ManagedResourceRuntimeCatalog,
) -> Result<ProjectEnvReconciliationSummary, DaemonError> {
    let project =
        database
            .project_by_id(project_id)?
            .ok_or_else(|| StateError::ProjectNotFound {
                target: project_id.to_string(),
            })?;

    match reconcile_loaded_project(
        paths,
        database,
        &project,
        Some(catalog),
        None,
        &BTreeSet::new(),
        DaemonDownloadProgress::disabled(),
    )
    .await
    {
        Ok(summary) => Ok(summary),
        Err(error) => {
            let message = error.to_string();
            record_project_env_failure(database, &project.id, &message)?;

            Err(error)
        }
    }
}

pub(crate) fn validate_project_config_for_gateway(
    paths: &PvPaths,
    database: &Database,
    project: &ProjectRecord,
    config_file: &ProjectConfigFile,
) -> Result<(), DaemonError> {
    let _plan = validate_project_config_and_plan(paths, database, project, config_file, None)?;

    Ok(())
}

pub(crate) fn discover_project_demand(
    paths: &PvPaths,
    database: &Database,
    project: &ProjectRecord,
) -> Result<ProjectDemand, DaemonError> {
    let discovered = (|| {
        let config_file = ProjectConfigFile::read_from_root(&project.path)?;
        let candidate_project = project_with_config_mode(project, &config_file.config);
        let plan = validate_project_config_and_plan(
            paths,
            database,
            &candidate_project,
            &config_file,
            None,
        )?;
        let php_track = maybe_resolve_project_php_track(
            paths,
            database,
            &candidate_project,
            config_file.config.php.as_ref(),
            candidate_project.mode == ProjectMode::Served,
            None,
        )?;

        Ok::<_, DaemonError>(project_demand_from_plan(
            &plan,
            &config_file.config,
            candidate_project.mode == ProjectMode::Served,
            php_track,
        ))
    })();

    // Discovery is conservative: current-config errors retain last-applied demand so the resource
    // pass cannot tear down runtimes before authoritative application reports persistent errors.
    match discovered {
        Ok(demand) => Ok(demand),
        Err(_error) => persisted_project_demand(database, project),
    }
}

fn project_demand_from_plan(
    plan: &ProjectResourcePlan,
    config: &ProjectConfig,
    serves_http: bool,
    php_track: Option<String>,
) -> ProjectDemand {
    let php = config.php.as_ref();
    let resource_selections = plan
        .resources
        .iter()
        .map(|resource| {
            (
                resource.resource_name.clone(),
                ProjectResourceTrackDemand {
                    version_selector: config
                        .resources
                        .get(&resource.resource_name)
                        .and_then(|config| config.track.clone()),
                    track: resource.track.clone(),
                },
            )
        })
        .collect();
    let mut resource_tracks = plan
        .resources
        .iter()
        .map(|resource| {
            DemandedResourceTrack::new(resource.resource_name.clone(), resource.track.clone())
        })
        .collect::<BTreeSet<_>>();
    if let Some(track) = php_track.as_deref() {
        resource_tracks.insert(DemandedResourceTrack::new("php", track));
        resource_tracks.insert(DemandedResourceTrack::new("frankenphp", track));
    }

    ProjectDemand {
        resource_tracks,
        resource_selections,
        php_track: php_track.map(|track| ProjectPhpTrackDemand {
            php_configured: php.is_some(),
            version_selector: php
                .and_then(config::PhpConfig::version_selector)
                .map(str::to_owned),
            serves_http,
            track,
        }),
        used_persisted_state: false,
    }
}

fn persisted_project_demand(
    database: &Database,
    project: &ProjectRecord,
) -> Result<ProjectDemand, DaemonError> {
    let mut resource_tracks = database
        .project_managed_resources(&project.id)?
        .into_iter()
        .map(|resource| DemandedResourceTrack::new(resource.resource_name, resource.track))
        .collect::<BTreeSet<_>>();
    if let Some(track) = &project.php_runtime.track {
        resource_tracks.insert(DemandedResourceTrack::new("php", track.clone()));
        resource_tracks.insert(DemandedResourceTrack::new("frankenphp", track.clone()));
    }

    Ok(ProjectDemand {
        resource_tracks,
        resource_selections: BTreeMap::new(),
        php_track: None,
        used_persisted_state: true,
    })
}

async fn reconcile_loaded_project(
    paths: &PvPaths,
    database: &mut Database,
    project: &ProjectRecord,
    catalog: Option<&ManagedResourceRuntimeCatalog>,
    discovered_demand: Option<&ProjectDemand>,
    demanded_tracks: &BTreeSet<DemandedResourceTrack>,
    progress: DaemonDownloadProgress,
) -> Result<ProjectEnvReconciliationSummary, DaemonError> {
    let config_file = match ProjectConfigFile::read_from_root(&project.path) {
        Ok(config_file) => config_file,
        Err(error) => {
            maintain_existing_project_tls_after_config_error(paths, project, false);
            return Err(error.into());
        }
    };
    let candidate_project = project_with_config_mode(project, &config_file.config);
    let serves_http = candidate_project.mode == ProjectMode::Served;
    let preflight_result = (|| {
        let plan = validate_project_config_and_plan(
            paths,
            database,
            &candidate_project,
            &config_file,
            discovered_demand,
        )?;
        let php_track = maybe_resolve_project_php_track(
            paths,
            database,
            &candidate_project,
            config_file.config.php.as_ref(),
            serves_http,
            discovered_demand.and_then(|demand| demand.php_track.as_ref()),
        )?;

        Ok::<_, DaemonError>((plan, php_track))
    })();
    let (plan, php_track) = match preflight_result {
        Ok(result) => result,
        Err(error) => {
            maintain_existing_project_tls_after_config_error(
                paths,
                project,
                config_file.config.uses_tls_placeholders(),
            );
            return Err(error);
        }
    };
    if discovered_demand.is_some()
        && let Some(track) = php_track.as_deref()
        && let Err(error) = install_project_php_pair(paths, catalog, track, progress.clone()).await
    {
        maintain_existing_project_tls_after_config_error(
            paths,
            project,
            config_file.config.uses_tls_placeholders(),
        );
        return Err(error);
    }
    let resolved_php_runtime = match php_track
        .map(|track| {
            resolve_project_php_runtime_for_track(database, config_file.config.php.as_ref(), track)
        })
        .transpose()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            maintain_existing_project_tls_after_config_error(
                paths,
                project,
                config_file.config.uses_tls_placeholders(),
            );
            return Err(error);
        }
    };
    let tls_maintenance_result = if serves_http && config_file.config.uses_tls_placeholders() {
        Some(ensure_project_tls_files(paths, &candidate_project))
    } else {
        None
    };
    let has_env_mappings = config_file.config.has_env_mappings();
    let pre_render_result = async {
        if let Some(runtime) = &resolved_php_runtime {
            record_project_php_runtime_resource_requirements(database, runtime)?;
        }
        apply_project_resource_plan(database, &candidate_project.id, &plan)?;

        let resource_result = if let Some(catalog) = catalog {
            crate::managed_resources::reconcile_project_resources_with_catalog_and_progress(
                paths,
                database,
                &candidate_project,
                &plan,
                catalog,
                demanded_tracks,
                progress,
            )
            .await
        } else {
            crate::managed_resources::reconcile_project_resources_with_progress(
                paths,
                database,
                &candidate_project,
                &plan,
                demanded_tracks,
                progress,
            )
            .await
        };
        resource_result?;

        let runtime_warnings = resolved_php_runtime
            .as_ref()
            .map(ignored_php_extension_warnings)
            .unwrap_or_default();
        let requested_php_extensions = config_file
            .config
            .php
            .as_ref()
            .is_some_and(|php| !php.requested_extensions().is_empty());

        Ok::<_, DaemonError>((plan, runtime_warnings, requested_php_extensions))
    }
    .await;
    let (plan, runtime_warnings, requested_php_extensions) = match pre_render_result {
        Ok(values) => values,
        Err(error) => {
            if let Some(Err(tls_error)) = tls_maintenance_result.as_ref() {
                structured_log::project_tls_maintenance_failed(
                    paths,
                    &candidate_project.id,
                    &tls_error.to_string(),
                );
            }
            return Err(error);
        }
    };

    if let Some(Err(error)) = tls_maintenance_result {
        return Err(error);
    }

    if !has_env_mappings {
        let status = if runtime_warnings.is_empty() {
            ProjectEnvObservedStatus::Rendered
        } else {
            ProjectEnvObservedStatus::Warning
        };
        let message = if runtime_warnings.is_empty() {
            "no Project env mappings configured"
        } else {
            "Project runtime has warnings"
        };
        finalize_project_reconciliation_state(
            database,
            project,
            &config_file,
            resolved_php_runtime.as_ref(),
            status,
            message,
            &runtime_warnings,
        )?;

        let summary = if runtime_warnings.is_empty() {
            ProjectEnvReconciliationSummary {
                message: "Project env unchanged; no mappings configured",
                requested_php_extensions,
            }
        } else {
            ProjectEnvReconciliationSummary {
                message: "Project env unchanged with warnings",
                requested_php_extensions,
            }
        };

        return Ok(summary);
    }

    let context = project_env_context_for_plan(
        paths,
        database,
        &candidate_project,
        &config_file.config,
        &plan,
    )?;
    let rendered = config::render_project_env(&config_file.config, &context)?;
    let env_file_path =
        config::resolve_project_env_file_path(&candidate_project.path, &config_file.config)?;
    let transform = config::write_project_env_file(&env_file_path, &rendered)?;
    let mut warnings = observed_warnings(&transform.warnings);
    warnings.extend(runtime_warnings);
    let status = if warnings.is_empty() {
        ProjectEnvObservedStatus::Rendered
    } else {
        ProjectEnvObservedStatus::Warning
    };
    let message = if warnings.is_empty() {
        "rendered Project env"
    } else {
        "rendered Project env with warnings"
    };

    finalize_project_reconciliation_state(
        database,
        project,
        &config_file,
        resolved_php_runtime.as_ref(),
        status,
        message,
        &warnings,
    )?;

    let summary = if warnings.is_empty() {
        ProjectEnvReconciliationSummary {
            message: "Project env rendered",
            requested_php_extensions,
        }
    } else {
        ProjectEnvReconciliationSummary {
            message: "Project env rendered with warnings",
            requested_php_extensions,
        }
    };

    Ok(summary)
}

fn maintain_existing_project_tls_after_config_error(
    paths: &PvPaths,
    project: &ProjectRecord,
    required_by_config: bool,
) {
    if project.mode == ProjectMode::Served
        && (required_by_config || matches!(project_tls_artifact_exists(paths, project), Ok(true)))
        && let Err(error) = ensure_project_tls_files(paths, project)
    {
        structured_log::project_tls_maintenance_failed(paths, &project.id, &error.to_string());
    }
}

fn ensure_project_tls_files(paths: &PvPaths, project: &ProjectRecord) -> Result<(), DaemonError> {
    let primary_hostname = served_project_hostname(project)?;
    let ca_certificate_pem = state::fs::read_to_string(&paths.ca_certificate())?;
    let ca_private_key_pem = state::fs::read_to_string(&paths.ca_private_key())?;

    if project_tls_files_are_current(paths, project, &ca_certificate_pem)? {
        return Ok(());
    }

    let certificate_path = paths.project_tls_certificate(&project.id);
    let private_key_path = paths.project_tls_private_key(&project.id);

    let generated = platform::generate_project_certificate(
        primary_hostname,
        &ca_certificate_pem,
        &ca_private_key_pem,
    )?;
    let certificate_chain_pem = format!("{}{}", generated.certificate_pem, ca_certificate_pem);

    state::fs::write_sensitive_file(&certificate_path, &certificate_chain_pem)?;
    state::fs::write_sensitive_file(&private_key_path, &generated.private_key_pem)?;

    Ok(())
}

pub(crate) fn project_tls_files_are_current(
    paths: &PvPaths,
    project: &ProjectRecord,
    ca_certificate_pem: &str,
) -> Result<bool, DaemonError> {
    let primary_hostname = served_project_hostname(project)?;
    let certificate_pem = read_optional_file(&paths.project_tls_certificate(&project.id))?;
    let private_key_pem = read_optional_file(&paths.project_tls_private_key(&project.id))?;

    Ok(match (certificate_pem, private_key_pem) {
        (Some(certificate_pem), Some(private_key_pem)) => platform::project_certificate_matches(
            &certificate_pem,
            &private_key_pem,
            primary_hostname,
            ca_certificate_pem,
        ),
        _ => false,
    })
}

pub(crate) fn project_tls_artifact_exists(
    paths: &PvPaths,
    project: &ProjectRecord,
) -> Result<bool, DaemonError> {
    Ok(
        state::fs::path_entry_exists(&paths.project_tls_certificate(&project.id))?
            || state::fs::path_entry_exists(&paths.project_tls_private_key(&project.id))?,
    )
}

fn read_optional_file(path: &Utf8PathBuf) -> Result<Option<String>, DaemonError> {
    match state::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

async fn install_project_php_pair(
    paths: &PvPaths,
    runtime_catalog: Option<&ManagedResourceRuntimeCatalog>,
    track: &str,
    progress: DaemonDownloadProgress,
) -> Result<(), DaemonError> {
    let production_catalog;
    let catalog = if let Some(catalog) = runtime_catalog {
        catalog
    } else {
        production_catalog = ManagedResourceRuntimeCatalog::production()?;
        &production_catalog
    };
    let demanded_tracks = BTreeSet::from([
        DemandedResourceTrack::new("php", track),
        DemandedResourceTrack::new("frankenphp", track),
    ]);

    crate::managed_resources::install_missing_resource_demands_with_catalog_and_progress(
        paths,
        catalog,
        &demanded_tracks,
        progress,
    )
    .await
}

fn maybe_resolve_project_php_track(
    paths: &PvPaths,
    database: &Database,
    project: &ProjectRecord,
    php: Option<&config::PhpConfig>,
    serves_http: bool,
    discovered_php_track: Option<&ProjectPhpTrackDemand>,
) -> Result<Option<String>, DaemonError> {
    if !serves_http && php.is_none() {
        return Ok(None);
    }

    if let Some(discovered_php_track) = discovered_php_track
        && discovered_php_track.matches(php, serves_http)
    {
        // Keep `latest` stable when the resource pass refreshes the manifest between phases.
        return Ok(Some(discovered_php_track.track.clone()));
    }

    if php.is_none()
        && project.desired_php_track.is_none()
        && !paths.downloads().join("manifest.json").exists()
        && database.global_php_default_track()?.is_none()
    {
        return Ok(None);
    }

    selected_project_php_track(paths, database, project, php).map(Some)
}

pub(crate) fn resolve_project_php_runtime(
    paths: &PvPaths,
    database: &Database,
    project: &ProjectRecord,
    php: Option<&config::PhpConfig>,
) -> Result<ResolvedPhpRuntime, DaemonError> {
    let track = selected_project_php_track(paths, database, project, php)?;

    resolve_project_php_runtime_for_track(database, php, track)
}

fn selected_project_php_track(
    paths: &PvPaths,
    database: &Database,
    project: &ProjectRecord,
    php: Option<&config::PhpConfig>,
) -> Result<String, DaemonError> {
    let selector = php.and_then(config::PhpConfig::version_selector);
    let global_selector = database.global_php_default_track()?;
    let stored_selector = if selector.is_some()
        || (!paths.downloads().join("manifest.json").exists() && global_selector.is_none())
    {
        project.desired_php_track.as_deref()
    } else {
        None
    };
    resolve_project_php_track(paths, selector, stored_selector, global_selector.as_deref())
}

fn resolve_project_php_runtime_for_track(
    database: &Database,
    php: Option<&config::PhpConfig>,
    track: String,
) -> Result<ResolvedPhpRuntime, DaemonError> {
    let requested_extensions = php
        .map(|php| php.requested_extensions().to_vec())
        .unwrap_or_default();
    let release = installed_php_release(database, &track)?;
    let resolution = match release {
        Some(release) => resources::resolve_php_extension_request(&release, &requested_extensions)?,
        None => resources::PhpExtensionResolution {
            requested: requested_extensions.clone(),
            loaded: Vec::new(),
            ignored: requested_extensions.clone(),
        },
    };
    let loaded_extensions = resolution
        .loaded
        .iter()
        .map(|module| module.name.clone())
        .collect::<Vec<_>>();
    let runtime_key = state::php_runtime_key(&track, &loaded_extensions)?;

    Ok(ResolvedPhpRuntime {
        track,
        runtime_key,
        requested_extensions: resolution.requested,
        loaded_extensions,
        ignored_extensions: resolution.ignored,
        loaded_modules: resolution.loaded,
    })
}

fn installed_php_release(
    database: &Database,
    track: &str,
) -> Result<Option<Utf8PathBuf>, DaemonError> {
    let release = database
        .managed_resource_tracks()?
        .into_iter()
        .find_map(|record| {
            if record.resource_name == "php"
                && record.track == track
                && record.desired_state == ManagedResourceDesiredState::Installed
                && record.installed_version.is_some()
            {
                return record.current_artifact_path;
            }

            None
        });

    Ok(release)
}

fn record_project_php_runtime_resource_requirements(
    database: &mut Database,
    runtime: &ResolvedPhpRuntime,
) -> Result<(), DaemonError> {
    database.record_managed_resource_track_desired(
        "php",
        &runtime.track,
        ManagedResourceDesiredState::Installed,
    )?;
    database.record_managed_resource_track_desired(
        "frankenphp",
        &runtime.track,
        ManagedResourceDesiredState::Installed,
    )?;

    Ok(())
}

fn validate_project_config_and_plan(
    paths: &PvPaths,
    database: &Database,
    project: &ProjectRecord,
    config_file: &ProjectConfigFile,
    discovered_demand: Option<&ProjectDemand>,
) -> Result<ProjectResourcePlan, DaemonError> {
    if project.mode == ProjectMode::Served && config_file.config.serve {
        database.validate_project_hostnames(
            &project.id,
            served_project_hostname(project)?,
            &config_file.config.hostnames,
        )?;
    }
    config::validate_project_env_shape(&config_file.config)?;

    let plan = project_resource_plan(
        paths,
        database,
        project,
        &config_file.config,
        discovered_demand,
    )?;
    if config_file.config.has_env_mappings() {
        let existing_content = read_optional_project_env_file(project, &config_file.config)?;
        config::validate_managed_env_block(existing_content.as_deref())?;
    }

    Ok(plan)
}

fn finalize_project_reconciliation_state(
    database: &mut Database,
    project: &ProjectRecord,
    config_file: &ProjectConfigFile,
    resolved_php_runtime: Option<&ResolvedPhpRuntime>,
    env_status: ProjectEnvObservedStatus,
    env_message: &str,
    env_warnings: &[ProjectEnvObservedWarningInput],
) -> Result<ProjectRecord, DaemonError> {
    let project =
        database
            .project_by_id(&project.id)?
            .ok_or_else(|| StateError::ProjectNotFound {
                target: project.id.clone(),
            })?;
    let mode = project_mode_for_config(&config_file.config);
    let primary_hostname = project
        .primary_hostname
        .clone()
        .unwrap_or_else(|| format!("{}.test", project.slug));
    let php_runtime = resolved_php_runtime.map(|runtime| ProjectPhpRuntimeInput {
        track: runtime.track.clone(),
        requested_extensions: runtime.requested_extensions.clone(),
        loaded_extensions: runtime.loaded_extensions.clone(),
        ignored_extensions: runtime.ignored_extensions.clone(),
    });
    let project = database.finalize_project_reconciliation(ProjectReconciliationStateInput {
        project_id: project.id.clone(),
        link: LinkProjectInput {
            path: project.path.clone(),
            original_path: project.original_path.clone(),
            primary_hostname,
            config_path: config_file.path.clone(),
            desired_php_track: None,
            additional_hostnames: config_file.config.hostnames.clone(),
        },
        mode,
        php_runtime,
        env_status,
        env_message: Some(env_message.to_string()),
        env_warnings: env_warnings.to_vec(),
    })?;

    Ok(project)
}

fn project_with_config_mode(project: &ProjectRecord, config: &ProjectConfig) -> ProjectRecord {
    let mut candidate = project.clone();
    candidate.mode = project_mode_for_config(config);
    if candidate.mode == ProjectMode::Served && candidate.primary_hostname.is_none() {
        candidate.primary_hostname = Some(format!("{}.test", candidate.slug));
    }

    candidate
}

fn project_mode_for_config(config: &ProjectConfig) -> ProjectMode {
    if config.serve {
        ProjectMode::Served
    } else {
        ProjectMode::ResourceOnly
    }
}

fn project_resource_plan(
    paths: &PvPaths,
    database: &Database,
    project: &ProjectRecord,
    config: &ProjectConfig,
    discovered_demand: Option<&ProjectDemand>,
) -> Result<ProjectResourcePlan, DaemonError> {
    let mut resources = Vec::new();
    let mut allocation_plans = BTreeMap::new();
    let existing_resource_tracks = database
        .project_managed_resources(&project.id)?
        .into_iter()
        .map(|resource| (resource.resource_name, resource.track))
        .collect::<BTreeMap<_, _>>();

    for (resource, resource_config) in &config.resources {
        let resource_name = ResourceName::new(resource.clone())?;
        let existing_track = existing_resource_tracks.get(resource);
        let discovered_track = discovered_demand
            .and_then(|demand| demand.resource_selections.get(resource))
            .filter(|selection| selection.version_selector == resource_config.track);
        let track = if let Some(selection) = discovered_track {
            selection.track.clone()
        } else {
            resolved_project_resource_track(
                paths,
                &resource_name,
                resource_config.track.as_deref(),
                existing_track.map(String::as_str),
            )?
        };

        resources.push(ProjectManagedResourceInput {
            resource_name: resource.clone(),
            track: track.clone(),
        });

        let existing_allocations = database
            .resource_allocations(&project.id, resource)?
            .into_iter()
            .map(|allocation| (allocation.allocation_name.clone(), allocation))
            .collect::<BTreeMap<_, _>>();
        let mut allocations = Vec::new();
        for allocation in resource_config.allocations.keys() {
            let generated_name =
                allocation_generated_name(resource, project, allocation, &existing_allocations)?;

            allocations.push(ResourceAllocationInput {
                allocation_name: allocation.clone(),
                generated_name,
            });
        }

        allocation_plans.insert(
            resource.clone(),
            ProjectResourceAllocationPlan { allocations },
        );
    }

    Ok(ProjectResourcePlan {
        resources,
        allocations: allocation_plans,
    })
}

fn allocation_generated_name(
    resource: &str,
    project: &ProjectRecord,
    allocation: &str,
    existing_allocations: &BTreeMap<String, ResourceAllocationRecord>,
) -> Result<String, DaemonError> {
    if let Some(existing) = existing_allocations.get(allocation) {
        return Ok(existing.generated_name.clone());
    }

    let generated = generated_allocation_name(resource, &project.slug, allocation)?;

    Ok(generated.generated_name().to_string())
}

fn resolved_project_resource_track(
    paths: &PvPaths,
    resource_name: &ResourceName,
    selector: Option<&str>,
    existing_track: Option<&str>,
) -> Result<String, DaemonError> {
    let selector = selector
        .map(|selector| TrackSelector::parse(selector.to_string()))
        .transpose()?
        .unwrap_or(TrackSelector::Latest);
    let track = match selector {
        TrackSelector::Latest => match existing_track {
            Some(track) => track.to_string(),
            None => ArtifactManifestCache::new(paths.downloads())
                .load_cached()?
                .resolve_track(resource_name, TrackSelector::Latest)?
                .as_str()
                .to_string(),
        },
        TrackSelector::Track(track) => track.as_str().to_string(),
    };
    let concrete_track = ConcreteTrackName::new(track)?;

    Ok(concrete_track.as_str().to_string())
}

pub(crate) fn resolve_project_php_track(
    paths: &PvPaths,
    config_selector: Option<&str>,
    stored_selector: Option<&str>,
    global_selector: Option<&str>,
) -> Result<String, DaemonError> {
    let selector = config_selector.map(TrackSelector::parse).transpose()?;
    let track = match selector {
        Some(TrackSelector::Latest) => match stored_selector {
            Some(track) => track.to_string(),
            None => default_project_php_track(paths)?,
        },
        Some(TrackSelector::Track(track)) => track.as_str().to_owned(),
        None => match stored_selector {
            Some(track) => track.to_string(),
            None => match global_selector {
                Some(track) => track.to_string(),
                None => default_project_php_track(paths)?,
            },
        },
    };
    let track = ConcreteTrackName::new(track)?;

    Ok(track.as_str().to_owned())
}

fn default_project_php_track(paths: &PvPaths) -> Result<String, DaemonError> {
    let manifest = ArtifactManifestCache::new(paths.downloads().to_path_buf()).load_cached()?;
    let php = ResourceName::new("php")?;
    let track = manifest.resolve_track(&php, TrackSelector::Latest)?;

    Ok(track.as_str().to_owned())
}

fn apply_project_resource_plan(
    database: &mut Database,
    project_id: &str,
    plan: &ProjectResourcePlan,
) -> Result<(), DaemonError> {
    let existing_resources = database.project_managed_resources(project_id)?;

    database.replace_project_managed_resources(project_id, &plan.resources)?;

    for resource in &plan.resources {
        let Some(allocation_plan) = plan.allocations.get(&resource.resource_name) else {
            continue;
        };

        database.replace_project_resource_allocations(
            project_id,
            &resource.resource_name,
            &resource.track,
            &allocation_plan.allocations,
        )?;
    }

    for existing in existing_resources {
        if plan
            .allocations
            .contains_key(existing.resource_name.as_str())
        {
            continue;
        }

        database.replace_project_resource_allocations(
            project_id,
            &existing.resource_name,
            &existing.track,
            &[],
        )?;
    }

    Ok(())
}

fn project_env_context_for_plan(
    paths: &PvPaths,
    database: &Database,
    project: &ProjectRecord,
    config: &ProjectConfig,
    plan: &ProjectResourcePlan,
) -> Result<ProjectEnvContext, DaemonError> {
    let mut resources = BTreeMap::new();

    for resource in &plan.resources {
        let allocations = planned_allocation_contexts(
            database,
            &project.id,
            &resource.resource_name,
            &resource.track,
            plan.allocations.get(&resource.resource_name),
        )?;
        let track = database.managed_resource_track(&resource.resource_name, &resource.track)?;
        if track.env.is_empty() {
            return Err(config::ConfigError::MissingResourceEnvContext {
                resource: resource.resource_name.clone(),
            }
            .into());
        }

        resources.insert(
            resource.resource_name.clone(),
            ResourceEnvContext {
                track: resource.track.clone(),
                values: track.env,
                allocations,
            },
        );
    }

    let serves_http = project.mode == ProjectMode::Served && config.serve;
    Ok(ProjectEnvContext {
        primary_hostname: if serves_http {
            served_project_hostname(project)?.to_string()
        } else {
            String::new()
        },
        tls_ca_path: if serves_http {
            paths.ca_certificate().to_string()
        } else {
            String::new()
        },
        tls_cert_path: if serves_http {
            paths.project_tls_certificate(&project.id).to_string()
        } else {
            String::new()
        },
        tls_key_path: if serves_http {
            paths.project_tls_private_key(&project.id).to_string()
        } else {
            String::new()
        },
        resources,
    })
}

fn planned_allocation_contexts(
    database: &Database,
    project_id: &str,
    resource_name: &str,
    track: &str,
    allocation_plan: Option<&ProjectResourceAllocationPlan>,
) -> Result<BTreeMap<String, AllocationEnvContext>, DaemonError> {
    let Some(allocation_plan) = allocation_plan else {
        return Ok(BTreeMap::new());
    };
    let existing_allocations = database
        .resource_allocations(project_id, resource_name)?
        .into_iter()
        .map(|allocation| (allocation.allocation_name.clone(), allocation))
        .collect::<BTreeMap<_, _>>();
    let mut allocations = BTreeMap::new();

    for allocation in &allocation_plan.allocations {
        let Some(existing) = existing_allocations.get(&allocation.allocation_name) else {
            return Err(config::ConfigError::MissingAllocationEnvContext {
                resource: resource_name.to_string(),
                allocation: allocation.allocation_name.clone(),
            }
            .into());
        };
        if existing.track != track || existing.status != ResourceAllocationStatus::Ready {
            return Err(config::ConfigError::MissingAllocationEnvContext {
                resource: resource_name.to_string(),
                allocation: allocation.allocation_name.clone(),
            }
            .into());
        }
        allocations.insert(
            allocation.allocation_name.clone(),
            AllocationEnvContext {
                generated_name: existing.generated_name.clone(),
                values: existing.env.clone(),
            },
        );
    }

    Ok(allocations)
}

fn read_optional_project_env_file(
    project: &ProjectRecord,
    config: &ProjectConfig,
) -> Result<Option<String>, DaemonError> {
    let path = config::resolve_project_env_file_path(&project.path, config)?;
    match state::fs::read_to_string(&path) {
        Ok(content) => Ok(Some(content)),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

fn served_project_hostname(project: &ProjectRecord) -> Result<&str, DaemonError> {
    project
        .primary_hostname
        .as_deref()
        .ok_or_else(|| StateError::ProjectNotServed {
            project_id: project.id.clone(),
        })
        .map_err(Into::into)
}

fn observed_warnings(warnings: &[ProjectEnvWarning]) -> Vec<ProjectEnvObservedWarningInput> {
    warnings
        .iter()
        .map(|warning| match warning {
            ProjectEnvWarning::DuplicateExistingKey { key } => ProjectEnvObservedWarningInput {
                kind: "duplicate_key".to_string(),
                message: format!(
                    "generated Project env key `{key}` already exists outside the PV-managed block"
                ),
            },
        })
        .collect()
}

fn ignored_php_extension_warnings(
    runtime: &ResolvedPhpRuntime,
) -> Vec<ProjectEnvObservedWarningInput> {
    runtime
        .ignored_extensions
        .iter()
        .map(|extension| ProjectEnvObservedWarningInput {
            kind: "ignored_php_extension".to_string(),
            message: format!("ignored unsupported PHP extension `{extension}`"),
        })
        .collect()
}

fn record_project_env_failure(
    database: &mut Database,
    project_id: &str,
    message: &str,
) -> Result<(), DaemonError> {
    database.record_project_env_observed_snapshot(
        project_id,
        ProjectEnvObservedStatus::Failed,
        Some(message),
        &[],
    )?;

    Ok(())
}
