use camino::Utf8Path;
use platform::clock::{self, Deadline, Limit};
use std::time::Duration;

#[expect(
    clippy::disallowed_types,
    reason = "PV release tooling owns explicit local smoke hook execution"
)]
type StdCommand = std::process::Command;

/// How long a smoke hook may run before it is killed.
pub const SMOKE_HOOK_TIMEOUT: Limit = Limit::new("smoke hook", Duration::from_secs(120));
const SMOKE_HOOK_POLL_INTERVAL: Duration = Duration::from_millis(10);

pub fn run_smoke_hook(hook: &Utf8Path, artifact_root: &Utf8Path) -> crate::Result<()> {
    let mut child = StdCommand::new(hook)
        .arg(artifact_root)
        .spawn()
        .map_err(|error| crate::ReleaseError::Filesystem {
            path: hook.to_string(),
            reason: error.to_string(),
        })?;
    let deadline = Deadline::after(&SMOKE_HOOK_TIMEOUT);

    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| crate::ReleaseError::Filesystem {
                path: hook.to_string(),
                reason: error.to_string(),
            })?
        {
            return if status.success() {
                Ok(())
            } else {
                Err(crate::ReleaseError::SmokeHookFailed {
                    hook: hook.to_string(),
                    status: status.to_string(),
                })
            };
        }

        if deadline.is_expired() {
            child
                .kill()
                .map_err(|error| crate::ReleaseError::Filesystem {
                    path: hook.to_string(),
                    reason: error.to_string(),
                })?;
            child
                .wait()
                .map_err(|error| crate::ReleaseError::Filesystem {
                    path: hook.to_string(),
                    reason: error.to_string(),
                })?;
            return Err(crate::ReleaseError::SmokeHookTimedOut {
                hook: hook.to_string(),
                timeout: format!("{}s", SMOKE_HOOK_TIMEOUT.duration().as_secs()),
            });
        }

        clock::poll_blocking(SMOKE_HOOK_POLL_INTERVAL);
    }
}
