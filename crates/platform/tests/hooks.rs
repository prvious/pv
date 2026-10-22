#![cfg(unix)]

#[cfg(target_os = "macos")]
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
#[cfg(target_os = "macos")]
use std::thread;
use std::time::Duration;

use anyhow::{Result, anyhow};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use insta::{assert_debug_snapshot, assert_snapshot};
use platform::hooks::{
    HookCommandContext, HookEnvironment, HookError, HookOutput, capture_environment, run_commands,
};
#[cfg(target_os = "macos")]
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use state::{PvPaths, fs};
use tokio::sync::watch;
use tokio::time::{Instant, sleep, timeout};

#[tokio::test]
async fn hook_capture_ignores_startup_noise_and_prompt_and_applies_overlay() -> Result<()> {
    let temporary = tempdir()?;
    let paths = PvPaths::for_home(temporary.path().join("home with spaces"));
    let project = temporary.path().join("project");
    let source = temporary.path().join("main checkout");
    fs::ensure_user_dir(paths.home())?;
    fs::ensure_user_dir(&project)?;
    let shell = fake_shell(
        temporary.path(),
        &format!(
            r#"
test "$1:$2:$3" = '-i:-l:-c' || exit 91
printf '%s' "$PWD" > "$PV_HOOK_PROJECT/capture-start"
printf 'Welcome to your shell\n'
printf 'Startup prompt: ' >&2
read -r answer || :
export MULTILINE='first line
second=line'
export OLDPWD=/stale
export PV_SOURCE_PATH=/stale-source
export PATH={path}
/bin/sh -c "$4"
printf 'Goodbye\n'
"#,
            path = quote(&format!(
                "{}:/usr/bin:{}:/bin",
                paths.bin(),
                paths.composer().join("vendor/bin")
            )),
        ),
    )?;
    let (_sender, mut shutdown) = watch::channel(false);
    let environment =
        capture_environment(shell.as_std_path(), paths.home(), &project, &mut shutdown).await?;
    let capture_start = fs::read_to_string(&project.join("capture-start"))?;
    assert_eq!(
        fs::canonicalize(Utf8Path::new(&capture_start))?,
        fs::canonicalize(paths.home())?
    );
    let log = temporary.path().join("hook.log");
    let commands = vec![
        r#"if read -r answer; then exit 91; fi; printf 'hook stderr\n' >&2"#.to_string(),
        r#"printf '%s\n' "$PATH" "$COMPOSER_HOME" "$COMPOSER_CACHE_DIR" "$PV_SOURCE_PATH" "$MULTILINE" "${OLDPWD-unset}" "${PV_HOOK_PROJECT-unset}""#.to_string(),
    ];
    run_commands(
        &HookCommandContext {
            paths: &paths,
            project_path: &project,
            source_path: Some(&source),
            environment: &environment,
            output: HookOutput::Log(&log),
        },
        "prepare",
        &commands,
        &mut shutdown,
        |_| Err(std::io::Error::other("log output must not use terminal")),
    )
    .await?;
    let content = fs::read_to_string(&log)?;
    insta::with_settings!({filters => vec![(temporary.path().as_str(), "[TEMP]")]}, {
        assert_snapshot!(content);
    });
    assert_eq!(file_mode(&log)?, 0o600);
    assert_eq!(
        file_mode(log.parent().ok_or_else(|| anyhow!("missing log parent"))?)?,
        0o700
    );

    run_commands(
        &HookCommandContext {
            paths: &paths,
            project_path: &project,
            source_path: None,
            environment: &environment,
            output: HookOutput::Log(&log),
        },
        "setup",
        &[r#"printf '%s\n' "${PV_SOURCE_PATH-unset}""#.to_string()],
        &mut shutdown,
        |_| Ok(()),
    )
    .await?;
    assert_snapshot!(
        "hook_ordinary_project_unsets_source_and_replaces_log",
        fs::read_to_string(&log)?
    );
    Ok(())
}

#[tokio::test]
async fn hook_capture_preserves_marker_text_inside_values_and_later_entries() -> Result<()> {
    let temporary = tempdir()?;
    let paths = PvPaths::for_home(temporary.path());
    let shell = fake_shell(
        temporary.path(),
        "printf '__PV_HOOK_ENV_START__\\0FIRST=prefix__PV_HOOK_ENV_END__\\0AFTER=retained\\0__PV_HOOK_ENV_END__\\0'",
    )?;
    let (_sender, mut shutdown) = watch::channel(false);
    let environment = capture_environment(
        shell.as_std_path(),
        paths.home(),
        temporary.path(),
        &mut shutdown,
    )
    .await?;
    let log = temporary.path().join("hook.log");
    run_commands(
        &HookCommandContext {
            paths: &paths,
            project_path: temporary.path(),
            source_path: None,
            environment: &environment,
            output: HookOutput::Log(&log),
        },
        "prepare",
        &[r#"printf '%s\n' "$FIRST" "$AFTER""#.to_string()],
        &mut shutdown,
        |_| Ok(()),
    )
    .await?;
    assert_snapshot!(fs::read_to_string(&log)?);
    Ok(())
}

#[tokio::test]
async fn hook_capture_does_not_wait_for_background_jobs_holding_output() -> Result<()> {
    let temporary = tempdir()?;
    let shell = fake_shell(
        temporary.path(),
        r#"/bin/sleep 150 & descendant=$!; printf '%s' "$descendant" > "$PV_HOOK_PROJECT/pid"; /bin/sh -c "$4""#,
    )?;
    let (_sender, mut shutdown) = watch::channel(false);
    timeout(
        Duration::from_secs(5),
        capture_environment(
            shell.as_std_path(),
            temporary.path(),
            temporary.path(),
            &mut shutdown,
        ),
    )
    .await??;
    assert_stopped(fs::read_to_string(&temporary.path().join("pid"))?.parse::<u32>()?).await?;
    Ok(())
}

#[tokio::test]
async fn hook_capture_discards_output_written_after_the_end_marker() -> Result<()> {
    let temporary = tempdir()?;
    // More than a pipe buffer after the marker, like a verbose exit hook.
    let shell = fake_shell(
        temporary.path(),
        r#"/bin/sh -c "$4" || exit; /bin/dd if=/dev/zero bs=65536 count=4 2>/dev/null"#,
    )?;
    let (_sender, mut shutdown) = watch::channel(false);
    timeout(
        Duration::from_secs(5),
        capture_environment(
            shell.as_std_path(),
            temporary.path(),
            temporary.path(),
            &mut shutdown,
        ),
    )
    .await??;
    Ok(())
}

#[tokio::test]
async fn hook_capture_rejects_nonzero_exit_and_missing_markers() -> Result<()> {
    let temporary = tempdir()?;
    let (_sender, mut shutdown) = watch::channel(false);
    let mut failures = Vec::new();
    for script in [
        "printf 'shell startup failed' >&2; exit 9",
        "printf 'banner without environment'",
        "printf '__PV_HOOK_ENV_START__\\0PATH=/bin\\0'",
        "printf '__PV_HOOK_ENV_START__\\0private-value-without-equals\\0__PV_HOOK_ENV_END__\\0'",
    ] {
        let shell = fake_shell(temporary.path(), script)?;
        let result = capture_environment(
            shell.as_std_path(),
            temporary.path(),
            temporary.path(),
            &mut shutdown,
        )
        .await;
        let Err(HookError::Environment { reason, stderr }) = result else {
            return Err(anyhow!("expected shell environment failure"));
        };
        failures.push((reason, stderr));
    }
    assert_debug_snapshot!(failures);
    Ok(())
}

#[tokio::test]
async fn hook_capture_times_out_shell_startup() -> Result<()> {
    let temporary = tempdir()?;
    let shell = fake_shell(
        temporary.path(),
        r#"/bin/sleep 150 & descendant=$!; printf '%s' "$descendant" > "$PV_HOOK_PROJECT/pid.tmp"; mv "$PV_HOOK_PROJECT/pid.tmp" "$PV_HOOK_PROJECT/pid"; wait "$descendant""#,
    )?;
    let (_sender, mut shutdown) = watch::channel(false);
    let started = Instant::now();
    let result = capture_environment(
        shell.as_std_path(),
        temporary.path(),
        temporary.path(),
        &mut shutdown,
    )
    .await;
    let Err(error @ HookError::Environment { .. }) = result else {
        return Err(anyhow!("expected environment timeout"));
    };
    assert_snapshot!(error.to_string());
    assert!(started.elapsed() < Duration::from_secs(20));
    assert_stopped(fs::read_to_string(&temporary.path().join("pid"))?.parse::<u32>()?).await?;
    Ok(())
}

#[tokio::test]
async fn hook_commands_fail_fast_and_announce_in_order() -> Result<()> {
    let temporary = tempdir()?;
    let paths = PvPaths::for_home(temporary.path());
    let (_sender, mut shutdown) = watch::channel(false);
    let environment = test_environment(temporary.path(), &mut shutdown).await?;
    let mut announcements = Vec::new();
    let result = run_commands(
        &HookCommandContext {
            paths: &paths,
            project_path: temporary.path(),
            source_path: None,
            environment: &environment,
            output: HookOutput::Terminal,
        },
        "setup",
        &[
            "printf first > order".to_string(),
            "printf second >> order; exit 7".to_string(),
            "printf third >> order".to_string(),
        ],
        &mut shutdown,
        |announcement| {
            announcements.push(announcement.to_string());
            Ok(())
        },
    )
    .await;
    let Err(error @ HookError::CommandFailed { .. }) = result else {
        return Err(anyhow!("expected failed hook command"));
    };
    assert_debug_snapshot!((
        announcements,
        error.to_string(),
        fs::read_to_string(&temporary.path().join("order"))?
    ));
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the PTY regression reexecutes its own test binary with an isolated fixture home"
)]
fn hooks_detach_the_login_shell_and_commands_under_a_terminal() -> Result<()> {
    const CHILD_HOME: &str = "PV_HOOK_PTY_CHILD_HOME";
    if let Some(home) = std::env::var_os(CHILD_HOME) {
        let home = Utf8PathBuf::from_path_buf(home.into())
            .map_err(|_| anyhow!("non-UTF-8 fixture home"))?;
        return tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(async {
                let (sender, mut shutdown) = watch::channel(false);
                let cancel = async {
                    let shell = wait_for_pid(&home.join("shell.pid")).await?;
                    let descendant = wait_for_pid(&home.join("descendant.pid")).await?;
                    sender.send(true)?;
                    Ok::<_, anyhow::Error>((shell, descendant))
                };
                let (result, processes) = timeout(Duration::from_secs(5), async {
                    tokio::join!(
                        capture_environment(
                            Utf8Path::new("/bin/zsh").as_std_path(),
                            &home,
                            &home,
                            &mut shutdown
                        ),
                        cancel
                    )
                })
                .await?;
                assert!(matches!(result, Err(HookError::Interrupted)));
                let (shell, descendant) = processes?;
                assert_stopped(shell).await?;
                assert_stopped(descendant).await?;
                let paths = PvPaths::for_home(&home);
                let (_sender, mut shutdown) = watch::channel(false);
                let environment = test_environment(&home, &mut shutdown).await?;
                let result = timeout(
                    Duration::from_secs(5),
                    run_commands(
                        &HookCommandContext {
                            paths: &paths,
                            project_path: &home,
                            source_path: None,
                            environment: &environment,
                            output: HookOutput::Terminal,
                        },
                        "setup",
                        &["read x < /dev/tty".to_string()],
                        &mut shutdown,
                        |_| Ok(()),
                    ),
                )
                .await?;
                assert!(matches!(result, Err(HookError::CommandFailed { .. })));
                Ok(())
            });
    }

    let temporary = tempdir()?;
    fs::write_sensitive_file(&temporary.path().join(".zshenv"), "unsetopt GLOBAL_RCS\n")?;
    fs::write_sensitive_file(
        &temporary.path().join(".zshrc"),
        "/bin/sleep 150 & descendant=$!\nprintf '%s' \"$$\" > \"$HOME/shell.tmp\"\nmv \"$HOME/shell.tmp\" \"$HOME/shell.pid\"\nprintf '%s' \"$descendant\" > \"$HOME/descendant.tmp\"\nmv \"$HOME/descendant.tmp\" \"$HOME/descendant.pid\"\nwait \"$descendant\"\n",
    )?;
    let pair = native_pty_system().openpty(PtySize::default())?;
    let mut command = CommandBuilder::new(std::env::current_exe()?);
    command.args([
        "--exact",
        "hooks_detach_the_login_shell_and_commands_under_a_terminal",
        "--nocapture",
    ]);
    command.env(CHILD_HOME, temporary.path().as_str());
    command.env("HOME", temporary.path().as_str());
    command.env("ZDOTDIR", temporary.path().as_str());
    let mut child = pair.slave.spawn_command(command)?;
    drop(pair.slave);
    // The PTY must be drained while the child runs, including while it exits.
    let mut reader = pair.master.try_clone_reader()?;
    let output = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Err(error) => break Err(error.into()),
            Ok(None) if Instant::now() >= deadline => {
                break Err(anyhow!("PTY capture regression timed out"));
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
        }
    };
    if outcome.is_err() {
        child.kill()?;
        child.wait()?;
    }
    let output = output
        .join()
        .map_err(|_| anyhow!("PTY reader panicked"))??;
    assert!(outcome?.success(), "{}", String::from_utf8_lossy(&output));
    Ok(())
}

