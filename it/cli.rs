use std::process::ExitStatus;

use anyhow::Result;
use assert_cmd::Command;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::{assert_debug_snapshot, assert_snapshot};
use state::{Database, ProjectEnvObservedStatus, ProjectEnvObservedWarningInput, PvPaths};

#[derive(Debug)]
struct CommandOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_pv(args: &[&str]) -> Result<CommandOutput> {
    run_pv_with_env(args, &[])
}

fn run_pv_with_env(args: &[&str], env: &[(&str, &str)]) -> Result<CommandOutput> {
    let mut command = Command::cargo_bin("pv")?;
    command.env_remove("NO_COLOR");

    for (key, value) in env {
        command.env(key, value);
    }

    let output = command.args(args).output()?;

    Ok(CommandOutput {
        code: status_code(output.status),
        stdout: String::from_utf8(output.stdout)?,
        stderr: String::from_utf8(output.stderr)?,
    })
}

fn run_pv_in_dir_with_home(
    args: &[&str],
    current_dir: &Utf8Path,
    home: &Utf8Path,
) -> Result<CommandOutput> {
    let mut command = Command::cargo_bin("pv")?;
    command.env_remove("NO_COLOR");
    command.env("HOME", home.as_str());
    command.current_dir(current_dir);
    let output = command.args(args).output()?;

    Ok(CommandOutput {
        code: status_code(output.status),
        stdout: String::from_utf8(output.stdout)?,
        stderr: String::from_utf8(output.stderr)?,
    })
}

fn run_pv_without_env(args: &[&str], env: &[&str]) -> Result<CommandOutput> {
    let mut command = Command::cargo_bin("pv")?;
    command.env_remove("NO_COLOR");

    for key in env {
        command.env_remove(key);
    }

    let output = command.args(args).output()?;

    Ok(CommandOutput {
        code: status_code(output.status),
        stdout: String::from_utf8(output.stdout)?,
        stderr: String::from_utf8(output.stderr)?,
    })
}

fn status_code(status: ExitStatus) -> Option<i32> {
    status.code()
}

