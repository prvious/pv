use std::ffi::OsString;
use std::io;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};
use console::Term;

use crate::prompt::{Answer, Prompt};

pub trait Environment {
    fn var_os(&self, key: &str) -> Option<OsString>;

    fn home_dir(&self) -> Option<PathBuf>;

    fn current_dir(&self) -> io::Result<PathBuf>;

    fn current_exe(&self) -> io::Result<PathBuf>;

    fn stdin_is_terminal(&self) -> bool;

    fn stdout_is_terminal(&self) -> bool {
        false
    }

    fn stderr_is_terminal(&self) -> bool {
        false
    }

    /// The terminal width in columns, when a terminal is attached.
    fn terminal_width(&self) -> Option<usize> {
        None
    }

    /// Asks a keyboard prompt. Callers check `Streams::interactive` first.
    fn prompt(&self, prompt: &Prompt<'_>) -> io::Result<Answer> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("cannot ask `{}` in this environment", prompt.message),
        ))
    }

    /// Sets whether libraries that draw on the terminal by themselves, such
    /// as prompts and progress bars, may use color.
    fn set_terminal_colors(&self, enabled: bool) {
        let _enabled = enabled;
    }

    fn open_url(&self, url: &str) -> io::Result<()>;

    fn exec(&self, program: &Path, args: &[String]) -> io::Result<ExitCode> {
        let _program = program;
        let _args = args;

        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "process replacement is not available in this environment",
        ))
    }

    fn exec_with_env(
        &self,
        program: &Path,
        args: &[String],
        env: &[(OsString, OsString)],
    ) -> io::Result<ExitCode> {
        let _env = env;

        self.exec(program, args)
    }

    fn daemon_registration_path(&self) -> PathBuf {
        platform::daemon_registration_path(&self.home_dir().unwrap_or_default())
    }

    fn enable_daemon_registration(&self, path: &Utf8Path) -> Result<(), platform::PlatformError> {
        platform::enable_daemon_registration(path)
    }

    fn stop_daemon_registration(&self) -> Result<(), platform::PlatformError> {
        platform::stop_daemon_registration()
    }

    fn remove_daemon_registration(&self, path: &Utf8Path) -> Result<(), platform::PlatformError> {
        platform::remove_daemon_registration(path)
    }

    fn daemon_process_for_stop(
        &self,
        paths: &state::PvPaths,
    ) -> Result<Option<daemon::DaemonProcess>, daemon::DaemonError> {
        daemon::daemon_process_for_stop(paths)
    }

    fn restart_daemon_registration(&self) -> Result<(), platform::PlatformError> {
        platform::restart_daemon_registration()
    }

    fn os_release_path(&self) -> PathBuf {
        PathBuf::from("/etc/os-release")
    }

    /// The system's `.test` resolver configuration. Only [`ProcessEnvironment`] reads the host's.
    fn inspect_system_resolver(
        &self,
        _expected: Option<&platform::ResolverConfig>,
    ) -> platform::ResolverFileState {
        platform::ResolverFileState::Missing {
            path: platform::SYSTEM_RESOLVER_TEST_PATH.into(),
        }
    }

    fn apply_system_resolver(&self, _port: u16) -> Result<(), platform::PlatformError> {
        Err(platform::PlatformError::PrivilegedHelperUnavailable)
    }

    fn remove_system_resolver(&self) -> Result<(), platform::PlatformError> {
        Err(platform::PlatformError::PrivilegedHelperUnavailable)
    }

    fn platform_target(&self) -> Result<platform::PlatformTarget, platform::PlatformError> {
        platform::PlatformTarget::current()
    }

    #[cfg(target_os = "linux")]
    fn check_linux_setup_host(&self) -> Result<(), platform::PlatformError> {
        platform::check_linux_setup_host()
    }

    fn check_linux_resolved_stub(&self) -> Result<(), platform::PlatformError> {
        Ok(())
    }

    fn inspect_low_ports(&self) -> Result<platform::LowPortInspection, platform::PlatformError>;

    /// Linux: PV's kernel setting for ports 80 and 443.
    #[cfg(target_os = "linux")]
    fn inspect_unprivileged_ports(&self) -> platform::UnprivilegedPortsState {
        platform::UnprivilegedPortsState::Unreadable {
            message: "unavailable in this environment".to_owned(),
        }
    }

    #[cfg(target_os = "linux")]
    fn apply_unprivileged_ports(&self) -> Result<(), platform::PlatformError> {
        Err(platform::PlatformError::PrivilegedHelperUnavailable)
    }

    #[cfg(target_os = "linux")]
    fn remove_unprivileged_ports(&self) -> Result<(), platform::PlatformError> {
        Err(platform::PlatformError::PrivilegedHelperUnavailable)
    }

    // This is probably temporary. We should remember to remove it on the next release.
    /// Whether an older PV left its pf port redirect on this Mac.
    fn legacy_pf_present(&self) -> Result<bool, platform::PlatformError> {
        Ok(false)
    }

    // This is probably temporary. We should remember to remove it on the next release.
    fn remove_legacy_pf(&self) -> Result<(), platform::PlatformError> {
        Err(platform::PlatformError::PrivilegedHelperUnavailable)
    }

    fn trusted_ca_certificates(
        &self,
    ) -> Result<Vec<platform::KeychainCertificate>, platform::PlatformError> {
        platform::SystemTrustInspector::trusted_certificates(&platform::NativeSystemTrustInspector)
    }

    fn privileged_helper_status(
        &self,
    ) -> Result<platform::PrivilegedHelperStatus, platform::PlatformError> {
        platform::PrivilegedHelperClient.status()
    }

    fn install_privileged_helper(
        &self,
        candidate_path: &Utf8Path,
        prepared_directory: &Utf8Path,
        expected_sha256: &str,
        helper_version: &str,
        protocol_version: u32,
    ) -> Result<platform::PrivilegedHelperInstallOutcome, platform::PlatformError> {
        platform::install_privileged_helper(
            candidate_path,
            prepared_directory,
            expected_sha256,
            helper_version,
            protocol_version,
        )
    }

    fn remove_privileged_helper(&self) -> Result<(), platform::PlatformError> {
        platform::remove_privileged_helper()
    }

    fn browser_ca_databases(
        &self,
        _home: &Utf8Path,
    ) -> Result<Vec<Utf8PathBuf>, platform::PlatformError> {
        Ok(Vec::new())
    }

    fn inspect_browser_ca_trust(
        &self,
        database: &Utf8Path,
        metadata: Option<&platform::LocalCaMetadata>,
    ) -> platform::TrustDomainState {
        platform::inspect_browser_ca_trust(database, metadata)
    }

    fn trust_browser_ca(
        &self,
        database: &Utf8Path,
        certificate: &Utf8Path,
    ) -> Result<(), platform::PlatformError> {
        platform::trust_browser_ca(database, certificate)
    }

    fn untrust_browser_ca(&self, database: &Utf8Path) -> Result<(), platform::PlatformError> {
        platform::untrust_browser_ca(database)
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

    fn app_update_manifest_url(&self) -> Option<String> {
        None
    }

    fn app_update_platform(&self) -> Option<self_update::AppUpdatePlatform> {
        None
    }

    fn resource_http_client(&self) -> Option<&dyn resources::ResourceHttpClient> {
        None
    }

    fn target_platform(&self) -> Option<resources::TargetPlatform> {
        None
    }

    fn resolve_target_platform(&self) -> resources::Result<resources::TargetPlatform> {
        self.target_platform()
            .map_or_else(resources::TargetPlatform::current, Ok)
    }
}

