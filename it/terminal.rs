//! Terminal ownership tests: run the real `pv` binary in a pseudo-terminal,
//! press keys, and snapshot the screen a user would see.

use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use insta::assert_snapshot;
use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};
use support::create_laravel_init_fixture;

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

/// A `pv` process attached to a pseudo-terminal, with its screen emulated.
struct Session {
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    output: Receiver<Vec<u8>>,
    parser: vt100::Parser,
}

impl Session {
    fn spawn(args: &[&str], current_dir: &Utf8Path, home: &Utf8Path) -> Result<Self> {
        let pair = native_pty_system().openpty(PtySize {
            rows: ROWS,
            cols: COLUMNS,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_pv"));
        command.args(args);
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
        })
    }

    /// Types `keys` after a short pause. A key that arrives before the
    /// prompt switches the terminal into raw mode would be echoed by the tty.
    fn press(&mut self, keys: &str) -> Result<()> {
        thread::sleep(KEY_DELAY);
        self.writer.write_all(keys.as_bytes())?;
        self.writer.flush()?;

        Ok(())
    }

    /// Waits until `text` is on screen. Wait for the last line of a frame
    /// before snapshotting, because frames can arrive in several reads.
    fn wait_for(&mut self, text: &str) -> Result<()> {
        let deadline = Instant::now() + TIMEOUT;
        while !self.contents().contains(text) {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                bail!(
                    "timed out waiting for {text:?}; screen:\n{}",
                    self.contents()
                );
            };
            match self.output.recv_timeout(remaining) {
                Ok(bytes) => self.parser.process(&bytes),
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
        let deadline = Instant::now() + TIMEOUT;
        let status = loop {
            if let Some(status) = self
                .child
                .try_wait()
                .context("checking whether pv exited")?
            {
                break status;
            }
            if Instant::now() >= deadline {
                bail!(
                    "timed out waiting for pv to exit; screen:\n{}",
                    self.contents()
                );
            }
            if let Ok(bytes) = self.output.recv_timeout(Duration::from_millis(20)) {
                self.parser.process(&bytes);
            }
        };
        while let Ok(bytes) = self.output.recv_timeout(Duration::from_millis(50)) {
            self.parser.process(&bytes);
        }

        Ok(status.exit_code())
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
