//! Terminal ownership tests: run the real `pv` binary in a pseudo-terminal,
//! press keys, and snapshot the screen a user would see.

use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use insta::assert_snapshot;
use platform::clock::{self, real};
use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};
use state::{Database, LinkProjectInput, ProjectMode, PvPaths};
use support::{create_dir, create_laravel_init_fixture, run_pv_in, write_file};

mod support;

const ROWS: u16 = 120;
const COLUMNS: u16 = 100;
const TIMEOUT: Duration = Duration::from_secs(15);
const KEY_DELAY: Duration = Duration::from_millis(50);

const ENTER: &str = "\r";
const ESCAPE: &str = "\u{1b}";
const CTRL_C: &str = "\u{3}";
const DOWN: &str = "\u{1b}[B";
const BACKSPACE: &str = "\u{7f}";

// Footer hints end each prompt frame, so waiting for them means the whole
// frame has been drawn.
const SELECT_HINT: &str = "↑↓ move · enter confirm";
const MULTISELECT_HINT: &str = "space toggle · enter confirm";
const CONFIRM_HINT: &str = "enter accepts the highlighted answer";

#[test]
fn init_prompts_accept_defaults_and_collapse_after_answers() -> Result<()> {
    let tempdir = tempdir()?;
    let project = laravel_project(tempdir.path())?;
    let mut session = Session::spawn(&["init"], &project, tempdir.path())?;

    session.wait_for(SELECT_HINT)?;
    assert_screen_snapshot("init_select_active", tempdir.path(), &session);

    session.press(ENTER)?;
    session.wait_for("Write Project config?")?;
    session.press("y")?;
    session.wait_for("VITE_DEV_SERVER_KEY.")?;
    let exit_code = session.wait_for_exit()?;

    assert_eq!(exit_code, 0);
    assert!(project.join("pv.yml").is_file());
    assert!(session.cursor_visible());
    assert_screen_snapshot("init_completed", tempdir.path(), &session);

    Ok(())
}

#[test]
fn init_edit_path_moves_toggles_validates_and_declines() -> Result<()> {
    let tempdir = tempdir()?;
    let project = laravel_project(tempdir.path())?;
    let mut session = Session::spawn(&["init"], &project, tempdir.path())?;

    session.wait_for(SELECT_HINT)?;
    session.press("j")?;
    session.press(DOWN)?;
    session.press(ENTER)?;
    session.wait_for("PHP track")?;
    session.press("8.5")?;
    session.press(ENTER)?;
    session.wait_for("Document root")?;
    session.press(ENTER)?;
    session.wait_for("Select Project resources")?;
    session.press("j")?;
    session.press(" ")?;
    session.wait_for("[x] Postgres")?;
    session.wait_for(MULTISELECT_HINT)?;
    assert_screen_snapshot("init_multiselect_active", tempdir.path(), &session);

    session.press(ENTER)?;
    session.wait_for("MySQL track")?;
    session.press(ENTER)?;
    session.wait_for("MySQL allocations")?;
    session.press("App")?;
    session.press(ENTER)?;
    session.wait_for("use only a-z, 0-9, _, or -")?;
    assert_screen_snapshot("init_text_validation_error", tempdir.path(), &session);

    session.press(&BACKSPACE.repeat(3))?;
    session.press(ENTER)?;
    session.wait_for("Postgres track")?;
    for label in [
        "Postgres allocations",
        "Redis track",
        "Redis allocations",
        "Mailpit track",
        "RustFS/S3 track",
        "RustFS/S3 allocations",
    ] {
        session.press(ENTER)?;
        session.wait_for(label)?;
    }
    session.press(ENTER)?;
    session.wait_for("Write Project config?")?;
    session.press("n")?;
    session.wait_for("pv init cancelled; no files changed.")?;
    let exit_code = session.wait_for_exit()?;

    assert_eq!(exit_code, 1);
    assert!(!project.join("pv.yml").exists());
    assert_screen_snapshot("init_edit_declined", tempdir.path(), &session);

    Ok(())
}