#[test]
fn version_builds_and_runs_from_source() -> Result<()> {
    let output = run_pv(&["--version"])?;

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn rejects_space_separated_namespace_commands() -> Result<()> {
    let output = run_pv(&["php", "install", "8.4"])?;

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn routes_literal_colon_commands_without_space_aliases() -> Result<()> {
    let output = run_pv(&["php:install", "--help"])?;

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn php_management_commands_are_documented() -> Result<()> {
    let output = [
        run_pv(&["php:use", "--help"])?,
        run_pv(&["php:install", "--help"])?,
        run_pv(&["php:update", "--help"])?,
        run_pv(&["php:uninstall", "--help"])?,
        run_pv(&["php:list", "--help"])?,
    ];

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn composer_commands_are_documented() -> Result<()> {
    let output = [
        run_pv(&["composer:install", "--help"])?,
        run_pv(&["composer:update", "--help"])?,
        run_pv(&["composer:uninstall", "--help"])?,
    ];

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn mailpit_commands_are_documented() -> Result<()> {
    let output = [
        run_pv(&["mailpit:install", "--help"])?,
        run_pv(&["mailpit:update", "--help"])?,
        run_pv(&["mailpit:uninstall", "--help"])?,
        run_pv(&["mailpit:list", "--help"])?,
        run_pv(&["mailpit:open", "--help"])?,
        run_pv(&["mail:open", "--help"])?,
    ];

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn redis_commands_are_documented() -> Result<()> {
    let output = [
        run_pv(&["redis:install", "--help"])?,
        run_pv(&["redis:update", "--help"])?,
        run_pv(&["redis:uninstall", "--help"])?,
        run_pv(&["redis:list", "--help"])?,
    ];

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn rustfs_commands_are_documented() -> Result<()> {
    let output = [
        run_pv(&["rustfs:install", "--help"])?,
        run_pv(&["rustfs:update", "--help"])?,
        run_pv(&["rustfs:uninstall", "--help"])?,
        run_pv(&["rustfs:list", "--help"])?,
        run_pv(&["rustfs:open", "--help"])?,
        run_pv(&["s3:install", "--help"])?,
        run_pv(&["s3:update", "--help"])?,
        run_pv(&["s3:uninstall", "--help"])?,
        run_pv(&["s3:list", "--help"])?,
        run_pv(&["s3:open", "--help"])?,
    ];

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn mysql_commands_are_documented() -> Result<()> {
    let output = [
        run_pv(&["mysql:install", "--help"])?,
        run_pv(&["mysql:update", "--help"])?,
        run_pv(&["mysql:uninstall", "--help"])?,
        run_pv(&["mysql:list", "--help"])?,
    ];

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn postgres_commands_are_documented() -> Result<()> {
    let output = [
        run_pv(&["postgres:install", "--help"])?,
        run_pv(&["postgres:update", "--help"])?,
        run_pv(&["postgres:uninstall", "--help"])?,
        run_pv(&["postgres:list", "--help"])?,
        run_pv(&["pg:install", "--help"])?,
        run_pv(&["pg:update", "--help"])?,
        run_pv(&["pg:uninstall", "--help"])?,
        run_pv(&["pg:list", "--help"])?,
    ];

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn daemon_run_is_hidden_from_top_level_help() -> Result<()> {
    let output = run_pv(&["--help"])?;

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn daemon_lifecycle_commands_are_documented_without_running_them() -> Result<()> {
    let output = [
        run_pv(&["daemon:enable", "--help"])?,
        run_pv(&["daemon:disable", "--help"])?,
        run_pv(&["daemon:restart", "--help"])?,
    ];

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn diagnostic_commands_are_documented() -> Result<()> {
    let output = [
        run_pv(&["status", "--help"])?,
        run_pv(&["logs", "--help"])?,
        run_pv(&["doctor", "--help"])?,
        run_pv(&["jobs", "--help"])?,
        run_pv(&["update", "--help"])?,
        run_pv(&["list", "--help"])?,
    ];

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn diagnostic_command_options_are_documented() -> Result<()> {
    let output = [
        run_pv(&["status", "--json", "--help"])?,
        run_pv(&["logs", "--follow", "-n", "0", "--all", "--help"])?,
        run_pv(&["logs", "--resource", "pg", "--track", "latest", "--help"])?,
        run_pv(&["doctor", "--help"])?,
        run_pv(&["jobs", "--json", "--help"])?,
        run_pv(&["update", "--check", "--json", "--help"])?,
    ];

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn core_workflow_command_help_is_documented() -> Result<()> {
    let output = [
        run_pv(&["setup", "--help"])?,
        run_pv(&["uninstall", "--help"])?,
        run_pv(&["init", "--help"])?,
        run_pv(&["link", "--help"])?,
        run_pv(&["unlink", "--help"])?,
        run_pv(&["open", "--help"])?,
        run_pv(&["project:env", "--help"])?,
    ];

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn system_integration_command_help_is_documented() -> Result<()> {
    let output = [
        run_pv(&["ca:status", "--help"])?,
        run_pv(&["ca:trust", "--help"])?,
        run_pv(&["ca:untrust", "--help"])?,
        run_pv(&["dns:status", "--help"])?,
        run_pv(&["dns:install", "--help"])?,
        run_pv(&["dns:uninstall", "--help"])?,
        run_pv(&["ports:status", "--help"])?,
        run_pv(&["ports:install", "--help"])?,
        run_pv(&["ports:uninstall", "--help"])?,
    ];

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn shell_command_help_is_documented() -> Result<()> {
    let output = [
        run_pv(&["env", "--help"])?,
        run_pv(&["completions", "--help"])?,
    ];

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn env_zsh_output_is_shell_startup_safe() -> Result<()> {
    let output = run_pv(&["env", "--shell", "zsh"])?;

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn env_bash_output_matches_posix_shells() -> Result<()> {
    let output = run_pv(&["env", "--shell", "bash"])?;

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn env_fish_output_is_idempotent() -> Result<()> {
    let output = run_pv(&["env", "--shell", "fish"])?;

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn env_detects_shell_from_environment() -> Result<()> {
    let output = run_pv_with_env(&["env"], &[("SHELL", "/bin/zsh")])?;

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn env_reports_missing_detected_shell() -> Result<()> {
    let output = run_pv_without_env(&["env"], &["SHELL"])?;

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn env_reports_unsupported_detected_shell() -> Result<()> {
    let output = run_pv_with_env(&["env"], &[("SHELL", "/bin/tcsh")])?;

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn no_color_global_flag_is_accepted() -> Result<()> {
    let output = run_pv(&["--no-color", "env", "--shell", "zsh"])?;

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn no_color_environment_keeps_errors_plain() -> Result<()> {
    let output = run_pv_with_env(&["php", "install", "8.4"], &[("NO_COLOR", "1")])?;

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn env_rejects_unsupported_shells() -> Result<()> {
    let output = run_pv(&["env", "--shell", "tcsh"])?;

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn completions_generate_zsh_script() -> Result<()> {
    let output = run_pv(&["completions", "zsh"])?;

    assert_snapshot!(output.stdout);
    assert_debug_snapshot!((output.code, output.stderr));

    Ok(())
}

#[test]
fn completions_generate_bash_script() -> Result<()> {
    let output = run_pv(&["completions", "bash"])?;

    assert_snapshot!(output.stdout);
    assert_debug_snapshot!((output.code, output.stderr));

    Ok(())
}

#[test]
fn completions_generate_fish_script() -> Result<()> {
    let output = run_pv(&["completions", "fish"])?;

    assert_snapshot!(output.stdout);
    assert_debug_snapshot!((output.code, output.stderr));

    Ok(())
}

#[test]
fn completions_reject_unsupported_shells() -> Result<()> {
    let output = run_pv(&["completions", "tcsh"])?;

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn project_link_list_and_unlink_use_injected_home() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("Acme Store");
    create_dir(&project.join("public"))?;
    write_file(
        &project.join("pv.yml"),
        "php: 8.4\nhostnames:\n  - api.acme-store.test\nenv:\n  APP_URL: \"${project_url}\"\n",
    )?;

    let link = run_pv_in_dir_with_home(&["link"], &project, &home)?;
    let list_after_link = run_pv_in_dir_with_home(&["list"], &project, &home)?;
    let unlink = run_pv_in_dir_with_home(&["unlink", "api.acme-store.test"], &project, &home)?;
    let list_after_unlink = run_pv_in_dir_with_home(&["list"], &project, &home)?;

    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((link, list_after_link, unlink, list_after_unlink));
    });

    Ok(())
}

#[test]
fn project_link_accepts_relative_path_arguments() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let parent = tempdir.path().join("parent");
    let project = parent.join("Acme Store");
    let work = parent.join("work");
    create_dir(&project.join("public"))?;
    create_dir(&work)?;
    write_file(&project.join("pv.yml"), "php: 8.4\n")?;

    let link = run_pv_in_dir_with_home(&["link", "../Acme Store"], &work, &home)?;
    let list = run_pv_in_dir_with_home(&["list"], &work, &home)?;

    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((link, list));
    });

    Ok(())
}

#[test]
fn project_init_prints_detected_laravel_config_without_writing() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("Acme Store");
    create_laravel_init_fixture(&project)?;

    let output = run_pv_in_dir_with_home(&["init", "--print"], &project, &home)?;

    assert_eq!(output.code, Some(0));
    assert!(output.stderr.is_empty());
    assert!(!path_exists(&project.join("pv.yml"))?);
    assert!(!output.stdout.contains("Vite config"));
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!(output);
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[test]
fn project_init_yes_writes_detected_laravel_config() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("Acme Store");
    create_laravel_init_fixture(&project)?;

    let output = run_pv_in_dir_with_home(&["init", "--yes"], &project, &home)?;
    let config = read_file(&project.join("pv.yml"))?;

    assert_eq!(output.code, Some(0));
    assert!(output.stderr.is_empty());
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((output, config));
    });

    Ok(())
}

#[test]
fn project_init_requires_yes_or_print_when_non_interactive() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("Acme Store");
    create_laravel_init_fixture(&project)?;

    let output = run_pv_in_dir_with_home(&["init"], &project, &home)?;

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn project_init_accepts_relative_target_path_arguments() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let parent = tempdir.path().join("parent");
    let project = parent.join("Acme Store");
    let work = parent.join("work");
    create_laravel_init_fixture(&project)?;
    create_dir(&work)?;

    let output = run_pv_in_dir_with_home(&["init", "../Acme Store", "--yes"], &work, &home)?;
    let config = read_file(&project.join("pv.yml"))?;

    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((output, config));
    });

    Ok(())
}

#[test]
fn project_init_updates_existing_pv_yaml_without_creating_pv_yml() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("Acme Store");
    create_laravel_init_fixture(&project)?;
    write_file(
        &project.join("pv.yaml"),
        "php: 8.3\nenv:\n  USER_VALUE: preserved\n",
    )?;

    let output = run_pv_in_dir_with_home(&["init", "--yes"], &project, &home)?;
    let config = read_file(&project.join("pv.yaml"))?;

    assert!(!path_exists(&project.join("pv.yml"))?);
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((output, config));
    });

    Ok(())
}

#[test]
fn project_init_rejects_conflicting_config_files_without_writing() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("Acme Store");
    create_laravel_init_fixture(&project)?;
    let preferred = "php: 8.3\n";
    let alternate = "php: 8.4\n";
    write_file(&project.join("pv.yml"), preferred)?;
    write_file(&project.join("pv.yaml"), alternate)?;

    let output = run_pv_in_dir_with_home(&["init", "--yes"], &project, &home)?;

    assert_eq!(read_file(&project.join("pv.yml"))?, preferred);
    assert_eq!(read_file(&project.join("pv.yaml"))?, alternate);
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!(output);
    });

    Ok(())
}

#[test]
fn project_init_rejects_invalid_existing_config_without_writing() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("Acme Store");
    create_laravel_init_fixture(&project)?;
    let config = "unexpected: true\n";
    write_file(&project.join("pv.yml"), config)?;

    let output = run_pv_in_dir_with_home(&["init", "--yes"], &project, &home)?;

    assert_eq!(read_file(&project.join("pv.yml"))?, config);
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!(output);
    });

    Ok(())
}

#[test]
fn project_init_preserves_existing_config_values_while_adding_detected_defaults() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("Acme Store");
    create_laravel_init_fixture(&project)?;
    write_file(
        &project.join("pv.yml"),
        "php: 8.3\nenv:\n  APP_URL: https://user.test\n  USER_VALUE: preserved\n",
    )?;

    let output = run_pv_in_dir_with_home(&["init", "--yes"], &project, &home)?;
    let config = read_file(&project.join("pv.yml"))?;

    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((output, config));
    });

    Ok(())
}

#[test]
fn project_init_rejects_yes_with_print() -> Result<()> {
    let output = run_pv(&["init", "--yes", "--print"])?;

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn project_list_reports_invalid_linked_config() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("Acme Store");
    create_dir(&project)?;
    write_file(&project.join("pv.yml"), "php: 8.4\n")?;

    let link = run_pv_in_dir_with_home(&["link"], &project, &home)?;
    write_file(&project.join("pv.yml"), "unexpected: true\n")?;
    let list = run_pv_in_dir_with_home(&["list"], &project, &home)?;

    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((link, list));
    });

    Ok(())
}

#[test]
fn project_list_reports_config_hostname_validation_errors() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("Acme Store");
    create_dir(&project)?;
    write_file(&project.join("pv.yml"), "php: 8.4\n")?;

    let link = run_pv_in_dir_with_home(&["link"], &project, &home)?;
    write_file(
        &project.join("pv.yml"),
        "php: 8.4\nhostnames:\n  - acme-store.test\n",
    )?;
    let list = run_pv_in_dir_with_home(&["list"], &project, &home)?;

    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((link, list));
    });

    Ok(())
}

