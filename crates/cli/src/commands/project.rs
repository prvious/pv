use std::io;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use config::{
    AllocationEnvContext, ProjectConfigFile, ProjectEnvContext, ProjectHooks, ResourceEnvContext,
};
use resources::{ArtifactManifestCache, ConcreteTrackName, ResourceName, TrackSelector};
use serde::Serialize;
use state::{
    Database, LinkProjectInput, LinkProjectStatus, ProjectEnvObservedStateRecord,
    ProjectEnvObservedStatus, ProjectEnvStateContext, ProjectMode, ProjectRecord, PvPaths,
    RuntimeLifecycleLock, StateError,
};

use super::project_hooks::{self, LinkHookRun};
use crate::args::{LinkArgs, ListArgs, OpenArgs, ProjectEnvArgs, UnlinkArgs};
use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Line, Mark, Streams, Table, Tone};
use crate::prompt::{self, Choice};

pub(crate) fn link(
    args: LinkArgs,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
    mut runtime_lifecycle_lock: Option<RuntimeLifecycleLock>,
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
    let mut hooks_enabled = !args.skip_hooks && !config_file.config.hooks.is_empty();
    if hooks_enabled && let Some(project) = &existing {
        refuse_concurrent_hooks(&database, project)?;
    }
    if hooks_enabled {
        hooks_enabled = approve_hooks(
            &mut database,
            &config_file.config.hooks,
            &project_path,
            environment,
            streams,
        )?;
    }
    let source_path = match &existing {
        Some(project) => discovered_source_path(&database, project)?,
        None => None,
    };
    let mode = if config_file.config.serve {
        ProjectMode::Served
    } else {
        ProjectMode::ResourceOnly
    };
    let explicit_hostname = config_file.config.serve.then_some(args.hostname).flatten();
    let mut primary_hostname = match (explicit_hostname.as_deref(), existing.as_ref()) {
        (Some(hostname), _) => config::normalize_primary_hostname(hostname)?,
        (None, Some(project)) => linked_hostname(project),
        (None, None) => config::hostname_from_project_path(&project_path)?,
    };
    drop(database);
    let mut hook_run = LinkHookRun::new(&paths, &project_path, source_path)?;
    let mut hooks_attempted = false;
    if hooks_enabled && !config_file.config.hooks.prepare.is_empty() {
        hooks_attempted = true;
        let result = project_hooks::run_with_admission(
            &mut hook_run,
            "prepare",
            &config_file.config.hooks.prepare,
            streams,
            &mut runtime_lifecycle_lock,
        )?;
        if let Err(error) = result {
            if let Some(project) = &existing {
                project_hooks::finish_run(&paths, project, Err(error), false)?;
                return Ok(ExitCode::FAILURE);
            }
            return Err(error);
        }
    }
    let mut database = Database::open(&paths)?;
    // Discovery may have linked this worktree while prepare ran. Keep its name, and don't run
    // terminal hooks alongside the daemon's.
    if existing.is_none()
        && let Some(project) = database.project_by_path(&project_path)?
    {
        if hooks_enabled {
            refuse_concurrent_hooks(&database, &project)?;
        }
        if explicit_hostname.is_none() {
            primary_hostname = linked_hostname(&project);
        }
        hook_run.set_source_path(discovered_source_path(&database, &project)?);
    }
    let result = database.link_project_with_mode(
        LinkProjectInput {
            path: project_path.clone(),
            original_path: original_project_path,
            primary_hostname,
            config_path: config_file.path,
            desired_php_track,
        },
        mode,
    )?;
    drop(database);

    let output = &mut streams.out;
    let project_name = if result.project.mode == ProjectMode::ResourceOnly {
        format!("{} (resource-only)", result.project.slug)
    } else if let Some(hostname) = result.project.primary_hostname.as_deref() {
        format!("https://{hostname}")
    } else {
        super::project_display_name(&result.project).to_string()
    };
    let (mark, verb) = match result.status {
        LinkProjectStatus::Created => (Mark::Success, "Linked"),
        LinkProjectStatus::Updated => (Mark::Success, "Updated"),
        LinkProjectStatus::Unchanged => (Mark::Idle, "Already linked"),
    };
    if output.surface().decorated() {
        if let Some(hostname) = result.project.primary_hostname.as_deref() {
            let verb = if result.status == LinkProjectStatus::Updated {
                "Link updated"
            } else {
                verb
            };
            output.status(
                mark,
                Line::field(&format!("{verb} "), format!("https://{hostname}")),
            )?;
        } else {
            output.status(
                mark,
                format!("{verb} {} · resources only", result.project.slug),
            )?;
        }
        output.detail(Line::default().value(result.project.path.as_str()))?;
    } else {
        output.status(
            mark,
            Line::field(&format!("{verb} {project_name} -> "), &result.project.path),
        )?;
    }
    let mut setup_skipped = false;
    let submitted = hook_run.request_reconciliation(&result.project, streams, hooks_enabled);
    let outcome = match submitted {
        Err(error) => {
            hooks_attempted |= hooks_enabled && !config_file.config.hooks.setup.is_empty();
            Err(error)
        }
        Ok(job) if hooks_enabled && !config_file.config.hooks.setup.is_empty() => {
            if let Some(job) = job {
                hooks_attempted = true;
                match hook_run.wait_for_reconciliation(&job.id, streams) {
                    Err(error) => Err(error),
                    Ok(()) => project_hooks::run_with_admission(
                        &mut hook_run,
                        "setup",
                        &config_file.config.hooks.setup,
                        streams,
                        &mut runtime_lifecycle_lock,
                    )?,
                }
            } else {
                setup_skipped = true;
                streams
                    .err
                    .warning("Setup hooks skipped; run `pv link` again after `pv setup`")?;
                Ok(())
            }
        }
        Ok(_) => Ok(()),
    };
    if hooks_attempted {
        project_hooks::finish_run(&paths, &result.project, outcome, !setup_skipped)?;
    } else {
        outcome?;
    }

    Ok(ExitCode::SUCCESS)
}