#[test]
fn init_escape_cancels_restores_the_cursor_and_exits_130() -> Result<()> {
    let tempdir = tempdir()?;
    let project = laravel_project(tempdir.path())?;
    let mut session = Session::spawn(&["init"], &project, tempdir.path())?;

    session.wait_for(SELECT_HINT)?;
    session.press(ESCAPE)?;
    session.wait_for("Cancelled.")?;
    let exit_code = session.wait_for_exit()?;

    assert_eq!(exit_code, 130);
    assert!(session.cursor_visible());
    assert!(!project.join("pv.yml").exists());
    assert!(!session.contents().contains("panicked"));
    assert_screen_snapshot("init_escape_cancelled", tempdir.path(), &session);

    Ok(())
}

#[test]
fn init_ctrl_c_cancels_like_escape() -> Result<()> {
    let tempdir = tempdir()?;
    let project = laravel_project(tempdir.path())?;
    let mut session = Session::spawn(&["init"], &project, tempdir.path())?;

    session.wait_for(SELECT_HINT)?;
    session.press(CTRL_C)?;
    session.wait_for("Cancelled.")?;
    let exit_code = session.wait_for_exit()?;

    assert_eq!(exit_code, 130);
    assert!(session.cursor_visible());

    Ok(())
}

#[test]
fn prune_confirmation_declines_with_n_or_the_default_answer() -> Result<()> {
    for (key, snapshot) in [
        ("n", "prune_declined_with_n"),
        (ENTER, "prune_declined_by_default"),
    ] {
        let tempdir = tempdir()?;
        let mut session = Session::spawn(
            &["redis:uninstall", "8.8", "--prune"],
            tempdir.path(),
            tempdir.path(),
        )?;

        session.wait_for(CONFIRM_HINT)?;
        session.press(key)?;
        session.wait_for("nothing changed")?;
        let exit_code = session.wait_for_exit()?;

        assert_eq!(exit_code, 0);
        assert!(session.cursor_visible());
        assert_screen_snapshot(snapshot, tempdir.path(), &session);
    }

    Ok(())
}

#[test]
fn link_shows_hooks_to_approve_when_stdout_is_redirected() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(
        &project.join("pv.yml"),
        "serve: false\nhooks:\n  prepare:\n    - touch prepared\n",
    )?;
    let mut command = CommandBuilder::new("/bin/sh");
    command.args(["-c", "exec \"$0\" link > out.log", env!("CARGO_BIN_EXE_pv")]);
    let mut session = Session::spawn_command(command, &project, &home)?;

    session.wait_for(CONFIRM_HINT)?;
    // The list is on the prompt's terminal, not in the redirected output.
    assert!(session.contents().contains("prepare  touch prepared"));
    session.press("n")?;

    assert_eq!(session.wait_for_exit()?, 0);
    assert!(!project.join("prepared").exists());

    Ok(())
}

#[test]
fn link_asks_once_to_approve_hooks_and_remembers_the_answer() -> Result<()> {
    for (key, approved) in [("y", true), ("n", false)] {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let project = tempdir.path().join("acme");
        create_dir(&project)?;
        write_file(
            &project.join("pv.yml"),
            "serve: false\nhooks:\n  prepare:\n    - touch prepared\n",
        )?;
        let mut session = Session::spawn(&["link"], &project, &home)?;

        session.wait_for(CONFIRM_HINT)?;
        if approved {
            assert_screen_snapshot("link_hook_approval_prompt", tempdir.path(), &session);
        }
        session.press(key)?;

        assert_eq!(session.wait_for_exit()?, 0);
        assert_eq!(project.join("prepared").exists(), approved);
        // Without a terminal, an approved command runs and a declined one is skipped again.
        let relink = run_pv_in(&["link"], &project, &home)?;
        assert!(relink.status.success());
        assert_eq!(
            String::from_utf8(relink.stdout)?.contains("Running prepare: touch prepared"),
            approved
        );
    }

    Ok(())
}