#[test]
fn project_list_reports_env_observed_status() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("Acme Store");
    create_dir(&project)?;
    write_file(
        &project.join("pv.yml"),
        "env:\n  APP_URL: \"${project_url}\"\n",
    )?;

    let link = run_pv_in_dir_with_home(&["link"], &project, &home)?;
    let list_pending = run_pv_in_dir_with_home(&["list"], &project, &home)?;
    let paths = PvPaths::for_home(home.clone());
    let mut database = Database::open(&paths)?;
    let linked_project = database
        .projects()?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing linked project"))?;
    database.record_project_env_observed_snapshot(
        &linked_project.id,
        ProjectEnvObservedStatus::Rendered,
        Some("rendered Project env"),
        &[],
    )?;
    let list_rendered = run_pv_in_dir_with_home(&["list"], &project, &home)?;
    database.record_project_env_observed_snapshot(
        &linked_project.id,
        ProjectEnvObservedStatus::Warning,
        Some("rendered with warnings"),
        &[ProjectEnvObservedWarningInput {
            kind: "duplicate_key".to_string(),
            message: "APP_URL already exists outside the PV block".to_string(),
        }],
    )?;
    let list_warning = run_pv_in_dir_with_home(&["list"], &project, &home)?;
    database.record_project_env_observed_snapshot(
        &linked_project.id,
        ProjectEnvObservedStatus::Failed,
        Some("Project config error: duplicate rendered Project env key `DATABASE_URL`"),
        &[],
    )?;
    let list_failed = run_pv_in_dir_with_home(&["list"], &project, &home)?;

    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((link, list_pending, list_rendered, list_warning, list_failed));
    });

    Ok(())
}

