use std::process::ExitStatus;

use anyhow::Result;
use assert_cmd::Command;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::{assert_debug_snapshot, assert_snapshot};

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
    let output = run_pv(&["php:install", "8.4"])?;

    assert_debug_snapshot!(output);

    Ok(())
}

#[test]
fn php_install_allows_manifest_default_track_when_omitted() -> Result<()> {
    let output = run_pv(&["php:install"])?;

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
fn daemon_lifecycle_commands_are_routed_as_stubs() -> Result<()> {
    let output = [
        run_pv(&["daemon:enable"])?,
        run_pv(&["daemon:disable"])?,
        run_pv(&["daemon:restart"])?,
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
