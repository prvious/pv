#[cfg(target_os = "linux")]
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use camino::Utf8Path;
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
        let home = self.home_dir().unwrap_or_default();
        let config_home = if cfg!(target_os = "linux") {
            self.var_os("XDG_CONFIG_HOME").map(PathBuf::from)
        } else {
            None
        };
        platform::daemon_registration_path(&home, config_home.as_deref())
    }

    fn enable_daemon_registration(&self, path: &Utf8Path) -> Result<(), platform::PlatformError> {
        platform::enable_daemon_registration(
            path,
            #[cfg(target_os = "linux")]
            &daemon_registration_session_address(self)?,
        )
    }

    #[cfg(target_os = "linux")]
    fn reload_daemon_registration(&self) -> Result<(), platform::PlatformError> {
        platform::reload_daemon_registration(&daemon_registration_session_address(self)?)
    }

    fn stop_daemon_registration(&self) -> Result<(), platform::PlatformError> {
        platform::stop_daemon_registration(
            #[cfg(target_os = "linux")]
            &daemon_registration_session_address(self)?,
        )
    }

    fn remove_daemon_registration(&self, path: &Utf8Path) -> Result<(), platform::PlatformError> {
        platform::remove_daemon_registration(
            path,
            #[cfg(target_os = "linux")]
            &daemon_registration_session_address(self)?,
        )
    }

    #[cfg(target_os = "linux")]
    fn inspect_daemon_registration(
        &self,
    ) -> Result<Option<platform::DaemonRegistrationStatus>, platform::PlatformError> {
        platform::inspect_daemon_registration(&daemon_registration_session_address(self)?)
    }

    fn daemon_process_for_stop(
        &self,
        paths: &state::PvPaths,
    ) -> Result<Option<daemon::DaemonProcess>, daemon::DaemonError> {
        daemon::daemon_process_for_stop(paths)
    }

    fn restart_daemon_registration(&self) -> Result<(), platform::PlatformError> {
        platform::restart_daemon_registration(
            #[cfg(target_os = "linux")]
            &daemon_registration_session_address(self)?,
        )
    }

    fn resolver_test_path(&self) -> PathBuf {
        PathBuf::from(platform::SYSTEM_RESOLVER_TEST_PATH)
    }

    fn os_release_path(&self) -> PathBuf {
        PathBuf::from("/etc/os-release")
    }

    fn inspect_resolver_file(
        &self,
        path: &Utf8Path,
        expected: Option<&platform::ResolverConfig>,
    ) -> platform::ResolverFileState {
        let state = platform::inspect_resolver_file(path, expected);
        if matches!(state, platform::ResolverFileState::Unreadable { .. })
            && path == Utf8Path::new(platform::SYSTEM_RESOLVER_TEST_PATH)
            && let Ok(helper_state) = platform::PrivilegedHelperClient.inspect_dns(expected)
        {
            return helper_state;
        }

        state
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

    fn platform_target(&self) -> Result<platform::PlatformTarget, platform::PlatformError> {
        platform::PlatformTarget::current()
    }

    #[cfg(target_os = "linux")]
    fn check_linux_setup_host(&self) -> Result<(), platform::PlatformError> {
        let address = daemon_registration_session_address(self).map_err(|error| {
            platform::PlatformError::SystemIntegration(format!(
                "{error}; run `pv setup` from a logged-in systemd user session"
            ))
        })?;
        platform::check_linux_setup_host(&address)
    }

    #[cfg(target_os = "linux")]
    fn check_linux_setup_dns(&self) -> Result<(), platform::PlatformError> {
        platform::check_linux_setup_dns()
    }

    #[cfg(target_os = "linux")]
    fn socket_proxy_directory(&self) -> PathBuf {
        PathBuf::from(platform::SYSTEM_SOCKET_PROXY_DIRECTORY)
    }

    #[cfg(target_os = "linux")]
    fn socket_proxy_files_current(
        &self,
        config: &platform::PfRedirectConfig,
        prepared_directory: &Utf8Path,
        installed_directory: &Utf8Path,
    ) -> bool {
        platform::socket_proxy_files_current(config, prepared_directory, installed_directory)
    }

    #[cfg(target_os = "linux")]
    fn repairable_socket_proxy_ports(&self) -> Result<BTreeSet<u16>, platform::PlatformError> {
        platform::repairable_socket_proxy_ports()
    }

    #[cfg(target_os = "linux")]
    fn install_socket_proxy_units(
        &self,
        config: &platform::PfRedirectConfig,
    ) -> Result<(), platform::PlatformError> {
        platform::PrivilegedHelperClient.apply_pf(config)
    }

    #[cfg(target_os = "linux")]
    fn remove_socket_proxy_units(&self) -> Result<(), platform::PlatformError> {
        platform::PrivilegedHelperClient.remove_pf()
    }

    fn loopback_tcp_port_available(&self, port: u16) -> bool {
        platform::loopback_tcp_port_available(port)
    }

    fn inspect_low_ports(&self) -> Result<platform::LowPortInspection, platform::PlatformError>;

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

    fn inspect_active_pf_redirects_unprivileged(
        &self,
    ) -> Result<platform::ActivePfRedirectInspection, platform::PlatformError> {
        platform::inspect_active_pf_redirects_unprivileged()
            .or_else(|_error| platform::PrivilegedHelperClient.inspect_pf())
    }

    fn probe_gateway_redirects(
        &self,
        _expected: &platform::PfRedirectConfig,
        _ca_certificate_path: &Utf8Path,
    ) -> Result<(), String> {
        Err("Gateway identity probing is unavailable in this environment".to_owned())
    }

    fn request_system_reconciliation(&self, _paths: &state::PvPaths) -> Result<(), String> {
        Err("daemon reconciliation requests are unavailable in this environment".to_owned())
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
            .or_else(|_error| platform::PrivilegedHelperClient.inspect_ca())
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

    #[cfg(target_os = "linux")]
    fn installed_helper_artifacts_present(&self) -> Result<bool, platform::PlatformError> {
        platform::installed_helper_artifacts_present()
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

#[cfg(target_os = "linux")]
fn daemon_registration_session_address(
    environment: &(impl Environment + ?Sized),
) -> Result<String, platform::PlatformError> {
    let address = environment.var_os("DBUS_SESSION_BUS_ADDRESS");
    let runtime_directory = environment.var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    platform::daemon_registration_session_address(address.as_deref(), runtime_directory.as_deref())
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
    fn inspect_low_ports(&self) -> Result<platform::LowPortInspection, platform::PlatformError> {
        platform::PrivilegedHelperClient.inspect_low_ports()
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

    fn probe_gateway_redirects(
        &self,
        expected: &platform::PfRedirectConfig,
        ca_certificate_path: &Utf8Path,
    ) -> Result<(), String> {
        daemon::gateway::probe_gateway_identity_blocking(expected, ca_certificate_path)
            .map_err(|error| error.to_string())
    }

    fn request_system_reconciliation(&self, paths: &state::PvPaths) -> Result<(), String> {
        daemon::submit_job_blocking(paths.clone(), "reconcile", "system")
            .map(|_job| ())
            .map_err(|error| error.to_string())
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
        manifest_url: Option<String>,
        vars: BTreeMap<String, OsString>,
        home: Option<PathBuf>,
    }

    impl TestEnvironment {
        fn with_manifest_url(mut self, manifest_url: &str) -> Self {
            self.manifest_url = Some(manifest_url.to_string());
            self
        }

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

        fn artifact_manifest_url(&self) -> Option<String> {
            self.manifest_url.clone()
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn daemon_registration_path_uses_xdg_config_home_through_the_environment() {
        for (config_home, expected) in [
            (None, "/home/alice/.config/systemd/user/pv-daemon.service"),
            (Some("/config"), "/config/systemd/user/pv-daemon.service"),
            (
                Some(""),
                "/home/alice/.config/systemd/user/pv-daemon.service",
            ),
            (
                Some("relative"),
                "/home/alice/.config/systemd/user/pv-daemon.service",
            ),
        ] {
            let mut environment = TestEnvironment {
                home: Some(PathBuf::from("/home/alice")),
                ..TestEnvironment::default()
            };
            if let Some(config_home) = config_home {
                environment = environment.with_var("XDG_CONFIG_HOME", config_home);
            }
            assert_eq!(
                environment.daemon_registration_path(),
                PathBuf::from(expected)
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn daemon_registration_session_uses_only_the_injected_session_environment() -> anyhow::Result<()>
    {
        for (address, runtime_directory, expected) in [
            (
                Some("unix:path=/session/bus"),
                None,
                "unix:path=/session/bus",
            ),
            (
                Some("unix:path=/session/bus"),
                Some("/runtime"),
                "unix:path=/session/bus",
            ),
            (None, Some("/runtime"), "unix:path=/runtime/bus"),
            (Some(""), Some("/runtime"), "unix:path=/runtime/bus"),
            (None, Some("/runtime user"), "unix:path=/runtime%20user/bus"),
        ] {
            let mut environment = TestEnvironment::default();
            if let Some(address) = address {
                environment = environment.with_var("DBUS_SESSION_BUS_ADDRESS", address);
            }
            if let Some(runtime_directory) = runtime_directory {
                environment = environment.with_var("XDG_RUNTIME_DIR", runtime_directory);
            }
            assert_eq!(daemon_registration_session_address(&environment)?, expected);
        }

        for runtime_directory in [None, Some(""), Some("relative")] {
            let mut environment = TestEnvironment::default();
            if let Some(runtime_directory) = runtime_directory {
                environment = environment.with_var("XDG_RUNTIME_DIR", runtime_directory);
            }
            let path = Utf8Path::new("/not-a-real-registration.service");
            for result in [
                environment.inspect_daemon_registration().map(|_| ()),
                environment.enable_daemon_registration(path),
                environment.restart_daemon_registration(),
                environment.stop_daemon_registration(),
                environment.remove_daemon_registration(path),
            ] {
                assert!(matches!(
                    result,
                    Err(platform::PlatformError::UserManagerSessionEnvironment)
                ));
            }
        }
        Ok(())
    }

    #[test]
    fn artifact_manifest_url_uses_compiled_default_without_test_override() {
        let environment = TestEnvironment::default();

        assert_eq!(
            artifact_manifest_url(&environment),
            resources::default_artifact_manifest_url()
        );
    }

    #[test]
    fn artifact_manifest_url_preserves_test_injection() {
        let environment = TestEnvironment::default()
            .with_manifest_url("https://fixtures.example.test/manifest.json");

        assert_eq!(
            artifact_manifest_url(&environment),
            "https://fixtures.example.test/manifest.json"
        );
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