#[test]
fn project_list_reports_ignored_php_extensions() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("Acme Store");
    create_dir(&project)?;
    write_file(
        &project.join("pv.yml"),
        "php:\n  extensions: [redis, missing]\n",
    )?;

    let link = run_pv_in_dir_with_home(&["link"], &project, &home)?;
    let paths = PvPaths::for_home(home.clone());
    let mut database = Database::open(&paths)?;
    let linked_project = database
        .projects()?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing linked project"))?;
    database.record_project_env_observed_snapshot(
        &linked_project.id,
        ProjectEnvObservedStatus::Warning,
        Some("Project runtime has warnings"),
        &[
            ProjectEnvObservedWarningInput {
                kind: "ignored_php_extension".to_string(),
                message: "ignored unsupported PHP extension `missing`".to_string(),
            },
            ProjectEnvObservedWarningInput {
                kind: "ignored_php_extension".to_string(),
                message: "ignored unsupported PHP extension `typo`".to_string(),
            },
        ],
    )?;

    let list = run_pv_in_dir_with_home(&["list"], &project, &home)?;

    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((link, list));
    });

    Ok(())
}

#[test]
fn project_list_clears_stale_env_status_without_mappings() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("Acme Store");
    create_dir(&project)?;

    let link = run_pv_in_dir_with_home(&["link"], &project, &home)?;
    let paths = PvPaths::for_home(home.clone());
    let mut database = Database::open(&paths)?;
    let linked_project = database
        .projects()?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing linked project"))?;
    let list_initial = run_pv_in_dir_with_home(&["list"], &project, &home)?;

    database.record_project_env_observed_snapshot(
        &linked_project.id,
        ProjectEnvObservedStatus::Pending,
        Some("Project env reconciliation pending"),
        &[],
    )?;
    let list_stale_pending = run_pv_in_dir_with_home(&["list"], &project, &home)?;

    database.record_project_env_observed_snapshot(
        &linked_project.id,
        ProjectEnvObservedStatus::Warning,
        Some("rendered with warnings"),
        &[ProjectEnvObservedWarningInput {
            kind: "duplicate_key".to_string(),
            message: "APP_URL already exists outside the PV block".to_string(),
        }],
    )?;
    let list_stale_warning = run_pv_in_dir_with_home(&["list"], &project, &home)?;

    database.record_project_env_observed_snapshot(
        &linked_project.id,
        ProjectEnvObservedStatus::Failed,
        Some("Project config error: missing Project env context"),
        &[],
    )?;
    let list_stale_failed = run_pv_in_dir_with_home(&["list"], &project, &home)?;

    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((
            link,
            list_initial,
            list_stale_pending,
            list_stale_warning,
            list_stale_failed
        ));
    });

    Ok(())
}

