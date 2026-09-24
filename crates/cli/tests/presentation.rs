//! Presentation contracts that hold for every command: machine output is
//! never decorated, raw payloads are identical on and off a terminal, color
//! follows `NO_COLOR` and `--no-color`, and stdout and stderr keep their
//! roles, each rendered for its own destination.

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::tempdir;
use cli::{Environment, run_with_environment};
use insta::{Settings, assert_debug_snapshot};

const ESCAPE: char = '\u{1b}';

/// Which streams are terminals, and whether `NO_COLOR` is set.
#[derive(Clone, Copy, Debug)]
struct Terminals {
    stdout: bool,
    stderr: bool,
    no_color_env: bool,
}

const PIPED: Terminals = Terminals {
    stdout: false,
    stderr: false,
    no_color_env: false,
};
const TERMINAL: Terminals = Terminals {
    stdout: true,
    stderr: true,
    no_color_env: false,
};

#[derive(Debug)]
struct TestEnvironment {
    home: PathBuf,
    current_dir: PathBuf,
    terminals: Terminals,
}

impl TestEnvironment {
    fn new(home: &Utf8Path, current_dir: &Utf8Path, terminals: Terminals) -> Self {
        Self {
            home: home.as_std_path().to_path_buf(),
            current_dir: current_dir.as_std_path().to_path_buf(),
            terminals,
        }
    }
}

impl Environment for TestEnvironment {
    fn var_os(&self, key: &str) -> Option<OsString> {
        (key == "NO_COLOR" && self.terminals.no_color_env).then(|| OsString::from("1"))
    }

    fn home_dir(&self) -> Option<PathBuf> {
        Some(self.home.clone())
    }

    fn current_dir(&self) -> io::Result<PathBuf> {
        Ok(self.current_dir.clone())
    }

    fn current_exe(&self) -> io::Result<PathBuf> {
        Ok(PathBuf::from("/bin/pv"))
    }

    fn stdout_is_terminal(&self) -> bool {
        self.terminals.stdout
    }

    fn stderr_is_terminal(&self) -> bool {
        self.terminals.stderr
    }

    fn terminal_width(&self) -> Option<usize> {
        (self.terminals.stdout || self.terminals.stderr).then_some(100)
    }

    fn stdin_is_terminal(&self) -> bool {
        false
    }

    fn open_url(&self, _url: &str) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
struct RunOutput {
    exit_code: ExitCode,
    stdout: String,
    stderr: String,
}

fn run_pv(args: &[&str], environment: &TestEnvironment) -> anyhow::Result<RunOutput> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let args = std::iter::once("pv").chain(args.iter().copied());
    let exit_code = run_with_environment(args, environment, &mut stdout, &mut stderr)?;

    Ok(RunOutput {
        exit_code,
        stdout: String::from_utf8(stdout)?,
        stderr: String::from_utf8(stderr)?,
    })
}

/// Links a Project whose `.env` already defines a mapped key outside the PV
/// block, so `pv project:env` warns on stderr, and returns the canonical path
/// commands resolve it from.
fn linked_project_with_env_warning(
    home: &Utf8Path,
    project: &Utf8Path,
) -> anyhow::Result<Utf8PathBuf> {
    create_dir(project)?;
    write_file(
        &project.join("pv.yml"),
        "env:\n  APP_URL: \"${project_url}\"\n",
    )?;
    write_file(
        &project.join(".env"),
        "APP_URL=https://user.test\nOTHER=value\n",
    )?;
    let environment = TestEnvironment::new(home, project, TERMINAL);
    let link = run_pv(&["link"], &environment)?;
    assert_eq!(link.exit_code, ExitCode::SUCCESS);
    // Commands resolve the Project from its canonical path, which `pv link`
    // recorded.
    let list = run_pv(&["list", "--json"], &environment)?;
    let projects = serde_json::from_str::<serde_json::Value>(&list.stdout)?;
    let Some(path) = projects["projects"][0]["path"].as_str() else {
        return Err(anyhow::anyhow!("expected the linked Project's path"));
    };

    Ok(Utf8PathBuf::from(path))
}

#[test]
fn json_output_is_valid_and_undecorated_on_a_color_terminal() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = linked_project_with_env_warning(&home, &tempdir.path().join("acme"))?;
    let environment = TestEnvironment::new(&home, &project, TERMINAL);

    for args in [
        ["list", "--json"],
        ["jobs", "--json"],
        ["php:list", "--json"],
        ["mysql:list", "--json"],
        ["project:env", "--json"],
    ] {
        let output = run_pv(&args, &environment)?;

        assert_eq!(output.exit_code, ExitCode::SUCCESS, "{args:?}");
        assert!(
            !output.stdout.contains(ESCAPE),
            "{args:?}: {}",
            output.stdout
        );
        serde_json::from_str::<serde_json::Value>(&output.stdout)?;
    }
    // Warnings stay on stderr, decorated for the terminal they reach.
    let env = run_pv(&["project:env", "--json"], &environment)?;
    assert!(env.stderr.contains("warning:"));
    assert!(env.stderr.contains(ESCAPE));

    Ok(())
}