/// Hooks run only when every command is approved for the Project's folder. Approving needs a
/// terminal; without one, or when the user declines, the Project is linked without hooks.
fn approve_hooks(
    database: &mut Database,
    hooks: &ProjectHooks,
    project_path: &Utf8Path,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<bool, ExecuteError> {
    let folder = daemon::hook_approval_folder(project_path);
    let commands = hooks
        .prepare
        .iter()
        .chain(&hooks.setup)
        .cloned()
        .collect::<Vec<_>>();
    let unapproved = database.unapproved_hook_commands(&folder, &commands)?;
    if unapproved.is_empty() {
        return Ok(true);
    }
    // The prompt draws on stderr, so the commands it asks about go there too.
    let output = &mut streams.err;
    output.status(
        Mark::Warning,
        Line::field("Hook commands not approved for ", visible(folder.as_str())),
    )?;
    for command in &unapproved {
        let phase = if hooks.prepare.contains(command) {
            "prepare"
        } else {
            "setup"
        };
        output.detail(
            Line::default()
                .text(format!("{phase:<8} "))
                .value(visible(command)),
        )?;
    }
    if streams.interactive
        && prompt::confirm(
            environment,
            &streams.out,
            "Run them and remember the approval?",
            false,
        )?
    {
        database.approve_hook_commands(&folder, &unapproved)?;
        return Ok(true);
    }
    streams.err.warning(
        "Hooks skipped because they aren't approved; run `pv link` here in a terminal to review them",
    )?;

    Ok(false)
}

/// Shows branch-controlled text so it can't hide part of itself: control characters and Unicode
/// direction marks appear as escapes such as `\u{1b}`. Backslashes appear doubled, so a literal
/// `\u{a}` never looks like a real newline. The exact text is still what's approved.
fn visible(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character == '\\' {
                "\\\\".to_owned()
            } else if character.is_control()
                || matches!(
                    character,
                    '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
                )
            {
                character.escape_unicode().to_string()
            } else {
                character.to_string()
            }
        })
        .collect()
}

/// Terminal hooks wait while the daemon owns this Project's hooks.
fn refuse_concurrent_hooks(
    database: &Database,
    project: &ProjectRecord,
) -> Result<(), ExecuteError> {
    if database.held_project_ids()?.contains(&project.id) {
        return Err(CliError::ProjectFirstReconciliationPending.into());
    }
    if let Some(job) = database.active_hook_job(&format!("project:{}", project.id))? {
        return Err(CliError::ProjectHooksInProgress { job_id: job.id }.into());
    }
    Ok(())
}

