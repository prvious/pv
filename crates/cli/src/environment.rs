use std::ffi::OsString;
use std::io;
use std::path::PathBuf;

pub trait Environment {
    fn var_os(&self, key: &str) -> Option<OsString>;

    fn current_dir(&self) -> io::Result<PathBuf>;

    fn open_url(&self, url: &str) -> io::Result<()>;
}

#[derive(Debug, Default)]
pub struct ProcessEnvironment;

impl Environment for ProcessEnvironment {
    fn var_os(&self, key: &str) -> Option<OsString> {
        process_var_os(key)
    }

    fn current_dir(&self) -> io::Result<PathBuf> {
        process_current_dir()
    }

    fn open_url(&self, url: &str) -> io::Result<()> {
        process_open_url(url)
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV environment helper owns direct process environment reads"
)]
fn process_var_os(key: &str) -> Option<OsString> {
    std::env::var_os(key)
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV environment helper owns current directory reads for testable Project resolution"
)]
fn process_current_dir() -> io::Result<PathBuf> {
    std::env::current_dir()
}

#[expect(
    clippy::disallowed_types,
    reason = "PV environment helper owns the macOS browser handoff for `pv open`"
)]
fn process_open_url(url: &str) -> io::Result<()> {
    let status = std::process::Command::new("open").arg(url).status()?;
    if status.success() {
        return Ok(());
    }

    Err(io::Error::other(format!(
        "browser open failed with {status}"
    )))
}