#[test]
fn raw_payloads_are_byte_identical_on_and_off_a_terminal() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = linked_project_with_env_warning(&home, &tempdir.path().join("acme"))?;
    let terminal = TestEnvironment::new(&home, &project, TERMINAL);
    let piped = TestEnvironment::new(&home, &project, PIPED);

    for args in [
        &["env", "--shell", "zsh"][..],
        &["completions", "zsh"][..],
        &["init", "--print"][..],
        &["project:env"][..],
    ] {
        let on_terminal = run_pv(args, &terminal)?;
        let off_terminal = run_pv(args, &piped)?;

        assert_eq!(on_terminal.exit_code, ExitCode::SUCCESS, "{args:?}");
        assert!(!on_terminal.stdout.is_empty(), "{args:?}");
        assert_eq!(on_terminal.stdout, off_terminal.stdout, "{args:?}");
        assert!(!on_terminal.stdout.contains(ESCAPE), "{args:?}");
    }

    Ok(())
}

#[test]
fn color_follows_no_color_while_decoration_follows_the_terminal() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let color = TestEnvironment::new(&home, tempdir.path(), TERMINAL);
    let no_color_env = TestEnvironment::new(
        &home,
        tempdir.path(),
        Terminals {
            no_color_env: true,
            ..TERMINAL
        },
    );
    let piped = TestEnvironment::new(&home, tempdir.path(), PIPED);

    let colored = run_pv(&["jobs"], &color)?;
    let flag = run_pv(&["jobs", "--no-color"], &color)?;
    let env = run_pv(&["jobs"], &no_color_env)?;
    let plain = run_pv(&["jobs"], &piped)?;

    assert!(colored.stdout.contains(ESCAPE));
    for uncolored in [&flag, &env] {
        assert!(uncolored.stdout.contains('○'));
        assert!(!uncolored.stdout.contains(ESCAPE));
    }
    assert!(!plain.stdout.contains('○'));
    assert!(!plain.stdout.contains(ESCAPE));

    Ok(())
}

#[test]
fn each_stream_is_rendered_for_its_own_destination() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let stderr_only = TestEnvironment::new(
        &home,
        tempdir.path(),
        Terminals {
            stdout: false,
            ..TERMINAL
        },
    );
    let stdout_only = TestEnvironment::new(
        &home,
        tempdir.path(),
        Terminals {
            stderr: false,
            ..TERMINAL
        },
    );

    let piped_help = run_pv(&["--help"], &stderr_only)?;
    let terminal_usage_error = run_pv(&["--unknown-flag"], &stderr_only)?;
    let piped_usage_error = run_pv(&["--unknown-flag"], &stdout_only)?;
    let piped_error = run_pv(&["unlink"], &stdout_only)?;

    assert!(piped_help.stdout.starts_with("Laravel-first"));
    assert!(!piped_help.stdout.contains(ESCAPE));
    assert!(terminal_usage_error.stderr.contains(ESCAPE));
    assert!(!piped_usage_error.stderr.contains(ESCAPE));
    assert!(piped_error.stderr.starts_with("error: "));

    Ok(())
}

#[test]
fn stdout_carries_results_and_stderr_carries_diagnostics_on_a_terminal() -> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let home = tempdir.path().join("home");
    let project = tempdir.path().join("acme");
    create_dir(&project)?;
    write_file(&project.join("pv.yml"), "php: 8.4\n")?;
    let uncolored = Terminals {
        no_color_env: true,
        ..TERMINAL
    };

    let link = run_pv(&["link"], &TestEnvironment::new(&home, &project, uncolored))?;
    let unlink = run_pv(
        &["unlink"],
        &TestEnvironment::new(&home, tempdir.path(), uncolored),
    )?;

    let mut settings = Settings::clone_current();
    settings.add_filter(tempdir.path().as_str(), "<tempdir>");
    settings.add_filter("/private<tempdir>", "<tempdir>");
    settings.bind(|| {
        assert_debug_snapshot!((link, unlink));
    });

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI presentation tests create fixture directories"
)]
fn create_dir(path: &Utf8Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(path)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "CLI presentation tests write fixture config files"
)]
fn write_file(path: &Utf8Path, contents: &str) -> anyhow::Result<()> {
    std::fs::write(path, contents)?;

    Ok(())
}