#[test]
fn hooks_trust_approves_exact_commands_without_running_or_linking() -> Result<()> {
    let mut settings = insta::Settings::clone_current();
    // macOS temporary paths can wrap this heading before the path is redacted.
    settings.add_filter(
        "Hook commands not approved for\n +",
        "Hook commands not approved for ",
    );
    let _settings = settings.bind_to_scope();
    let directory = tempdir()?;
    let home = directory.path().join("home");
    let project = directory.path().join("acme");
    create_dir(&project)?;
    let config = "serve: false\nhooks:\n  prepare:\n    - touch prepared\n  setup:\n    - \"touch set-up # \\e[2K\\rhidden\"\n";
    write_file(&project.join("pv.yml"), config)?;
    let mut session = Session::spawn(&["hooks:trust"], &project, &home)?;
    session.wait_for(CONFIRM_HINT)?;
    assert_screen_snapshot("hooks_trust_approval_prompt", directory.path(), &session);
    session.press("y")?;
    session.wait_for("no hooks were run")?;
    assert_eq!(session.wait_for_exit()?, 0);
    assert!(session.cursor_visible());
    let mut database = Database::open(&PvPaths::for_home(&home))?;
    let folder = state::fs::canonicalize(&project)?;
    let commands = vec![
        "touch prepared".to_owned(),
        "touch set-up # \u{1b}[2K\rhidden".to_owned(),
    ];
    assert!(
        database
            .unapproved_hook_commands(&folder, &commands)?
            .is_empty()
    );
    assert!(database.projects()?.is_empty());
    assert!(database.recent_jobs()?.is_empty());
    assert!(!project.join("prepared").exists());
    assert!(!project.join("set-up").exists());
    assert!(!project.join(".env").exists());
    assert_eq!(state::fs::read_to_string(&project.join("pv.yml"))?, config);

    // A linked Project can review an edit without changing its registration or running hooks.
    let linked = database
        .link_project_with_mode(
            LinkProjectInput {
                path: folder.clone(),
                original_path: folder.clone(),
                primary_hostname: "acme.test".to_owned(),
                config_path: folder.join("pv.yml"),
                desired_php_track: None,
            },
            ProjectMode::ResourceOnly,
        )?
        .project;
    database.record_finished_job(
        "hooks",
        &format!("project:{}", linked.id),
        Ok("setup skipped: hook commands aren't approved"),
        &[],
    )?;
    let jobs = database.recent_jobs()?;
    write_file(&project.join(".env"), "APP_KEY=existing-project-value\n")?;
    write_file(
        &project.join("pv.yml"),
        "serve: false\nhooks:\n  setup:\n    - touch prepared\n    - touch edited\n",
    )?;
    let mut edited = Session::spawn(&["hooks:trust"], &project, &home)?;
    edited.wait_for(CONFIRM_HINT)?;
    assert_screen_snapshot("hooks_trust_edited_command", directory.path(), &edited);
    edited.press("y")?;
    edited.wait_for("no hooks were run")?;
    assert_eq!(edited.wait_for_exit()?, 0);
    assert!(!project.join("edited").exists());
    assert_eq!(database.projects()?, vec![linked]);
    assert_eq!(database.recent_jobs()?, jobs);
    assert_eq!(
        state::fs::read_to_string(&project.join(".env"))?,
        "APP_KEY=existing-project-value\n"
    );

    // Removing/reordering commands or moving them between phases needs no approval.
    write_file(
        &project.join("pv.yml"),
        "serve: false\nhooks:\n  prepare:\n    - touch edited\n    - touch prepared\n",
    )?;
    let repeated = run_pv_in(&["hooks:trust"], &project, &home)?;
    assert!(repeated.status.success());
    assert!(repeated.stderr.is_empty());
    assert_eq!(
        String::from_utf8(repeated.stdout)?,
        "All Project hook commands are approved; no hooks were run\n"
    );
    Ok(())
}

