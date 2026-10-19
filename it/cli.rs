#[cfg(target_os = "macos")]
use std::collections::BTreeMap;
#[cfg(target_os = "macos")]
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(target_os = "macos")]
use std::os::unix::net::UnixListener;
use std::process::ExitStatus;
#[cfg(target_os = "macos")]
use std::process::{Child, Stdio};
#[cfg(target_os = "macos")]
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
#[cfg(target_os = "macos")]
use std::thread::{self, JoinHandle};
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

use anyhow::Result;
use assert_cmd::Command;
use camino::Utf8Path;
use camino_tempfile::tempdir;
#[cfg(target_os = "macos")]
use daemon::{MonitorCleanup, MonitorExit, MonitorStart, MonitorStop, StopSignal};
use insta::{assert_debug_snapshot, assert_snapshot};
#[cfg(target_os = "macos")]
use serde_json::{Value, json};
#[cfg(target_os = "macos")]
use state::MonitorReservation;
use state::{
    Database, ProjectEnvObservedStatus, ProjectEnvObservedWarningInput, PvPaths, UpdateLock,
};
#[cfg(target_os = "macos")]
use state::{JobDiagnosticSubject, JobStatus, RuntimeLifecycleLock};
use support::{create_dir, create_laravel_init_fixture, run_pv_in, write_file};

mod support;

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
    let output = run_pv_in(args, current_dir, home)?;

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

