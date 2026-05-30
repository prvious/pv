use std::io;
use std::io::Write;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use config::{
    AllocationEnvContext, ProjectConfigFile, ProjectEnvContext, ProjectEnvWarning,
    ResourceEnvContext,
};
use state::{
    Database, LinkProjectInput, LinkProjectStatus, ProjectEnvObservedStateRecord,
    ProjectEnvObservedStatus, ProjectEnvStateContext, ProjectRecord, PvPaths, StateError,
};

use crate::args::{LinkArgs, OpenArgs, ProjectEnvArgs, UnlinkArgs};
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
    let project_path = project_root_from_config_path(&config_file.path)?;
    let mut database = Database::open(&paths)?;
    let existing = database.project_by_path(&project_path)?;
    let primary_hostname = match (args.hostname, existing.as_ref()) {
        (Some(hostname), _) => config::normalize_primary_hostname(&hostname)?,
        (None, Some(project)) => project.primary_hostname.clone(),
        (None, None) => config::hostname_from_project_path(&project_path)?,
    };
    let result = database.link_project(LinkProjectInput {
        path: project_path.clone(),
        original_path: original_project_path,
        primary_hostname,
        config_path: config_file.path,
        desired_php_track: config_file.config.php,
        additional_hostnames: config_file.config.hostnames,
    })?;

    let mut output = Output::new(stdout, OutputMode::plain());
    match result.status {
        LinkProjectStatus::Created => output.line(&format!(
            "Linked {} -> {}",
            result.project.primary_hostname, result.project.path
        ))?,
        LinkProjectStatus::Updated => output.line(&format!(
            "Updated {} -> {}",
            result.project.primary_hostname, result.project.path
        ))?,
        LinkProjectStatus::Unchanged => output.line(&format!(
            "Already linked {} -> {}",
            result.project.primary_hostname, result.project.path
        ))?,
    }
    request_project_reconciliation(&paths, &result.project, &mut output)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn unlink(
    args: UnlinkArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let mut database = Database::open(&paths)?;
    let project = resolve_project(&database, args.hostname.as_deref(), environment)?;
    let project = database.unlink_project(&project.id)?;
    let mut output = Output::new(stdout, OutputMode::plain());

    output.line(&format!(
        "Unlinked {} -> {}",
        project.primary_hostname, project.path
    ))?;
    request_project_reconciliation(&paths, &project, &mut output)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn open(
    args: OpenArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let database = Database::open(&paths)?;
    let (project, hostname) = match args.hostname {
        Some(hostname) => {
            let hostname = config::normalize_primary_hostname(&hostname)?;
            let project = database
                .project_by_hostname(&hostname)?
                .ok_or(CliError::ProjectNotResolved)?;

            (project, hostname)
        }
        None => resolve_open_project(&database, environment, stdout)?,
    };
    let url = format!("https://{hostname}");

    environment.open_url(&url)?;

    let mut output = Output::new(stdout, OutputMode::plain());
    output.line(&format!("Opened {} for {}", url, project.primary_hostname))?;

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
    database.validate_project_hostnames(
        &project.id,
        &project.primary_hostname,
        &config_file.config.hostnames,
    )?;

    let context = project_env_context(database.project_env_context(&project.id)?);
    let rendered = config::render_project_env(&config_file.config, &context)?;
    let existing_env = read_project_env_file(&project.path)?;
    let transform = config::transform_managed_env_block(existing_env.as_deref(), &rendered)?;

    if args.json {
        serde_json::to_writer(&mut *stdout, &rendered.values)?;
        writeln!(stdout)?;
        write_project_env_warnings(&transform.warnings, stderr)?;

        return Ok(ExitCode::SUCCESS);
    }

    let content = config::format_project_env(&rendered);
    if content.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }

    write!(stdout, "{content}")?;
    write_project_env_warnings(&transform.warnings, stderr)?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn list(
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let database = Database::open(&paths)?;
    let projects = database.projects()?;
    let mut output = Output::new(stdout, OutputMode::plain());

    if projects.is_empty() {
        output.line("No linked Projects")?;
        return Ok(ExitCode::SUCCESS);
    }

    output.line("Hostname  PHP  Status  Resources  Env  Path")?;
    for project in projects {
        let status = project_list_status(&database, &project)?;
        output.line(&format!(
            "{}  {}  {}  unknown  {}  {}",
            project.primary_hostname,
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
            job.id, project.primary_hostname
        ))?,
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
        let hostname = project.primary_hostname.clone();
        return Ok((project, hostname));
    }

    if !environment.stdin_is_terminal() {
        return Err(CliError::ProjectNotResolved.into());
    }

    let project = select_project(database.projects()?, environment, stdout)?;
    let hostname = project.primary_hostname.clone();

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
            project.primary_hostname,
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

fn read_project_env_file(project_path: &Utf8Path) -> Result<Option<String>, ExecuteError> {
    let env_path = project_path.join(".env");
    match state::fs::read_to_string(&env_path) {
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
    hostname: Option<&str>,
    environment: &impl Environment,
) -> Result<ProjectRecord, ExecuteError> {
    if let Some(hostname) = hostname {
        let hostname = config::normalize_primary_hostname(hostname)?;
        return database
            .project_by_hostname(&hostname)?
            .ok_or_else(|| CliError::ProjectNotResolved.into());
    }

    let current_dir = current_dir(environment)?;
    database
        .nearest_project_for_path(&current_dir)?
        .ok_or_else(|| CliError::ProjectNotResolved.into())
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
    if let Err(error) = database.validate_project_hostnames(
        &project.id,
        &project.primary_hostname,
        &config_file.config.hostnames,
    ) {
        return Ok(ProjectListStatus {
            project: ProjectStatus::ConfigInvalid,
            env: ProjectEnvStatus::Invalid,
            config_error: Some(error.to_string()),
            env_detail: None,
        });
    }

    let has_env_mappings = !config_file.config.env.is_empty()
        || config_file.config.resources.values().any(|resource| {
            !resource.env.is_empty()
                || resource
                    .allocations
                    .values()
                    .any(|allocation| !allocation.env.is_empty())
        });
    let (env, env_detail) = project_list_env_status(
        has_env_mappings,
        database.project_env_observed_state(&project.id)?,
    );

    Ok(ProjectListStatus {
        project: ProjectStatus::Unknown,
        env,
        config_error: None,
        env_detail,
    })
}

fn project_list_env_status(
    has_env_mappings: bool,
    observed: Option<ProjectEnvObservedStateRecord>,
) -> (ProjectEnvStatus, Option<String>) {
    let Some(observed) = observed else {
        return if has_env_mappings {
            (ProjectEnvStatus::Pending, None)
        } else {
            (ProjectEnvStatus::None, None)
        };
    };

    match observed.status {
        ProjectEnvObservedStatus::Failed => (
            ProjectEnvStatus::Failed,
            observed.message.map(|message| format!("failed: {message}")),
        ),
        ProjectEnvObservedStatus::Pending => (ProjectEnvStatus::Pending, None),
        ProjectEnvObservedStatus::Rendered if has_env_mappings => {
            (ProjectEnvStatus::Rendered, None)
        }
        ProjectEnvObservedStatus::Rendered => (ProjectEnvStatus::None, None),
        ProjectEnvObservedStatus::Warning => (
            ProjectEnvStatus::Warning,
            Some(project_env_observed_warning_summary(&observed)),
        ),
    }
}

fn project_env_observed_warning_summary(observed: &ProjectEnvObservedStateRecord) -> String {
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