#[test]
fn hooks_trust_decline_default_and_cancellation_save_no_approvals() -> Result<()> {
    for (key, exit_code) in [("n", 0), (ENTER, 0), (ESCAPE, 130), (CTRL_C, 130)] {
        let directory = tempdir()?;
        let home = directory.path().join("home");
        let project = directory.path().join("acme");
        create_dir(&project)?;
        write_file(
            &project.join("pv.yml"),
            "serve: false\nhooks:\n  prepare:\n    - touch prepared\n",
        )?;
        let mut session = Session::spawn(&["hooks:trust"], &project, &home)?;
        session.wait_for(CONFIRM_HINT)?;
        session.press(key)?;
        session.wait_for(if exit_code == 130 {
            "Cancelled."
        } else {
            "Hook approval skipped"
        })?;
        assert_eq!(session.wait_for_exit()?, exit_code);
        assert!(session.cursor_visible());
        let database = Database::open(&PvPaths::for_home(&home))?;
        let commands = vec!["touch prepared".to_owned()];
        assert_eq!(
            database.unapproved_hook_commands(&state::fs::canonicalize(&project)?, &commands)?,
            commands
        );
        assert!(database.projects()?.is_empty());
        assert!(database.recent_jobs()?.is_empty());
        assert!(!project.join("prepared").exists());
    }
    Ok(())
}

#[test]
fn hooks_trust_worktree_uses_its_main_checkout_project_folder() -> Result<()> {
    let directory = tempdir()?;
    let root = state::fs::canonicalize(directory.path())?;
    let home = root.join("home");
    let main = root.join("main");
    let worktree = root.join("elsewhere/feature");
    let entry = main.join(".git/worktrees/feature");
    create_dir(&entry)?;
    create_dir(&worktree)?;
    write_file(&entry.join("commondir"), "../..\n")?;
    write_file(&entry.join("gitdir"), &format!("{worktree}/.git\n"))?;
    write_file(&worktree.join(".git"), &format!("gitdir: {entry}\n"))?;
    let config = "serve: false\nhooks:\n  prepare:\n    - touch prepared\n";
    let main_project = main.join("apps/web");
    let worktree_project = worktree.join("apps/web");
    for project in [&main_project, &worktree_project] {
        create_dir(project)?;
        write_file(&project.join("pv.yml"), config)?;
    }
    let mut session = Session::spawn(&["hooks:trust"], &worktree_project, &home)?;
    session.wait_for(CONFIRM_HINT)?;
    session.press("y")?;
    session.wait_for("no hooks were run")?;
    assert_eq!(session.wait_for_exit()?, 0);
    let database = Database::open(&PvPaths::for_home(&home))?;
    let commands = vec!["touch prepared".to_owned()];
    assert!(
        database
            .unapproved_hook_commands(&main_project, &commands)?
            .is_empty()
    );
    assert_eq!(
        database.unapproved_hook_commands(&worktree_project, &commands)?,
        commands
    );
    assert_eq!(
        database.unapproved_hook_commands(&main, &commands)?,
        commands
    );
    let repeated = run_pv_in(&["hooks:trust"], &main_project, &home)?;
    assert!(repeated.status.success());
    assert!(repeated.stderr.is_empty());
    assert!(database.projects()?.is_empty());
    assert!(database.recent_jobs()?.is_empty());
    assert!(!main_project.join("prepared").exists());
    assert!(!worktree_project.join("prepared").exists());
    Ok(())
}

