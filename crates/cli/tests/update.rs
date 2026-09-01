#[cfg(unix)]
mod update_tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::ffi::OsString;
    use std::fs::Permissions;
    use std::io::{self, BufRead, BufReader, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::path::{Path, PathBuf};
    use std::process::ExitCode;
    use std::thread;
    use std::time::{Duration, Instant};

    use camino::{Utf8Path, Utf8PathBuf};
    use camino_tempfile::tempdir;
    use cli::{Environment, run_with_environment};
    use insta::{Settings, assert_debug_snapshot};
    use platform::{
        LAUNCH_AGENT_LABEL, LaunchAgentConfig, PRIVILEGED_HELPER_VERSION, PrivilegedHelperStatus,
    };
    use resources::{ResourceHttpClient, ResourcesError};
    use serde_json::json;
    use state::{AppReleaseLayout, PvPaths};

    const APP_MANIFEST_URL: &str = "https://updates.example.test/pv-app-manifest.json";
    const APP_BINARY_URL: &str = "https://downloads.example.test/pv/0.3.0/pv-darwin-arm64";
    const APP_BINARY: &[u8] = b"pv 0.3.0\n";
    const APP_BINARY_SHA256: &str =
        "daa36b28494155c914f2745dcb1908b9e97f07f718313751c5c0be96f9c28b74";
    const HELPER_BINARY: &[u8] = b"pv helper 1.0.0\n";
    const HELPER_BINARY_SHA256: &str =
        "f15b9ec9f06fc9e7e92af6e7cdfe82ae574bdc11f09bf796e44280989b1378d2";
    const CURRENT_APP_VERSION: &str = env!("CARGO_PKG_VERSION");

    struct TestEnvironment {
        home: PathBuf,
        client: Box<dyn ResourceHttpClient>,
        operations: RefCell<Vec<String>>,
        execs: RefCell<Vec<(PathBuf, Vec<String>)>>,
        exec_result: RefCell<Result<ExitCode, io::Error>>,
        launch_agent_unloaded: bool,
        delete_on_first_kickstart: RefCell<Option<Utf8PathBuf>>,
        lock_probe: RefCell<Option<PvPaths>>,
        startup_marker_on_first_kickstart: RefCell<Option<(Utf8PathBuf, String)>>,
        helper_status: RefCell<Option<PrivilegedHelperStatus>>,
        helper_protocol_mismatch: bool,
        helper_install_results: RefCell<VecDeque<Result<(), String>>>,
        helper_install_count: RefCell<usize>,
        helper_install_cleanup_warnings: RefCell<VecDeque<Option<String>>>,
        helper_promotion_failure_parent: RefCell<Option<Utf8PathBuf>>,
    }

    impl TestEnvironment {
        fn new(home: &Utf8Path, client: impl ResourceHttpClient + 'static) -> Self {
            Self {
                home: home.as_std_path().to_path_buf(),
                client: Box::new(client),
                operations: RefCell::new(Vec::new()),
                execs: RefCell::new(Vec::new()),
                exec_result: RefCell::new(Ok(ExitCode::SUCCESS)),
                launch_agent_unloaded: false,
                delete_on_first_kickstart: RefCell::new(None),
                lock_probe: RefCell::new(None),
                startup_marker_on_first_kickstart: RefCell::new(None),
                helper_status: RefCell::new(Some(PrivilegedHelperStatus {
                    version: PRIVILEGED_HELPER_VERSION.to_string(),
                    protocol_version: platform::HELPER_PROTOCOL_VERSION,
                    owner_uid: 501,
                })),
                helper_protocol_mismatch: false,
                helper_install_results: RefCell::new(VecDeque::new()),
                helper_install_count: RefCell::new(0),
                helper_install_cleanup_warnings: RefCell::new(VecDeque::new()),
                helper_promotion_failure_parent: RefCell::new(None),
            }
        }

        fn operations(&self) -> Vec<String> {
            self.operations.borrow().clone()
        }

        fn execs(&self) -> Vec<(PathBuf, Vec<String>)> {
            self.execs.borrow().clone()
        }

        fn with_exec_error(self, error: io::Error) -> Self {
            let _previous = self.exec_result.replace(Err(error));
            self
        }

        fn with_unloaded_launch_agent(mut self) -> Self {
            self.launch_agent_unloaded = true;
            self
        }

        fn with_delete_on_first_kickstart(self, path: Utf8PathBuf) -> Self {
            self.delete_on_first_kickstart.replace(Some(path));
            self
        }

        fn with_update_lock_probe(self, paths: PvPaths) -> Self {
            self.lock_probe.replace(Some(paths));
            self
        }

        fn with_startup_marker_on_first_kickstart(self, path: Utf8PathBuf, content: &str) -> Self {
            self.startup_marker_on_first_kickstart
                .replace(Some((path, content.to_string())));
            self
        }

        fn with_missing_helper(self) -> Self {
            self.helper_status.replace(None);
            self
        }

        fn with_helper_install_error(self, message: &str) -> Self {
            self.helper_status.replace(None);
            self.helper_install_results
                .borrow_mut()
                .push_back(Err(message.to_string()));
            self
        }

        fn with_helper_replacement_error(self, message: &str) -> Self {
            self.helper_install_results
                .borrow_mut()
                .push_back(Err(message.to_string()));
            self
        }

        fn with_helper_protocol_mismatch(mut self) -> Self {
            self.helper_protocol_mismatch = true;
            self
        }

        fn with_helper_restore_error(self, message: &str) -> Self {
            self.helper_install_results
                .borrow_mut()
                .extend([Ok(()), Err(message.to_string())]);
            self
        }

        fn with_helper_status(self, version: &str, protocol_version: u32) -> Self {
            self.helper_status.replace(Some(PrivilegedHelperStatus {
                version: version.to_string(),
                protocol_version,
                owner_uid: 501,
            }));
            self
        }

        fn with_helper_promotion_failure(self, release_helper_parent: Utf8PathBuf) -> Self {
            self.helper_promotion_failure_parent
                .replace(Some(release_helper_parent));
            self
        }

        fn with_helper_rollback_cleanup_warning(self, message: &str) -> Self {
            self.helper_install_cleanup_warnings
                .borrow_mut()
                .extend([None, Some(message.to_string())]);
            self
        }
    }

    impl Environment for TestEnvironment {
        fn var_os(&self, _key: &str) -> Option<OsString> {
            None
        }

        fn home_dir(&self) -> Option<PathBuf> {
            Some(self.home.clone())
        }

        fn current_dir(&self) -> io::Result<PathBuf> {
            Ok(self.home.clone())
        }

        fn current_exe(&self) -> io::Result<PathBuf> {
            Ok(PathBuf::from("/bin/pv"))
        }

        fn stdin_is_terminal(&self) -> bool {
            false
        }

        fn read_line(&self) -> io::Result<String> {
            Ok(String::new())
        }

        fn open_url(&self, _url: &str) -> io::Result<()> {
            Ok(())
        }

        fn exec(&self, program: &Path, args: &[String]) -> io::Result<ExitCode> {
            self.execs
                .borrow_mut()
                .push((program.to_path_buf(), args.to_vec()));

            self.exec_result.replace(Ok(ExitCode::SUCCESS))
        }

        fn app_update_manifest_url(&self) -> Option<String> {
            Some(APP_MANIFEST_URL.to_string())
        }

        fn app_update_platform(&self) -> Option<self_update::AppUpdatePlatform> {
            Some(self_update::AppUpdatePlatform::DarwinArm64)
        }

        fn resource_http_client(&self) -> Option<&dyn ResourceHttpClient> {
            Some(self.client.as_ref())
        }

        fn bootstrap_launch_agent(
            &self,
            plist_path: &Utf8Path,
        ) -> Result<(), platform::PlatformError> {
            self.operations
                .borrow_mut()
                .push(format!("bootstrap {plist_path}"));

            Ok(())
        }

        fn bootout_launch_agent(&self) -> Result<(), platform::PlatformError> {
            self.operations
                .borrow_mut()
                .push(format!("bootout {LAUNCH_AGENT_LABEL}"));

            if self.launch_agent_unloaded {
                return Err(platform::PlatformError::LaunchAgent(
                    "Boot-out failed: 3: No such process".to_string(),
                ));
            }

            Ok(())
        }

        fn kickstart_launch_agent(&self) -> Result<(), platform::PlatformError> {
            self.operations
                .borrow_mut()
                .push(format!("kickstart {LAUNCH_AGENT_LABEL}"));
            if let Some(paths) = self.lock_probe.borrow().as_ref() {
                let probe = match state::UpdateLock::acquire(paths) {
                    Ok(_lock) => "lock probe free".to_string(),
                    Err(state::StateError::CoordinationLockHeld { .. }) => {
                        "lock probe held".to_string()
                    }
                    Err(error) => format!("lock probe failed: {error}"),
                };
                self.operations.borrow_mut().push(probe);
            }
            if let Some(path) = self.delete_on_first_kickstart.borrow_mut().take() {
                state::fs::remove_file_if_exists(&path)
                    .map_err(|error| platform::PlatformError::LaunchAgent(error.to_string()))?;
            }
            if let Some((path, content)) =
                self.startup_marker_on_first_kickstart.borrow_mut().take()
            {
                state::fs::write_sensitive_file(&path, &content)
                    .map_err(|error| platform::PlatformError::LaunchAgent(error.to_string()))?;
            }

            Ok(())
        }

        fn privileged_helper_status(
            &self,
        ) -> Result<PrivilegedHelperStatus, platform::PlatformError> {
            if self.helper_protocol_mismatch {
                return Err(platform::PlatformError::PrivilegedHelperProtocolMismatch {
                    expected: platform::HELPER_PROTOCOL_VERSION,
                    actual: platform::HELPER_PROTOCOL_VERSION + 1,
                });
            }

            self.helper_status
                .borrow()
                .clone()
                .ok_or(platform::PlatformError::PrivilegedHelperUnavailable)
        }

        fn install_privileged_helper(
            &self,
            candidate_path: &Utf8Path,
            _prepared_directory: &Utf8Path,
            expected_sha256: &str,
            helper_version: &str,
            protocol_version: u32,
        ) -> Result<platform::PrivilegedHelperInstallOutcome, platform::PlatformError> {
            let install_count = *self.helper_install_count.borrow() + 1;
            self.helper_install_count.replace(install_count);
            if let Some(Err(message)) = self.helper_install_results.borrow_mut().pop_front() {
                return Err(platform::PlatformError::PrivilegedHelperInstallation(
                    message,
                ));
            }
            let status = PrivilegedHelperStatus {
                version: helper_version.to_string(),
                protocol_version,
                owner_uid: 501,
            };
            self.helper_status.replace(Some(status.clone()));
            self.operations.borrow_mut().push(format!(
                "install helper {candidate_path} sha256 {}",
                expected_sha256
            ));
            if let Some(parent) = self.helper_promotion_failure_parent.borrow().as_ref() {
                let metadata_path = parent.join("pv-helper.json");
                if install_count == 1 {
                    state::fs::remove_file_if_exists(&metadata_path).map_err(|error| {
                        platform::PlatformError::PrivilegedHelperInstallation(error.to_string())
                    })?;
                    state::fs::ensure_user_dir(&metadata_path).map_err(|error| {
                        platform::PlatformError::PrivilegedHelperInstallation(error.to_string())
                    })?;
                } else {
                    state::fs::delete_dir_all(&metadata_path).map_err(|error| {
                        platform::PlatformError::PrivilegedHelperInstallation(error.to_string())
                    })?;
                }
            }

            let outcome = platform::PrivilegedHelperInstallOutcome::successful(status);
            if let Some(cleanup_warning) = self
                .helper_install_cleanup_warnings
                .borrow_mut()
                .pop_front()
                .flatten()
            {
                return Ok(outcome.with_cleanup_warning(cleanup_warning));
            }

            Ok(outcome)
        }

        fn remove_privileged_helper(&self) -> Result<(), platform::PlatformError> {
            self.helper_status.replace(None);
            self.operations
                .borrow_mut()
                .push("remove helper".to_string());

            Ok(())
        }
    }

    #[test]
    fn update_check_reports_app_and_managed_resource_updates() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let daemon = FakeDaemon::start(
            &paths,
            vec![
                health_response(),
                managed_resource_update_check_response(
                    paths.resources().join("redis/8.8/releases/8.8.0-pv1"),
                ),
            ],
        )?;
        let environment =
            TestEnvironment::new(&home, ScriptedClient::new().with_text(APP_MANIFEST));

        let output = run_pv(&["update", "--check"], &environment)?;

        daemon.join()?;
        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert!(output.stderr.is_empty());
        assert_update_snapshot(
            "update_check_reports_app_and_managed_resource_updates",
            output,
        );

        Ok(())
    }

    #[test]
    fn update_check_json_reports_app_and_managed_resource_updates() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let daemon = FakeDaemon::start(
            &paths,
            vec![
                health_response(),
                managed_resource_update_check_response(
                    paths.resources().join("redis/8.8/releases/8.8.0-pv1"),
                ),
            ],
        )?;
        let environment =
            TestEnvironment::new(&home, ScriptedClient::new().with_text(APP_MANIFEST));

        let output = run_pv(&["update", "--check", "--json"], &environment)?;

        daemon.join()?;
        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert!(output.stderr.is_empty());
        assert_update_snapshot(
            "update_check_json_reports_app_and_managed_resource_updates",
            output,
        );

        Ok(())
    }

    #[test]
    fn update_check_marks_helper_only_protocol_change_unavailable() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let daemon = FakeDaemon::start(
            &paths,
            vec![
                health_response(),
                managed_resource_update_check_response(
                    paths.resources().join("redis/8.8/releases/8.8.0-pv1"),
                ),
            ],
        )?;
        let manifest = app_manifest(
            CURRENT_APP_VERSION,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            12_345_678,
        )
        .replace("\"version\": \"1.0.0\"", "\"version\": \"1.1.0\"")
        .replace("\"protocol_version\": 1", "\"protocol_version\": 2")
        .replace("pv-helper-1.0.0", "pv-helper-1.1.0");
        let environment = TestEnvironment::new(&home, ScriptedClient::new().with_text(&manifest));

        let output = run_pv(&["update", "--check"], &environment)?;

        daemon.join()?;
        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert!(output.stdout.contains("Privileged helper: unavailable"));
        assert!(
            output
                .stdout
                .contains("protocol change requires a matching PV application update")
        );

        Ok(())
    }

    #[test]
    fn update_check_reports_missing_daemon_before_fetching_manifests() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let environment = TestEnvironment::new(&home, PanickingClient);

        let output = run_pv(&["update", "--check"], &environment)?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert!(output.stdout.is_empty());
        assert_debug_snapshot!(output);

        Ok(())
    }

    #[test]
    fn update_check_reports_daemon_rejected() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let daemon = FakeDaemon::start(
            &paths,
            vec![
                health_response(),
                json!({
                    "type": "response",
                    "protocol_version": daemon::PROTOCOL_VERSION,
                    "status": "error",
                    "message": "update in progress"
                }),
            ],
        )?;
        let environment =
            TestEnvironment::new(&home, ScriptedClient::new().with_text(APP_MANIFEST));

        let output = run_pv(&["update", "--check"], &environment)?;

        daemon.join()?;
        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert!(output.stdout.is_empty());
        assert_debug_snapshot!(output);

        Ok(())
    }

    #[test]
    fn update_check_reports_app_manifest_parse_failure() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let daemon = FakeDaemon::start(&paths, vec![health_response()])?;
        let environment = TestEnvironment::new(&home, ScriptedClient::new().with_text("not json"));

        let output = run_pv(&["update", "--check"], &environment)?;

        daemon.join()?;
        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert!(output.stdout.is_empty());
        assert_debug_snapshot!(output);

        Ok(())
    }

    #[test]
    fn update_check_reports_app_manifest_network_failure() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let daemon = FakeDaemon::start(&paths, vec![health_response()])?;
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new().with_error(ResourcesError::HttpRequestFailed {
                url: APP_MANIFEST_URL.to_string(),
                reason: "network error".to_string(),
            }),
        );

        let output = run_pv(&["update", "--check"], &environment)?;

        daemon.join()?;
        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert!(output.stdout.is_empty());
        assert_debug_snapshot!(output);

        Ok(())
    }

    #[test]
    fn update_check_reports_current_managed_resource() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let daemon = FakeDaemon::start(
            &paths,
            vec![
                health_response(),
                managed_resource_status_response(
                    "current",
                    paths.resources().join("redis/8.8/releases/8.8.0-pv1"),
                    json!({"latest_artifact_version": "8.8.0-pv1"}),
                ),
            ],
        )?;
        let environment =
            TestEnvironment::new(&home, ScriptedClient::new().with_text(APP_MANIFEST));
        let output = run_pv(&["update", "--check"], &environment)?;
        daemon.join()?;
        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert_update_snapshot("update_check_reports_current_managed_resource", output);
        Ok(())
    }

    #[test]
    fn update_check_reports_revoked_managed_resource() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let daemon = FakeDaemon::start(
            &paths,
            vec![
                health_response(),
                managed_resource_status_response(
                    "revoked",
                    paths.resources().join("redis/8.8/releases/8.8.0-pv1"),
                    json!({
                        "current_revocation": {
                            "artifact_version": "8.8.0-pv1",
                            "reason": "security vulnerability"
                        },
                        "latest_artifact_version": "8.8.1-pv1"
                    }),
                ),
            ],
        )?;
        let environment =
            TestEnvironment::new(&home, ScriptedClient::new().with_text(APP_MANIFEST));
        let output = run_pv(&["update", "--check"], &environment)?;
        daemon.join()?;
        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert_update_snapshot("update_check_reports_revoked_managed_resource", output);
        Ok(())
    }

    #[test]
    fn update_check_reports_blocked_managed_resource() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let daemon = FakeDaemon::start(
            &paths,
            vec![
                health_response(),
                managed_resource_status_response(
                    "blocked",
                    paths.resources().join("redis/8.8/releases/8.8.0-pv1"),
                    json!({
                        "blocked_by": {
                            "minimum_pv_version": "0.5.0",
                            "current_pv_version": "0.1.0"
                        }
                    }),
                ),
            ],
        )?;
        let environment =
            TestEnvironment::new(&home, ScriptedClient::new().with_text(APP_MANIFEST));
        let output = run_pv(&["update", "--check"], &environment)?;
        daemon.join()?;
        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert_update_snapshot("update_check_reports_blocked_managed_resource", output);
        Ok(())
    }

    #[test]
    fn update_check_reports_unavailable_managed_resource() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let daemon = FakeDaemon::start(
            &paths,
            vec![
                health_response(),
                managed_resource_status_response(
                    "unavailable",
                    paths.resources().join("redis/8.8/releases/8.8.0-pv1"),
                    json!({"reason": "no installable artifact"}),
                ),
            ],
        )?;
        let environment =
            TestEnvironment::new(&home, ScriptedClient::new().with_text(APP_MANIFEST));
        let output = run_pv(&["update", "--check"], &environment)?;
        daemon.join()?;
        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert_update_snapshot("update_check_reports_unavailable_managed_resource", output);
        Ok(())
    }

    #[test]
    fn update_reports_current_app_without_restarting_daemon() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let daemon = FakeDaemon::start_with_response_lines(
            &paths,
            vec![vec![
                job_accepted_response("job_1"),
                job_completed("job_1", "current"),
            ]],
        )?;
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new().with_text(&app_manifest(
                CURRENT_APP_VERSION,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                12_345_678,
            )),
        );

        let output = run_pv(&["update"], &environment)?;
        let daemon_requests = daemon.join()?;

        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert!(output.stderr.is_empty());
        assert_eq!(
            layout.active_release()?,
            Some(CURRENT_APP_VERSION.to_string())
        );
        assert_eq!(
            daemon_requests,
            vec![json!({
                "protocol_version": daemon::PROTOCOL_VERSION,
                "command": "run_job",
                "kind": "update",
                "scope": "system",
            })]
        );
        assert!(environment.operations().is_empty());
        assert!(environment.execs().is_empty());
        assert_update_snapshot(
            "update_reports_current_app_without_restarting_daemon",
            output,
        );

        Ok(())
    }

    #[test]
    fn update_installs_changed_helper_without_reactivating_current_app() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let daemon = FakeDaemon::start_with_response_lines(
            &paths,
            vec![vec![
                job_accepted_response("job_1"),
                job_completed("job_1", "current"),
            ]],
        )?;
        let manifest = app_manifest(
            CURRENT_APP_VERSION,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            12_345_678,
        )
        .replace("\"version\": \"1.0.0\"", "\"version\": \"1.1.0\"")
        .replace("pv-helper-1.0.0", "pv-helper-1.1.0");
        let environment = TestEnvironment::new(&home, ScriptedClient::new().with_text(&manifest))
            .with_missing_helper();

        let output = run_pv(&["update"], &environment)?;
        let _daemon_requests = daemon.join()?;

        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert_eq!(
            layout.active_release()?,
            Some(CURRENT_APP_VERSION.to_string())
        );
        assert!(state::fs::path_is_file(
            &paths.app_release_helper(CURRENT_APP_VERSION)
        )?);
        assert!(
            output
                .stdout
                .contains("Privileged helper: updated to 1.1.0 (protocol 1)")
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&state::fs::read_to_string(
                &paths
                    .app_release_helper(CURRENT_APP_VERSION)
                    .with_file_name("pv-helper.json"),
            )?)?,
            json!({
                "version": "1.1.0",
                "protocol_version": 1,
                "sha256": HELPER_BINARY_SHA256,
            })
        );
        assert_eq!(environment.operations().len(), 1);

        Ok(())
    }

    #[test]
    fn update_rejects_helper_only_protocol_change_before_download() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let manifest = app_manifest(
            CURRENT_APP_VERSION,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            12_345_678,
        )
        .replace("\"version\": \"1.0.0\"", "\"version\": \"1.1.0\"")
        .replace("\"protocol_version\": 1", "\"protocol_version\": 2")
        .replace("pv-helper-1.0.0", "pv-helper-1.1.0");
        let environment = TestEnvironment::new(&home, ScriptedClient::new().with_text(&manifest));

        let output = run_pv(&["update"], &environment)?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert!(
            output
                .stderr
                .contains("requires a matching PV application update")
        );
        assert_eq!(
            layout.active_release()?,
            Some(CURRENT_APP_VERSION.to_string())
        );
        assert!(environment.operations().is_empty());

        Ok(())
    }

    #[test]
    fn update_rejects_registered_helper_identity_that_cannot_be_rolled_back() -> anyhow::Result<()>
    {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let manifest = helper_only_update_manifest();
        let environment = TestEnvironment::new(&home, ScriptedClient::new().with_text(&manifest))
            .with_helper_status("0.9.0", platform::HELPER_PROTOCOL_VERSION);

        let output = run_pv(&["update"], &environment)?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert!(environment.operations().is_empty());
        assert!(update_temporary_files(&paths)?.is_empty());
        assert_update_snapshot(
            "update_rejects_registered_helper_identity_that_cannot_be_rolled_back",
            output,
        );

        Ok(())
    }

    #[test]
    fn update_rejects_protocol_mismatched_helper_that_cannot_be_rolled_back() -> anyhow::Result<()>
    {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let manifest = helper_only_update_manifest();
        let environment = TestEnvironment::new(&home, ScriptedClient::new().with_text(&manifest))
            .with_helper_protocol_mismatch();

        let output = run_pv(&["update"], &environment)?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert!(environment.operations().is_empty());
        assert!(update_temporary_files(&paths)?.is_empty());
        assert_update_snapshot(
            "update_rejects_protocol_mismatched_helper_that_cannot_be_rolled_back",
            output,
        );

        Ok(())
    }

    #[test]
    fn update_rejects_corrupt_release_helper_without_leaving_a_rollback_candidate()
    -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        write_bytes(
            &paths.app_release_helper(CURRENT_APP_VERSION),
            b"corrupt helper\n",
        )?;
        let manifest = helper_only_update_manifest();
        let environment = TestEnvironment::new(&home, ScriptedClient::new().with_text(&manifest));

        let output = run_pv(&["update"], &environment)?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert!(environment.operations().is_empty());
        assert!(update_temporary_files(&paths)?.is_empty());
        assert_update_snapshot(
            "update_rejects_corrupt_release_helper_without_leaving_a_rollback_candidate",
            output,
        );

        Ok(())
    }

    #[test]
    fn failed_helper_only_update_preserves_current_release_candidate() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let manifest = app_manifest(
            CURRENT_APP_VERSION,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            12_345_678,
        )
        .replace("\"version\": \"1.0.0\"", "\"version\": \"1.1.0\"")
        .replace("pv-helper-1.0.0", "pv-helper-1.1.0");
        let environment = TestEnvironment::new(&home, ScriptedClient::new().with_text(&manifest))
            .with_helper_replacement_error("authentication cancelled");

        let output = run_pv(&["update"], &environment)?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert_eq!(
            state::fs::read_to_string(&paths.app_release_helper(CURRENT_APP_VERSION))?,
            String::from_utf8(HELPER_BINARY.to_vec())?
        );
        let metadata = state::fs::read_to_string(
            &paths
                .app_release_helper(CURRENT_APP_VERSION)
                .with_file_name("pv-helper.json"),
        )?;
        assert!(metadata.contains("\"version\": \"1.0.0\""));
        assert_eq!(environment.operations().len(), 1);

        Ok(())
    }

    #[test]
    fn helper_only_promotion_failure_restores_registered_and_release_helpers() -> anyhow::Result<()>
    {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let manifest = app_manifest(
            CURRENT_APP_VERSION,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            12_345_678,
        )
        .replace("\"version\": \"1.0.0\"", "\"version\": \"1.1.0\"")
        .replace("pv-helper-1.0.0", "pv-helper-1.1.0");
        let release_helper = paths.app_release_helper(CURRENT_APP_VERSION);
        let release_helper_parent = release_helper
            .parent()
            .ok_or_else(|| anyhow::anyhow!("release helper must have a parent"))?
            .to_path_buf();
        let environment = TestEnvironment::new(&home, ScriptedClient::new().with_text(&manifest))
            .with_helper_promotion_failure(release_helper_parent);

        let output = run_pv(&["update"], &environment)?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        let operations = environment.operations();
        assert_eq!(
            environment.helper_status.borrow().as_ref(),
            Some(&PrivilegedHelperStatus {
                version: PRIVILEGED_HELPER_VERSION.to_string(),
                protocol_version: platform::HELPER_PROTOCOL_VERSION,
                owner_uid: 501,
            }),
            "output: {output:?}; operations: {operations:?}"
        );
        assert_eq!(
            state::fs::read_to_string(&release_helper)?,
            String::from_utf8(HELPER_BINARY.to_vec())?
        );
        let metadata_path = release_helper.with_file_name("pv-helper.json");
        assert!(
            state::fs::path_is_file(&metadata_path)?,
            "output: {output:?}; operations: {operations:?}"
        );
        assert!(state::fs::read_to_string(&metadata_path)?.contains("\"version\": \"1.0.0\""));
        assert_eq!(operations.len(), 2);
        assert!(
            state::fs::read_dir_paths(paths.downloads())?
                .iter()
                .all(|path| !path.file_name().unwrap_or("").contains("helper-rollback"))
        );

        Ok(())
    }

    #[test]
    fn update_ignores_helper_identity_from_an_older_app_manifest() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let daemon = FakeDaemon::start_with_response_lines(
            &paths,
            vec![vec![
                job_accepted_response("job_1"),
                job_completed("job_1", "current"),
            ]],
        )?;
        let manifest = app_manifest(
            "0.1.0",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            12_345_678,
        )
        .replace("\"protocol_version\": 1", "\"protocol_version\": 2");
        let environment = TestEnvironment::new(&home, ScriptedClient::new().with_text(&manifest))
            .with_helper_status("2.0.0", 2);

        let output = run_pv(&["update"], &environment)?;
        let _daemon_requests = daemon.join()?;

        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert!(
            output
                .stdout
                .contains("Privileged helper: retained 2.0.0 (protocol 2)")
        );
        assert!(output.stdout.contains("app manifest 0.1.0 is older"));
        assert!(environment.operations().is_empty());

        Ok(())
    }

    #[test]
    fn update_accepts_newer_same_protocol_helper_from_older_app_manifest() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let daemon = FakeDaemon::start_with_response_lines(
            &paths,
            vec![vec![
                job_accepted_response("job_1"),
                job_completed("job_1", "current"),
            ]],
        )?;
        let manifest = app_manifest(
            "0.1.0",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            12_345_678,
        )
        .replace("\"version\": \"1.0.0\"", "\"version\": \"1.1.0\"")
        .replace("pv-helper-1.0.0", "pv-helper-1.1.0");
        let environment = TestEnvironment::new(&home, ScriptedClient::new().with_text(&manifest));

        let output = run_pv(&["update"], &environment)?;
        let _daemon_requests = daemon.join()?;

        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert_eq!(
            layout.active_release()?,
            Some(CURRENT_APP_VERSION.to_string())
        );
        let operations = environment.operations();
        assert_eq!(operations.len(), 1);
        assert!(operations[0].contains("downloads/pv-helper-1.1.0-"));
        assert!(operations[0].ends_with(&format!("sha256 {HELPER_BINARY_SHA256}")));
        assert!(
            state::fs::read_to_string(
                &paths
                    .app_release_helper(CURRENT_APP_VERSION)
                    .with_file_name("pv-helper.json")
            )?
            .contains("\"version\": \"1.1.0\"")
        );
        assert!(output.stdout.contains("app manifest 0.1.0 is older"));

        Ok(())
    }

    #[test]
    fn update_does_not_activate_app_when_required_helper_update_fails() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new()
                .with_text(&app_manifest(
                    "0.3.0",
                    APP_BINARY_SHA256,
                    u64::try_from(APP_BINARY.len())?,
                ))
                .with_download(APP_BINARY),
        )
        .with_helper_install_error("authentication cancelled");

        let output = run_pv(&["update"], &environment)?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert_eq!(
            layout.active_release()?,
            Some(CURRENT_APP_VERSION.to_string())
        );
        assert!(!state::fs::path_entry_exists(
            &paths.app_release_binary("0.3.0")
        )?);
        assert_eq!(environment.operations(), ["remove helper"]);
        assert!(output.stderr.contains("authentication cancelled"));

        Ok(())
    }

    #[test]
    fn update_downloads_and_activates_new_app_then_reexecs_managed_resource_continuation()
    -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let daemon = FakeDaemon::start(&paths, vec![health_response()])?;
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new()
                .with_text(&app_manifest(
                    "0.3.0",
                    APP_BINARY_SHA256,
                    u64::try_from(APP_BINARY.len())?,
                ))
                .with_download(APP_BINARY),
        );

        let output = run_pv(&["update"], &environment)?;
        let daemon_requests = if output.exit_code == ExitCode::SUCCESS {
            daemon.join()?
        } else {
            Vec::new()
        };
        let mut releases = state::fs::read_dir_paths(&paths.app_releases_dir())?
            .into_iter()
            .map(|path| path.file_name().unwrap_or("").to_string())
            .collect::<Vec<_>>();
        releases.sort();

        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert!(output.stderr.is_empty());
        assert_eq!(layout.active_release()?, Some("0.3.0".to_string()));
        assert_eq!(
            state::fs::read_to_string(&paths.app_release_binary("0.3.0"))?,
            "pv 0.3.0\n"
        );
        assert_eq!(
            daemon_requests,
            vec![json!({
                "protocol_version": daemon::PROTOCOL_VERSION,
                "command": "health"
            })]
        );
        assert_eq!(
            environment.operations(),
            vec![
                format!("bootout {LAUNCH_AGENT_LABEL}"),
                format!("bootstrap {}", launch_agent_path(&paths)),
                format!("kickstart {LAUNCH_AGENT_LABEL}"),
            ]
        );
        assert_eq!(
            environment.execs(),
            vec![(
                paths.active_pv_binary().as_std_path().to_path_buf(),
                vec!["internal:update-managed-resources".to_string()]
            )]
        );
        assert_update_snapshot(
            "update_downloads_and_activates_new_app_then_reexecs_managed_resource_continuation",
            (output, releases),
        );

        Ok(())
    }

    #[test]
    fn update_installs_combined_helper_before_committing_new_app_release() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let daemon = FakeDaemon::start(&paths, vec![health_response()])?;
        let manifest = app_manifest("0.3.0", APP_BINARY_SHA256, u64::try_from(APP_BINARY.len())?)
            .replace("\"version\": \"1.0.0\"", "\"version\": \"1.1.0\"")
            .replace("\"protocol_version\": 1", "\"protocol_version\": 2")
            .replace("pv-helper-1.0.0", "pv-helper-1.1.0");
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new()
                .with_text(&manifest)
                .with_download(APP_BINARY),
        );

        let output = run_pv(&["update"], &environment)?;
        let daemon_requests = daemon.join()?;

        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert_eq!(layout.active_release()?, Some("0.3.0".to_string()));
        assert_eq!(
            state::fs::read_to_string(&paths.app_release_helper("0.3.0"))?,
            String::from_utf8(HELPER_BINARY.to_vec())?
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&state::fs::read_to_string(
                &paths
                    .app_release_helper("0.3.0")
                    .with_file_name("pv-helper.json")
            )?)?,
            json!({
                "version": "1.1.0",
                "protocol_version": 2,
                "sha256": HELPER_BINARY_SHA256,
            })
        );
        assert_eq!(
            environment.operations(),
            vec![
                format!(
                    "install helper {} sha256 {HELPER_BINARY_SHA256}",
                    paths.app_release_helper("0.3.0")
                ),
                format!("bootout {LAUNCH_AGENT_LABEL}"),
                format!("bootstrap {}", launch_agent_path(&paths)),
                format!("kickstart {LAUNCH_AGENT_LABEL}"),
            ]
        );
        assert_eq!(
            daemon_requests,
            vec![json!({
                "protocol_version": daemon::PROTOCOL_VERSION,
                "command": "health"
            })]
        );
        assert!(
            output
                .stdout
                .contains("Privileged helper: updated to 1.1.0 (protocol 2)")
        );

        Ok(())
    }

    #[test]
    fn internal_managed_resource_continuation_skips_app_phase_and_submits_update_job()
    -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let daemon = FakeDaemon::start_with_response_lines(
            &paths,
            vec![vec![
                job_accepted_response("job_1"),
                job_completed(
                    "job_1",
                    "updated 2 artifact(s); reconciled: Gateway runtime skipped",
                ),
            ]],
        )?;
        let environment = TestEnvironment::new(&home, PanickingClient);

        let output = run_pv(&["internal:update-managed-resources"], &environment)?;
        let daemon_requests = daemon.join()?;

        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert!(output.stderr.is_empty());
        assert_eq!(
            daemon_requests,
            vec![json!({
                "protocol_version": daemon::PROTOCOL_VERSION,
                "command": "run_job",
                "kind": "update",
                "scope": "system",
            })]
        );
        assert!(environment.operations().is_empty());
        assert_update_snapshot(
            "internal_managed_resource_continuation_skips_app_phase_and_submits_update_job",
            output,
        );

        Ok(())
    }

    #[test]
    fn update_reports_reexec_failure_without_rolling_back_updated_app() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let daemon = FakeDaemon::start(&paths, vec![health_response()])?;
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new()
                .with_text(&app_manifest(
                    "0.3.0",
                    APP_BINARY_SHA256,
                    u64::try_from(APP_BINARY.len())?,
                ))
                .with_download(APP_BINARY),
        )
        .with_exec_error(io::Error::other("exec failed"));

        let output = run_pv(&["update"], &environment)?;
        let daemon_requests = daemon.join()?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert_eq!(layout.active_release()?, Some("0.3.0".to_string()));
        assert_eq!(
            daemon_requests,
            vec![json!({
                "protocol_version": daemon::PROTOCOL_VERSION,
                "command": "health"
            })]
        );
        assert_eq!(
            environment.execs(),
            vec![(
                paths.active_pv_binary().as_std_path().to_path_buf(),
                vec!["internal:update-managed-resources".to_string()]
            )]
        );
        assert_update_snapshot(
            "update_reports_reexec_failure_without_rolling_back_updated_app",
            output,
        );

        Ok(())
    }

    #[test]
    fn update_reloads_stale_launch_agent_before_restarting_updated_app() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &tempdir.path().join("old-pv"))?;
        let daemon = FakeDaemon::start(&paths, vec![health_response()])?;
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new()
                .with_text(&app_manifest(
                    "0.3.0",
                    APP_BINARY_SHA256,
                    u64::try_from(APP_BINARY.len())?,
                ))
                .with_download(APP_BINARY),
        );

        let output = run_pv(&["update"], &environment)?;
        let _daemon_requests = daemon.join()?;

        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert_eq!(layout.active_release()?, Some("0.3.0".to_string()));
        assert_eq!(
            environment.operations(),
            vec![
                format!("bootout {LAUNCH_AGENT_LABEL}"),
                format!("bootstrap {}", launch_agent_path(&paths)),
                format!("kickstart {LAUNCH_AGENT_LABEL}"),
            ]
        );

        Ok(())
    }

    #[test]
    fn update_bootstraps_launch_agent_when_bootout_reports_no_such_process() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let daemon = FakeDaemon::start(&paths, vec![health_response()])?;
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new()
                .with_text(&app_manifest(
                    "0.3.0",
                    APP_BINARY_SHA256,
                    u64::try_from(APP_BINARY.len())?,
                ))
                .with_download(APP_BINARY),
        )
        .with_unloaded_launch_agent();

        let output = run_pv(&["update"], &environment)?;
        let _daemon_requests = daemon.join()?;

        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert_eq!(layout.active_release()?, Some("0.3.0".to_string()));
        assert_eq!(
            environment.operations(),
            vec![
                format!("bootout {LAUNCH_AGENT_LABEL}"),
                format!("bootstrap {}", launch_agent_path(&paths)),
                format!("kickstart {LAUNCH_AGENT_LABEL}"),
            ]
        );

        Ok(())
    }

    #[test]
    fn update_holds_lock_until_daemon_transition_finishes() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let daemon = FakeDaemon::start(&paths, vec![health_response()])?;
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new()
                .with_text(&app_manifest(
                    "0.3.0",
                    APP_BINARY_SHA256,
                    u64::try_from(APP_BINARY.len())?,
                ))
                .with_download(APP_BINARY),
        )
        .with_update_lock_probe(paths.clone());

        let output = run_pv(&["update"], &environment)?;
        let _daemon_requests = daemon.join()?;
        let reacquired = state::UpdateLock::acquire(&paths);

        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert_eq!(layout.active_release()?, Some("0.3.0".to_string()));
        assert_eq!(
            environment.operations(),
            vec![
                format!("bootout {LAUNCH_AGENT_LABEL}"),
                format!("bootstrap {}", launch_agent_path(&paths)),
                format!("kickstart {LAUNCH_AGENT_LABEL}"),
                "lock probe held".to_string(),
            ]
        );
        assert!(reacquired.is_ok());

        Ok(())
    }

    #[test]
    fn update_normalizes_stale_launch_agent_without_restarting_when_current() -> anyhow::Result<()>
    {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &tempdir.path().join("old-pv"))?;
        let daemon = FakeDaemon::start_with_response_lines(
            &paths,
            vec![vec![
                job_accepted_response("job_1"),
                job_completed("job_1", "current"),
            ]],
        )?;
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new().with_text(&app_manifest(
                CURRENT_APP_VERSION,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                12_345_678,
            )),
        );

        let output = run_pv(&["update"], &environment)?;
        let requests = daemon.join()?;
        let plist = state::fs::read_to_string(&launch_agent_path(&paths))?;
        let parsed = LaunchAgentConfig::parse(&plist);

        assert_eq!(
            requests,
            vec![json!({
                "protocol_version": daemon::PROTOCOL_VERSION,
                "command": "run_job",
                "kind": "update",
                "scope": "system"
            })]
        );
        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert!(output.stderr.is_empty());
        assert_eq!(
            layout.active_release()?,
            Some(CURRENT_APP_VERSION.to_string())
        );
        assert!(environment.operations().is_empty());
        assert_eq!(
            parsed,
            Some(LaunchAgentConfig::new(
                paths.active_pv_binary(),
                paths.logs().join("launchd.out.log"),
                paths.logs().join("launchd.err.log"),
            ))
        );
        assert_update_snapshot(
            "update_normalizes_stale_launch_agent_without_restarting_when_current",
            output,
        );

        Ok(())
    }

    #[test]
    fn update_rejects_concurrent_update_lock_before_fetching_manifest() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let _update_lock = state::UpdateLock::acquire(&paths)?;
        let environment = TestEnvironment::new(&home, PanickingClient);

        let output = run_pv(&["update"], &environment)?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert_eq!(output.stdout, "PV update\n");
        assert!(output.stderr.contains(paths.update_lock().as_str()));
        assert_update_snapshot(
            "update_rejects_concurrent_update_lock_before_fetching_manifest",
            output,
        );

        Ok(())
    }

    #[test]
    fn update_check_rejects_update_lock_before_daemon_or_manifest() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let _update_lock = state::UpdateLock::acquire(&paths)?;
        let environment = TestEnvironment::new(&home, PanickingClient);

        let output = run_pv(&["update", "--check"], &environment)?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert!(output.stdout.is_empty());
        assert!(output.stderr.contains(paths.update_lock().as_str()));
        assert_update_snapshot(
            "update_check_rejects_update_lock_before_daemon_or_manifest",
            output,
        );

        Ok(())
    }

    #[test]
    fn internal_managed_resource_continuation_rejects_update_lock_before_daemon()
    -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let _update_lock = state::UpdateLock::acquire(&paths)?;
        let environment = TestEnvironment::new(&home, PanickingClient);

        let output = run_pv(&["internal:update-managed-resources"], &environment)?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert!(output.stdout.is_empty());
        assert!(output.stderr.contains(paths.update_lock().as_str()));
        assert_update_snapshot(
            "internal_managed_resource_continuation_rejects_update_lock_before_daemon",
            output,
        );

        Ok(())
    }

    #[test]
    fn update_checksum_mismatch_leaves_active_release_untouched() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new()
                .with_text(
                    &app_manifest(
                        "0.3.0",
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                        u64::try_from(APP_BINARY.len())?,
                    )
                    .replace("\"version\": \"1.0.0\"", "\"version\": \"1.1.0\"")
                    .replace("pv-helper-1.0.0", "pv-helper-1.1.0"),
                )
                .with_download(APP_BINARY),
        );

        let output = run_pv(&["update"], &environment)?;
        let temporary_downloads = update_temporary_files(&paths)?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert_eq!(output.stdout, "PV update\n");
        assert_eq!(
            layout.active_release()?,
            Some(CURRENT_APP_VERSION.to_string())
        );
        assert!(!state::fs::path_entry_exists(
            &paths.app_release_binary("0.3.0")
        )?);
        assert!(temporary_downloads.is_empty());
        assert_update_snapshot(
            "update_checksum_mismatch_leaves_active_release_untouched",
            output,
        );

        Ok(())
    }

    #[test]
    fn update_download_failure_removes_temporary_download() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new()
                .with_text(
                    &app_manifest("0.3.0", APP_BINARY_SHA256, u64::try_from(APP_BINARY.len())?)
                        .replace("\"version\": \"1.0.0\"", "\"version\": \"1.1.0\"")
                        .replace("pv-helper-1.0.0", "pv-helper-1.1.0"),
                )
                .with_download_error(ResourcesError::HttpRequestFailed {
                    url: APP_BINARY_URL.to_string(),
                    reason: "connection reset".to_string(),
                }),
        );

        let output = run_pv(&["update"], &environment)?;
        let temporary_downloads = update_temporary_files(&paths)?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert_eq!(output.stdout, "PV update\n");
        assert_eq!(
            layout.active_release()?,
            Some(CURRENT_APP_VERSION.to_string())
        );
        assert!(!state::fs::path_entry_exists(
            &paths.app_release_binary("0.3.0")
        )?);
        assert!(temporary_downloads.is_empty());

        Ok(())
    }

    #[test]
    fn update_preserves_checksum_error_when_download_cleanup_fails() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let environment = TestEnvironment::new(
            &home,
            ReadOnlyDownloadsClient {
                manifest: app_manifest(
                    "0.3.0",
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    u64::try_from(APP_BINARY.len())?,
                ),
                bytes: APP_BINARY.to_vec(),
                downloads: paths.downloads().to_path_buf(),
            },
        );

        let result = run_pv(&["update"], &environment);
        set_permissions(paths.downloads(), 0o700)?;
        let output = result?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert_eq!(output.stdout, "PV update\n");
        assert!(output.stderr.contains("checksum mismatch"));
        assert!(output.stderr.contains("warning: failed to remove"));
        assert_eq!(
            layout.active_release()?,
            Some(CURRENT_APP_VERSION.to_string())
        );

        Ok(())
    }

    #[test]
    fn update_accepts_non_utf8_installed_binary_fixture() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_active_release(&paths, CURRENT_APP_VERSION, b"\xffpv current\n")?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let daemon = FakeDaemon::start_with_response_lines(
            &paths,
            vec![vec![
                job_accepted_response("job_1"),
                job_completed("job_1", "current"),
            ]],
        )?;
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new().with_text(&app_manifest(
                CURRENT_APP_VERSION,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                12_345_678,
            )),
        );

        let output = run_pv(&["update"], &environment)?;
        let requests = daemon.join()?;

        assert_eq!(
            requests,
            vec![json!({
                "protocol_version": daemon::PROTOCOL_VERSION,
                "command": "run_job",
                "kind": "update",
                "scope": "system"
            })]
        );
        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert!(output.stderr.is_empty());
        assert_eq!(
            layout.active_release()?,
            Some(CURRENT_APP_VERSION.to_string())
        );

        Ok(())
    }

    #[test]
    fn update_rolls_back_when_daemon_health_fails_after_activation() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let daemon = FakeDaemon::start(
            &paths,
            vec![
                daemon_error_response("daemon boot failed"),
                health_response(),
            ],
        )?;
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new()
                .with_text(&app_manifest(
                    "0.3.0",
                    APP_BINARY_SHA256,
                    u64::try_from(APP_BINARY.len())?,
                ))
                .with_download(APP_BINARY),
        );

        let output = match run_pv(&["update"], &environment) {
            Ok(output) => output,
            Err(error) => RunOutput {
                exit_code: ExitCode::FAILURE,
                stdout: String::new(),
                stderr: error.to_string(),
            },
        };
        let daemon_requests = if environment.operations().len() >= 2 {
            daemon.join()?
        } else {
            Vec::new()
        };

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert_eq!(
            layout.active_release()?,
            Some(CURRENT_APP_VERSION.to_string())
        );
        assert!(!state::fs::path_entry_exists(
            &paths.app_release_binary("0.3.0")
        )?);
        assert_eq!(
            environment.operations(),
            vec![
                format!("bootout {LAUNCH_AGENT_LABEL}"),
                format!("bootstrap {}", launch_agent_path(&paths)),
                format!("kickstart {LAUNCH_AGENT_LABEL}"),
                format!("bootout {LAUNCH_AGENT_LABEL}"),
                format!("bootstrap {}", launch_agent_path(&paths)),
                format!("kickstart {LAUNCH_AGENT_LABEL}"),
            ]
        );
        assert_eq!(
            daemon_requests,
            vec![
                json!({
                    "protocol_version": daemon::PROTOCOL_VERSION,
                    "command": "health"
                }),
                json!({
                    "protocol_version": daemon::PROTOCOL_VERSION,
                    "command": "health"
                }),
            ]
        );
        assert_update_snapshot(
            "update_rolls_back_when_daemon_health_fails_after_activation",
            output,
        );

        Ok(())
    }

    #[test]
    fn update_accepts_protocol_mismatch_health_after_activation() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let daemon = FakeDaemon::start_until_idle(
            &paths,
            vec![
                health_response_with_protocol(daemon::PROTOCOL_VERSION + 1),
                health_response(),
            ],
            Duration::from_millis(100),
        )?;
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new()
                .with_text(&app_manifest(
                    "0.3.0",
                    APP_BINARY_SHA256,
                    u64::try_from(APP_BINARY.len())?,
                ))
                .with_download(APP_BINARY),
        );

        let output = run_pv(&["update"], &environment)?;
        let daemon_requests = daemon.join()?;

        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert_eq!(layout.active_release()?, Some("0.3.0".to_string()));
        assert_eq!(
            environment.operations(),
            vec![
                format!("bootout {LAUNCH_AGENT_LABEL}"),
                format!("bootstrap {}", launch_agent_path(&paths)),
                format!("kickstart {LAUNCH_AGENT_LABEL}"),
            ]
        );
        assert_eq!(
            daemon_requests,
            vec![json!({
                "protocol_version": daemon::PROTOCOL_VERSION,
                "command": "health"
            })]
        );
        assert_update_snapshot(
            "update_accepts_protocol_mismatch_health_after_activation",
            output,
        );

        Ok(())
    }

    #[test]
    fn update_rolls_back_protocol_mismatch_health_error_after_activation() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let daemon = FakeDaemon::start_until_idle(
            &paths,
            vec![
                daemon_error_response_with_protocol(
                    daemon::PROTOCOL_VERSION + 1,
                    "daemon boot failed",
                ),
                health_response(),
            ],
            Duration::from_millis(100),
        )?;
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new()
                .with_text(&app_manifest(
                    "0.3.0",
                    APP_BINARY_SHA256,
                    u64::try_from(APP_BINARY.len())?,
                ))
                .with_download(APP_BINARY),
        );

        let output = run_pv(&["update"], &environment)?;
        let daemon_requests = daemon.join()?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert_eq!(
            layout.active_release()?,
            Some(CURRENT_APP_VERSION.to_string())
        );
        assert_eq!(
            daemon_requests,
            vec![
                json!({
                    "protocol_version": daemon::PROTOCOL_VERSION,
                    "command": "health"
                }),
                json!({
                    "protocol_version": daemon::PROTOCOL_VERSION,
                    "command": "health"
                })
            ]
        );
        assert_update_snapshot(
            "update_rolls_back_protocol_mismatch_health_error_after_activation",
            output,
        );

        Ok(())
    }

    #[test]
    fn update_restores_previous_helper_when_updated_app_rolls_back() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let daemon = FakeDaemon::start(
            &paths,
            vec![
                daemon_error_response("updated daemon boot failed"),
                health_response(),
            ],
        )?;
        let manifest = app_manifest("0.3.0", APP_BINARY_SHA256, u64::try_from(APP_BINARY.len())?)
            .replace("\"version\": \"1.0.0\"", "\"version\": \"1.1.0\"")
            .replace("pv-helper-1.0.0", "pv-helper-1.1.0");
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new()
                .with_text(&manifest)
                .with_download(APP_BINARY),
        )
        .with_helper_rollback_cleanup_warning(
            "installed privileged helper but could not remove lifecycle transaction files: retained rollback metadata",
        );

        let output = run_pv(&["update"], &environment)?;
        let _daemon_requests = daemon.join()?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert_eq!(
            layout.active_release()?,
            Some(CURRENT_APP_VERSION.to_string())
        );
        assert_eq!(
            environment.helper_status.borrow().as_ref(),
            Some(&PrivilegedHelperStatus {
                version: PRIVILEGED_HELPER_VERSION.to_string(),
                protocol_version: platform::HELPER_PROTOCOL_VERSION,
                owner_uid: 501,
            })
        );
        let operations = environment.operations();
        assert!(operations.iter().any(|operation| {
            operation.contains("install helper") && operation.contains("helper-rollback")
        }));
        assert_update_snapshot(
            "update_restores_previous_helper_and_reports_cleanup_warning",
            output,
        );

        Ok(())
    }

    #[test]
    fn update_retains_helper_candidate_when_restoring_helper_during_app_rollback_fails()
    -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let daemon = FakeDaemon::start(
            &paths,
            vec![daemon_error_response("updated daemon boot failed")],
        )?;
        let manifest = app_manifest("0.3.0", APP_BINARY_SHA256, u64::try_from(APP_BINARY.len())?)
            .replace("\"version\": \"1.0.0\"", "\"version\": \"1.1.0\"")
            .replace("pv-helper-1.0.0", "pv-helper-1.1.0");
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new()
                .with_text(&manifest)
                .with_download(APP_BINARY),
        )
        .with_helper_restore_error("restoring the registered helper failed");

        let output = run_pv(&["update"], &environment)?;
        let _daemon_requests = daemon.join()?;
        let rollback_candidates = state::fs::read_dir_paths(paths.downloads())?
            .into_iter()
            .filter(|path| path.file_name().unwrap_or("").contains("helper-rollback"))
            .collect::<Vec<_>>();

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert_eq!(
            layout.active_release()?,
            Some(CURRENT_APP_VERSION.to_string())
        );
        assert_eq!(rollback_candidates.len(), 1);
        assert_eq!(
            state::fs::read_to_string(&rollback_candidates[0])?.as_bytes(),
            HELPER_BINARY
        );
        assert_update_snapshot(
            "update_retains_helper_candidate_when_restoring_helper_during_app_rollback_fails",
            output,
        );

        Ok(())
    }

    #[test]
    fn update_reports_rollback_symlink_restore_failure() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let daemon = FakeDaemon::start(&paths, vec![daemon_error_response("daemon boot failed")])?;
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new()
                .with_text(&app_manifest(
                    "0.3.0",
                    APP_BINARY_SHA256,
                    u64::try_from(APP_BINARY.len())?,
                ))
                .with_download(APP_BINARY),
        )
        .with_delete_on_first_kickstart(paths.app_release_binary(CURRENT_APP_VERSION));

        let output = run_pv(&["update"], &environment)?;
        let _daemon_requests = daemon.join()?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert_eq!(layout.active_release()?, Some("0.3.0".to_string()));
        assert_update_snapshot("update_reports_rollback_symlink_restore_failure", output);

        Ok(())
    }

    #[test]
    fn update_reports_rollback_daemon_restart_failure_after_restore() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let daemon = FakeDaemon::start(
            &paths,
            vec![
                daemon_error_response("updated daemon boot failed"),
                daemon_error_response("rollback daemon boot failed"),
            ],
        )?;
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new()
                .with_text(&app_manifest(
                    "0.3.0",
                    APP_BINARY_SHA256,
                    u64::try_from(APP_BINARY.len())?,
                ))
                .with_download(APP_BINARY),
        );

        let output = run_pv(&["update"], &environment)?;
        let _daemon_requests = daemon.join()?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert_eq!(
            layout.active_release()?,
            Some(CURRENT_APP_VERSION.to_string())
        );
        assert!(!state::fs::path_entry_exists(
            &paths.app_release_binary("0.3.0")
        )?);
        assert_update_snapshot(
            "update_reports_rollback_daemon_restart_failure_after_restore",
            output,
        );

        Ok(())
    }

    #[test]
    fn update_reports_migration_failure_marker_after_health_failure() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let daemon = FakeDaemon::start(
            &paths,
            vec![
                daemon_error_response("daemon boot failed"),
                health_response(),
            ],
        )?;
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new()
                .with_text(&app_manifest(
                    "0.3.0",
                    APP_BINARY_SHA256,
                    u64::try_from(APP_BINARY.len())?,
                ))
                .with_download(APP_BINARY),
        )
        .with_startup_marker_on_first_kickstart(
            paths.daemon_startup_error(),
            r#"{"kind":"migration_failed","message":"migration 7 (resource_port_roles) failed"}"#,
        );

        let output = run_pv(&["update"], &environment)?;
        let _daemon_requests = daemon.join()?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert_eq!(
            layout.active_release()?,
            Some(CURRENT_APP_VERSION.to_string())
        );
        assert!(!state::fs::path_entry_exists(
            &paths.app_release_binary("0.3.0")
        )?);
        assert_update_snapshot(
            "update_reports_migration_failure_marker_after_health_failure",
            output,
        );

        Ok(())
    }

    #[test]
    fn update_ignores_stale_startup_failure_marker_after_health_failure() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        state::fs::write_sensitive_file(
            &paths.daemon_startup_error(),
            r#"{"kind":"migration_failed","message":"stale migration failure"}"#,
        )?;
        let daemon = FakeDaemon::start(
            &paths,
            vec![
                daemon_error_response("daemon boot failed"),
                health_response(),
            ],
        )?;
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new()
                .with_text(&app_manifest(
                    "0.3.0",
                    APP_BINARY_SHA256,
                    u64::try_from(APP_BINARY.len())?,
                ))
                .with_download(APP_BINARY),
        );

        let output = run_pv(&["update"], &environment)?;
        let _daemon_requests = daemon.join()?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert_eq!(
            layout.active_release()?,
            Some(CURRENT_APP_VERSION.to_string())
        );
        assert!(!output.stderr.contains("stale migration failure"));
        assert_update_snapshot(
            "update_ignores_stale_startup_failure_marker_after_health_failure",
            output,
        );

        Ok(())
    }

    #[test]
    fn update_fails_before_network_when_active_release_version_differs() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_active_release(&paths, "0.3.0", b"pv 0.3.0\n")?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let environment = TestEnvironment::new(&home, PanickingClient);

        let output = run_pv(&["update"], &environment)?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert_eq!(layout.active_release()?, Some("0.3.0".to_string()));
        assert_update_snapshot(
            "update_fails_before_network_when_active_release_version_differs",
            output,
        );

        Ok(())
    }

    #[test]
    fn update_fails_before_activation_when_launch_agent_is_missing() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        let environment = TestEnvironment::new(&home, PanickingClient);

        let output = run_pv(&["update"], &environment)?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert_eq!(
            layout.active_release()?,
            Some(CURRENT_APP_VERSION.to_string())
        );
        assert_update_snapshot(
            "update_fails_before_activation_when_launch_agent_is_missing",
            output,
        );

        Ok(())
    }

    #[test]
    fn update_fails_before_activation_when_launch_agent_is_not_pv_owned() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_conflicting_launch_agent(&paths)?;
        let environment = TestEnvironment::new(&home, PanickingClient);

        let output = run_pv(&["update"], &environment)?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert_eq!(
            layout.active_release()?,
            Some(CURRENT_APP_VERSION.to_string())
        );
        assert_update_snapshot(
            "update_fails_before_activation_when_launch_agent_is_not_pv_owned",
            output,
        );

        Ok(())
    }

    #[test]
    fn update_size_mismatch_leaves_active_release_untouched() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new()
                .with_text(&app_manifest("0.3.0", APP_BINARY_SHA256, 999))
                .with_download(APP_BINARY),
        );

        let output = run_pv(&["update"], &environment)?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert_eq!(output.stdout, "PV update\n");
        assert_eq!(
            layout.active_release()?,
            Some(CURRENT_APP_VERSION.to_string())
        );
        assert!(!state::fs::path_entry_exists(
            &paths.app_release_binary("0.3.0")
        )?);
        assert_update_snapshot(
            "update_size_mismatch_leaves_active_release_untouched",
            output,
        );

        Ok(())
    }

    #[test]
    fn update_prunes_older_app_releases_after_success() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        let old_source = paths.downloads().join("pv-0.0.9");
        write_bytes(&old_source, b"pv 0.0.9\n")?;
        layout.install_release_binary("0.0.9", &old_source)?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let daemon = FakeDaemon::start(&paths, vec![health_response()])?;
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new()
                .with_text(&app_manifest(
                    "0.3.0",
                    APP_BINARY_SHA256,
                    u64::try_from(APP_BINARY.len())?,
                ))
                .with_download(APP_BINARY),
        );

        let output = run_pv(&["update"], &environment)?;
        let _daemon_requests = daemon.join()?;
        let mut releases = state::fs::read_dir_paths(&paths.app_releases_dir())?
            .into_iter()
            .map(|path| path.file_name().unwrap_or("").to_string())
            .collect::<Vec<_>>();
        releases.sort();

        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert_eq!(layout.active_release()?, Some("0.3.0".to_string()));
        assert_eq!(releases, vec![CURRENT_APP_VERSION, "0.3.0"]);

        Ok(())
    }

    #[test]
    fn update_warns_when_pruning_old_app_release_fails() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let paths = PvPaths::for_home(home.clone());
        state::fs::ensure_layout(&paths)?;
        let layout = install_current_release(&paths)?;
        state::fs::write_sensitive_file(&paths.app_releases_dir().join("0.0.8"), "not a dir")?;
        write_launch_agent(&paths, &paths.active_pv_binary())?;
        let daemon = FakeDaemon::start(&paths, vec![health_response()])?;
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new()
                .with_text(&app_manifest(
                    "0.3.0",
                    APP_BINARY_SHA256,
                    u64::try_from(APP_BINARY.len())?,
                ))
                .with_download(APP_BINARY),
        );

        let output = run_pv(&["update"], &environment)?;
        let _daemon_requests = daemon.join()?;

        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert_eq!(layout.active_release()?, Some("0.3.0".to_string()));
        assert!(state::fs::path_entry_exists(
            &paths.app_releases_dir().join("0.0.8")
        )?);
        assert_update_snapshot("update_warns_when_pruning_old_app_release_fails", output);

        Ok(())
    }

    #[derive(Debug)]
    struct FakeDaemon {
        handle: thread::JoinHandle<anyhow::Result<Vec<serde_json::Value>>>,
    }

    impl FakeDaemon {
        fn start(paths: &PvPaths, responses: Vec<serde_json::Value>) -> anyhow::Result<Self> {
            let responses = responses
                .into_iter()
                .map(|response| vec![response])
                .collect();

            Self::start_with_response_lines(paths, responses)
        }

        #[expect(
            clippy::disallowed_methods,
            reason = "update tests use a one-shot fake Unix socket daemon"
        )]
        fn start_with_response_lines(
            paths: &PvPaths,
            responses: Vec<Vec<serde_json::Value>>,
        ) -> anyhow::Result<Self> {
            let listener = UnixListener::bind(paths.daemon_socket())?;
            let handle = thread::spawn(move || {
                let mut requests = Vec::new();
                for response in responses {
                    let (mut stream, _address) = listener.accept()?;
                    let mut request = String::new();
                    BufReader::new(stream.try_clone()?).read_line(&mut request)?;
                    for line in response {
                        stream.write_all(format!("{line}\n").as_bytes())?;
                    }
                    requests.push(serde_json::from_str(request.trim_end())?);
                }

                Ok(requests)
            });

            Ok(Self { handle })
        }

        #[expect(
            clippy::disallowed_methods,
            reason = "update tests use a bounded fake Unix socket daemon"
        )]
        fn start_until_idle(
            paths: &PvPaths,
            responses: Vec<serde_json::Value>,
            idle_timeout: Duration,
        ) -> anyhow::Result<Self> {
            let listener = UnixListener::bind(paths.daemon_socket())?;
            listener.set_nonblocking(true)?;
            let handle = thread::spawn(move || {
                let mut requests = Vec::new();
                let mut responses = VecDeque::from(responses);
                let mut last_activity = Instant::now();
                while let Some(response) = responses.front() {
                    match listener.accept() {
                        Ok((mut stream, _address)) => {
                            stream.set_nonblocking(false)?;
                            let mut request = String::new();
                            BufReader::new(stream.try_clone()?).read_line(&mut request)?;
                            stream.write_all(format!("{response}\n").as_bytes())?;
                            requests.push(serde_json::from_str(request.trim_end())?);
                            responses.pop_front();
                            last_activity = Instant::now();
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            if !requests.is_empty() && last_activity.elapsed() >= idle_timeout {
                                break;
                            }
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => return Err(error.into()),
                    }
                }

                Ok(requests)
            });

            Ok(Self { handle })
        }

        fn join(self) -> anyhow::Result<Vec<serde_json::Value>> {
            self.handle.join().map_err(|error| {
                let payload = if let Some(s) = error.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = error.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    "unknown panic".to_string()
                };
                anyhow::anyhow!("fake daemon thread panicked: {payload}")
            })?
        }
    }

    #[derive(Debug)]
    struct PanickingClient;

    impl ResourceHttpClient for PanickingClient {
        fn get_text(&self, url: &str) -> resources::Result<String> {
            Err(ResourcesError::HttpRequestFailed {
                url: url.to_string(),
                reason: "get_text should not be called when daemon is missing".to_string(),
            })
        }

        fn download(&self, url: &str, _writer: &mut dyn Write) -> resources::Result<()> {
            Err(ResourcesError::HttpRequestFailed {
                url: url.to_string(),
                reason: "download should not be called when daemon is missing".to_string(),
            })
        }
    }

    #[derive(Debug)]
    struct ScriptedClient {
        text_responses: RefCell<VecDeque<Result<String, ResourcesError>>>,
        download_responses: RefCell<VecDeque<Result<Vec<u8>, ResourcesError>>>,
    }

    impl ScriptedClient {
        fn new() -> Self {
            Self {
                text_responses: RefCell::new(VecDeque::new()),
                download_responses: RefCell::new(VecDeque::new()),
            }
        }

        fn with_text(self, text: &str) -> Self {
            self.text_responses
                .borrow_mut()
                .push_back(Ok(text.to_string()));
            self
        }

        fn with_error(self, error: ResourcesError) -> Self {
            self.text_responses.borrow_mut().push_back(Err(error));
            self
        }

        fn with_download(self, bytes: &[u8]) -> Self {
            self.download_responses
                .borrow_mut()
                .push_back(Ok(bytes.to_vec()));
            self
        }

        fn with_download_error(self, error: ResourcesError) -> Self {
            self.download_responses.borrow_mut().push_back(Err(error));
            self
        }
    }

    impl ResourceHttpClient for ScriptedClient {
        fn get_text(&self, url: &str) -> resources::Result<String> {
            self.text_responses
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| {
                    Err(ResourcesError::HttpRequestFailed {
                        url: url.to_string(),
                        reason: "no scripted text response".to_string(),
                    })
                })
        }

        fn download(&self, url: &str, writer: &mut dyn Write) -> resources::Result<()> {
            let bytes = self
                .download_responses
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| {
                    if url.contains("/pv-helper-") {
                        Ok(HELPER_BINARY.to_vec())
                    } else {
                        Err(ResourcesError::HttpRequestFailed {
                            url: url.to_string(),
                            reason: "no scripted byte response".to_string(),
                        })
                    }
                })?;
            writer
                .write_all(&bytes)
                .map_err(|source| ResourcesError::HttpRequestFailed {
                    url: url.to_string(),
                    reason: source.to_string(),
                })
        }
    }

    #[derive(Debug)]
    struct ReadOnlyDownloadsClient {
        manifest: String,
        bytes: Vec<u8>,
        downloads: Utf8PathBuf,
    }

    impl ResourceHttpClient for ReadOnlyDownloadsClient {
        fn get_text(&self, _url: &str) -> resources::Result<String> {
            Ok(self.manifest.clone())
        }

        fn download(&self, url: &str, writer: &mut dyn Write) -> resources::Result<()> {
            writer
                .write_all(&self.bytes)
                .map_err(|source| ResourcesError::HttpRequestFailed {
                    url: url.to_string(),
                    reason: source.to_string(),
                })?;
            set_permissions(&self.downloads, 0o500).map_err(|source| {
                ResourcesError::HttpRequestFailed {
                    url: url.to_string(),
                    reason: source.to_string(),
                }
            })
        }
    }

    #[derive(Debug)]
    struct RunOutput {
        exit_code: ExitCode,
        stdout: String,
        stderr: String,
    }

    fn run_pv(args: &[&str], environment: &impl Environment) -> anyhow::Result<RunOutput> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let args = std::iter::once("pv").chain(args.iter().copied());
        let exit_code = run_with_environment(args, environment, &mut stdout, &mut stderr)?;

        Ok(RunOutput {
            exit_code,
            stdout: String::from_utf8(stdout)?,
            stderr: String::from_utf8(stderr)?,
        })
    }

    fn managed_resource_update_check_response(
        current_artifact_path: impl AsRef<Path>,
    ) -> serde_json::Value {
        managed_resource_status_response(
            "update_available",
            current_artifact_path,
            json!({"latest_artifact_version": "8.8.1-pv1"}),
        )
    }

    fn managed_resource_status_response(
        status: &str,
        current_artifact_path: impl AsRef<Path>,
        extra: serde_json::Value,
    ) -> serde_json::Value {
        let mut resource = json!({
            "status": status,
            "resource": "redis",
            "track": "8.8",
            "current_artifact_version": "8.8.0-pv1",
            "current_artifact_path": current_artifact_path.as_ref().to_string_lossy(),
            "latest_artifact_version": null,
            "current_revocation": null,
            "latest_revocation": null,
            "blocked_by": null,
            "reason": null
        });

        if let Some(obj) = extra.as_object() {
            for (key, value) in obj {
                resource[key] = value.clone();
            }
        }

        json!({
            "type": "response",
            "protocol_version": daemon::PROTOCOL_VERSION,
            "status": "ok",
            "message": "Managed Resource update check completed",
            "update_check": {
                "managed_resources": [resource]
            }
        })
    }

    fn health_response() -> serde_json::Value {
        health_response_with_protocol(daemon::PROTOCOL_VERSION)
    }

    fn health_response_with_protocol(protocol_version: u16) -> serde_json::Value {
        json!({
            "type": "response",
            "protocol_version": protocol_version,
            "status": "ok",
            "message": "daemon healthy"
        })
    }

    fn daemon_error_response(message: &str) -> serde_json::Value {
        daemon_error_response_with_protocol(daemon::PROTOCOL_VERSION, message)
    }

    fn daemon_error_response_with_protocol(
        protocol_version: u16,
        message: &str,
    ) -> serde_json::Value {
        json!({
            "type": "response",
            "protocol_version": protocol_version,
            "status": "error",
            "message": message
        })
    }

    fn job_accepted_response(job_id: &str) -> serde_json::Value {
        json!({
            "type": "response",
            "protocol_version": daemon::PROTOCOL_VERSION,
            "status": "accepted",
            "message": "job accepted",
            "job_id": job_id,
        })
    }

    fn job_completed(job_id: &str, summary: &str) -> serde_json::Value {
        json!({
            "type": "job_completed",
            "job_id": job_id,
            "summary": summary,
        })
    }

    fn assert_update_snapshot(name: &'static str, snapshot: impl std::fmt::Debug) {
        let mut settings = Settings::clone_current();
        settings.add_filter(r#"/[^ \n"]+/home/\.pv"#, "<pv-home>");
        settings.add_filter(
            r#"/[^ \n"]+/home/Library/LaunchAgents/com\.prvious\.pv\.daemon\.plist"#,
            "<launch-agent>",
        );
        settings.add_filter(r"/[\w\-]+/home/\.pv", "<pv-home>");
        settings.add_filter(r"/private<tempdir>", "<tempdir>");
        settings.add_filter(r"[a-f0-9]{64}", "<sha256>");
        settings.add_filter(
            r"pv-helper-rollback-[0-9]+-[0-9]+\.tmp",
            "pv-helper-rollback-<id>.tmp",
        );
        settings.bind(|| {
            assert_debug_snapshot!(name, snapshot);
        });
    }

    fn install_active_release(
        paths: &PvPaths,
        version: &str,
        content: &[u8],
    ) -> anyhow::Result<AppReleaseLayout> {
        let layout = AppReleaseLayout::new(paths.clone());
        let source = paths.downloads().join(format!("pv-{version}"));
        write_bytes(&source, content)?;
        layout.install_release_binary(version, &source)?;
        layout.activate_release(version)?;

        Ok(layout)
    }

    fn install_current_release(paths: &PvPaths) -> anyhow::Result<AppReleaseLayout> {
        let content = format!("pv {CURRENT_APP_VERSION}\n");
        let layout = install_active_release(paths, CURRENT_APP_VERSION, content.as_bytes())?;
        let helper_source = paths.downloads().join("current-pv-helper");
        write_bytes(&helper_source, HELPER_BINARY)?;
        let helper_path = layout.install_release_helper(CURRENT_APP_VERSION, &helper_source)?;
        state::fs::write_sensitive_file(
            &helper_path.with_file_name("pv-helper.json"),
            &format!(
                "{}\n",
                serde_json::to_string_pretty(&json!({
                    "version": PRIVILEGED_HELPER_VERSION,
                    "protocol_version": platform::HELPER_PROTOCOL_VERSION,
                    "sha256": HELPER_BINARY_SHA256,
                }))?
            ),
        )?;

        Ok(layout)
    }

    fn write_launch_agent(paths: &PvPaths, program_path: &Utf8Path) -> anyhow::Result<()> {
        let config = LaunchAgentConfig::new(
            program_path.to_path_buf(),
            paths.logs().join("launchd.out.log"),
            paths.logs().join("launchd.err.log"),
        );
        state::fs::write_sensitive_file(&launch_agent_path(paths), &config.render()?)?;

        Ok(())
    }

    fn write_conflicting_launch_agent(paths: &PvPaths) -> anyhow::Result<()> {
        let config = LaunchAgentConfig::new(
            paths.active_pv_binary(),
            paths.logs().join("launchd.out.log"),
            paths.logs().join("launchd.err.log"),
        );
        let content = config.render()?.replace("<!-- Managed by PV -->\n", "");
        state::fs::write_sensitive_file(&launch_agent_path(paths), &content)?;

        Ok(())
    }

    fn launch_agent_path(paths: &PvPaths) -> camino::Utf8PathBuf {
        paths
            .home()
            .join("Library/LaunchAgents/com.prvious.pv.daemon.plist")
    }

    fn update_temporary_files(paths: &PvPaths) -> anyhow::Result<Vec<Utf8PathBuf>> {
        Ok(state::fs::read_dir_paths(paths.downloads())?
            .into_iter()
            .filter(|path| path.extension() == Some("tmp"))
            .collect())
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "update tests write raw binary fixture bytes"
    )]
    fn write_bytes(path: &Utf8Path, content: &[u8]) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            state::fs::ensure_user_dir(parent)?;
        }
        std::fs::write(path, content)?;

        Ok(())
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "update tests force cleanup failures with temporary permissions"
    )]
    fn set_permissions(path: &Utf8Path, mode: u32) -> anyhow::Result<()> {
        std::fs::set_permissions(path, Permissions::from_mode(mode))?;

        Ok(())
    }

    fn app_manifest(version: &str, sha256: &str, size: u64) -> String {
        format!(
            r#"
{{
  "schema_version": 2,
  "channel": "stable",
  "version": "{version}",
  "minimum_pv_version": "0.1.0",
  "published_at": "2026-06-11T12:00:00Z",
  "assets": [
    {{
      "platform": "darwin-arm64",
      "url": "{APP_BINARY_URL}",
      "sha256": "{sha256}",
      "size": {size},
      "helper": {{
        "version": "1.0.0",
        "protocol_version": 1,
        "url": "https://downloads.example.test/pv/{version}/pv-helper-1.0.0-darwin-arm64",
        "sha256": "{HELPER_BINARY_SHA256}",
        "size": {}
      }}
    }},
    {{
      "platform": "darwin-amd64",
      "url": "https://downloads.example.test/pv/{version}/pv-darwin-amd64",
      "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
      "size": {size},
      "helper": {{
        "version": "1.0.0",
        "protocol_version": 1,
        "url": "https://downloads.example.test/pv/{version}/pv-helper-1.0.0-darwin-amd64",
        "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "size": {}
      }}
    }}
  ]
}}
"#,
            HELPER_BINARY.len(),
            HELPER_BINARY.len(),
        )
    }

    fn helper_only_update_manifest() -> String {
        app_manifest(
            CURRENT_APP_VERSION,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            12_345_678,
        )
        .replace("\"version\": \"1.0.0\"", "\"version\": \"1.1.0\"")
        .replace("pv-helper-1.0.0", "pv-helper-1.1.0")
    }

    const APP_MANIFEST: &str = r#"
{
  "schema_version": 2,
  "channel": "stable",
  "version": "0.3.0",
  "minimum_pv_version": "0.1.0",
  "published_at": "2026-06-11T12:00:00Z",
  "assets": [
    {
      "platform": "darwin-arm64",
      "url": "https://downloads.example.test/pv/0.3.0/pv-darwin-arm64",
      "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "size": 12345678,
      "helper": {
        "version": "1.0.0",
        "protocol_version": 1,
        "url": "https://downloads.example.test/pv/0.3.0/pv-helper-1.0.0-darwin-arm64",
        "sha256": "f15b9ec9f06fc9e7e92af6e7cdfe82ae574bdc11f09bf796e44280989b1378d2",
        "size": 16
      }
    },
    {
      "platform": "darwin-amd64",
      "url": "https://downloads.example.test/pv/0.3.0/pv-darwin-amd64",
      "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
      "size": 12345678,
      "helper": {
        "version": "1.0.0",
        "protocol_version": 1,
        "url": "https://downloads.example.test/pv/0.3.0/pv-helper-1.0.0-darwin-amd64",
        "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "size": 16
      }
    }
  ]
}
"#;
}
