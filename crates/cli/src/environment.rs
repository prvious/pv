use std::ffi::OsString;
use std::io;
use std::io::IsTerminal;
use std::path::PathBuf;

pub trait Environment {
    fn var_os(&self, key: &str) -> Option<OsString>;

    fn home_dir(&self) -> Option<PathBuf>;

    fn current_dir(&self) -> io::Result<PathBuf>;

    fn stdin_is_terminal(&self) -> bool;

    fn read_line(&self) -> io::Result<String>;

    fn open_url(&self, url: &str) -> io::Result<()>;

    fn resolver_test_path(&self) -> PathBuf {
        PathBuf::from(macos::SYSTEM_RESOLVER_TEST_PATH)
    }

    fn pf_anchor_path(&self) -> PathBuf {
        PathBuf::from(macos::SYSTEM_PF_ANCHOR_PATH)
    }

    fn pf_conf_path(&self) -> PathBuf {
        PathBuf::from(macos::SYSTEM_PF_CONF_PATH)
    }

    fn loopback_tcp_listener_ports(
        &self,
    ) -> Result<std::collections::BTreeSet<u16>, macos::MacosError> {
        macos::loopback_tcp_listener_ports()
    }

    fn trusted_ca_certificates(
        &self,
    ) -> Result<Vec<macos::KeychainCertificate>, macos::MacosError> {
        macos::SystemTrustInspector::trusted_certificates(&macos::MacosSystemTrustInspector)
    }
}

#[derive(Debug, Default)]
pub struct ProcessEnvironment;

impl Environment for ProcessEnvironment {
    fn var_os(&self, key: &str) -> Option<OsString> {
        process_var_os(key)
    }

    fn home_dir(&self) -> Option<PathBuf> {
        home::home_dir()
    }

    fn current_dir(&self) -> io::Result<PathBuf> {
        process_current_dir()
    }

    fn stdin_is_terminal(&self) -> bool {
        io::stdin().is_terminal()
    }

    fn read_line(&self) -> io::Result<String> {
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;

        Ok(line)
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
