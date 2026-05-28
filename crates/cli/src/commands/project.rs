use std::io;
use std::io::Write;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use config::ProjectConfigFile;
use state::{Database, LinkProjectInput, LinkProjectStatus, ProjectRecord, PvPaths};

use crate::args::{LinkArgs, OpenArgs, UnlinkArgs};
use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Output, OutputMode};

pub(crate) fn link(
    args: LinkArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let paths = PvPaths::default_home()?;
    let project_path = resolve_project_path(args.path.as_deref(), environment)?;
    let config_file = ProjectConfigFile::read_from_root(&project_path)?;
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
    let paths = PvPaths::default_home()?;
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
    let paths = PvPaths::default_home()?;
    let database = Database::open(&paths)?;
    let (project, hostname) = match args.hostname {
        Some(hostname) => {
            let hostname = config::normalize_primary_hostname(&hostname)?;
            let project = database
                .project_by_hostname(&hostname)?
                .ok_or(CliError::ProjectNotResolved)?;

            (project, hostname)
        }
        None => {
            let current_dir = current_dir(environment)?;
            let project = database
                .nearest_project_for_path(&current_dir)?
                .ok_or(CliError::ProjectNotResolved)?;
            let hostname = project.primary_hostname.clone();

            (project, hostname)
        }
    };
    let url = format!("https://{hostname}");

    environment.open_url(&url)?;

    let mut output = Output::new(stdout, OutputMode::plain());
    output.line(&format!("Opened {} for {}", url, project.primary_hostname))?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn list(stdout: &mut impl Write) -> Result<ExitCode, ExecuteError> {
    let paths = PvPaths::default_home()?;
    let database = Database::open(&paths)?;
    let projects = database.projects()?;
    let mut output = Output::new(stdout, OutputMode::plain());

    if projects.is_empty() {
        output.line("No linked Projects")?;
        return Ok(ExitCode::SUCCESS);
    }

    output.line("Hostname  PHP  Status  Resources  Env  Path")?;
    for project in projects {
        output.line(&format!(
            "{}  {}  pending  pending  {}  {}",
            project.primary_hostname,
            project.desired_php_track.as_deref().unwrap_or("default"),
            if project_has_env_config(&project) {
                "configured"
            } else {
                "none"
            },
            project.path
        ))?;
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
            "warning: PV daemon is not running; run `pv setup` before this Project is reachable",
        )?,
        Err(error) => return Err(error.into()),
    }

    Ok(())
}

fn daemon_is_unavailable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
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

fn project_has_env_config(project: &ProjectRecord) -> bool {
    let Ok(config_file) = ProjectConfigFile::read_from_root(&project.path) else {
        return false;
    };

    !config_file.config.env.is_empty()
        || config_file.config.resources.values().any(|resource| {
            !resource.env.is_empty()
                || resource
                    .allocations
                    .values()
                    .any(|allocation| !allocation.env.is_empty())
        })
}