#[test]
fn project_list_reports_env_shape_validation_errors() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("Acme Store");
    create_dir(&project)?;
    write_file(
        &project.join("pv.yml"),
        "env:\n  APP_URL: \"${project_url}\"\n",
    )?;

    let link = run_pv_in_dir_with_home(&["link"], &project, &home)?;
    write_file(
        &project.join("pv.yml"),
        r#"mysql:
  env:
    DB_HOST: "${host}"
redis:
  env:
    DB_HOST: "${host}"
"#,
    )?;
    let list = run_pv_in_dir_with_home(&["list"], &project, &home)?;

    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((link, list));
    });

    Ok(())
}

#[test]
fn project_env_renders_values_from_binary_without_mutating_dotenv() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("Acme Store");
    let env_path = project.join(".env");
    create_dir(&project)?;
    write_file(
        &project.join("pv.yml"),
        "env:\n  APP_URL: \"${project_url}\"\n",
    )?;
    write_file(&env_path, "APP_URL=https://user.test\n")?;

    let link = run_pv_in_dir_with_home(&["link"], &project, &home)?;
    let project_env = run_pv_in_dir_with_home(&["project:env"], &project, &home)?;
    let env_after = read_file(&env_path)?;

    assert_eq!(env_after, "APP_URL=https://user.test\n");
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((link, project_env, env_after));
    });

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI integration tests create fixture directories"
)]
fn create_dir(path: &Utf8Path) -> Result<()> {
    std::fs::create_dir_all(path)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI integration tests write fixture files"
)]
fn write_file(path: &Utf8Path, contents: &str) -> Result<()> {
    std::fs::write(path, contents)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI integration tests read fixture files"
)]
fn read_file(path: &Utf8Path) -> Result<String> {
    Ok(std::fs::read_to_string(path)?)
}

fn create_laravel_init_fixture(project: &Utf8Path) -> Result<()> {
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
        r#"APP_URL=http://localhost
DB_CONNECTION=mysql
REDIS_HOST=127.0.0.1
CACHE_STORE=redis
MAIL_MAILER=smtp
AWS_ACCESS_KEY_ID=
AWS_SECRET_ACCESS_KEY=
"#,
    )?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI integration tests check fixture file presence"
)]
fn path_exists(path: &Utf8Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_metadata) => Ok(true),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(source.into()),
    }
}
