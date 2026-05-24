use std::process::ExitStatus;

use anyhow::Result;
use assert_cmd::Command;
use insta::assert_debug_snapshot;

#[derive(Debug)]
struct CommandOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_pv(args: &[&str]) -> Result<CommandOutput> {
    let output = Command::cargo_bin("pv")?.args(args).output()?;

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
    let CommandOutput {
        code,
        stdout,
        stderr,
    } = output;

    assert_debug_snapshot!(CommandOutput {
        code,
        stdout,
        stderr,
    });

    Ok(())
}