/// Hooks in a discovered worktree get its source Project's path as `PV_SOURCE_PATH`.
fn discovered_source_path(
    database: &Database,
    project: &ProjectRecord,
) -> Result<Option<Utf8PathBuf>, ExecuteError> {
    let Some(discovered) = database
        .discovered_projects()?
        .into_iter()
        .find(|discovered| discovered.project_id == project.id)
    else {
        return Ok(None);
    };
    Ok(database
        .project_by_id(&discovered.source_project_id)?
        .map(|source| source.path))
}

fn linked_hostname(project: &ProjectRecord) -> String {
    project
        .primary_hostname
        .clone()
        .unwrap_or_else(|| format!("{}.test", project.slug))
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
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let mut database = Database::open(&paths)?;
    let project = resolve_project(&database, args.hostname.as_deref(), environment)?;
    // Unlinking a source also unlinks the worktree Projects PV discovered for it.
    let project_id = project.id.clone();
    let mut unlinked = vec![project];
    for discovered in database.discovered_projects()? {
        if discovered.source_project_id == project_id
            && let Some(project) = database.project_by_id(&discovered.project_id)?
        {
            unlinked.push(project);
        }
    }
    for project in &unlinked {
        delete_optional_project_tls_dir(&paths, project)?;
    }
    database.unlink_project(&project_id)?;
    for project in &unlinked {
        if streams.out.surface().decorated() {
            streams.out.success(Line::field(
                "Unlinked ",
                super::project_display_name(project),
            ))?;
            streams
                .out
                .detail(Line::default().value(project.path.as_str()))?;
        } else {
            streams.out.success(Line::field(
                &format!("Unlinked {} -> ", super::project_display_name(project)),
                &project.path,
            ))?;
        }
    }
    super::request_system_reconciliation(&paths, streams)?;

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
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let database = Database::open(&paths)?;
    let (project, hostname) = match args.hostname {
        Some(selector) => {
            let project = resolve_project_selector(&database, &selector)?;
            let hostname = served_project_hostname(&project)?.to_string();
            (project, hostname)
        }
        None => resolve_open_project(&database, environment, streams)?,
    };
    let url = format!("https://{hostname}");

    environment.open_url(&url)?;

    let displayed_hostname = if streams.out.surface().decorated() {
        hostname.as_str()
    } else {
        super::project_display_name(&project)
    };
    streams.out.success(
        Line::from("Opened ")
            .value(url)
            .text(format!(" for {displayed_hostname}")),
    )?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn env(
    args: ProjectEnvArgs,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let database = Database::open(&paths)?;
    let project = resolve_project(&database, args.hostname.as_deref(), environment)?;
    let config_file = ProjectConfigFile::read_from_root(&project.path)?;
    let serves_http = project.mode == ProjectMode::Served && config_file.config.serve;
    if serves_http {
        database
            .validate_project_primary_hostname(&project.id, served_project_hostname(&project)?)?;
    }
    config::validate_project_env_shape(&config_file.config)?;

    let context = project_env_context(
        &paths,
        database.project_env_context(&project.id)?,
        serves_http,
    );
    let rendered = config::render_project_env(&config_file.config, &context)?;
    if config_file.config.has_env_mappings() {
        let env_file_path =
            config::resolve_project_env_file_path(&project.path, &config_file.config)?;
        let existing_env = read_project_env_file(&env_file_path)?;
        config::validate_managed_env_block(existing_env.as_deref())?;
    }

    if args.json {
        streams.out.json(&rendered.values)?;

        return Ok(ExitCode::SUCCESS);
    }

    let content = config::format_project_env(&rendered);
    if content.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }

    write!(streams.out.writer(), "{content}")?;

    Ok(ExitCode::SUCCESS)
}