#[test]
fn open_picker_lists_served_projects_and_escape_cancels() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let outside = tempdir.path().join("outside");
    create_dir(&outside)?;
    for (directory, hostname) in [("zeta-project", "zeta"), ("alpha-project", "alpha")] {
        let project = tempdir.path().join(directory);
        create_dir(&project)?;
        let link = run_pv_in(&["link", "--hostname", hostname], &project, &home)?;
        assert!(link.status.success());
    }
    let mut session = Session::spawn(&["open"], &outside, &home)?;

    session.wait_for(SELECT_HINT)?;
    session.press("j")?;
    session.wait_for("● zeta.test")?;
    assert_screen_snapshot("open_picker_active", tempdir.path(), &session);

    session.press(ESCAPE)?;
    session.wait_for("Cancelled.")?;
    let exit_code = session.wait_for_exit()?;

    assert_eq!(exit_code, 130);
    assert!(session.cursor_visible());

    Ok(())
}

#[test]
fn linked_project_uses_full_https_url_and_grouped_list_report() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;

    let mut link = Session::spawn(&["link", "--hostname", "acme"], &project, &home)?;
    link.wait_for("Start it with `pv setup`")?;
    assert_eq!(link.wait_for_exit()?, 0);
    assert_screen_snapshot("link_full_https_url", tempdir.path(), &link);

    let mut list = Session::spawn(&["list", "--no-color"], &project, &home)?;
    list.wait_for("1 linked")?;
    assert_eq!(list.wait_for_exit()?, 0);
    assert_screen_snapshot("list_grouped_report", tempdir.path(), &list);

    Ok(())
}

#[test]
fn init_yes_reports_written_config_before_detection_details() -> Result<()> {
    let tempdir = tempdir()?;
    let project = laravel_project(tempdir.path())?;
    let mut session = Session::spawn(&["init", "--yes"], &project, tempdir.path())?;

    session.wait_for("VITE_DEV_SERVER_KEY.")?;
    assert_eq!(session.wait_for_exit()?, 0);
    assert!(project.join("pv.yml").is_file());
    assert_screen_snapshot("init_yes_result", tempdir.path(), &session);

    Ok(())
}

#[test]
fn json_on_a_real_terminal_parses_and_is_never_decorated() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");

    // `pv jobs --json` writes nothing to stderr, so the merged terminal
    // stream is exactly its stdout.
    let mut session = Session::spawn(&["jobs", "--json"], tempdir.path(), &home)?;
    session.wait_for("}")?;

    assert_eq!(session.wait_for_exit()?, 0);
    assert!(!session.raw().contains(ESCAPE));
    serde_json::from_str::<serde_json::Value>(&session.raw())?;

    Ok(())
}

#[test]
fn human_output_on_a_real_terminal_is_decorated_and_honors_no_color() -> Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");

    let mut colored = Session::spawn(&["jobs"], tempdir.path(), &home)?;
    colored.wait_for("No recent daemon jobs")?;
    let mut uncolored = Session::spawn(&["jobs", "--no-color"], tempdir.path(), &home)?;
    uncolored.wait_for("No recent daemon jobs")?;

    assert_eq!(colored.wait_for_exit()?, 0);
    assert_eq!(uncolored.wait_for_exit()?, 0);
    assert!(colored.raw().contains(ESCAPE));
    assert!(!uncolored.raw().contains(ESCAPE));
    assert!(uncolored.screen().starts_with('○'));

    Ok(())
}

/// A `pv` process attached to a pseudo-terminal, with its screen emulated.
struct Session {
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    output: Receiver<Vec<u8>>,
    parser: vt100::Parser,
    /// Every byte the terminal received, for escape-sequence assertions.
    raw: Vec<u8>,
}

impl Session {
    fn spawn(args: &[&str], current_dir: &Utf8Path, home: &Utf8Path) -> Result<Self> {
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_pv"));
        command.args(args);
        Self::spawn_command(command, current_dir, home)
    }

