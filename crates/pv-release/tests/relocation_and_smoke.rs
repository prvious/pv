use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::assert_debug_snapshot;
use pv_release::ReleaseError;
use pv_release::relocation::{scan_file, scan_relocation_text};
use pv_release::smoke::run_smoke_hook;

#[test]
fn relocation_scan_rejects_blocked_runtime_paths() -> Result<()> {
    assert_debug_snapshot!((
        scan_relocation_text(Utf8Path::new("clean"), "load /usr/lib/libSystem.B.dylib"),
        scan_relocation_text(
            Utf8Path::new("homebrew"),
            "load /opt/homebrew/Cellar/redis/lib/libssl.dylib"
        ),
        scan_relocation_text(
            Utf8Path::new("cellar"),
            "load /usr/local/Cellar/postgresql/lib/libpq.dylib"
        ),
        scan_relocation_text(Utf8Path::new("runner"), "rpath /Users/runner/work/pv/build"),
    ));
    Ok(())
}

#[test]
fn relocation_scan_reads_files_as_lossy_text() -> Result<()> {
    let tempdir = tempdir()?;
    let clean = tempdir.path().join("clean.bin");
    let blocked = tempdir.path().join("blocked.bin");
    write_file(&clean, b"\xff\xfeload /usr/lib/libSystem.B.dylib")?;
    write_file(&blocked, b"\xff\xferpath /opt/homebrew/lib")?;

    assert_debug_snapshot!((
        summarize_result(scan_file(&clean)),
        summarize_result(scan_file(&blocked)),
    ));

    Ok(())
}

#[test]
fn smoke_hook_reports_success_and_failure() -> Result<()> {
    let tempdir = tempdir()?;
    let success = tempdir.path().join("success.sh");
    let failure = tempdir.path().join("failure.sh");
    write_executable(&success, "#!/bin/sh\nexit 0\n")?;
    write_executable(&failure, "#!/bin/sh\nexit 42\n")?;

    assert_debug_snapshot!((
        summarize_result(run_smoke_hook(&success, tempdir.path())),
        summarize_result(run_smoke_hook(&failure, tempdir.path())),
    ));

    Ok(())
}

fn summarize_result(result: pv_release::Result<()>) -> Result<(), ErrorSummary> {
    result.map_err(ErrorSummary::from)
}

#[derive(Debug, PartialEq, Eq)]
struct ErrorSummary {
    kind: &'static str,
    path: String,
    reason: String,
}

impl From<ReleaseError> for ErrorSummary {
    fn from(error: ReleaseError) -> Self {
        match error {
            ReleaseError::Relocation { path, reason } => Self {
                kind: "Relocation",
                path: file_name(&path),
                reason,
            },
            ReleaseError::SmokeHookFailed { hook, status } => Self {
                kind: "SmokeHookFailed",
                path: file_name(&hook),
                reason: status,
            },
            error => Self {
                kind: "Other",
                path: String::new(),
                reason: error.to_string(),
            },
        }
    }
}

fn file_name(path: &str) -> String {
    match Utf8Path::new(path).file_name() {
        Some(file_name) => file_name.to_string(),
        None => path.to_string(),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests write local relocation scan fixtures"
)]
fn write_file(path: &Utf8Path, content: &[u8]) -> Result<()> {
    std::fs::write(path, content)?;
    Ok(())
}

#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests create executable smoke hook fixtures"
)]
fn write_executable(path: &Utf8Path, content: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::write(path, content)?;
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests create executable smoke hook fixtures"
)]
fn write_executable(path: &Utf8Path, content: &str) -> Result<()> {
    std::fs::write(path, content)?;
    Ok(())
}