pub(crate) fn list(
    args: ListArgs,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let paths = pv_paths(environment)?;
    let database = Database::open(&paths)?;
    let mut projects = database.projects()?;
    projects.sort_by(|left, right| {
        super::project_display_name(left)
            .cmp(super::project_display_name(right))
            .then_with(|| left.id.cmp(&right.id))
    });

    if args.json {
        let projects = projects
            .into_iter()
            .map(|project| project_list_item(&database, project))
            .collect::<Result<Vec<_>, _>>()?;
        streams.out.json(&ProjectListOutput { projects })?;

        return Ok(ExitCode::SUCCESS);
    }

    if projects.is_empty() {
        streams.out.note("No linked Projects")?;
        if streams.out.surface().decorated() {
            streams
                .out
                .hint("In your Project directory, run", "pv link")?;
        }
        return Ok(ExitCode::SUCCESS);
    }

    streams.out.report_heading("list", None)?;
    if streams.out.surface().decorated() {
        streams.out.line("")?;
        const HEADER: &str = "PROJECT  MODE  PHP  STATUS  RESOURCES  ENV  PATH";
        streams
            .out
            .line(if streams.out.surface().width() >= HEADER.len() {
                HEADER
            } else {
                "PROJECTS"
            })?;
        let mut warnings = 0;
        let mut failures = 0;
        for project in &projects {
            let status = project_list_status(&database, project)?;
            let resources = status
                .declared_resources
                .map_or_else(|| "unknown".to_string(), |count| count.to_string());
            let resource_label = if status.declared_resources == Some(1) {
                "resource"
            } else {
                "resources"
            };
            let failed = matches!(status.project, ProjectStatus::ConfigInvalid)
                || matches!(
                    status.env,
                    ProjectEnvStatus::Failed | ProjectEnvStatus::Invalid
                );
            let warning = matches!(status.env, ProjectEnvStatus::Warning);
            let displayed_status = if matches!(status.env, ProjectEnvStatus::Failed) {
                "failed"
            } else {
                status.project.as_str()
            };
            failures += usize::from(failed);
            warnings += usize::from(warning);
            streams.out.line("")?;
            streams.out.status(
                if failed {
                    Mark::Failure
                } else if warning {
                    Mark::Warning
                } else {
                    Mark::Idle
                },
                format!(
                    "{} · {}",
                    super::project_display_name(project),
                    displayed_status
                ),
            )?;
            streams.out.detail(format!(
                "{} · PHP {} · {resources} {resource_label} · env {}",
                project.mode.as_str(),
                project.desired_php_track.as_deref().unwrap_or("default"),
                status.env.as_str(),
            ))?;
            streams
                .out
                .detail(Line::default().value(project.path.as_str()))?;
            if let Some(error) = status.config_error {
                streams
                    .out
                    .detail(Line::default().toned(Tone::Failure, format!("✗ {error}")))?;
            }
            if let Some(detail) = status.env_detail {
                streams.out.detail(Line::default().toned(
                    if failed { Tone::Failure } else { Tone::Warning },
                    format!(
                        "{} {}",
                        if failed { "✗" } else { "⚠" },
                        detail.strip_prefix("warning: ").unwrap_or(&detail),
                    ),
                ))?;
            }
        }
        streams.out.line("")?;
        streams.out.status(
            if failures > 0 {
                Mark::Failure
            } else if warnings > 0 {
                Mark::Warning
            } else {
                Mark::Running
            },
            format!(
                "{} linked · {warnings} warning{} · {failures} failed",
                projects.len(),
                if warnings == 1 { "" } else { "s" }
            ),
        )?;
        if let Some(default_php) = database.global_php_default_track()? {
            streams
                .out
                .detail(Line::field("default php ", default_php))?;
        }
        return Ok(ExitCode::SUCCESS);
    }

    let mut table = Table::new(&[
        "Project",
        "Mode",
        "PHP",
        "Status",
        "Resources",
        "Env",
        "Path",
    ]);
    for project in projects {
        let status = project_list_status(&database, &project)?;
        let resources = status
            .declared_resources
            .map_or_else(|| "unknown".to_string(), |count| count.to_string());
        table.row(vec![
            Line::from(super::project_display_name(&project)),
            Line::from(project.mode.as_str()),
            Line::from(project.desired_php_track.as_deref().unwrap_or("default")),
            status.project.cell(),
            Line::from(resources),
            status.env.cell(),
            Line::default().value(project.path.to_string()),
        ]);
        if let Some(error) = status.config_error {
            table.detail(format!("config: {error}"));
        }
        if let Some(detail) = status.env_detail {
            table.detail(format!("env: {detail}"));
        }
    }
    streams.out.table(&table)?;
    if let Some(default_php) = database.global_php_default_track()? {
        streams
            .out
            .detail(Line::field("default php ", default_php))?;
    }

    Ok(ExitCode::SUCCESS)
}

