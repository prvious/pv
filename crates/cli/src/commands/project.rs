use std::io;
use std::io::Write;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use config::{
    AllocationEnvContext, ProjectConfigFile, ProjectEnvContext, ProjectEnvWarning,
    ResourceEnvContext,
};
use resources::{ArtifactManifestCache, ConcreteTrackName, ResourceName, TrackSelector};
use serde::Serialize;
use state::{
    Database, LinkProjectInput, LinkProjectStatus, ProjectEnvObservedStateRecord,
    ProjectEnvObservedStatus, ProjectEnvStateContext, ProjectMode, ProjectRecord, PvPaths,
    StateError,
};

use crate::args::{LinkArgs, ListArgs, OpenArgs, ProjectEnvArgs, UnlinkArgs};
use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Output, OutputMode};

pub(crate) fn link(
    args: LinkArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let original_project_path = resolve_project_path(args.path.as_deref(), environment)?;
    let config_file = ProjectConfigFile::read_from_root(&original_project_path)?;
    config::validate_project_env_shape(&config_file.config)?;
    let project_path = project_root_from_config_path(&config_file.path)?;
    let desired_php_track = resolved_project_php_track(
        &paths,
        config_file
            .config
            .php
            .as_ref()
            .and_then(|php| php.version_selector()),
    )?;
    let mut database = Database::open(&paths)?;
    let existing = database.project_by_path(&project_path)?;
    let mode = if config_file.config.serve {
        ProjectMode::Served
    } else {
        ProjectMode::ResourceOnly
    };
    let primary_hostname = match (
        config_file.config.serve.then_some(args.hostname).flatten(),
        existing.as_ref(),
    ) {
        (Some(hostname), _) => config::normalize_primary_hostname(&hostname)?,
        (None, Some(project)) => project
            .primary_hostname
            .clone()
            .unwrap_or_else(|| format!("{}.test", project.slug)),
        (None, None) => config::hostname_from_project_path(&project_path)?,
    };
    let result = database.link_project_with_mode(
        LinkProjectInput {
            path: project_path.clone(),
            original_path: original_project_path,
            primary_hostname,
            config_path: config_file.path,
            desired_php_track,
            additional_hostnames: config_file.config.hostnames,
        },
        mode,
    )?;

    let mut output = Output::new(stdout, OutputMode::plain());
    let project_name = if result.project.mode == ProjectMode::ResourceOnly {
        format!("{} (resource-only)", result.project.slug)
    } else {
        project_display_name(&result.project).to_string()
    };
    match result.status {
        LinkProjectStatus::Created => {
            output.line(&format!("Linked {project_name} -> {}", result.project.path,))?
        }
        LinkProjectStatus::Updated => output.line(&format!(
            "Updated {project_name} -> {}",
            result.project.path,
        ))?,
        LinkProjectStatus::Unchanged => output.line(&format!(
            "Already linked {project_name} -> {}",
            result.project.path,
        ))?,
    }
    request_project_reconciliation(&paths, &result.project, &mut output)?;

    Ok(ExitCode::SUCCESS)
}

fn resolved_project_php_track(
    paths: &PvPaths,
    selector: Option<&str>,
) -> Result<Option<String>, ExecuteError> {
    let Some(selector) = selector else {
        return Ok(None);
    };
    let selector = TrackSelector::parse(selector)?;
    let track = match selector {
        TrackSelector::Latest => {
            let manifest = ArtifactManifestCache::new(paths.downloads()).load_cached()?;
            let php = ResourceName::new("php")?;

            manifest
                .resolve_track(&php, TrackSelector::Latest)?
                .as_str()
                .to_owned()
        }
        TrackSelector::Track(track) => track.as_str().to_owned(),
    };
    let track = ConcreteTrackName::new(track)?;

    Ok(Some(track.as_str().to_owned()))
}

