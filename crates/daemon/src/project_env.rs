use std::collections::BTreeMap;

use config::{
    AllocationEnvContext, ProjectConfig, ProjectConfigFile, ProjectEnvContext, ProjectEnvWarning,
    ResourceEnvContext,
};
use resources::{ConcreteTrackName, ResourceName, TrackSelector, generated_allocation_name};
use state::{
    Database, ProjectEnvObservedStatus, ProjectEnvObservedWarningInput, ProjectEnvStateContext,
    ProjectManagedResourceInput, ProjectRecord, PvPaths, ResourceAllocationInput, StateError,
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

    match reconcile_loaded_project(&mut database, &project) {
        Ok(summary) => Ok(summary),
        Err(error) => {
            let message = error.to_string();
            record_project_env_failure(&mut database, &project.id, &message)?;

            Err(error)
        }
    }
}

fn reconcile_loaded_project(
    database: &mut Database,
    project: &ProjectRecord,
) -> Result<ProjectEnvReconciliationSummary, DaemonError> {
    let config_file = ProjectConfigFile::read_from_root(&project.path)?;

    database.validate_project_hostnames(
        &project.id,
        &project.primary_hostname,
        &config_file.config.hostnames,
    )?;

    let plan = project_resource_plan(project, &config_file.config)?;
    apply_project_resource_plan(database, &project.id, plan)?;

    let context = project_env_context(database.project_env_context(&project.id)?);
    let rendered = config::render_project_env(&config_file.config, &context)?;

    if !has_env_mappings(&config_file.config) {
        database.replace_project_env_observed_warnings(&project.id, &[])?;
        database.record_project_env_observed_state(
            &project.id,
            ProjectEnvObservedStatus::Rendered,
            Some("no Project env mappings configured"),
        )?;

        return Ok(ProjectEnvReconciliationSummary {
            message: "Project env unchanged; no mappings configured",
        });
    }

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

    database.replace_project_env_observed_warnings(&project.id, &warnings)?;
    database.record_project_env_observed_state(&project.id, status, Some(message))?;

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

fn project_resource_plan(
    project: &ProjectRecord,
    config: &ProjectConfig,
) -> Result<ProjectResourcePlan, DaemonError> {
    let mut resources = Vec::new();
    let mut allocation_plans = BTreeMap::new();

    for (resource, resource_config) in &config.resources {
        if resource_config
            .track
            .as_deref()
            .is_some_and(TrackSelector::is_reserved_alias)
        {
            return Err(DaemonError::UnresolvedProjectResourceLatest {
                project_id: project.id.clone(),
                resource: resource.clone(),
            });
        }

        let resource_name = ResourceName::new(resource.clone())?;
        let concrete_track =
            ConcreteTrackName::required(&resource_name, resource_config.track.as_deref())?;
        let track = concrete_track.as_str().to_string();

        resources.push(ProjectManagedResourceInput {
            resource_name: resource.clone(),
            track: track.clone(),
        });

        let mut allocations = Vec::new();
        for allocation in resource_config.allocations.keys() {
            let generated =
                generated_allocation_name(resource, &project.primary_hostname, allocation)?;

            allocations.push(ResourceAllocationInput {
                allocation_name: allocation.clone(),
                generated_name: generated.generated_name().to_string(),
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

fn apply_project_resource_plan(
    database: &mut Database,
    project_id: &str,
    plan: ProjectResourcePlan,
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

fn project_env_context(context: ProjectEnvStateContext) -> ProjectEnvContext {
    ProjectEnvContext {
        primary_hostname: context.primary_hostname,
        resources: context
            .resources
            .into_iter()
            .map(|(resource_name, resource)| {
                (
                    resource_name,
                    ResourceEnvContext {
                        track: resource.track,
                        values: resource.values,
                        allocations: resource
                            .allocations
                            .into_iter()
                            .map(|(allocation_name, allocation)| {
                                (
                                    allocation_name,
                                    AllocationEnvContext {
                                        generated_name: allocation.generated_name,
                                        values: allocation.values,
                                    },
                                )
                            })
                            .collect(),
                    },
                )
            })
            .collect(),
    }
}

fn has_env_mappings(config: &ProjectConfig) -> bool {
    !config.env.is_empty()
        || config.resources.values().any(|resource| {
            !resource.env.is_empty()
                || resource
                    .allocations
                    .values()
                    .any(|allocation| !allocation.env.is_empty())
        })
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
    database.replace_project_env_observed_warnings(project_id, &[])?;
    database.record_project_env_observed_state(
        project_id,
        ProjectEnvObservedStatus::Failed,
        Some(message),
    )?;

    Ok(())
}
