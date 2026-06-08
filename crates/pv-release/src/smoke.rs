use camino::Utf8Path;
use std::time::{Duration, Instant};

#[expect(
    clippy::disallowed_types,
    reason = "PV release tooling owns explicit local smoke hook execution"
)]
type StdCommand = std::process::Command;

const DEFAULT_SMOKE_HOOK_TIMEOUT: Duration = Duration::from_secs(120);
const SMOKE_HOOK_POLL_INTERVAL: Duration = Duration::from_millis(10);

pub fn run_smoke_hook(hook: &Utf8Path, artifact_root: &Utf8Path) -> crate::Result<()> {
    run_smoke_hook_with_timeout(hook, artifact_root, DEFAULT_SMOKE_HOOK_TIMEOUT)
}

pub fn run_smoke_hook_with_timeout(
    hook: &Utf8Path,
    artifact_root: &Utf8Path,
    timeout: Duration,
) -> crate::Result<()> {
    let mut child = StdCommand::new(hook)
        .arg(artifact_root)
        .spawn()
        .map_err(|error| crate::ReleaseError::Filesystem {
            path: hook.to_string(),
            reason: error.to_string(),
        })?;
    let started = Instant::now();

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

        if started.elapsed() >= timeout {
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
                timeout: format_duration(timeout),
            });
        }

        let remaining = timeout.saturating_sub(started.elapsed());
        std::thread::sleep(remaining.min(SMOKE_HOOK_POLL_INTERVAL));
    }
}

fn format_duration(duration: Duration) -> String {
    if duration.subsec_millis() == 0 {
        format!("{}s", duration.as_secs())
    } else {
        format!("{}ms", duration.as_millis())
    }
}
