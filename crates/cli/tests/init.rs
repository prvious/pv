use std::ffi::OsString;
use std::io;
use std::iter::repeat_n;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::Utf8Path;
use camino_tempfile::tempdir;
use cli::{Answer, Environment, Prompt, run_with_environment};
use insta::{assert_debug_snapshot, assert_snapshot};

#[path = "support/prompts.rs"]
mod prompts;

use prompts::{ScriptedPrompts, Step};

/// Chooses Edit at "Use these selections?".
const EDIT: Step = Step::Answer(Answer::Selected(2));

#[derive(Debug)]
struct TestEnvironment {
    current_dir: PathBuf,
    terminal: bool,
    decorated: bool,
    prompts: ScriptedPrompts,
}

impl TestEnvironment {
    fn interactive(current_dir: &Utf8Path, steps: impl IntoIterator<Item = Step>) -> Self {
        Self {
            current_dir: current_dir.as_std_path().to_path_buf(),
            terminal: true,
            decorated: false,
            prompts: ScriptedPrompts::new(steps),
        }
    }

    fn non_interactive(current_dir: &Utf8Path) -> Self {
        Self {
            terminal: false,
            ..Self::interactive(current_dir, [])
        }
    }

    /// Stdout is an 80-column terminal, so rows render decorated.
    fn decorated(mut self) -> Self {
        self.decorated = true;
        self
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
        Ok(PathBuf::from("/bin/pv"))
    }

    fn stdin_is_terminal(&self) -> bool {
        self.terminal
    }

    fn stdout_is_terminal(&self) -> bool {
        self.decorated
    }

    fn stderr_is_terminal(&self) -> bool {
        self.terminal
    }

    fn terminal_width(&self) -> Option<usize> {
        Some(80)
    }

    fn read_line(&self) -> io::Result<String> {
        Ok(String::new())
    }

    fn prompt(&self, prompt: &Prompt<'_>) -> io::Result<Answer> {
        self.prompts.ask(prompt)
    }

    fn open_url(&self, _url: &str) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
struct Session {
    exit_code: ExitCode,
    stdout: String,
    stderr: String,
    prompts: Vec<String>,
}

#[test]
fn init_interactive_accepts_defaults_and_writes_config() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let project = laravel_project(tempdir.path())?;
    let environment = TestEnvironment::interactive(&project, [Step::Accept, Step::Accept]);

    let session = run_init(&[], &environment)?;

    assert_eq!(session.exit_code, ExitCode::SUCCESS);
    assert_session_snapshot(
        "init_interactive_accepts_defaults_and_writes_config",
        tempdir.path(),
        &session,
    );
    assert_snapshot!(
        "init_interactive_accepts_defaults_and_writes_config_config",
        read_file(&project.join("pv.yml"))?
    );

    Ok(())
}

#[test]
fn init_interactive_renders_the_decorated_flow_on_a_terminal() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let project = laravel_project(tempdir.path())?;
    let environment =
        TestEnvironment::interactive(&project, [Step::Accept, Step::Accept]).decorated();

    let session = run_init(&["--no-color"], &environment)?;

    assert_eq!(session.exit_code, ExitCode::SUCCESS);
    assert_session_snapshot(
        "init_interactive_renders_the_decorated_flow_on_a_terminal",
        tempdir.path(),
        &session,
    );

    Ok(())
}

#[test]
fn init_interactive_initial_cancel_leaves_new_project_unchanged() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let project = laravel_project(tempdir.path())?;
    let environment = TestEnvironment::interactive(&project, [Step::Answer(Answer::Selected(1))]);

    let session = run_init(&[], &environment)?;

    assert_eq!(session.exit_code, ExitCode::FAILURE);
    assert!(!path_exists(&project.join("pv.yml"))?);
    assert_session_snapshot(
        "init_interactive_initial_cancel_leaves_new_project_unchanged",
        tempdir.path(),
        &session,
    );

    Ok(())
}

