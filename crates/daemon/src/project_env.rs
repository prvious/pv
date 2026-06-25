use std::collections::BTreeMap;
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
    Database, ManagedResourceDesiredState, ProjectEnvObservedStatus,
    ProjectEnvObservedWarningInput, ProjectManagedResourceInput, ProjectPhpRuntimeInput,
    ProjectRecord, PvPaths, ResourceAllocationInput, ResourceAllocationRecord,
    ResourceAllocationStatus, StateError,
};

use crate::DaemonError;
use crate::managed_resources::ManagedResourceRuntimeCatalog;

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

pub(crate) async fn reconcile_project_env(
    paths: &PvPaths,
    project_id: &str,
) -> Result<ProjectEnvReconciliationSummary, DaemonError> {
    reconcile_project_env_with_runtime_catalog(paths, project_id, None).await
}

pub(crate) async fn reconcile_project_env_with_runtime_catalog(
    paths: &PvPaths,
    project_id: &str,
    catalog: Option<&ManagedResourceRuntimeCatalog>,
) -> Result<ProjectEnvReconciliationSummary, DaemonError> {
    let mut database = Database::open(paths)?;
    let project =
        database
            .project_by_id(project_id)?
            .ok_or_else(|| StateError::ProjectNotFound {
                target: project_id.to_string(),
            })?;

    match reconcile_loaded_project(paths, &mut database, &project, catalog).await {
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

    match reconcile_loaded_project(paths, database, &project, Some(catalog)).await {
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
    let _plan = validate_project_config_and_plan(paths, database, project, config_file)?;

    Ok(())
}

async fn reconcile_loaded_project(
    paths: &PvPaths,
    database: &mut Database,
    project: &ProjectRecord,
    catalog: Option<&ManagedResourceRuntimeCatalog>,
) -> Result<ProjectEnvReconciliationSummary, DaemonError> {
    let config_file = ProjectConfigFile::read_from_root(&project.path)?;
    let plan = validate_project_config_and_plan(paths, database, project, &config_file)?;
    let resolved_php_runtime = maybe_resolve_project_php_runtime(
        paths,
        database,
        project,
        config_file.config.php.as_ref(),
    )?;
    let has_env_mappings = config_file.config.has_env_mappings();

    if let Some(runtime) = &resolved_php_runtime {
        record_project_php_runtime_resource_requirements(database, runtime)?;
    }
    apply_project_resource_plan(database, &project.id, &plan)?;
    if let Some(catalog) = catalog {
        crate::managed_resources::reconcile_project_resources_with_catalog(
            paths, database, project, &plan, catalog,
        )
        .await?;
    } else {
        crate::managed_resources::reconcile_project_resources(paths, database, project, &plan)
            .await?;
    }
    if let Some(runtime) = &resolved_php_runtime {
        database.replace_project_php_runtime(
            &project.id,
            Some(&ProjectPhpRuntimeInput {
                track: runtime.track.clone(),
                requested_extensions: runtime.requested_extensions.clone(),
                loaded_extensions: runtime.loaded_extensions.clone(),
                ignored_extensions: runtime.ignored_extensions.clone(),
            }),
        )?;
    } else if project.desired_php_track.is_some() {
        database.replace_project_php_runtime(&project.id, None)?;
    }
    database.replace_project_additional_hostnames(&project.id, &config_file.config.hostnames)?;

    let runtime_warnings = resolved_php_runtime
        .as_ref()
        .map(ignored_php_extension_warnings)
        .unwrap_or_default();
    let requested_php_extensions = config_file
        .config
        .php
        .as_ref()
        .is_some_and(|php| !php.requested_extensions().is_empty());
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
        database.record_project_env_observed_snapshot(
            &project.id,
            status,
            Some(message),
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

    let context = project_env_context_for_plan(database, project, &plan)?;
    let rendered = config::render_project_env(&config_file.config, &context)?;
    let transform = config::write_project_env_file(&project.path.join(".env"), &rendered)?;
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

    database.record_project_env_observed_snapshot(&project.id, status, Some(message), &warnings)?;

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

fn maybe_resolve_project_php_runtime(
    paths: &PvPaths,
    database: &Database,
    project: &ProjectRecord,
    php: Option<&config::PhpConfig>,
) -> Result<Option<ResolvedPhpRuntime>, DaemonError> {
    if php.is_none()
        && project.desired_php_track.is_none()
        && !paths.downloads().join("manifest.json").exists()
        && database.global_php_default_track()?.is_none()
    {
        return Ok(None);
    }

    resolve_project_php_runtime(paths, database, project, php).map(Some)
}

pub(crate) fn resolve_project_php_runtime(
    paths: &PvPaths,
    database: &Database,
    project: &ProjectRecord,
    php: Option<&config::PhpConfig>,
) -> Result<ResolvedPhpRuntime, DaemonError> {
    let selector = php.and_then(config::PhpConfig::version_selector);
    let global_selector = database.global_php_default_track()?;
    let stored_selector = if selector.is_some()
        || (!paths.downloads().join("manifest.json").exists() && global_selector.is_none())
    {
        project.desired_php_track.as_deref()
    } else {
        None
    };
    let track =
        resolve_project_php_track(paths, selector, stored_selector, global_selector.as_deref())?;
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
) -> Result<ProjectResourcePlan, DaemonError> {
    database.validate_project_hostnames(
        &project.id,
        &project.primary_hostname,
        &config_file.config.hostnames,
    )?;
    config::validate_project_env_shape(&config_file.config)?;

    let plan = project_resource_plan(paths, database, project, &config_file.config)?;
    if config_file.config.has_env_mappings() {
        let existing_content = read_optional_dotenv(project)?;
        config::validate_managed_env_block(existing_content.as_deref())?;
    }

    Ok(plan)
}

fn project_resource_plan(
    paths: &PvPaths,
    database: &Database,
    project: &ProjectRecord,
    config: &ProjectConfig,
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
        let track = resolved_project_resource_track(
            paths,
            &resource_name,
            resource_config.track.as_deref(),
            existing_track.map(String::as_str),
        )?;

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

    let generated = generated_allocation_name(resource, &project.primary_hostname, allocation)?;

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
    database: &Database,
    project: &ProjectRecord,
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

    Ok(ProjectEnvContext {
        primary_hostname: project.primary_hostname.clone(),
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

fn read_optional_dotenv(project: &ProjectRecord) -> Result<Option<String>, DaemonError> {
    match state::fs::read_to_string(&project.path.join(".env")) {
        Ok(content) => Ok(Some(content)),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
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
