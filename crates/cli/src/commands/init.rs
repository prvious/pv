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
    let mut output = Output::new(stdout, OutputMode::plain());
    write_detection_summary(&mut output, &detection, &selection)?;
    write_resource_checklist(&mut output, &selection.resources)?;
    output.line(
        "Use these selections? Enter y to preview, n to cancel, or edit to change selections:",
    )?;

    match environment
        .read_line()?
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "" | "y" | "yes" => preview_and_confirm_write(
            project_root,
            content,
            selection.include_vite_tls,
            environment,
            &mut output,
        ),
        "n" | "no" => cancelled(&mut output),
        "e" | "edit" => {
            run_structured_edit(project_root, detection, selection, environment, &mut output)
        }
        _ => {
            output.line("Invalid selection. Enter y, n, or edit.")?;
            Ok(ExitCode::FAILURE)
        }
    }
}

fn preview_and_confirm_write(
    project_root: Utf8PathBuf,
    content: String,
    include_vite_tls: bool,
    environment: &impl Environment,
    output: &mut Output<'_, impl Write>,
) -> Result<ExitCode, ExecuteError> {
    output.line("Project config preview:")?;
    for line in content.lines() {
        output.line(line)?;
    }
    output.line("Write Project config? Enter y to continue:")?;
    if !matches!(
        environment
            .read_line()?
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "y" | "yes"
    ) {
        return cancelled(output);
    }

    let config = config::ProjectConfig::parse(&content)?;
    let written = write_project_config(&project_root, &config)?;
    output.line(&format!("Wrote Project config: {}", written.path))?;
    if include_vite_tls {
        output.line(
            "Vite HTTPS: configure the app's Vite config to read VITE_DEV_SERVER_CERT and VITE_DEV_SERVER_KEY.",
        )?;
    }

    Ok(ExitCode::SUCCESS)
}

fn cancelled(output: &mut Output<'_, impl Write>) -> Result<ExitCode, ExecuteError> {
    output.line("pv init cancelled; no files changed.")?;
    Ok(ExitCode::FAILURE)
}

fn write_resource_checklist(
    output: &mut Output<'_, impl Write>,
    resources: &std::collections::BTreeMap<
        config::ProjectInitResourceName,
        config::ProjectInitResourceSelection,
    >,
) -> Result<(), ExecuteError> {
    output.line("Resource checklist:")?;
    for name in resource_names() {
        let Some(resource) = resources.get(&name) else {
            continue;
        };
        let marker = if resource.selected { "[x]" } else { "[ ]" };
        output.line(&format!("  {marker} {}", resource_label(name)))?;
    }

    Ok(())
}

fn run_structured_edit(
    project_root: Utf8PathBuf,
    detection: config::ProjectInitDetection,
    mut selection: config::ProjectInitSelection,
    environment: &impl Environment,
    output: &mut Output<'_, impl Write>,
) -> Result<ExitCode, ExecuteError> {
    output.line(&format!("PHP track [{}]:", selection.php))?;
    let php = environment.read_line()?;
    if !php.trim().is_empty() {
        selection.php = php.trim().to_string();
    }

    let document_root = selection
        .document_root
        .as_ref()
        .map_or(".", |path| path.as_str());
    output.line(&format!("Document root [{document_root}]:"))?;
    let document_root = environment.read_line()?;
    if !document_root.trim().is_empty() {
        selection.document_root = Some(Utf8PathBuf::from(document_root.trim()));
    }

    let selected_resources = selected_resource_names(&selection.resources);
    output.line(&format!("Selected resources [{selected_resources}]:"))?;
    let selected_resources = environment.read_line()?;
    if !selected_resources.trim().is_empty()
        && !apply_selected_resources(&mut selection, selected_resources.trim(), output)?
    {
        return Ok(ExitCode::FAILURE);
    }

    for name in resource_names() {
        prompt_resource_details(name, &mut selection, environment, output)?;
    }

    let config = render_project_init_config(&detection, &selection)?;
    let content =
        yaml_serde::to_string(&config).map_err(|source| config::ConfigError::Parse { source })?;
    preview_and_confirm_write(
        project_root,
        content,
        selection.include_vite_tls,
        environment,
        output,
    )
}