pub(crate) fn artifact_manifest_url(environment: &impl Environment) -> String {
    environment
        .artifact_manifest_url()
        .unwrap_or_else(|| resources::default_artifact_manifest_url().to_string())
}

pub(crate) fn app_update_manifest_url(environment: &impl Environment) -> String {
    environment
        .app_update_manifest_url()
        .unwrap_or_else(|| self_update::default_app_update_manifest_url().to_string())
}

#[derive(Debug, Default)]
pub struct ProcessEnvironment;

impl Environment for ProcessEnvironment {
    #[cfg(target_os = "linux")]
    fn check_linux_resolved_stub(&self) -> Result<(), platform::PlatformError> {
        platform::check_linux_resolved_stub()
    }

    fn browser_ca_databases(
        &self,
        home: &Utf8Path,
    ) -> Result<Vec<Utf8PathBuf>, platform::PlatformError> {
        let config_home = self
            .var_os("XDG_CONFIG_HOME")
            .and_then(|path| Utf8PathBuf::from_path_buf(path.into()).ok())
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        platform::browser_ca_databases(home, &config_home)
    }

    fn inspect_low_ports(&self) -> Result<platform::LowPortInspection, platform::PlatformError> {
        platform::PrivilegedHelperClient.inspect_low_ports()
    }