#[test]
fn init_interactive_final_cancel_leaves_existing_config_unchanged() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let project = laravel_project(tempdir.path())?;
    let original = "php: 8.3\ndocument_root: public\nenv:\n  USER_VALUE: preserved\n";
    write_file(&project.join("pv.yml"), original)?;
    let environment = TestEnvironment::interactive(
        &project,
        [Step::Accept, Step::Answer(Answer::Confirmed(false))],
    );

    let session = run_init(&[], &environment)?;

    assert_eq!(session.exit_code, ExitCode::FAILURE);
    assert_eq!(read_file(&project.join("pv.yml"))?, original);
    assert_session_snapshot(
        "init_interactive_final_cancel_leaves_existing_config_unchanged",
        tempdir.path(),
        &session,
    );

    Ok(())
}

#[test]
fn init_escape_cancels_without_writing() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let project = laravel_project(tempdir.path())?;
    let environment = TestEnvironment::interactive(&project, [Step::Cancel]);

    let session = run_init(&[], &environment)?;

    assert_eq!(session.exit_code, ExitCode::from(130));
    assert!(session.stderr.is_empty());
    assert!(!path_exists(&project.join("pv.yml"))?);
    assert_session_snapshot(
        "init_escape_cancels_without_writing",
        tempdir.path(),
        &session,
    );

    Ok(())
}

#[test]
fn init_interactive_edits_php_resources_and_allocations() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let project = laravel_project(tempdir.path())?;
    let environment = TestEnvironment::interactive(
        &project,
        [
            EDIT,
            text("8.5"),
            text("public"),
            Step::Answer(Answer::SelectedMany(vec![0, 2, 3, 4])),
            text("latest"),
            text("app,analytics"),
            text("latest"),
            text("cache"),
            text("latest"),
            text("latest"),
            text("uploads"),
            Step::Accept,
        ],
    );

    let session = run_init(&[], &environment)?;

    assert_eq!(session.exit_code, ExitCode::SUCCESS);
    assert_session_snapshot(
        "init_interactive_edits_php_resources_and_allocations",
        tempdir.path(),
        &session,
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
    let project = laravel_project(tempdir.path())?;
    write_file(
        &project.join("pv.yml"),
        "php: 8.3\ndocument_root: public\nmysql:\n  version: 8.4\n  allocations:\n    primary: {}\nredis:\n  version: 7.2\n  allocations:\n    sessions: {}\nmailpit:\n  version: 1.0\nrustfs:\n  version: 1.1\n  allocations:\n    media: {}\n",
    )?;
    let steps = [EDIT].into_iter().chain(repeat_n(Step::Accept, 11));
    let environment = TestEnvironment::interactive(&project, steps);

    let session = run_init(&[], &environment)?;

    assert_eq!(session.exit_code, ExitCode::SUCCESS);
    assert_session_snapshot(
        "init_interactive_blank_edits_preserve_existing_defaults",
        tempdir.path(),
        &session,
    );
    assert_snapshot!(
        "init_interactive_blank_edits_preserve_existing_defaults_config",
        read_file(&project.join("pv.yml"))?
    );

    Ok(())
}

#[test]
fn init_interactive_explicit_allocation_edits_replace_existing_allocations() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let project = laravel_project(tempdir.path())?;
    write_file(
        &project.join("pv.yml"),
        "php: 8.3\ndocument_root: public\nmysql:\n  version: 8.4\n  allocations:\n    primary: {}\n",
    )?;
    let environment = TestEnvironment::interactive(&project, edit_mysql_allocations(&["app"]));

    let session = run_init(&[], &environment)?;

    assert_eq!(session.exit_code, ExitCode::SUCCESS);
    let config = read_file(&project.join("pv.yml"))?;
    assert_snapshot!(
        "init_interactive_explicit_allocation_edits_replace_existing_allocations_config",
        config
    );

    Ok(())
}