pub(crate) fn unlink(
    args: UnlinkArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let mut database = Database::open(&paths)?;
    let project = resolve_project(&database, args.hostname.as_deref(), environment)?;
    delete_optional_project_tls_dir(&paths, &project)?;
    let project = database.unlink_project(&project.id)?;
    let mut output = Output::new(stdout, OutputMode::plain());

    output.line(&format!(
        "Unlinked {} -> {}",
        project_display_name(&project),
        project.path
    ))?;
    request_system_reconciliation(&paths, &mut output)?;

    Ok(ExitCode::SUCCESS)
}

fn delete_optional_project_tls_dir(
    paths: &PvPaths,
    project: &ProjectRecord,
) -> Result<(), ExecuteError> {
    match state::fs::delete_dir_all(&paths.project_tls_dir(&project.id)) {
        Ok(()) => Ok(()),
        Err(StateError::Filesystem { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn open(
    args: OpenArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let database = Database::open(&paths)?;
    let (project, hostname) = match args.hostname {
        Some(selector) => {
            let resolved = resolve_project_selector(&database, &selector)?;
            let hostname = served_project_hostname(&resolved.project)?.to_string();
            (
                resolved.project,
                resolved.matched_hostname.unwrap_or(hostname),
            )
        }
        None => resolve_open_project(&database, environment, stdout)?,
    };
    let url = format!("https://{hostname}");

    environment.open_url(&url)?;

    let mut output = Output::new(stdout, OutputMode::plain());
    output.line(&format!(
        "Opened {} for {}",
        url,
        project_display_name(&project)
    ))?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn env(
    args: ProjectEnvArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let database = Database::open(&paths)?;
    let project = resolve_project(&database, args.hostname.as_deref(), environment)?;
    let config_file = ProjectConfigFile::read_from_root(&project.path)?;
    let serves_http = project.mode == ProjectMode::Served && config_file.config.serve;
    if serves_http {
        database.validate_project_hostnames(
            &project.id,
            served_project_hostname(&project)?,
            &config_file.config.hostnames,
        )?;
    }
    config::validate_project_env_shape(&config_file.config)?;

    let context = project_env_context(
        &paths,
        database.project_env_context(&project.id)?,
        serves_http,
    );
    let rendered = config::render_project_env(&config_file.config, &context)?;
    let warnings = if config_file.config.has_env_mappings() {
        let env_file_path =
            config::resolve_project_env_file_path(&project.path, &config_file.config)?;
        let existing_env = read_project_env_file(&env_file_path)?;
        config::transform_managed_env_block(existing_env.as_deref(), &rendered)?.warnings
    } else {
        Vec::new()
    };

    if args.json {
        serde_json::to_writer(&mut *stdout, &rendered.values)?;
        writeln!(stdout)?;
        write_project_env_warnings(&warnings, stderr)?;

        return Ok(ExitCode::SUCCESS);
    }

    let content = config::format_project_env(&rendered);
    if content.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }

    write!(stdout, "{content}")?;
    write_project_env_warnings(&warnings, stderr)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn list(
    args: ListArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let database = Database::open(&paths)?;
    let mut projects = database.projects()?;
    projects.sort_by(|left, right| {
        project_display_name(left)
            .cmp(project_display_name(right))
            .then_with(|| left.id.cmp(&right.id))
    });

    if args.json {
        let projects = projects
            .into_iter()
            .map(|project| project_list_item(&database, project))
            .collect::<Result<Vec<_>, _>>()?;
        serde_json::to_writer(&mut *stdout, &ProjectListOutput { projects })?;
        writeln!(stdout)?;

        return Ok(ExitCode::SUCCESS);
    }

    let mut output = Output::new(stdout, OutputMode::plain());

    if projects.is_empty() {
        output.line("No linked Projects")?;
        return Ok(ExitCode::SUCCESS);
    }

    output.line("Project  Mode  PHP  Status  Resources  Env  Path")?;
    for project in projects {
        let status = project_list_status(&database, &project)?;
        output.line(&format!(
            "{}  {}  {}  {}  unknown  {}  {}",
            project_display_name(&project),
            project.mode.as_str(),
            project.desired_php_track.as_deref().unwrap_or("default"),
            status.project.as_str(),
            status.env.as_str(),
            project.path
        ))?;
        if let Some(error) = status.config_error {
            output.line(&format!("  config: {error}"))?;
        }
        if let Some(detail) = status.env_detail {
            output.line(&format!("  env: {detail}"))?;
        }
    }

    Ok(ExitCode::SUCCESS)
}

fn request_project_reconciliation(
    paths: &PvPaths,
    project: &ProjectRecord,
    output: &mut Output<'_, impl Write>,
) -> Result<(), ExecuteError> {
    let scope = format!("project:{}", project.id);
    match daemon::submit_job_blocking(paths.clone(), "reconcile", &scope) {
        Ok(job) => output.line(&format!(
            "Queued reconciliation {} for {}",
            job.id,
            project_display_name(project)
        ))?,
        Err(daemon::DaemonError::Io(error)) if daemon_is_unavailable(&error) => output.line(
            "warning: PV daemon is not running; reconciliation will run after `pv setup` starts it",
        )?,
        Err(error) => return Err(error.into()),
    }

    Ok(())
}

fn request_system_reconciliation(
    paths: &PvPaths,
    output: &mut Output<'_, impl Write>,
) -> Result<(), ExecuteError> {
    match daemon::submit_job_blocking(paths.clone(), "reconcile", "system") {
        Ok(job) => output.line(&format!("System reconciliation requested: {}", job.id))?,
        Err(daemon::DaemonError::Io(error)) if daemon_is_unavailable(&error) => output.line(
            "warning: PV daemon is not running; reconciliation will run after `pv setup` starts it",
        )?,
        Err(error) => return Err(error.into()),
    }

    Ok(())
}

fn resolve_open_project(
    database: &Database,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<(ProjectRecord, String), ExecuteError> {
    let current_dir = current_dir(environment)?;
    if let Some(project) = database.nearest_project_for_path(&current_dir)? {
        let hostname = served_project_hostname(&project)?.to_string();
        return Ok((project, hostname));
    }

    if !environment.stdin_is_terminal() {
        return Err(CliError::ProjectNotResolved.into());
    }

    let mut projects = database
        .projects()?
        .into_iter()
        .filter(|project| project.mode == ProjectMode::Served && project.primary_hostname.is_some())
        .collect::<Vec<_>>();
    projects.sort_by(|left, right| {
        left.primary_hostname
            .cmp(&right.primary_hostname)
            .then_with(|| left.id.cmp(&right.id))
    });
    let project = select_project(projects, environment, stdout)?;
    let hostname = served_project_hostname(&project)?.to_string();

    Ok((project, hostname))
}

fn select_project(
    projects: Vec<ProjectRecord>,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ProjectRecord, ExecuteError> {
    if projects.is_empty() {
        return Err(CliError::ProjectNotResolved.into());
    }

    let mut output = Output::new(stdout, OutputMode::plain());
    output.line("Select a Project:")?;
    for (index, project) in projects.iter().enumerate() {
        output.line(&format!(
            "{}. {}  {}",
            index + 1,
            project_display_name(project),
            project.path
        ))?;
    }
    output.line("Enter selection:")?;

    let selection = environment.read_line()?;
    let selected_index =
        selection
            .trim()
            .parse::<usize>()
            .map_err(|_| CliError::InvalidProjectSelection {
                selection: selection.trim().to_string(),
                count: projects.len(),
            })?;
    let Some(index) = selected_index.checked_sub(1) else {
        return Err(CliError::InvalidProjectSelection {
            selection: selection.trim().to_string(),
            count: projects.len(),
        }
        .into());
    };
    let Some(project) = projects.get(index).cloned() else {
        return Err(CliError::InvalidProjectSelection {
            selection: selection.trim().to_string(),
            count: projects.len(),
        }
        .into());
    };

    Ok(project)
}

fn daemon_is_unavailable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

fn project_env_context(
    paths: &PvPaths,
    context: ProjectEnvStateContext,
    serves_http: bool,
) -> ProjectEnvContext {
    let project_id = context.project_id;

    ProjectEnvContext {
        primary_hostname: if serves_http {
            context.primary_hostname.unwrap_or_default()
        } else {
            String::new()
        },
        tls_ca_path: if serves_http {
            paths.ca_certificate().to_string()
        } else {
            String::new()
        },
        tls_cert_path: if serves_http {
            paths.project_tls_certificate(&project_id).to_string()
        } else {
            String::new()
        },
        tls_key_path: if serves_http {
            paths.project_tls_private_key(&project_id).to_string()
        } else {
            String::new()
        },
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

fn read_project_env_file(env_file_path: &Utf8Path) -> Result<Option<String>, ExecuteError> {
    match state::fs::read_to_string(env_file_path) {
        Ok(content) => Ok(Some(content)),
        Err(error) => {
            if let StateError::Filesystem { source, .. } = &error
                && source.kind() == io::ErrorKind::NotFound
            {
                return Ok(None);
            }

            Err(error.into())
        }
    }
}

fn write_project_env_warnings(
    warnings: &[ProjectEnvWarning],
    stderr: &mut impl Write,
) -> Result<(), ExecuteError> {
    let mut output = Output::new(stderr, OutputMode::plain());
    for warning in warnings {
        output.line(&format!("warning: {}", project_env_warning(warning)))?;
    }

    Ok(())
}

fn project_env_warning(warning: &ProjectEnvWarning) -> String {
    match warning {
        ProjectEnvWarning::DuplicateExistingKey { key } => {
            format!("generated Project env key `{key}` already exists outside the PV-managed block")
        }
    }
}

fn resolve_project(
    database: &Database,
    selector: Option<&str>,
    environment: &impl Environment,
) -> Result<ProjectRecord, ExecuteError> {
    if let Some(selector) = selector {
        return Ok(resolve_project_selector(database, selector)?.project);
    }

    let current_dir = current_dir(environment)?;
    database
        .nearest_project_for_path(&current_dir)?
        .ok_or_else(|| CliError::ProjectNotResolved.into())
}

struct ResolvedProjectSelector {
    project: ProjectRecord,
    matched_hostname: Option<String>,
}

fn resolve_project_selector(
    database: &Database,
    selector: &str,
) -> Result<ResolvedProjectSelector, ExecuteError> {
    if selector.contains('.') {
        let hostname = config::normalize_primary_hostname(selector)?;
        let project = database
            .project_by_hostname(&hostname)?
            .ok_or(CliError::ProjectNotResolved)?;

        return Ok(ResolvedProjectSelector {
            project,
            matched_hostname: Some(hostname),
        });
    }

    let slug_project = database.project_by_slug(selector)?;
    let hostname = config::normalize_primary_hostname(selector)?;
    let hostname_project = database.project_by_hostname(&hostname)?;

    if let (Some(slug_project), Some(hostname_project)) = (&slug_project, &hostname_project)
        && slug_project.id != hostname_project.id
    {
        return Err(CliError::AmbiguousProjectSelector {
            selector: selector.to_string(),
            hostname,
        }
        .into());
    }

    if let Some(project) = slug_project {
        return Ok(ResolvedProjectSelector {
            project,
            matched_hostname: None,
        });
    }
    if let Some(project) = hostname_project {
        return Ok(ResolvedProjectSelector {
            project,
            matched_hostname: Some(hostname),
        });
    }

    Err(CliError::ProjectNotResolved.into())
}

fn served_project_hostname(project: &ProjectRecord) -> Result<&str, ExecuteError> {
    if project.mode == ProjectMode::ResourceOnly {
        return Err(CliError::ResourceOnlyProjectCannotOpen {
            project: project.slug.clone(),
        }
        .into());
    }

    project
        .primary_hostname
        .as_deref()
        .ok_or_else(|| StateError::ProjectNotServed {
            project_id: project.id.clone(),
        })
        .map_err(Into::into)
}

fn project_display_name(project: &ProjectRecord) -> &str {
    if project.mode == ProjectMode::ResourceOnly {
        return project.slug.as_str();
    }

    project
        .primary_hostname
        .as_deref()
        .unwrap_or(project.slug.as_str())
}

fn resolve_project_path(
    path: Option<&str>,
    environment: &impl Environment,
) -> Result<Utf8PathBuf, ExecuteError> {
    let path = match path {
        Some(path) => {
            let path = Utf8Path::new(path);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                current_dir(environment)?.join(path)
            }
        }
        None => current_dir(environment)?,
    };

    Ok(path)
}

fn current_dir(environment: &impl Environment) -> Result<Utf8PathBuf, ExecuteError> {
    Utf8PathBuf::from_path_buf(environment.current_dir()?)
        .map_err(|path| CliError::NonUtf8Path { path }.into())
}

fn project_root_from_config_path(config_path: &Utf8Path) -> Result<Utf8PathBuf, ExecuteError> {
    config_path
        .parent()
        .map(Utf8Path::to_path_buf)
        .ok_or_else(|| CliError::ProjectNotResolved.into())
}

fn pv_paths(environment: &impl Environment) -> Result<PvPaths, ExecuteError> {
    let home = environment.home_dir().ok_or(StateError::MissingHome)?;
    let home = Utf8PathBuf::from_path_buf(home).map_err(|path| StateError::NonUtf8Home { path })?;

    Ok(PvPaths::for_home(home))
}

struct ProjectListStatus {
    project: ProjectStatus,
    env: ProjectEnvStatus,
    config_error: Option<String>,
    env_detail: Option<String>,
}

#[derive(Serialize)]
struct ProjectListOutput {
    projects: Vec<ProjectListItem>,
}

#[derive(Serialize)]
struct ProjectListItem {
    id: String,
    mode: &'static str,
    slug: String,
    hostname: Option<String>,
    additional_hostnames: Vec<String>,
    env_file: Option<String>,
    php: String,
    status: &'static str,
    env: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    config_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    env_detail: Option<String>,
    resources: Vec<ProjectListResource>,
    path: String,
    config_path: String,
}

#[derive(Serialize)]
struct ProjectListResource {
    resource: String,
    track: String,
}

enum ProjectStatus {
    ConfigInvalid,
    Unknown,
}

impl ProjectStatus {
    const fn as_str(&self) -> &'static str {
        match self {
            Self::ConfigInvalid => "config-invalid",
            Self::Unknown => "unknown",
        }
    }
}

enum ProjectEnvStatus {
    Failed,
    Invalid,
    None,
    Pending,
    Rendered,
    Warning,
}

impl ProjectEnvStatus {
    const fn as_str(&self) -> &'static str {
        match self {
            Self::Failed => "failed",
            Self::Invalid => "invalid",
            Self::None => "none",
            Self::Pending => "pending",
            Self::Rendered => "rendered",
            Self::Warning => "warning",
        }
    }
}

fn project_list_status(
    database: &Database,
    project: &ProjectRecord,
) -> Result<ProjectListStatus, ExecuteError> {
    let config_file = match ProjectConfigFile::read_from_root(&project.path) {
        Ok(config_file) => config_file,
        Err(error) => {
            return Ok(ProjectListStatus {
                project: ProjectStatus::ConfigInvalid,
                env: ProjectEnvStatus::Invalid,
                config_error: Some(error.to_string()),
                env_detail: None,
            });
        }
    };
    if project.mode == ProjectMode::Served
        && config_file.config.serve
        && let Some(primary_hostname) = project.primary_hostname.as_deref()
        && let Err(error) = database.validate_project_hostnames(
            &project.id,
            primary_hostname,
            &config_file.config.hostnames,
        )
    {
        return Ok(ProjectListStatus {
            project: ProjectStatus::ConfigInvalid,
            env: ProjectEnvStatus::Invalid,
            config_error: Some(error.to_string()),
            env_detail: None,
        });
    }
    if let Err(error) = config::validate_project_env_shape(&config_file.config) {
        return Ok(ProjectListStatus {
            project: ProjectStatus::ConfigInvalid,
            env: ProjectEnvStatus::Invalid,
            config_error: Some(error.to_string()),
            env_detail: None,
        });
    }

    let (env, env_detail) = project_list_env_status(
        config_file.config.has_env_mappings(),
        database.project_env_observed_state(&project.id)?,
    );

    Ok(ProjectListStatus {
        project: ProjectStatus::Unknown,
        env,
        config_error: None,
        env_detail,
    })
}

fn project_list_item(
    database: &Database,
    project: ProjectRecord,
) -> Result<ProjectListItem, ExecuteError> {
    let status = project_list_status(database, &project)?;
    let env_file = ProjectConfigFile::read_from_root(&project.path)
        .ok()
        .map(|config_file| config_file.config.env_file.to_string());
    let resources = database
        .project_managed_resources(&project.id)?
        .into_iter()
        .map(|resource| ProjectListResource {
            resource: resource.resource_name,
            track: resource.track,
        })
        .collect();

    Ok(ProjectListItem {
        id: project.id,
        mode: project.mode.as_str(),
        slug: project.slug,
        hostname: project.primary_hostname,
        additional_hostnames: project.additional_hostnames,
        env_file,
        php: project
            .desired_php_track
            .unwrap_or_else(|| "default".to_string()),
        status: status.project.as_str(),
        env: status.env.as_str(),
        config_error: status.config_error,
        env_detail: status.env_detail,
        resources,
        path: project.path.to_string(),
        config_path: project.config_path.to_string(),
    })
}

fn project_list_env_status(
    has_env_mappings: bool,
    observed: Option<ProjectEnvObservedStateRecord>,
) -> (ProjectEnvStatus, Option<String>) {
    let Some(observed) = observed else {
        if !has_env_mappings {
            return (ProjectEnvStatus::None, None);
        }

        return (ProjectEnvStatus::Pending, None);
    };

    if !has_env_mappings
        && (observed.status != ProjectEnvObservedStatus::Warning
            || !has_ignored_php_extension_warning(&observed))
    {
        return (ProjectEnvStatus::None, None);
    }

    match observed.status {
        ProjectEnvObservedStatus::Failed => (
            ProjectEnvStatus::Failed,
            observed.message.map(|message| format!("failed: {message}")),
        ),
        ProjectEnvObservedStatus::Pending => (ProjectEnvStatus::Pending, None),
        ProjectEnvObservedStatus::Rendered => (ProjectEnvStatus::Rendered, None),
        ProjectEnvObservedStatus::Warning => (
            ProjectEnvStatus::Warning,
            Some(project_env_observed_warning_summary(&observed)),
        ),
    }
}

fn project_env_observed_warning_summary(observed: &ProjectEnvObservedStateRecord) -> String {
    let ignored = observed
        .warnings
        .iter()
        .filter(|warning| warning.kind == "ignored_php_extension")
        .map(|warning| warning.message.as_str())
        .collect::<Vec<_>>();
    if !ignored.is_empty() {
        return format!("warning: {}", ignored.join("; "));
    }

    match observed.warnings.as_slice() {
        [warning] => format!("warning: {}", warning.message),
        [] => observed
            .message
            .as_ref()
            .map(|message| format!("warning: {message}"))
            .unwrap_or_else(|| "warning".to_string()),
        warnings => format!("warning: {} warnings", warnings.len()),
    }
}

fn has_ignored_php_extension_warning(observed: &ProjectEnvObservedStateRecord) -> bool {
    observed
        .warnings
        .iter()
        .any(|warning| warning.kind == "ignored_php_extension")
}