fn apply_selected_resources(
    selection: &mut config::ProjectInitSelection,
    value: &str,
    output: &mut Output<'_, impl Write>,
) -> Result<bool, ExecuteError> {
    let mut selected = Vec::new();
    for token in parse_csv(value) {
        let Some(name) = resource_from_token(&token) else {
            output.line(&format!("Unknown resource selection: {token}"))?;
            return Ok(false);
        };
        selected.push(name);
    }

    for (name, resource) in &mut selection.resources {
        resource.selected = selected.contains(name);
    }

    Ok(true)
}

fn prompt_resource_details(
    name: config::ProjectInitResourceName,
    selection: &mut config::ProjectInitSelection,
    environment: &impl Environment,
    output: &mut Output<'_, impl Write>,
) -> Result<(), ExecuteError> {
    let Some(resource) = selection.resources.get_mut(&name) else {
        return Ok(());
    };
    if !resource.selected {
        return Ok(());
    }

    output.line(&format!(
        "{} track [{}]:",
        resource_label(name),
        resource.track
    ))?;
    let track = environment.read_line()?;
    if !track.trim().is_empty() {
        resource.track = track.trim().to_string();
    }

    if name != config::ProjectInitResourceName::Mailpit {
        let allocations = resource.allocations.join(",");
        output.line(&format!(
            "{} allocations [{allocations}]:",
            resource_label(name)
        ))?;
        let allocations = environment.read_line()?;
        if !allocations.trim().is_empty() {
            resource.allocations = parse_csv(allocations.trim());
        }
    }

    Ok(())
}

fn parse_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn selected_resource_names(
    resources: &std::collections::BTreeMap<
        config::ProjectInitResourceName,
        config::ProjectInitResourceSelection,
    >,
) -> String {
    resource_names()
        .into_iter()
        .filter(|name| {
            resources
                .get(name)
                .is_some_and(|resource| resource.selected)
        })
        .map(resource_name)
        .collect::<Vec<_>>()
        .join(",")
}

fn resource_names() -> [config::ProjectInitResourceName; 5] {
    [
        config::ProjectInitResourceName::Mysql,
        config::ProjectInitResourceName::Postgres,
        config::ProjectInitResourceName::Redis,
        config::ProjectInitResourceName::Mailpit,
        config::ProjectInitResourceName::Rustfs,
    ]
}

fn resource_from_token(value: &str) -> Option<config::ProjectInitResourceName> {
    match value.to_ascii_lowercase().as_str() {
        "mailpit" => Some(config::ProjectInitResourceName::Mailpit),
        "mysql" => Some(config::ProjectInitResourceName::Mysql),
        "postgres" => Some(config::ProjectInitResourceName::Postgres),
        "redis" => Some(config::ProjectInitResourceName::Redis),
        "rustfs" | "s3" => Some(config::ProjectInitResourceName::Rustfs),
        _ => None,
    }
}

