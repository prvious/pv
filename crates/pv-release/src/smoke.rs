use camino::Utf8Path;

#[expect(
    clippy::disallowed_types,
    reason = "PV release tooling owns explicit local smoke hook execution"
)]
type StdCommand = std::process::Command;

pub fn run_smoke_hook(hook: &Utf8Path, artifact_root: &Utf8Path) -> crate::Result<()> {
    let status = StdCommand::new(hook)
        .arg(artifact_root)
        .status()
        .map_err(|error| crate::ReleaseError::Filesystem {
            path: hook.to_string(),
            reason: error.to_string(),
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(crate::ReleaseError::SmokeHookFailed {
            hook: hook.to_string(),
            status: status.to_string(),
        })
    }
}
