use std::cell::RefCell;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;

use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::assert_snapshot;
use platform::{DaemonRegistrationConfig, PlatformError, PrivilegedHelperStatus};
use resources::{ResourceHttpClient, ResourcesError};
use self_update::AppUpdatePlatform;
use serde_json::json;
use sha2::{Digest, Sha256};
use state::{AppReleaseLayout, PvPaths, fs};

use super::{AppUpdateStatusValue, app_update_status, run_app_update_phase};
use crate::environment::Environment;
use crate::error::ExecuteError;
use crate::output::{Presentation, Streams};

const MANIFEST_URL: &str = "https://updates.example.test/pv-app-manifest.json";

struct TestEnvironment {
    paths: PvPaths,
    artifacts: Result<bool, io::ErrorKind>,
    operations: RefCell<Vec<&'static str>>,
    manifest: String,
}

impl Environment for TestEnvironment {
    fn var_os(&self, _key: &str) -> Option<OsString> {
        None
    }

    fn home_dir(&self) -> Option<PathBuf> {
        Some(self.paths.home().as_std_path().to_owned())
    }

    fn current_dir(&self) -> io::Result<PathBuf> {
        Ok(self.paths.home().as_std_path().to_owned())
    }

    fn current_exe(&self) -> io::Result<PathBuf> {
        Ok(self.paths.active_pv_binary().into_std_path_buf())
    }

    fn stdin_is_terminal(&self) -> bool {
        false
    }

    fn open_url(&self, _url: &str) -> io::Result<()> {
        Err(io::Error::other("unexpected browser handoff"))
    }

    fn inspect_low_ports(&self) -> Result<platform::LowPortInspection, PlatformError> {
        Err(PlatformError::SystemIntegration(
            "unexpected low-port inspection".to_owned(),
        ))
    }

    fn reload_daemon_registration(&self) -> Result<(), PlatformError> {
        self.operations.borrow_mut().push("reload registration");
        Ok(())
    }

    fn privileged_helper_status(&self) -> Result<PrivilegedHelperStatus, PlatformError> {
        Err(PlatformError::PrivilegedHelperUnavailable)
    }

    fn installed_helper_artifacts_present(&self) -> Result<bool, PlatformError> {
        self.operations
            .borrow_mut()
            .push("inspect helper artifacts");
        self.artifacts.map_err(|kind| {
            PlatformError::PrivilegedHelperIo(io::Error::new(
                kind,
                "fixture artifact inspection refused",
            ))
        })
    }

    fn install_privileged_helper(
        &self,
        _candidate_path: &Utf8Path,
        _prepared_directory: &Utf8Path,
        _expected_sha256: &str,
        _helper_version: &str,
        _protocol_version: u32,
    ) -> Result<platform::PrivilegedHelperInstallOutcome, PlatformError> {
        self.operations.borrow_mut().push("install helper");
        Err(PlatformError::PrivilegedHelperInstallation(
            "unexpected helper installation".to_owned(),
        ))
    }

    fn remove_privileged_helper(&self) -> Result<(), PlatformError> {
        self.operations.borrow_mut().push("remove helper");
        Err(PlatformError::PrivilegedHelperInstallation(
            "unexpected helper removal".to_owned(),
        ))
    }

    fn app_update_manifest_url(&self) -> Option<String> {
        Some(MANIFEST_URL.to_owned())
    }

    fn app_update_platform(&self) -> Option<AppUpdatePlatform> {
        Some(AppUpdatePlatform::LinuxArm64)
    }

    fn resource_http_client(&self) -> Option<&dyn ResourceHttpClient> {
        Some(self)
    }
}

impl ResourceHttpClient for TestEnvironment {
    fn get_text(&self, url: &str) -> resources::Result<String> {
        assert_eq!(url, MANIFEST_URL);
        self.operations.borrow_mut().push("fetch manifest");
        Ok(self.manifest.clone())
    }

    fn download(&self, url: &str, _writer: &mut dyn Write) -> resources::Result<()> {
        self.operations.borrow_mut().push("download archive");
        Err(ResourcesError::HttpRequestFailed {
            url: url.to_owned(),
            reason: "fixture archive refused".to_owned(),
        })
    }
}