fn resource_label(name: config::ProjectInitResourceName) -> &'static str {
    match name {
        config::ProjectInitResourceName::Mailpit => "Mailpit",
        config::ProjectInitResourceName::Mysql => "MySQL",
        config::ProjectInitResourceName::Postgres => "Postgres",
        config::ProjectInitResourceName::Redis => "Redis",
        config::ProjectInitResourceName::Rustfs => "RustFS/S3",
    }
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::ffi::OsString;
    use std::io;
    use std::path::PathBuf;

    use camino::Utf8Path;
    use camino_tempfile::tempdir;
    use insta::assert_snapshot;

    use super::*;

    #[derive(Debug)]
    struct TestEnvironment {
        current_dir: PathBuf,
        input: RefCell<Vec<String>>,
    }

    impl TestEnvironment {
        fn new(current_dir: &Utf8Path, input: &[&str]) -> Self {
            Self {
                current_dir: current_dir.as_std_path().to_path_buf(),
                input: RefCell::new(input.iter().rev().map(|line| format!("{line}\n")).collect()),
            }
        }
    }

    impl Environment for TestEnvironment {
        fn var_os(&self, _key: &str) -> Option<OsString> {
            None
        }

        fn home_dir(&self) -> Option<PathBuf> {
            Some(self.current_dir.clone())
        }

        fn current_dir(&self) -> io::Result<PathBuf> {
            Ok(self.current_dir.clone())
        }

        fn current_exe(&self) -> io::Result<PathBuf> {
            Ok(self.current_dir.join("pv"))
        }

        fn stdin_is_terminal(&self) -> bool {
            true
        }

        fn read_line(&self) -> io::Result<String> {
            self.input
                .borrow_mut()
                .pop()
                .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "missing test input"))
        }

        fn open_url(&self, _url: &str) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn init_interactive_accepts_defaults_and_writes_config() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let project = tempdir.path().join("acme");
        create_laravel_fixture(&project)?;
        let environment = TestEnvironment::new(&project, &["y", "y"]);
        let mut stdout = Vec::new();

        let exit = run(default_args(), &environment, &mut stdout)?;

        assert_eq!(exit, ExitCode::SUCCESS);
        assert_output_snapshot(
            "init_interactive_accepts_defaults_and_writes_config_output",
            tempdir.path(),
            String::from_utf8(stdout)?,
        );
        assert_snapshot!(
            "init_interactive_accepts_defaults_and_writes_config_config",
            read_file(&project.join("pv.yml"))?
        );

        Ok(())
    }

    #[test]
    fn init_interactive_initial_cancel_leaves_new_project_unchanged() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let project = tempdir.path().join("acme");
        create_laravel_fixture(&project)?;
        let environment = TestEnvironment::new(&project, &["n"]);
        let mut stdout = Vec::new();

        let exit = run(default_args(), &environment, &mut stdout)?;

        assert_eq!(exit, ExitCode::FAILURE);
        assert!(!path_exists(&project.join("pv.yml"))?);
        assert_output_snapshot(
            "init_interactive_initial_cancel_leaves_new_project_unchanged",
            tempdir.path(),
            String::from_utf8(stdout)?,
        );

        Ok(())
    }

    #[test]
    fn init_interactive_final_cancel_leaves_existing_config_unchanged() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let project = tempdir.path().join("acme");
        create_laravel_fixture(&project)?;
        let original = "php: 8.3\ndocument_root: public\nenv:\n  USER_VALUE: preserved\n";
        write_file(&project.join("pv.yml"), original)?;
        let environment = TestEnvironment::new(&project, &["y", "n"]);
        let mut stdout = Vec::new();

        let exit = run(default_args(), &environment, &mut stdout)?;

        assert_eq!(exit, ExitCode::FAILURE);
        assert_eq!(read_file(&project.join("pv.yml"))?, original);
        assert_output_snapshot(
            "init_interactive_final_cancel_leaves_existing_config_unchanged",
            tempdir.path(),
            String::from_utf8(stdout)?,
        );

        Ok(())
    }

    #[test]
    fn init_interactive_edits_php_resources_and_allocations() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let project = tempdir.path().join("acme");
        create_laravel_fixture(&project)?;
        let environment = TestEnvironment::new(
            &project,
            &[
                "edit",
                "8.5",
                "public",
                "mysql,redis,mailpit,rustfs",
                "latest",
                "app,analytics",
                "latest",
                "cache",
                "latest",
                "latest",
                "uploads",
                "y",
            ],
        );
        let mut stdout = Vec::new();

        let exit = run(default_args(), &environment, &mut stdout)?;

        assert_eq!(exit, ExitCode::SUCCESS);
        assert_output_snapshot(
            "init_interactive_edits_php_resources_and_allocations_output",
            tempdir.path(),
            String::from_utf8(stdout)?,
        );
        assert_snapshot!(
            "init_interactive_edits_php_resources_and_allocations_config",
            read_file(&project.join("pv.yml"))?
        );

        Ok(())
    }

    #[test]
    fn init_interactive_blank_edits_preserve_existing_defaults() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let project = tempdir.path().join("acme");
        create_laravel_fixture(&project)?;
        write_file(
            &project.join("pv.yml"),
            "php: 8.3\ndocument_root: public\nmysql:\n  version: 8.4\n  allocations:\n    primary: {}\nredis:\n  version: 7.2\n  allocations:\n    sessions: {}\nmailpit:\n  version: 1.0\nrustfs:\n  version: 1.1\n  allocations:\n    media: {}\n",
        )?;
        let environment = TestEnvironment::new(
            &project,
            &["edit", "", "", "", "", "", "", "", "", "", "", "y"],
        );
        let mut stdout = Vec::new();

        let exit = run(default_args(), &environment, &mut stdout)?;

        assert_eq!(exit, ExitCode::SUCCESS);
        assert_output_snapshot(
            "init_interactive_blank_edits_preserve_existing_defaults_output",
            tempdir.path(),
            String::from_utf8(stdout)?,
        );
        assert_snapshot!(
            "init_interactive_blank_edits_preserve_existing_defaults_config",
            read_file(&project.join("pv.yml"))?
        );

        Ok(())
    }

    #[test]
    fn init_interactive_rejects_unknown_resource_selection_without_writing() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let project = tempdir.path().join("acme");
        create_laravel_fixture(&project)?;
        let environment = TestEnvironment::new(&project, &["edit", "", "", "mysql,unknown"]);
        let mut stdout = Vec::new();

        let exit = run(default_args(), &environment, &mut stdout)?;

        assert_eq!(exit, ExitCode::FAILURE);
        assert!(!path_exists(&project.join("pv.yml"))?);
        assert_output_snapshot(
            "init_interactive_rejects_unknown_resource_selection_without_writing",
            tempdir.path(),
            String::from_utf8(stdout)?,
        );

        Ok(())
    }

    fn default_args() -> InitArgs {
        InitArgs {
            path: None,
            yes: false,
            print: false,
        }
    }

    fn assert_output_snapshot(name: &str, tempdir: &Utf8Path, output: String) {
        let mut settings = insta::Settings::clone_current();
        settings.add_filter(tempdir.as_str(), "<tempdir>");
        settings.add_filter("/private<tempdir>", "<tempdir>");
        settings.bind(|| assert_snapshot!(name, output));
    }

    fn create_laravel_fixture(project: &Utf8Path) -> anyhow::Result<()> {
        create_dir(&project.join("bootstrap"))?;
        create_dir(&project.join("config"))?;
        create_dir(&project.join("public"))?;
        write_file(&project.join("artisan"), "")?;
        write_file(&project.join("bootstrap/app.php"), "<?php\n")?;
        write_file(&project.join("config/app.php"), "<?php\n")?;
        write_file(&project.join("public/index.php"), "<?php\n")?;
        write_file(
            &project.join("composer.json"),
            r#"{"require":{"php":"^8.4","laravel/framework":"^12.0"}}"#,
        )?;
        write_file(
            &project.join("package.json"),
            r#"{"devDependencies":{"vite":"^7.0.0","laravel-vite-plugin":"^2.0.0"}}"#,
        )?;
        write_file(
            &project.join(".env.example"),
            "APP_URL=http://localhost\nDB_CONNECTION=mysql\nREDIS_HOST=127.0.0.1\nCACHE_STORE=redis\nMAIL_MAILER=smtp\nAWS_ACCESS_KEY_ID=\nAWS_SECRET_ACCESS_KEY=\n",
        )?;

        Ok(())
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "CLI init tests create fixture directories"
    )]
    fn create_dir(path: &Utf8Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(path)?;

        Ok(())
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "CLI init tests write fixture files"
    )]
    fn write_file(path: &Utf8Path, contents: &str) -> anyhow::Result<()> {
        std::fs::write(path, contents)?;

        Ok(())
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "CLI init tests read fixture files"
    )]
    fn read_file(path: &Utf8Path) -> anyhow::Result<String> {
        Ok(std::fs::read_to_string(path)?)
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "CLI init tests check fixture file presence"
    )]
    fn path_exists(path: &Utf8Path) -> anyhow::Result<bool> {
        match std::fs::symlink_metadata(path) {
            Ok(_metadata) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
}
