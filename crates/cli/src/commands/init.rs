use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use config::{
    ProjectInitDetection, ProjectInitResourceName, ProjectInitSelection,
    default_project_init_selection, detect_project_init, render_project_init_config,
    write_project_config,
};

use crate::args::InitArgs;
use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Line, Mark, Output, Streams};
use crate::prompt::{self, Choice};

const VITE_NOTE: &str = "Vite HTTPS: configure the app's Vite config to read VITE_DEV_SERVER_CERT and VITE_DEV_SERVER_KEY.";

pub(crate) fn run(
    args: InitArgs,
    environment: &impl Environment,
    streams: &mut Streams<'_>,
) -> Result<ExitCode, ExecuteError> {
    let project_root = resolve_project_path(args.path.as_deref(), environment)?;
    let detection = detect_project_init(&project_root)?;
    let selection = default_project_init_selection(&detection);
    let config = render_project_init_config(&detection, &selection)?;
    let content =
        yaml_serde::to_string(&config).map_err(|source| config::ConfigError::Parse { source })?;

    if args.print {
        write!(streams.out.writer(), "{content}")?;
        return Ok(ExitCode::SUCCESS);
    }

    if args.yes {
        let output = &mut streams.out;
        output.flow_start("init", "PV init", project_root.file_name())?;
        write_detection_summary(output, &detection, &selection)?;
        let written = write_project_config(&project_root, &config)?;
        finish_written(output, &written.path, selection.include_vite_tls)?;
        return Ok(ExitCode::SUCCESS);
    }

    if !streams.interactive {
        return Err(CliError::InitRequiresTerminal.into());
    }

    run_interactive(
        project_root,
        detection,
        selection,
        content,
        environment,
        &mut streams.out,
    )
}

fn run_interactive(
    project_root: Utf8PathBuf,
    detection: ProjectInitDetection,
    selection: ProjectInitSelection,
    content: String,
    environment: &impl Environment,
    output: &mut Output<'_>,
) -> Result<ExitCode, ExecuteError> {
    output.flow_start("init", "PV init", project_root.file_name())?;
    write_detection_summary(output, &detection, &selection)?;
    write_resource_checklist(output, &selection)?;

    let choices = [
        Choice::new("Yes", "preview the config"),
        Choice::new("No", "cancel"),
        Choice::new("Edit", "change selections"),
    ];
    match prompt::select(environment, output, "Use these selections?", &choices, 0)? {
        0 => preview_and_confirm_write(
            &project_root,
            &content,
            selection.include_vite_tls,
            environment,
            output,
        ),
        1 => cancelled(output),
        _ => run_structured_edit(&project_root, detection, selection, environment, output),
    }
}

fn preview_and_confirm_write(
    project_root: &Utf8Path,
    content: &str,
    include_vite_tls: bool,
    environment: &impl Environment,
    output: &mut Output<'_>,
) -> Result<ExitCode, ExecuteError> {
    output.flow_step(Mark::Done, "Project config preview:")?;
    for line in content.lines() {
        output.quote(line)?;
    }
    if !prompt::confirm(environment, output, "Write Project config?", true)? {
        return cancelled(output);
    }

    let config = config::ProjectConfig::parse(content)?;
    let written = write_project_config(project_root, &config)?;
    finish_written(output, &written.path, include_vite_tls)?;

    Ok(ExitCode::SUCCESS)
}

fn finish_written(
    output: &mut Output<'_>,
    path: &Utf8Path,
    include_vite_tls: bool,
) -> Result<(), ExecuteError> {
    output.flow_end(
        Mark::Done,
        Line::from("Wrote Project config: ").value(path.as_str()),
    )?;
    if include_vite_tls {
        output.status(Mark::Warning, VITE_NOTE)?;
    }

    Ok(())
}

fn cancelled(output: &mut Output<'_>) -> Result<ExitCode, ExecuteError> {
    output.flow_end(Mark::Failure, "pv init cancelled; no files changed.")?;
    Ok(ExitCode::FAILURE)
}

fn write_resource_checklist(
    output: &mut Output<'_>,
    selection: &ProjectInitSelection,
) -> Result<(), ExecuteError> {
    output.flow_step(Mark::Done, "Resource checklist:")?;
    for name in available_resources(selection) {
        let selected = selection
            .resources
            .get(&name)
            .is_some_and(|resource| resource.selected);
        let marker = if selected { "[x]" } else { "[ ]" };
        output.detail(format!("{marker} {}", resource_label(name)))?;
    }

    Ok(())
}

fn run_structured_edit(
    project_root: &Utf8Path,
    mut detection: ProjectInitDetection,
    mut selection: ProjectInitSelection,
    environment: &impl Environment,
    output: &mut Output<'_>,
) -> Result<ExitCode, ExecuteError> {
    selection.php = prompt::text(environment, output, "PHP track", &selection.php, None)?;

    let document_root = selection
        .document_root
        .as_ref()
        .map_or(".", |path| path.as_str())
        .to_string();
    let answer = prompt::text(environment, output, "Document root", &document_root, None)?;
    if answer != document_root {
        selection.document_root = Some(Utf8PathBuf::from(answer));
    }

    select_resources(&mut selection, environment, output)?;

    let mut explicitly_edited_allocation_resources = Vec::new();
    for name in resource_names() {
        if prompt_resource_details(name, &mut selection, environment, output)? {
            explicitly_edited_allocation_resources.push(name);
        }
    }
    prune_explicitly_edited_allocations(
        &mut detection,
        &selection,
        &explicitly_edited_allocation_resources,
    );

    let config = render_project_init_config(&detection, &selection)?;
    let content =
        yaml_serde::to_string(&config).map_err(|source| config::ConfigError::Parse { source })?;
    preview_and_confirm_write(
        project_root,
        &content,
        selection.include_vite_tls,
        environment,
        output,
    )
}