#[tokio::test]
async fn hook_cancellation_stops_every_process_in_the_command_group() -> Result<()> {
    let temporary = tempdir()?;
    let paths = PvPaths::for_home(temporary.path());
    let (sender, mut shutdown) = watch::channel(false);
    let environment = test_environment(temporary.path(), &mut shutdown).await?;
    let log = temporary.path().join("hook.log");
    let context = HookCommandContext {
        paths: &paths,
        project_path: temporary.path(),
        source_path: None,
        environment: &environment,
        output: HookOutput::Log(&log),
    };
    let commands = vec!["trap '' TERM; /bin/sleep 150 & descendant=$!; printf '%s' \"$descendant\" > pid.tmp; mv pid.tmp pid; wait \"$descendant\"".to_string()];
    let cancel = async {
        let pid = wait_for_pid(&temporary.path().join("pid")).await?;
        sender.send(true)?;
        Ok::<_, anyhow::Error>(pid)
    };
    let (result, pid) = tokio::join!(
        run_commands(&context, "setup", &commands, &mut shutdown, |_| Ok(())),
        cancel
    );
    assert!(matches!(result, Err(HookError::Interrupted)));
    assert_stopped(pid?).await?;
    Ok(())
}

#[tokio::test]
async fn dropping_a_hook_run_stops_its_process_group() -> Result<()> {
    let temporary = tempdir()?;
    let paths = PvPaths::for_home(temporary.path());
    let (_sender, mut shutdown) = watch::channel(false);
    let environment = test_environment(temporary.path(), &mut shutdown).await?;
    let context = HookCommandContext {
        paths: &paths,
        project_path: temporary.path(),
        source_path: None,
        environment: &environment,
        output: HookOutput::Terminal,
    };
    let commands = vec!["/bin/sleep 150 & descendant=$!; printf '%s' \"$descendant\" > pid.tmp; mv pid.tmp pid; wait \"$descendant\"".to_string()];
    let mut running = Box::pin(run_commands(
        &context,
        "setup",
        &commands,
        &mut shutdown,
        |_| Ok(()),
    ));
    let pid_path = temporary.path().join("pid");
    let pid = tokio::select! {
        result = &mut running => return Err(anyhow!("hook exited before cancellation: {result:?}")),
        pid = wait_for_pid(&pid_path) => pid?,
    };
    drop(running);
    assert_stopped(pid).await?;
    Ok(())
}