#[cfg(target_os = "macos")]
#[tokio::test]
async fn hidden_monitor_commands_run_a_runtime_until_released() -> Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    state::fs::ensure_layout(&paths)?;
    let stop = MonitorStop {
        signal: StopSignal::Terminate,
        grace_ms: 5_000,
    };
    let start = MonitorStart {
        subject: "smoke".to_owned(),
        command: "/bin/sh".into(),
        arguments: vec!["-c".to_owned(), "while :; do sleep 0.1; done".to_owned()],
        private_environment: BTreeMap::new(),
        log_path: paths.logs().join("smoke.log"),
        fallback_stop: stop,
        lifeline_fd: Some(pv_fake::lifeline_fd()?),
    };

    let started =
        daemon::start_monitor(&paths, Utf8Path::new(env!("CARGO_BIN_EXE_pv")), start).await?;
    let stopped = daemon::stop_monitor(&paths, "smoke", stop).await?;
    daemon::release_monitor(&paths, "smoke").await?;

    assert_eq!(started.cleanup, MonitorCleanup::Pending);
    assert_eq!(stopped.exit, Some(MonitorExit::Signal(15)));
    assert_eq!(stopped.cleanup, MonitorCleanup::Complete);
    // The reservation is free only once the monitor process has exited.
    assert!(MonitorReservation::acquire(&paths, "smoke").is_ok());

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
        "php: 8.4\nenv:\n  APP_URL: \"${url}\"\n",
    )?;

    let link = run_pv_in_dir_with_home(&["link"], &project, &home)?;
    let list_after_link = run_pv_in_dir_with_home(&["list"], &project, &home)?;
    let unlink = run_pv_in_dir_with_home(&["unlink", "acme-store.test"], &project, &home)?;
    let list_after_unlink = run_pv_in_dir_with_home(&["list"], &project, &home)?;

    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((link, list_after_link, unlink, list_after_unlink));
    });

    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn project_link_runs_prepare_and_warns_about_setup_without_a_daemon() -> Result<()> {
    let directory = tempdir()?;
    let home = directory.path().join("home");
    let project = directory.path().join("acme");
    create_dir(&project)?;
    create_dir(&home)?;
    write_file(
        &project.join("pv.yml"),
        "serve: false\nhooks:\n  prepare:\n    - printf 'prepare output\\n'\n    - printf prepared > prepared\n  setup:\n    - printf setup > setup\n",
    )?;
    let output = run_hook_pv(&["link"], &project, &home)?;
    let database = Database::open(&PvPaths::for_home(&home))?;
    assert_eq!(output.code, Some(0));
    assert!(state::fs::path_entry_exists(&project.join("prepared"))?);
    assert!(!state::fs::path_entry_exists(&project.join("setup"))?);
    let jobs = database
        .recent_jobs()?
        .into_iter()
        .map(|job| (job.kind, job.status, job.summary))
        .collect::<Vec<_>>();
    hook_snapshot(directory.path(), || assert_debug_snapshot!((output, jobs)));
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn project_link_prepare_failure_preserves_existing_state_and_skip_hooks_bypasses_execution()
-> Result<()> {
    let directory = tempdir()?;
    let home = directory.path().join("home");
    let project = directory.path().join("acme");
    create_dir(&project)?;
    create_dir(&home)?;
    let source = "serve: false\nhooks:\n  prepare:\n    - printf 'prepare failed\\n'; exit 7\n  setup:\n    - printf setup > setup\n";
    write_file(&project.join("pv.yml"), source)?;
    let first = run_hook_pv(&["link"], &project, &home)?;
    let paths = PvPaths::for_home(&home);
    assert!(Database::open(&paths)?.projects()?.is_empty());
    assert!(Database::open(&paths)?.recent_jobs()?.is_empty());
    let skipped = run_hook_pv(&["link", "--skip-hooks"], &project, &home)?;
    let before = Database::open(&paths)?.projects()?;
    write_file(&project.join("pv.yml"), &format!("php: 8.4\n{source}"))?;
    let relink = run_hook_pv(&["link"], &project, &home)?;
    let database = Database::open(&paths)?;
    assert_eq!(database.projects()?, before);
    assert!(!state::fs::path_entry_exists(&project.join("setup"))?);
    let jobs = database
        .recent_jobs()?
        .into_iter()
        .map(|job| (job.kind, job.status, job.error))
        .collect::<Vec<_>>();
    hook_snapshot(directory.path(), || {
        assert_debug_snapshot!((first, skipped, relink, jobs))
    });
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn project_link_refuses_queued_daemon_hooks_unless_skipped() -> Result<()> {
    let directory = tempdir()?;
    let home = directory.path().join("home");
    let project = directory.path().join("acme");
    create_dir(&project)?;
    create_dir(&home)?;
    write_file(
        &project.join("pv.yml"),
        "serve: false\nhooks:\n  prepare:\n    - printf ran > ran\n",
    )?;
    run_hook_pv(&["link", "--skip-hooks"], &project, &home)?;
    let paths = PvPaths::for_home(&home);
    let mut database = Database::open(&paths)?;
    let linked = database.projects()?.remove(0);
    let job = database.start_job("hooks", &format!("project:{}", linked.id))?;
    let refused = run_hook_pv(&["link"], &project, &home)?;
    write_file(&project.join("pv.yml"), "serve: false\n")?;
    let refused_after_config_edit = run_hook_pv(&["link"], &project, &home)?;
    let skipped = run_hook_pv(&["link", "--skip-hooks"], &project, &home)?;
    assert!(!state::fs::path_entry_exists(&project.join("ran"))?);
    assert_eq!(
        database.job(&job.id)?.map(|job| job.status),
        Some(JobStatus::Running)
    );
    assert_eq!(database.recent_jobs()?.len(), 1);
    hook_snapshot(directory.path(), || {
        assert_debug_snapshot!((refused, refused_after_config_edit, skipped))
    });
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn project_link_waits_on_the_accepted_coalesced_job_then_runs_setup() -> Result<()> {
    let directory = tempdir()?;
    let home = directory.path().join("home");
    let project = directory.path().join("acme");
    create_dir(&project)?;
    create_dir(&home)?;
    write_file(
        &project.join("pv.yml"),
        "serve: false\nhooks:\n  prepare:\n    - printf 'prepare output\\n'\n  setup:\n    - test -f .env && printf 'setup output\\n' && printf setup > setup\n",
    )?;
    let paths = PvPaths::for_home(&home);
    let server = fake_link_reconciliation(&paths, &project, LinkJobOutcome::Succeeded)?;
    let output = run_hook_pv(&["link"], &project, &home)?;
    let request = server
        .join()
        .map_err(|_| anyhow::anyhow!("fixture daemon panicked"))??;
    assert_eq!(request["requested_by_link"], true);
    assert_eq!(output.code, Some(0));
    assert!(state::fs::path_entry_exists(&project.join("setup"))?);
    let jobs = Database::open(&paths)?
        .recent_jobs()?
        .into_iter()
        .map(|job| (job.kind, job.status))
        .collect::<Vec<_>>();
    hook_snapshot(directory.path(), || assert_debug_snapshot!((output, jobs)));
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn project_link_setup_failure_keeps_the_link_and_env_and_records_failure() -> Result<()> {
    let directory = tempdir()?;
    let home = directory.path().join("home");
    let project = directory.path().join("acme");
    create_dir(&project)?;
    create_dir(&home)?;
    write_file(
        &project.join("pv.yml"),
        "serve: false\nhooks:\n  setup:\n    - printf 'setup failed\\n'; exit 9\n",
    )?;
    let paths = PvPaths::for_home(&home);
    let server = fake_link_reconciliation(&paths, &project, LinkJobOutcome::Succeeded)?;
    let output = run_hook_pv(&["link"], &project, &home)?;
    server
        .join()
        .map_err(|_| anyhow::anyhow!("fixture daemon panicked"))??;
    let database = Database::open(&paths)?;
    assert_ne!(output.code, Some(0));
    assert_eq!(database.projects()?.len(), 1);
    assert_eq!(
        state::fs::read_to_string(&project.join(".env"))?,
        "READY=yes\n"
    );
    let jobs = database
        .recent_jobs()?
        .into_iter()
        .map(|job| (job.kind, job.status, job.error))
        .collect::<Vec<_>>();
    hook_snapshot(directory.path(), || assert_debug_snapshot!((output, jobs)));
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn project_link_skipped_setup_does_not_clear_an_earlier_hook_failure() -> Result<()> {
    let directory = tempdir()?;
    let home = directory.path().join("home");
    let project = directory.path().join("acme");
    create_dir(&project)?;
    create_dir(&home)?;
    write_file(
        &project.join("pv.yml"),
        "serve: false\nhooks:\n  prepare:\n    - printf prepared > prepared\n  setup:\n    - exit 9\n",
    )?;
    let paths = PvPaths::for_home(&home);
    let server = fake_link_reconciliation(&paths, &project, LinkJobOutcome::Succeeded)?;
    assert_ne!(run_hook_pv(&["link"], &project, &home)?.code, Some(0));
    server
        .join()
        .map_err(|_| anyhow::anyhow!("fixture daemon panicked"))??;
    state::fs::remove_file_if_exists(&paths.daemon_socket())?;
    let output = run_hook_pv(&["link"], &project, &home)?;
    assert_eq!(output.code, Some(0));
    let database = Database::open(&paths)?;
    let failures = database.unresolved_job_failures()?;
    assert_eq!(failures.len(), 1);
    let failure = &failures[0].job;
    let jobs = database
        .recent_jobs()?
        .into_iter()
        .map(|job| (job.kind, job.status, job.summary))
        .collect::<Vec<_>>();
    hook_snapshot(directory.path(), || {
        assert_debug_snapshot!((output, jobs, &failure.error))
    });
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn project_link_records_ctrl_c_during_prepare_and_reconciliation_retries() -> Result<()> {
    let directory = tempdir()?;
    let mut outputs = Vec::new();
    for phase in ["prepare", "reconciliation", "setup_only_submission"] {
        let home = directory.path().join(phase).join("home");
        let project = directory.path().join(phase).join("acme");
        create_dir(&project)?;
        create_dir(&home)?;
        write_file(&project.join("pv.yml"), "serve: false\n")?;
        run_hook_pv(&["link", "--skip-hooks"], &project, &home)?;
        let command = if phase == "prepare" {
            "touch ready; count=0; while test \"$count\" -lt 1500; do sleep 0.1; count=$((count + 1)); done"
        } else {
            "exit 0"
        };
        let hooks = if phase == "setup_only_submission" {
            "serve: false\nhooks:\n  setup:\n    - printf setup > setup\n".to_string()
        } else {
            format!("serve: false\nhooks:\n  prepare:\n    - {command}\n")
        };
        write_file(&project.join("pv.yml"), &hooks)?;
        let paths = PvPaths::for_home(&home);
        let retry_server = if phase != "prepare" {
            Some(reject_link_reconciliations(&paths, &project.join("ready"))?)
        } else {
            None
        };
        let mut child = HookCliProcess(
            HookTestCommand::new(env!("CARGO_BIN_EXE_pv"))
                .arg("link")
                .env("HOME", &home)
                .env_remove("NO_COLOR")
                .current_dir(&project)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?,
        );
        wait_for_hook_fixture(&project.join("ready"))?;
        let signal = HookTestCommand::new("/bin/kill")
            .args(["-INT", &child.0.id().to_string()])
            .status()?;
        anyhow::ensure!(signal.success(), "could not interrupt owned CLI child");
        let output = child.output()?;
        if let Some((stop, server)) = retry_server {
            stop.store(true, Ordering::SeqCst);
            server
                .join()
                .map_err(|_| anyhow::anyhow!("fixture daemon panicked"))??;
        }
        assert_ne!(output.code, Some(0));
        let jobs = Database::open(&paths)?
            .recent_jobs()?
            .into_iter()
            .map(|job| (job.kind, job.status, job.error))
            .collect::<Vec<_>>();
        outputs.push((phase, output, jobs));
    }
    hook_snapshot(directory.path(), || assert_debug_snapshot!(outputs));
    Ok(())
}

#[cfg(target_os = "macos")]
#[expect(
    clippy::disallowed_types,
    reason = "hook CLI integration tests own and interrupt a directly spawned child"
)]
type HookTestCommand = std::process::Command;

#[cfg(target_os = "macos")]
struct HookCliProcess(Child);

#[cfg(target_os = "macos")]
impl HookCliProcess {
    fn output(&mut self) -> Result<CommandOutput> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = self.0.try_wait()? {
                break status;
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "owned CLI did not stop after Ctrl-C"
            );
            thread::sleep(Duration::from_millis(10));
        };
        let mut stdout = String::new();
        let mut stderr = String::new();
        if let Some(mut pipe) = self.0.stdout.take() {
            pipe.read_to_string(&mut stdout)?;
        }
        if let Some(mut pipe) = self.0.stderr.take() {
            pipe.read_to_string(&mut stderr)?;
        }
        Ok(CommandOutput {
            code: status_code(status),
            stdout,
            stderr,
        })
    }
}

#[cfg(target_os = "macos")]
impl Drop for HookCliProcess {
    fn drop(&mut self) {
        let _kill_result = self.0.kill();
        let _wait_result = self.0.wait();
    }
}

#[cfg(target_os = "macos")]
fn wait_for_hook_fixture(path: &Utf8Path) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !state::fs::path_entry_exists(path)? {
        anyhow::ensure!(Instant::now() < deadline, "hook fixture never became ready");
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn reject_link_reconciliations(
    paths: &PvPaths,
    ready: &Utf8Path,
) -> Result<(Arc<AtomicBool>, JoinHandle<Result<()>>)> {
    let listener = UnixListener::bind(paths.daemon_socket())?;
    listener.set_nonblocking(true)?;
    let stop = Arc::new(AtomicBool::new(false));
    let task_stop = Arc::clone(&stop);
    let paths = paths.clone();
    let ready = ready.to_path_buf();
    let task = thread::Builder::new().name("link-retry-fixture".to_string()).spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !task_stop.load(Ordering::SeqCst) {
            anyhow::ensure!(Instant::now() < deadline, "retry fixture timed out");
            match listener.accept() {
                Ok((mut socket, _)) => {
                    socket.set_nonblocking(false)?;
                    socket.set_read_timeout(Some(Duration::from_secs(5)))?;
                    socket.set_write_timeout(Some(Duration::from_secs(5)))?;
                    let mut line = String::new();
                    BufReader::new(socket.try_clone()?).read_line(&mut line)?;
                    let request: Value = serde_json::from_str(&line)?;
                    writeln!(socket, "{}", json!({ "type": "response", "protocol_version": request["protocol_version"], "status": "error", "message": format!("coordination lock held: {}", paths.jobs_lock()) }))?;
                    write_file(&ready, "ready")?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(10)),
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    })?;
    Ok((stop, task))
}

#[cfg(target_os = "macos")]
#[test]
fn project_link_skips_setup_after_failed_or_uncovered_reconciliation() -> Result<()> {
    let directory = tempdir()?;
    let mut outputs = Vec::new();
    for (name, outcome) in [
        ("failed", LinkJobOutcome::Failed),
        ("uncovered", LinkJobOutcome::Uncovered),
    ] {
        let home = directory.path().join(name).join("home");
        let project = directory.path().join(name).join("acme");
        create_dir(&project)?;
        create_dir(&home)?;
        write_file(
            &project.join("pv.yml"),
            "serve: false\nhooks:\n  setup:\n    - printf setup > setup\n",
        )?;
        let paths = PvPaths::for_home(&home);
        let server = fake_link_reconciliation(&paths, &project, outcome)?;
        let output = run_hook_pv(&["link"], &project, &home)?;
        server
            .join()
            .map_err(|_| anyhow::anyhow!("fixture daemon panicked"))??;
        assert_ne!(output.code, Some(0));
        assert!(!state::fs::path_entry_exists(&project.join("setup"))?);
        outputs.push(output);
    }
    hook_snapshot(directory.path(), || assert_debug_snapshot!(outputs));
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn project_link_without_setup_returns_while_the_accepted_job_is_running() -> Result<()> {
    let directory = tempdir()?;
    let home = directory.path().join("home");
    let project = directory.path().join("acme");
    create_dir(&project)?;
    create_dir(&home)?;
    write_file(&project.join("pv.yml"), "serve: false\n")?;
    let paths = PvPaths::for_home(&home);
    let server = fake_link_reconciliation(&paths, &project, LinkJobOutcome::Running)?;
    let output = run_hook_pv(&["link"], &project, &home)?;
    server
        .join()
        .map_err(|_| anyhow::anyhow!("fixture daemon panicked"))??;
    assert_eq!(output.code, Some(0));
    assert_eq!(
        Database::open(&paths)?
            .job("job_000001")?
            .map(|job| job.status),
        Some(JobStatus::Running)
    );
    hook_snapshot(directory.path(), || assert_debug_snapshot!(output));
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn project_link_releases_admission_for_prepare_and_stops_if_reacquisition_fails() -> Result<()> {
    let directory = tempdir()?;
    let mut outputs = Vec::new();
    for exit_status in [0, 7] {
        let home = directory.path().join(exit_status.to_string()).join("home");
        let project = directory.path().join(exit_status.to_string()).join("acme");
        create_dir(&project)?;
        create_dir(&home)?;
        write_file(
            &project.join("pv.yml"),
            &format!(
                "serve: false\nhooks:\n  prepare:\n    - touch started; count=0; while test ! -f continue && test \"$count\" -lt 1500; do sleep 0.1; count=$((count + 1)); done; exit {exit_status}\n"
            ),
        )?;
        let task_home = home.clone();
        let task_project = project.clone();
        let command = thread::Builder::new()
            .name("link-hook-lifecycle".to_string())
            .spawn(move || run_hook_pv(&["link"], &task_project, &task_home))?;
        wait_for_hook_fixture(&project.join("started"))?;
        let paths = PvPaths::for_home(&home);
        let exclusive = RuntimeLifecycleLock::acquire_exclusive(&paths)?;
        write_file(&project.join("continue"), "go")?;
        let output = command
            .join()
            .map_err(|_| anyhow::anyhow!("fixture CLI panicked"))??;
        drop(exclusive);
        assert_ne!(output.code, Some(0));
        let database = Database::open(&paths)?;
        assert!(database.projects()?.is_empty());
        assert!(database.recent_jobs()?.is_empty());
        outputs.push(output);
    }
    hook_snapshot(directory.path(), || assert_debug_snapshot!(outputs));
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_hook_pv(args: &[&str], project: &Utf8Path, home: &Utf8Path) -> Result<CommandOutput> {
    let output = Command::cargo_bin("pv")?
        .env("HOME", home)
        .env_remove("NO_COLOR")
        .current_dir(project)
        .timeout(Duration::from_secs(20))
        .args(args)
        .output()?;
    Ok(CommandOutput {
        code: status_code(output.status),
        stdout: String::from_utf8(output.stdout)?,
        stderr: String::from_utf8(output.stderr)?,
    })
}

#[cfg(target_os = "macos")]
fn hook_snapshot(directory: &Utf8Path, assert: impl FnOnce()) {
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(directory.as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(assert);
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
enum LinkJobOutcome {
    Succeeded,
    Failed,
    Uncovered,
    Running,
}

#[cfg(target_os = "macos")]
fn fake_link_reconciliation(
    paths: &PvPaths,
    project: &Utf8Path,
    outcome: LinkJobOutcome,
) -> Result<JoinHandle<Result<Value>>> {
    let job = Database::open(paths)?.start_job("reconcile", "system")?;
    let listener = UnixListener::bind(paths.daemon_socket())?;
    listener.set_nonblocking(true)?;
    let paths = paths.clone();
    let project = project.to_path_buf();
    Ok(thread::Builder::new().name("link-reconciliation-fixture".to_string()).spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                Err(error) => return Err(error.into()),
            }
        };
        socket.set_nonblocking(false)?;
        socket.set_read_timeout(Some(Duration::from_secs(5)))?;
        socket.set_write_timeout(Some(Duration::from_secs(5)))?;
        let mut line = String::new();
        BufReader::new(socket.try_clone()?).read_line(&mut line)?;
        let request: Value = serde_json::from_str(&line)?;
        writeln!(socket, "{}", json!({ "type": "response", "protocol_version": request["protocol_version"], "status": "accepted", "message": "reconciliation already queued or running", "job_id": job.id }))?;
        if !matches!(outcome, LinkJobOutcome::Running) {
            thread::sleep(Duration::from_millis(50));
            let mut database = Database::open(&paths)?;
            let linked = database.project_by_path(&state::fs::canonicalize(&project)?)?.ok_or_else(|| anyhow::anyhow!("Project not recorded before reconciliation"))?;
            match outcome {
                LinkJobOutcome::Failed => database.fail_job(&job.id, "fixture reconciliation failed")?,
                LinkJobOutcome::Succeeded => {
                    write_file(&project.join(".env"), "READY=yes\n")?;
                    database.complete_job_with_coverage(&job.id, "reconciled", &[JobDiagnosticSubject::Project { id: linked.id }])?;
                }
                LinkJobOutcome::Uncovered => database.complete_job(&job.id, "unrelated Project reconciled")?,
                LinkJobOutcome::Running => {}
            }
        }
        Ok(request)
    })?)
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
    assert!(!path_exists(&project.join("pv.yaml"))?);
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
fn project_init_print_succeeds_without_writing_during_update() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("Acme Store");
    create_laravel_init_fixture(&project)?;
    let paths = PvPaths::for_home(home.clone());
    state::fs::ensure_layout(&paths)?;
    let _update_lock = UpdateLock::acquire(&paths)?;

    let output = run_pv_in_dir_with_home(&["init", "--print"], &project, &home)?;

    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!(output);
        Ok::<(), anyhow::Error>(())
    })?;
    assert_eq!(output.code, Some(0));
    assert!(output.stderr.is_empty());
    assert!(!path_exists(&project.join("pv.yml"))?);
    assert!(!path_exists(&project.join("pv.yaml"))?);

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

    assert!(!path_exists(&project.join("pv.yml"))?);
    assert!(!path_exists(&project.join("pv.yaml"))?);
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
fn project_list_shows_project_and_mode_for_served_and_resource_only_projects() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let served = tempdir.path().join("Zulu Web");
    let resource_only = tempdir.path().join("Beta Data");
    create_dir(&served)?;
    create_dir(&resource_only)?;
    write_file(&resource_only.join("pv.yml"), "serve: false\n")?;

    let link_served = run_pv_in_dir_with_home(&["link", "--hostname", "alpha"], &served, &home)?;
    let link_resource_only = run_pv_in_dir_with_home(&["link"], &resource_only, &home)?;
    let list = run_pv_in_dir_with_home(&["list"], &resource_only, &home)?;

    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((link_served, link_resource_only, list));
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

#[test]
fn project_list_reports_removed_hostnames_config_key() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("Acme Store");
    create_dir(&project)?;
    write_file(&project.join("pv.yml"), "php: 8.4\n")?;

    let link = run_pv_in_dir_with_home(&["link"], &project, &home)?;
    write_file(
        &project.join("pv.yml"),
        "php: 8.4\nhostnames:\n  - api.acme-store.test\n",
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
    write_file(&project.join("pv.yml"), "env:\n  APP_URL: \"${url}\"\n")?;

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
    write_file(&project.join("pv.yml"), "env:\n  APP_URL: \"${url}\"\n")?;

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
    write_file(&project.join("pv.yml"), "env:\n  APP_URL: \"${url}\"\n")?;
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
    reason = "CLI integration tests read fixture files"
)]
fn read_file(path: &Utf8Path) -> Result<String> {
    Ok(std::fs::read_to_string(path)?)
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