#[test]
fn linux_update_does_not_treat_unavailable_helpers_with_artifacts_as_missing() -> anyhow::Result<()>
{
    let mut rows = Vec::new();
    for (name, artifacts) in [
        ("present", Ok(true)),
        ("unreadable", Err(io::ErrorKind::PermissionDenied)),
        ("absent", Ok(false)),
    ] {
        let directory = tempdir()?;
        let paths = PvPaths::for_home(directory.path());
        fs::ensure_layout(&paths)?;
        let source = paths.downloads().join("fixture-source");
        fs::write_sensitive_file(&source, "fixture release")?;
        let version = env!("CARGO_PKG_VERSION");
        let layout = AppReleaseLayout::new(paths.clone());
        layout.install_release_binary(version, &source)?;
        layout.install_release_helper(version, &source)?;
        layout.activate_release(version)?;
        fs::write_sensitive_file(
            &paths
                .app_release_helper(version)
                .with_file_name("pv-helper.json"),
            &json!({
                "version": platform::PRIVILEGED_HELPER_VERSION,
                "protocol_version": platform::HELPER_PROTOCOL_VERSION,
                "sha256": format!("{:x}", Sha256::digest(b"fixture release")),
            })
            .to_string(),
        )?;
        let environment = TestEnvironment {
            paths,
            artifacts,
            operations: RefCell::new(Vec::new()),
            manifest: json!({
                "schema_version": 2,
                "channel": "stable",
                "version": version,
                "minimum_pv_version": "0.1.0",
                "published_at": "2026-06-11T12:00:00Z",
                "assets": [{
                    "platform": "linux-arm64",
                    "url": "https://downloads.example.test/pv/pv-linux-arm64",
                    "sha256": "a".repeat(64),
                    "size": 1,
                    "helper": {
                        "version": "2.1.0",
                        "protocol_version": platform::HELPER_PROTOCOL_VERSION,
                        "url": "https://downloads.example.test/pv/pv-helper-linux-arm64",
                        "sha256": "b".repeat(64),
                        "size": 1,
                    },
                }],
            })
            .to_string(),
        };
        let registration = environment.daemon_registration_path();
        let registration = Utf8Path::from_path(&registration)
            .ok_or_else(|| anyhow::anyhow!("fixture registration is not UTF-8"))?;
        platform::write_daemon_registration_file(
            registration,
            &DaemonRegistrationConfig::for_paths(&environment.paths),
        )?;
        let presentation = Presentation::detect(&["--no-color".into()], &environment);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let result = run_app_update_phase(
            &environment,
            &mut Streams::new(&mut stdout, &mut stderr, presentation),
        );
        let Err(error) = result else {
            anyhow::bail!("fixture update must refuse or reach the rejected download");
        };
        let operations = environment.operations.take();
        match artifacts {
            Ok(true) => assert!(
                matches!(
                    &error,
                    ExecuteError::Platform(PlatformError::PrivilegedHelperUnavailable)
                ),
                "{error}; operations: {operations:?}"
            ),
            Err(kind) => assert!(matches!(
                &error,
                ExecuteError::Platform(PlatformError::PrivilegedHelperIo(source))
                    if source.kind() == kind
                        && source.to_string() == "fixture artifact inspection refused"
            )),
            Ok(false) => {
                assert!(matches!(
                    &error,
                    ExecuteError::Resources(ResourcesError::HttpRequestFailed { reason, .. })
                        if reason == "fixture archive refused"
                ));
                assert!(operations.contains(&"download archive"));
            }
        }
        let status = app_update_status(&environment)?;
        let helper = status
            .helper
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("fixture manifest must include helper status"))?;
        if artifacts == Ok(false) {
            assert!(matches!(
                helper.status,
                AppUpdateStatusValue::UpdateAvailable
            ));
            assert!(helper.reason.is_none());
        } else {
            assert!(matches!(helper.status, AppUpdateStatusValue::Unavailable));
            assert!(helper.reason.is_some());
        }
        assert_eq!(layout.active_release()?.as_deref(), Some(version));
        assert_eq!(
            fs::read_to_string(&environment.paths.app_release_helper(version))?,
            "fixture release"
        );
        assert!(stderr.is_empty());
        rows.push(json!({
            "artifacts": name,
            "update_error": error.to_string(),
            "update_operations": operations,
            "check_helper": status.helper,
            "check_operations": environment.operations.take(),
        }));
    }
    assert_snapshot!(serde_json::to_string_pretty(&rows)?);
    Ok(())
}
