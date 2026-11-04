use std::process::Output;

use anyhow::Result;
use assert_cmd::Command;
use camino::Utf8Path;

/// Runs the built `pv` in `current_dir` with `home` as its home, without a
/// terminal.
pub(crate) fn run_pv_in(args: &[&str], current_dir: &Utf8Path, home: &Utf8Path) -> Result<Output> {
    let mut command = Command::cargo_bin("pv")?;
    command.env_remove("NO_COLOR");
    command.env("HOME", home.as_str());
    command.current_dir(current_dir);

    Ok(command.args(args).output()?)
}
