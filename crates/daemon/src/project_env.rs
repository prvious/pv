use std::collections::BTreeMap;
use std::io;

use config::{
    AllocationEnvContext, ProjectConfig, ProjectConfigFile, ProjectEnvContext, ProjectEnvWarning,
    ResourceEnvContext,
};
use resources::{
    ArtifactManifestCache, ConcreteTrackName, ResourceName, TrackSelector,
    generated_allocation_name,
};
use state::{
    Database, ProjectEnvObservedStatus, ProjectEnvObservedWarningInput,
    ProjectManagedResourceInput, ProjectRecord, PvPaths, ResourceAllocationInput,
    ResourceAllocationRecord, ResourceAllocationStatus, StateError,
};

use crate::DaemonError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectEnvReconciliationSummary {
    message: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProjectResourcePlan {
    resources: Vec<ProjectManagedResourceInput>,
    allocations: BTreeMap<String, ProjectResourceAllocationPlan>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProjectResourceAllocationPlan {
    track: String,
    allocations: Vec<ResourceAllocationInput>,
}

impl ProjectEnvReconciliationSummary {
    pub(crate) fn as_str(&self) -> &'static str {
        self.message
    }
}

pub(crate) fn reconcile_project_env(
    paths: &PvPaths,
    project_id: &str,
) -> Result<ProjectEnvReconciliationSummary, DaemonError> {
    let mut database = Database::open(paths)?;
    let project =
        database
            .project_by_id(project_id)?
            .ok_or_else(|| StateError::ProjectNotFound {
                target: project_id.to_string(),
            })?;

    match reconcile_loaded_project(paths, &mut database, &project) {
        Ok(summary) => Ok(summary),
        Err(error) => {
            let message = error.to_string();
            record_project_env_failure(&mut database, &project.id, &message)?;

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

fn reconcile_loaded_project(
    paths: &PvPaths,
    database: &mut Database,
    project: &ProjectRecord,
) -> Result<ProjectEnvReconciliationSummary, DaemonError> {
    let config_file = ProjectConfigFile::read_from_root(&project.path)?;
    let plan = validate_project_config_and_plan(paths, database, project, &config_file)?;
    let resolved_php_track = config_file
        .config
        .php
        .as_deref()
        .map(|selector| {
            resolve_project_php_track(paths, Some(selector), project.desired_php_track.as_deref())
        })
        .transpose()?;
    let has_env_mappings = config_file.config.has_env_mappings();

    apply_project_resource_plan(database, &project.id, &plan)?;
    if let Some(resolved_php_track) = resolved_php_track {
        database.replace_project_desired_php_track(&project.id, Some(&resolved_php_track))?;
    }
    database.replace_project_additional_hostnames(&project.id, &config_file.config.hostnames)?;

    if !has_env_mappings {
        database.record_project_env_observed_snapshot(
            &project.id,
            ProjectEnvObservedStatus::Rendered,
            Some("no Project env mappings configured"),
            &[],
        )?;

        return Ok(ProjectEnvReconciliationSummary {
            message: "Project env unchanged; no mappings configured",
        });
    }

    let context = project_env_context_for_plan(database, project, &plan)?;
    let rendered = config::render_project_env(&config_file.config, &context)?;
    let transform = config::write_project_env_file(&project.path.join(".env"), &rendered)?;
    let warnings = observed_warnings(&transform.warnings);
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
        }
    } else {
        ProjectEnvReconciliationSummary {
            message: "Project env rendered with warnings",
        }
    };

    Ok(summary)
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
            ProjectResourceAllocationPlan { track, allocations },
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
) -> Result<String, DaemonError> {
    let selector = config_selector
        .or(stored_selector)
        .map(TrackSelector::parse)
        .transpose()?
        .unwrap_or(TrackSelector::Latest);

    match selector {
        TrackSelector::Latest => {
            let manifest =
                ArtifactManifestCache::new(paths.downloads().to_path_buf()).load_cached()?;
            let php = ResourceName::new("php")?;
            let track = manifest.resolve_track(&php, TrackSelector::Latest)?;

            Ok(track.as_str().to_owned())
        }
        TrackSelector::Track(track) => Ok(track.as_str().to_owned()),
    }
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
            &allocation_plan.track,
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
