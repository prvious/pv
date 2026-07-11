use std::io::Write;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use config::{
    default_project_init_selection, detect_project_init, render_project_init_config,
    write_project_config,
};

use crate::args::InitArgs;
use crate::environment::Environment;
use crate::error::{CliError, ExecuteError};
use crate::output::{Output, OutputMode};

pub(crate) fn run(
    args: InitArgs,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let project_root = resolve_project_path(args.path.as_deref(), environment)?;
    let detection = detect_project_init(&project_root)?;
    let selection = default_project_init_selection(&detection);
    let config = render_project_init_config(&detection, &selection)?;
    let content =
        yaml_serde::to_string(&config).map_err(|source| config::ConfigError::Parse { source })?;

    if args.print {
        write!(stdout, "{content}")?;
        return Ok(ExitCode::SUCCESS);
    }

    if args.yes {
        let written = write_project_config(&project_root, &config)?;
        let mut output = Output::new(stdout, OutputMode::plain());
        output.line(&format!("Wrote Project config: {}", written.path))?;
        write_detection_summary(&mut output, &detection, &selection)?;
        if selection.include_vite_tls {
            output.line(
                "Vite HTTPS: configure the app's Vite config to read VITE_DEV_SERVER_CERT and VITE_DEV_SERVER_KEY.",
            )?;
        }
        return Ok(ExitCode::SUCCESS);
    }

    if !environment.stdin_is_terminal() {
        let mut output = Output::new(stdout, OutputMode::plain());
        output.line("pv init requires an interactive terminal; rerun with --yes or --print.")?;
        return Ok(ExitCode::FAILURE);
    }

    run_interactive(
        project_root,
        detection,
        selection,
        content,
        environment,
        stdout,
    )
}

fn run_interactive(
    project_root: Utf8PathBuf,
    detection: config::ProjectInitDetection,
    selection: config::ProjectInitSelection,
    content: String,
    environment: &impl Environment,
    stdout: &mut impl Write,
) -> Result<ExitCode, ExecuteError> {
    let _project_root = project_root;
    let _detection = detection;
    let _selection = selection;
    let _content = content;
    let _environment = environment;
    let mut output = Output::new(stdout, OutputMode::plain());
    output.line("Interactive pv init requires --yes or --print in this build.")?;
    Ok(ExitCode::FAILURE)
}

fn write_detection_summary(
    output: &mut Output<'_, impl Write>,
    detection: &config::ProjectInitDetection,
    selection: &config::ProjectInitSelection,
) -> Result<(), ExecuteError> {
    if detection.signals.is_empty() {
        output.line("No framework-specific Project signals detected.")?;
    } else {
        output.line("Detected Project signals:")?;
        for signal in &detection.signals {
            output.line(&format!("  {}: {}", signal.label, signal.detail))?;
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
        output.line("Selected Project resources:")?;
        for (name, resource) in selected_resources {
            output.line(&format!("  {}: {}", resource_name(*name), resource.reason))?;
        }
    }

    Ok(())
}

fn resource_name(name: config::ProjectInitResourceName) -> &'static str {
    match name {
        config::ProjectInitResourceName::Mailpit => "mailpit",
        config::ProjectInitResourceName::Mysql => "mysql",
        config::ProjectInitResourceName::Postgres => "postgres",
        config::ProjectInitResourceName::Redis => "redis",
        config::ProjectInitResourceName::Rustfs => "rustfs",
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