    fn inspect_system_resolver(
        &self,
        expected: Option<&platform::ResolverConfig>,
    ) -> platform::ResolverFileState {
        platform::inspect_system_resolver(expected)
    }

    fn apply_system_resolver(&self, port: u16) -> Result<(), platform::PlatformError> {
        platform::apply_system_resolver(port)
    }

    fn remove_system_resolver(&self) -> Result<(), platform::PlatformError> {
        platform::remove_system_resolver()
    }

    #[cfg(target_os = "linux")]
    fn inspect_unprivileged_ports(&self) -> platform::UnprivilegedPortsState {
        platform::inspect_unprivileged_ports()
    }

    #[cfg(target_os = "linux")]
    fn apply_unprivileged_ports(&self) -> Result<(), platform::PlatformError> {
        platform::PrivilegedHelperClient.apply_unprivileged_ports()
    }

    #[cfg(target_os = "linux")]
    fn remove_unprivileged_ports(&self) -> Result<(), platform::PlatformError> {
        platform::PrivilegedHelperClient.remove_unprivileged_ports()
    }

    // This is probably temporary. We should remember to remove it on the next release.
    #[cfg(target_os = "macos")]
    fn legacy_pf_present(&self) -> Result<bool, platform::PlatformError> {
        platform::legacy_pf_present()
    }

    // This is probably temporary. We should remember to remove it on the next release.
    fn remove_legacy_pf(&self) -> Result<(), platform::PlatformError> {
        platform::PrivilegedHelperClient.remove_legacy_pf()
    }

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

    fn stdout_is_terminal(&self) -> bool {
        io::stdout().is_terminal()
    }

    fn stderr_is_terminal(&self) -> bool {
        io::stderr().is_terminal()
    }

    fn terminal_width(&self) -> Option<usize> {
        Term::stdout()
            .size_checked()
            .or_else(|| Term::stderr().size_checked())
            .map(|(_rows, columns)| usize::from(columns))
    }

    fn prompt(&self, prompt: &Prompt<'_>) -> io::Result<Answer> {
        crate::prompt::interact(prompt)
    }

    fn set_terminal_colors(&self, enabled: bool) {
        console::set_colors_enabled(enabled);
        console::set_colors_enabled_stderr(enabled);
    }

    fn open_url(&self, url: &str) -> io::Result<()> {
        platform::open_url(url).map_err(io::Error::other)
    }

    fn exec(&self, program: &Path, args: &[String]) -> io::Result<ExitCode> {
        self.exec_with_env(program, args, &[])
    }

    fn exec_with_env(
        &self,
        program: &Path,
        args: &[String],
        env: &[(OsString, OsString)],
    ) -> io::Result<ExitCode> {
        platform::exec_replace_with_env(program, args, env)
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[derive(Debug, Default)]
    struct TestEnvironment {
        vars: BTreeMap<String, OsString>,
        home: Option<PathBuf>,
    }

    impl TestEnvironment {
        fn with_var(mut self, key: &str, value: &str) -> Self {
            self.vars.insert(key.to_string(), OsString::from(value));
            self
        }
    }

    impl Environment for TestEnvironment {
        fn inspect_low_ports(
            &self,
        ) -> Result<platform::LowPortInspection, platform::PlatformError> {
            Err(platform::PlatformError::PrivilegedHelperUnavailable)
        }

        fn var_os(&self, key: &str) -> Option<OsString> {
            self.vars.get(key).cloned()
        }

        fn home_dir(&self) -> Option<PathBuf> {
            self.home.clone()
        }

        fn current_dir(&self) -> io::Result<PathBuf> {
            Ok(PathBuf::new())
        }

        fn current_exe(&self) -> io::Result<PathBuf> {
            Ok(PathBuf::new())
        }

        fn stdin_is_terminal(&self) -> bool {
            false
        }

        fn open_url(&self, _url: &str) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn artifact_manifest_url_ignores_runtime_environment_variables() {
        let environment = TestEnvironment::default().with_var(
            resources::ARTIFACT_MANIFEST_URL_BUILD_ENV,
            "https://runtime.example.test/manifest.json",
        );

        assert_eq!(
            artifact_manifest_url(&environment),
            resources::default_artifact_manifest_url()
        );
    }
}