fn select_resources(
    selection: &mut ProjectInitSelection,
    environment: &impl Environment,
    output: &Output<'_>,
) -> Result<(), ExecuteError> {
    let available = available_resources(selection);
    let choices = available
        .iter()
        .map(|name| Choice::new(resource_label(*name), ""))
        .collect::<Vec<_>>();
    let selected = available
        .iter()
        .enumerate()
        .filter(|(_index, name)| {
            selection
                .resources
                .get(name)
                .is_some_and(|resource| resource.selected)
        })
        .map(|(index, _name)| index)
        .collect::<Vec<_>>();
    let chosen = prompt::multiselect(
        environment,
        output,
        "Select Project resources",
        &choices,
        &selected,
    )?;
    for (index, name) in available.iter().enumerate() {
        if let Some(resource) = selection.resources.get_mut(name) {
            resource.selected = chosen.contains(&index);
        }
    }

    Ok(())
}

/// Asks for a selected resource's track and allocations. Returns whether the
/// allocations were explicitly changed.
fn prompt_resource_details(
    name: ProjectInitResourceName,
    selection: &mut ProjectInitSelection,
    environment: &impl Environment,
    output: &Output<'_>,
) -> Result<bool, ExecuteError> {
    let Some(resource) = selection.resources.get_mut(&name) else {
        return Ok(false);
    };
    if !resource.selected {
        return Ok(false);
    }

    let label = resource_label(name);
    resource.track = prompt::text(
        environment,
        output,
        &format!("{label} track"),
        &resource.track,
        None,
    )?;

    if name == ProjectInitResourceName::Mailpit {
        return Ok(false);
    }
    let answer = prompt::text(
        environment,
        output,
        &format!("{label} allocations"),
        &resource.allocations.join(","),
        Some(validate_allocations),
    )?;
    let allocations = parse_csv(&answer);
    if allocations == resource.allocations {
        return Ok(false);
    }
    resource.allocations = allocations;

    Ok(true)
}

fn validate_allocations(value: &str) -> Result<(), String> {
    parse_csv(value)
        .iter()
        .try_for_each(|allocation| config::validate_allocation_name(allocation))
        .map_err(|error| {
            format!("{error}; start with a lowercase letter and use only a-z, 0-9, _, or -")
        })
}

fn prune_explicitly_edited_allocations(
    detection: &mut ProjectInitDetection,
    selection: &ProjectInitSelection,
    resources: &[ProjectInitResourceName],
) {
    for name in resources {
        let Some(selected_allocations) = selection
            .resources
            .get(name)
            .map(|resource| &resource.allocations)
        else {
            continue;
        };
        if let Some(existing_resource) = detection
            .config_file
            .config
            .resources
            .get_mut(resource_name(*name))
        {
            existing_resource
                .allocations
                .retain(|allocation, _config| selected_allocations.contains(allocation));
        }
    }
}

fn parse_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn resource_names() -> [ProjectInitResourceName; 5] {
    [
        ProjectInitResourceName::Mysql,
        ProjectInitResourceName::Postgres,
        ProjectInitResourceName::Redis,
        ProjectInitResourceName::Mailpit,
        ProjectInitResourceName::Rustfs,
    ]
}

fn available_resources(selection: &ProjectInitSelection) -> Vec<ProjectInitResourceName> {
    resource_names()
        .into_iter()
        .filter(|name| selection.resources.contains_key(name))
        .collect()
}

fn resource_label(name: ProjectInitResourceName) -> &'static str {
    match name {
        ProjectInitResourceName::Mailpit => "Mailpit",
        ProjectInitResourceName::Mysql => "MySQL",
        ProjectInitResourceName::Postgres => "Postgres",
        ProjectInitResourceName::Redis => "Redis",
        ProjectInitResourceName::Rustfs => "RustFS/S3",
    }
}

fn write_detection_summary(
    output: &mut Output<'_>,
    detection: &ProjectInitDetection,
    selection: &ProjectInitSelection,
) -> Result<(), ExecuteError> {
    if detection.signals.is_empty() {
        output.flow_step(
            Mark::Done,
            "No framework-specific Project signals detected.",
        )?;
    } else {
        output.flow_step(Mark::Done, "Detected Project signals:")?;
        for signal in &detection.signals {
            output.detail(format!("{}: {}", signal.label, signal.detail))?;
        }
    }

    let selected_resources = detection
        .resources
        .iter()
        .filter(|(name, resource)| {
            resource.selected
                && selection
                    .resources
                    .get(name)
                    .is_some_and(|selection| selection.selected)
        })
        .collect::<Vec<_>>();
    if !selected_resources.is_empty() {
        output.flow_step(Mark::Done, "Selected Project resources:")?;
        for (name, resource) in selected_resources {
            output.detail(format!("{}: {}", resource_name(*name), resource.reason))?;
        }
    }

    Ok(())
}

fn resource_name(name: ProjectInitResourceName) -> &'static str {
    match name {
        ProjectInitResourceName::Mailpit => "mailpit",
        ProjectInitResourceName::Mysql => "mysql",
        ProjectInitResourceName::Postgres => "postgres",
        ProjectInitResourceName::Redis => "redis",
        ProjectInitResourceName::Rustfs => "rustfs",
    }
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