    fn spawn_command(
        mut command: CommandBuilder,
        current_dir: &Utf8Path,
        home: &Utf8Path,
    ) -> Result<Self> {
        let pair = native_pty_system().openpty(PtySize {
            rows: ROWS,
            cols: COLUMNS,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        command.cwd(current_dir.as_std_path());
        command.env("HOME", home.as_str());
        command.env("TERM", "xterm-256color");
        command.env_remove("NO_COLOR");
        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        Ok(Self {
            child,
            writer,
            output: spawn_reader(reader),
            parser: vt100::Parser::new(ROWS, COLUMNS, 0),
            raw: Vec::new(),
        })
    }

    /// Types `keys` after a short pause. A key that arrives before the
    /// prompt switches the terminal into raw mode would be echoed by the tty.
    fn press(&mut self, keys: &str) -> Result<()> {
        clock::poll_blocking(KEY_DELAY);
        self.writer.write_all(keys.as_bytes())?;
        self.writer.flush()?;

        Ok(())
    }

    /// Waits until `text` is on screen. Wait for the last line of a frame
    /// before snapshotting, because frames can arrive in several reads.
    fn wait_for(&mut self, text: &str) -> Result<()> {
        let deadline = real::now() + TIMEOUT;
        while !self.contents().contains(text) {
            let Some(remaining) = deadline.checked_duration_since(real::now()) else {
                bail!(
                    "timed out waiting for {text:?}; screen:\n{}",
                    self.contents()
                );
            };
            match self.output.recv_timeout(remaining) {
                Ok(bytes) => self.receive(bytes),
                Err(error) => {
                    bail!(
                        "{error} while waiting for {text:?}; screen:\n{}",
                        self.contents()
                    )
                }
            }
        }

        Ok(())
    }

    /// Waits for the exit status. Callers first wait for the final screen
    /// text, because a pseudo-terminal may drop output still buffered when
    /// the process exits.
    fn wait_for_exit(&mut self) -> Result<u32> {
        let deadline = real::now() + TIMEOUT;
        let status = loop {
            if let Some(status) = self
                .child
                .try_wait()
                .context("checking whether pv exited")?
            {
                break status;
            }
            if real::now() >= deadline {
                bail!(
                    "timed out waiting for pv to exit; screen:\n{}",
                    self.contents()
                );
            }
            if let Ok(bytes) = self.output.recv_timeout(Duration::from_millis(20)) {
                self.receive(bytes);
            }
        };
        while let Ok(bytes) = self.output.recv_timeout(Duration::from_millis(50)) {
            self.receive(bytes);
        }

        Ok(status.exit_code())
    }

    fn receive(&mut self, bytes: Vec<u8>) {
        self.parser.process(&bytes);
        self.raw.extend(bytes);
    }

    /// Everything the terminal received, including escape sequences.
    fn raw(&self) -> String {
        String::from_utf8_lossy(&self.raw).into_owned()
    }

    fn contents(&self) -> String {
        self.parser.screen().contents()
    }

    fn cursor_visible(&self) -> bool {
        !self.parser.screen().hide_cursor()
    }

    /// The visible screen with trailing blanks trimmed, for snapshots.
    fn screen(&self) -> String {
        let contents = self.contents();
        let lines = contents.lines().map(str::trim_end).collect::<Vec<_>>();

        lines.join("\n").trim_end().to_string()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _kill_result = self.child.kill();
    }
}

fn assert_screen_snapshot(name: &str, tempdir: &Utf8Path, session: &Session) {
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| assert_snapshot!(name, session.screen()));
}

#[expect(
    clippy::disallowed_methods,
    reason = "the PTY reader blocks, so terminal tests read it on a dedicated thread"
)]
fn spawn_reader(mut reader: Box<dyn Read + Send>) -> Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut buffer = [0; 4096];
        while let Ok(read) = reader.read(&mut buffer) {
            if read == 0 || sender.send(buffer[..read].to_vec()).is_err() {
                break;
            }
        }
    });

    receiver
}

fn laravel_project(tempdir: &Utf8Path) -> Result<Utf8PathBuf> {
    let project = tempdir.join("acme");
    create_laravel_init_fixture(&project)?;

    Ok(project)
}
