#[cfg(unix)]
mod update_tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::ffi::OsString;
    use std::io::{self, BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::path::{Path, PathBuf};
    use std::process::ExitCode;
    use std::thread;

    use camino::Utf8Path;
    use camino_tempfile::tempdir;
    use cli::{Environment, run_with_environment};
    use insta::{Settings, assert_debug_snapshot};
    use resources::{ResourceHttpClient, ResourcesError};
    use serde_json::json;
    use state::PvPaths;

    const APP_MANIFEST_URL: &str = "https://updates.example.test/pv-app-manifest.json";

    struct TestEnvironment {
        home: PathBuf,
        client: Box<dyn ResourceHttpClient>,
    }

    impl TestEnvironment {
        fn new(home: &Utf8Path, client: impl ResourceHttpClient + 'static) -> Self {
            Self {
                home: home.as_std_path().to_path_buf(),
                client: Box::new(client),
            }
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

        fn app_update_manifest_url(&self) -> Option<String> {
            Some(APP_MANIFEST_URL.to_string())
        }

        fn app_update_platform(&self) -> Option<self_update::AppUpdatePlatform> {
            Some(self_update::AppUpdatePlatform::DarwinArm64)
        }

        fn resource_http_client(&self) -> Option<&dyn ResourceHttpClient> {
            Some(self.client.as_ref())
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
                    "protocol_version": 1,
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
        let _daemon = FakeDaemon::start(
            &paths,
            vec![
                health_response(),
                managed_resource_update_check_response(
                    paths.resources().join("redis/8.8/releases/8.8.0-pv1"),
                ),
            ],
        )?;
        let environment = TestEnvironment::new(&home, ScriptedClient::new().with_text("not json"));

        let output = run_pv(&["update", "--check"], &environment)?;

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
        let _daemon = FakeDaemon::start(
            &paths,
            vec![
                health_response(),
                managed_resource_update_check_response(
                    paths.resources().join("redis/8.8/releases/8.8.0-pv1"),
                ),
            ],
        )?;
        let environment = TestEnvironment::new(
            &home,
            ScriptedClient::new().with_error(ResourcesError::HttpRequestFailed {
                url: APP_MANIFEST_URL.to_string(),
                reason: "network error".to_string(),
            }),
        );

        let output = run_pv(&["update", "--check"], &environment)?;

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
    fn update_without_check_is_deferred_without_mutating() -> anyhow::Result<()> {
        let tempdir = tempdir()?;
        let home = tempdir.path().join("home");
        let environment = TestEnvironment::new(&home, ScriptedClient::new());

        let output = run_pv(&["update"], &environment)?;

        assert_eq!(output.exit_code, ExitCode::FAILURE);
        assert!(output.stdout.is_empty());
        assert_debug_snapshot!(output);

        Ok(())
    }

    #[derive(Debug)]
    struct FakeDaemon {
        handle: thread::JoinHandle<anyhow::Result<Vec<serde_json::Value>>>,
    }

    impl FakeDaemon {
        #[expect(
            clippy::disallowed_methods,
            reason = "update tests use a one-shot fake Unix socket daemon"
        )]
        fn start(paths: &PvPaths, responses: Vec<serde_json::Value>) -> anyhow::Result<Self> {
            let listener = UnixListener::bind(paths.daemon_socket())?;
            let handle = thread::spawn(move || {
                let mut requests = Vec::new();
                for response in responses {
                    let (mut stream, _address) = listener.accept()?;
                    let mut request = String::new();
                    BufReader::new(stream.try_clone()?).read_line(&mut request)?;
                    stream.write_all(format!("{response}\n").as_bytes())?;
                    requests.push(serde_json::from_str(request.trim_end())?);
                }

                Ok(requests)
            });

            Ok(Self { handle })
        }

        fn join(self) -> anyhow::Result<Vec<serde_json::Value>> {
            let result = self.handle.join().map_err(|error| {
                let payload = if let Some(s) = error.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = error.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    "unknown panic".to_string()
                };
                anyhow::anyhow!("fake daemon thread panicked: {payload}")
            })?;
            result
        }
    }

    #[derive(Debug)]
    struct PanickingClient;

    impl ResourceHttpClient for PanickingClient {
        fn get_text(&self, _url: &str) -> resources::Result<String> {
            panic!("get_text should not be called when daemon is missing")
        }

        fn download(&self, _url: &str, _writer: &mut dyn Write) -> resources::Result<()> {
            panic!("download should not be called when daemon is missing")
        }
    }

    #[derive(Debug)]
    struct ScriptedClient {
        text_responses: RefCell<VecDeque<Result<String, ResourcesError>>>,
    }

    impl ScriptedClient {
        fn new() -> Self {
            Self {
                text_responses: RefCell::new(VecDeque::new()),
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

        fn download(&self, url: &str, _writer: &mut dyn Write) -> resources::Result<()> {
            Err(ResourcesError::HttpRequestFailed {
                url: url.to_string(),
                reason: "downloads are not used by update checks".to_string(),
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
            "protocol_version": 1,
            "status": "ok",
            "message": "Managed Resource update check completed",
            "update_check": {
                "managed_resources": [
                    {
                        "status": "update_available",
                        "resource": "redis",
                        "track": "8.8",
                        "current_artifact_version": "8.8.0-pv1",
                        "current_artifact_path": current_artifact_path.as_ref().to_string_lossy(),
                        "latest_artifact_version": "8.8.1-pv1",
                        "current_revocation": null,
                        "latest_revocation": null,
                        "blocked_by": null,
                        "reason": null
                    }
                ]
            }
        })
    }

    fn health_response() -> serde_json::Value {
        json!({
            "type": "response",
            "protocol_version": 1,
            "status": "ok",
            "message": "daemon healthy"
        })
    }

    fn assert_update_snapshot(name: &'static str, snapshot: impl std::fmt::Debug) {
        let mut settings = Settings::clone_current();
        settings.add_filter(r#"/[^ \n"]+/home/\.pv"#, "<pv-home>");
        settings.add_filter(r"/[\w\-]+/home/\.pv", "<pv-home>");
        settings.add_filter(r"[a-f0-9]{64}", "<sha256>");
        settings.bind(|| {
            assert_debug_snapshot!(name, snapshot);
        });
    }

    const APP_MANIFEST: &str = r#"
{
  "schema_version": 1,
  "channel": "stable",
  "version": "0.2.0",
  "minimum_pv_version": "0.1.0",
  "published_at": "2026-06-11T12:00:00Z",
  "assets": [
    {
      "platform": "darwin-arm64",
      "url": "https://downloads.example.test/pv/0.2.0/pv-darwin-arm64",
      "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "size": 12345678
    },
    {
      "platform": "darwin-amd64",
      "url": "https://downloads.example.test/pv/0.2.0/pv-darwin-amd64",
      "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
      "size": 12345678
    }
  ]
}
"#;
}
