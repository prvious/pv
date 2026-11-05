pub(crate) mod prompts;
pub(crate) mod resource_cli;
pub(crate) mod runtime;

use std::process::ExitCode;

use cli::{Environment, run_with_environment};

/// What `pv` printed and returned. The app version reads `<version>`, so
/// snapshots don't change with every release.
#[derive(Debug)]
pub(crate) struct RunOutput {
    pub(crate) exit_code: ExitCode,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

pub(crate) fn run_pv(args: &[&str], environment: &impl Environment) -> anyhow::Result<RunOutput> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let args = std::iter::once("pv").chain(args.iter().copied());
    let exit_code = run_with_environment(args, environment, &mut stdout, &mut stderr)?;

    Ok(RunOutput {
        exit_code,
        stdout: without_app_version(&String::from_utf8(stdout)?),
        stderr: without_app_version(&String::from_utf8(stderr)?),
    })
}

/// Report headings pad `v<version>` to the right edge, so the padding goes
/// with the version.
fn without_app_version(output: &str) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let heading_version = format!("v{version}");
    let mut normalized = String::new();
    let mut rest = output;
    while let Some((before, after)) = rest.split_once(&heading_version) {
        normalized.push_str(before.trim_end_matches(' '));
        normalized.push_str(" v<version>");
        rest = after;
    }
    normalized.push_str(rest);

    normalized.replace(version, "<version>")
}