#[test]
fn init_interactive_explicit_allocation_edits_preserve_retained_allocation_config()
-> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let project = laravel_project(tempdir.path())?;
    write_file(
        &project.join("pv.yml"),
        "php: 8.3\ndocument_root: public\nmysql:\n  version: 8.4\n  allocations:\n    primary:\n      env:\n        CUSTOM_PRIMARY: preserved\n        DB_HOST: custom.internal\n    legacy:\n      env:\n        LEGACY_VALUE: remove-me\n",
    )?;
    let environment =
        TestEnvironment::interactive(&project, edit_mysql_allocations(&["primary,app"]));

    let session = run_init(&[], &environment)?;

    assert_eq!(session.exit_code, ExitCode::SUCCESS);
    let config = read_file(&project.join("pv.yml"))?;
    assert_snapshot!(
        "init_interactive_explicit_allocation_edits_preserve_retained_allocation_config_config",
        config
    );

    Ok(())
}

#[test]
fn init_interactive_rejects_invalid_allocation_names_and_asks_again() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let project = laravel_project(tempdir.path())?;
    let environment = TestEnvironment::interactive(
        &project,
        edit_mysql_allocations(&["App Db", "app,analytics"]),
    );

    let session = run_init(&[], &environment)?;

    assert_eq!(session.exit_code, ExitCode::SUCCESS);
    assert_debug_snapshot!(session.prompts);

    Ok(())
}

#[test]
fn init_without_a_terminal_refuses_with_rerun_flags() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let project = laravel_project(tempdir.path())?;
    let environment = TestEnvironment::non_interactive(&project);

    let session = run_init(&[], &environment)?;

    assert_eq!(session.exit_code, ExitCode::FAILURE);
    assert!(session.stdout.is_empty());
    assert!(!path_exists(&project.join("pv.yml"))?);
    assert_session_snapshot(
        "init_without_a_terminal_refuses_with_rerun_flags",
        tempdir.path(),
        &session,
    );

    Ok(())
}

#[test]
fn init_print_writes_only_the_generated_yaml() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let project = laravel_project(tempdir.path())?;
    let environment = TestEnvironment::non_interactive(&project).decorated();

    let session = run_init(&["--print"], &environment)?;

    assert_eq!(session.exit_code, ExitCode::SUCCESS);
    assert!(session.stderr.is_empty());
    assert!(!session.stdout.contains('\u{1b}'));
    assert!(!path_exists(&project.join("pv.yml"))?);
    assert_snapshot!(session.stdout);

    Ok(())
}

#[test]
fn init_yes_writes_detected_defaults_without_prompting() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let project = laravel_project(tempdir.path())?;
    let environment = TestEnvironment::non_interactive(&project);

    let session = run_init(&["--yes"], &environment)?;

    assert_eq!(session.exit_code, ExitCode::SUCCESS);
    assert!(path_exists(&project.join("pv.yml"))?);
    assert_session_snapshot(
        "init_yes_writes_detected_defaults_without_prompting",
        tempdir.path(),
        &session,
    );

    Ok(())
}

fn text(value: &str) -> Step {
    Step::Answer(Answer::Text(value.to_string()))
}

/// Chooses Edit and accepts every default except the MySQL allocations,
/// which get each of `answers` in turn.
fn edit_mysql_allocations(answers: &[&str]) -> Vec<Step> {
    [EDIT]
        .into_iter()
        .chain(repeat_n(Step::Accept, 4))
        .chain(answers.iter().map(|answer| text(answer)))
        .chain(repeat_n(Step::Accept, 6))
        .collect()
}

fn run_init(args: &[&str], environment: &TestEnvironment) -> anyhow::Result<Session> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let args = ["pv", "init"].into_iter().chain(args.iter().copied());
    let exit_code = run_with_environment(args, environment, &mut stdout, &mut stderr)?;

    Ok(Session {
        exit_code,
        stdout: String::from_utf8(stdout)?,
        stderr: String::from_utf8(stderr)?,
        prompts: environment.prompts.transcript(),
    })
}

fn assert_session_snapshot(name: &str, tempdir: &Utf8Path, session: &Session) {
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(tempdir.as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| assert_debug_snapshot!(name, session));
}

fn laravel_project(tempdir: &Utf8Path) -> anyhow::Result<camino::Utf8PathBuf> {
    let project = tempdir.join("acme");
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

    Ok(project)
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
