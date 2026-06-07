use std::ffi::OsString;
use std::io;
use std::io::IsTerminal;
use std::path::PathBuf;

use camino::Utf8Path;

pub trait Environment {
    fn var_os(&self, key: &str) -> Option<OsString>;

    fn home_dir(&self) -> Option<PathBuf>;

    fn current_dir(&self) -> io::Result<PathBuf>;

    fn current_exe(&self) -> io::Result<PathBuf>;

    fn stdin_is_terminal(&self) -> bool;

    fn read_line(&self) -> io::Result<String>;

    fn open_url(&self, url: &str) -> io::Result<()>;

    fn launch_agent_path(&self) -> PathBuf {
        self.home_dir()
            .unwrap_or_default()
            .join("Library")
            .join("LaunchAgents")
            .join(platform::LAUNCH_AGENT_FILE_NAME)
    }

    fn bootstrap_launch_agent(&self, plist_path: &Utf8Path) -> Result<(), platform::PlatformError> {
        platform::bootstrap_launch_agent(plist_path)
    }

    fn bootout_launch_agent(&self) -> Result<(), platform::PlatformError> {
        platform::bootout_launch_agent()
    }

    fn kickstart_launch_agent(&self) -> Result<(), platform::PlatformError> {
        platform::kickstart_launch_agent()
    }

    fn resolver_test_path(&self) -> PathBuf {
        PathBuf::from(platform::SYSTEM_RESOLVER_TEST_PATH)
    }

    fn install_resolver_config(
        &self,
        prepared_path: &Utf8Path,
        system_path: &Utf8Path,
    ) -> Result<(), platform::PlatformError> {
        platform::install_resolver_config(prepared_path, system_path)
    }

    fn remove_resolver_config(
        &self,
        system_path: &Utf8Path,
    ) -> Result<(), platform::PlatformError> {
        platform::remove_resolver_config(system_path)
    }

    fn pf_anchor_path(&self) -> PathBuf {
        PathBuf::from(platform::SYSTEM_PF_ANCHOR_PATH)
    }

    fn pf_conf_path(&self) -> PathBuf {
        PathBuf::from(platform::SYSTEM_PF_CONF_PATH)
    }

    fn loopback_tcp_listener_ports(
        &self,
    ) -> Result<std::collections::BTreeSet<u16>, platform::PlatformError> {
        platform::loopback_tcp_listener_ports()
    }

    fn install_pf_redirects(
        &self,
        prepared_anchor_path: &Utf8Path,
        prepared_reference_path: &Utf8Path,
        system_anchor_path: &Utf8Path,
        system_pf_conf_path: &Utf8Path,
    ) -> Result<(), platform::PlatformError> {
        platform::install_pf_redirects(
            prepared_anchor_path,
            prepared_reference_path,
            system_anchor_path,
            system_pf_conf_path,
        )
    }

    fn active_pf_redirect_config(
        &self,
    ) -> Result<Option<platform::PfRedirectConfig>, platform::PlatformError> {
        platform::active_pf_redirect_config()
    }

    fn remove_pf_redirects(
        &self,
        system_anchor_path: &Utf8Path,
        system_pf_conf_path: &Utf8Path,
        candidate_dir: &Utf8Path,
    ) -> Result<(), platform::PlatformError> {
        platform::remove_pf_redirects(system_anchor_path, system_pf_conf_path, candidate_dir)
    }

    fn trusted_ca_certificates(
        &self,
    ) -> Result<Vec<platform::KeychainCertificate>, platform::PlatformError> {
        platform::SystemTrustInspector::trusted_certificates(&platform::NativeSystemTrustInspector)
    }

    fn trust_system_ca(&self, certificate_path: &Utf8Path) -> Result<(), platform::PlatformError> {
        platform::trust_system_ca(certificate_path)
    }

    fn untrust_system_ca(&self, fingerprint: &str) -> Result<(), platform::PlatformError> {
        platform::untrust_system_ca(fingerprint)
    }

    fn artifact_manifest_url(&self) -> Option<String> {
        None
    }

    fn resource_http_client(&self) -> Option<&dyn resources::ResourceHttpClient> {
        None
    }

    fn target_platform(&self) -> Option<resources::TargetPlatform> {
        None
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

    fn current_exe(&self) -> io::Result<PathBuf> {
        process_current_exe()
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
        platform::open_url(url).map_err(io::Error::other)
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
    clippy::disallowed_methods,
    reason = "PV environment helper owns current executable reads for testable LaunchAgent setup"
)]
fn process_current_exe() -> io::Result<PathBuf> {
    std::env::current_exe()
}