async fn wait_for_pid(path: &Utf8Path) -> Result<u32> {
    timeout(Duration::from_secs(5), async {
        while !fs::path_entry_exists(path)? {
            sleep(Duration::from_millis(10)).await;
        }
        Ok::<_, anyhow::Error>(fs::read_to_string(path)?.parse::<u32>()?)
    })
    .await?
}

async fn assert_stopped(pid: u32) -> Result<()> {
    timeout(Duration::from_secs(5), async {
        loop {
            if process_stopped(pid)? {
                return Ok::<_, anyhow::Error>(());
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await?
}

fn process_stopped(pid: u32) -> Result<bool> {
    Ok(platform::inspect_process_start_identity(pid)?.is_none()
        || platform::process_is_zombie(pid)?)
}

async fn test_environment(
    directory: &Utf8Path,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<HookEnvironment> {
    let shell = fake_shell(directory, "exec /bin/sh -c \"$4\"")?;
    Ok(capture_environment(shell.as_std_path(), directory, directory, shutdown).await?)
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[expect(
    clippy::disallowed_methods,
    reason = "hook integration fixture marks a controlled fake login shell executable"
)]
fn fake_shell(directory: &Utf8Path, script: &str) -> Result<Utf8PathBuf> {
    let path = directory.join("shell");
    fs::write_sensitive_file(&path, &format!("#!/bin/sh\n{script}\n"))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    Ok(path)
}

#[expect(
    clippy::disallowed_methods,
    reason = "hook integration test inspects private fixture log permissions"
)]
fn file_mode(path: &Utf8Path) -> Result<u32> {
    Ok(std::fs::metadata(path)?.permissions().mode() & 0o777)
}