fn resolve_open_project(
    database: &Database,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<(ProjectRecord, String), ExecuteError> {
    let current_dir = current_dir(environment)?;
    if let Some(project) = database.nearest_project_for_path(&current_dir)? {
        let hostname = served_project_hostname(&project)?.to_string();
        return Ok((project, hostname));
    }

    if !streams.interactive {
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
    let project = select_project(projects, environment, streams)?;
    let hostname = served_project_hostname(&project)?.to_string();

    Ok((project, hostname))
}

fn select_project(
    mut projects: Vec<ProjectRecord>,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ProjectRecord, ExecuteError> {
    if projects.is_empty() {
        return Err(CliError::NoServedProjects.into());
    }
    let choices = projects
        .iter()
        .map(|project| Choice::new(super::project_display_name(project), project.path.as_str()))
        .collect::<Vec<_>>();
    let index = prompt::select(environment, &streams.out, "Select a Project", &choices, 0)?;

    Ok(projects.swap_remove(index))
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

fn resolve_project(
    database: &Database,
    selector: Option<&str>,
    environment: &impl Environment,
) -> Result<ProjectRecord, ExecuteError> {
    if let Some(selector) = selector {
        return resolve_project_selector(database, selector);
    }

    let current_dir = current_dir(environment)?;
    database
        .nearest_project_for_path(&current_dir)?
        .ok_or_else(|| CliError::ProjectNotResolved.into())
}

fn resolve_project_selector(
    database: &Database,
    selector: &str,
) -> Result<ProjectRecord, ExecuteError> {
    if selector.contains('.') {
        let hostname = config::normalize_primary_hostname(selector)?;

        return database.project_by_hostname(&hostname)?.ok_or_else(|| {
            CliError::ProjectSelectorNotFound {
                selector: selector.to_string(),
            }
            .into()
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

    if let Some(project) = slug_project.or(hostname_project) {
        return Ok(project);
    }

    Err(CliError::ProjectSelectorNotFound {
        selector: selector.to_string(),
    }
    .into())
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
    declared_resources: Option<usize>,
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
    Missing,
    Unknown,
}

impl ProjectStatus {
    const fn as_str(&self) -> &'static str {
        match self {
            Self::ConfigInvalid => "config-invalid",
            Self::Missing => "missing",
            Self::Unknown => "unknown",
        }
    }

    fn cell(&self) -> Line {
        let mark = match self {
            Self::ConfigInvalid | Self::Missing => Mark::Failure,
            Self::Unknown => Mark::Idle,
        };

        Line::marked(mark, self.as_str())
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

    fn cell(&self) -> Line {
        let tone = match self {
            Self::Failed | Self::Invalid => Tone::Failure,
            Self::Warning => Tone::Warning,
            Self::Rendered => Tone::Success,
            Self::None | Self::Pending => Tone::Dim,
        };

        Line::default().toned(tone, self.as_str())
    }
}

fn project_list_status(
    database: &Database,
    project: &ProjectRecord,
) -> Result<ProjectListStatus, ExecuteError> {
    // An unreadable path falls through to the config read, which reports it on this row alone.
    if matches!(state::fs::path_entry_exists(&project.path), Ok(false)) {
        return Ok(ProjectListStatus {
            project: ProjectStatus::Missing,
            env: ProjectEnvStatus::Invalid,
            declared_resources: None,
            config_error: None,
            env_detail: None,
        });
    }
    let config_file = match ProjectConfigFile::read_from_root(&project.path) {
        Ok(config_file) => config_file,
        Err(error) => {
            return Ok(ProjectListStatus {
                project: ProjectStatus::ConfigInvalid,
                env: ProjectEnvStatus::Invalid,
                declared_resources: None,
                config_error: Some(error.to_string()),
                env_detail: None,
            });
        }
    };
    let declared_resources = Some(config_file.config.resources.len());
    if project.mode == ProjectMode::Served
        && config_file.config.serve
        && let Some(primary_hostname) = project.primary_hostname.as_deref()
        && let Err(error) =
            database.validate_project_primary_hostname(&project.id, primary_hostname)
    {
        return Ok(ProjectListStatus {
            project: ProjectStatus::ConfigInvalid,
            env: ProjectEnvStatus::Invalid,
            declared_resources,
            config_error: Some(error.to_string()),
            env_detail: None,
        });
    }
    if let Err(error) = config::validate_project_env_shape(&config_file.config) {
        return Ok(ProjectListStatus {
            project: ProjectStatus::ConfigInvalid,
            env: ProjectEnvStatus::Invalid,
            declared_resources,
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
        declared_resources,
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
