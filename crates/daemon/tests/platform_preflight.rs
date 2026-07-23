#![cfg(not(target_os = "macos"))]

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use camino_tempfile::tempdir;
use daemon::{DaemonError, RunningDaemon, run_blocking};
use platform::{PlatformCapability, PlatformError};
use state::PvPaths;

#[tokio::test]
async fn public_daemon_start_rejects_unsupported_platform_without_creating_home()
-> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));

    let result = RunningDaemon::start(paths.clone()).await;

    assert!(matches!(
        result,
        Err(DaemonError::Platform(PlatformError::Unsupported {
            capability: PlatformCapability::DaemonIpc,
            ..
        }))
    ));
    assert!(!paths.home().exists());

    Ok(())
}

#[tokio::test]
async fn public_daemon_start_without_adapters_rejects_unsupported_platform_without_creating_home()
-> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));

    let result = RunningDaemon::start_without_managed_resource_adapters(paths.clone()).await;

    assert!(matches!(
        result,
        Err(DaemonError::Platform(PlatformError::Unsupported {
            capability: PlatformCapability::DaemonIpc,
            ..
        }))
    ));
    assert!(!paths.home().exists());

    Ok(())
}

#[tokio::test]
async fn public_daemon_start_with_manifest_client_rejects_unsupported_platform_without_side_effects()
-> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));
    let client = RecordingClient::default();

    let result = RunningDaemon::start_without_managed_resource_adapters_with_manifest_client(
        paths.clone(),
        "https://artifacts.example.test/manifest.json",
        client.clone(),
    )
    .await;

    assert!(matches!(
        result,
        Err(DaemonError::Platform(PlatformError::Unsupported {
            capability: PlatformCapability::DaemonIpc,
            ..
        }))
    ));
    assert!(!paths.home().exists());
    assert_eq!(client.request_count(), 0);

    Ok(())
}

#[test]
fn public_blocking_daemon_start_rejects_unsupported_platform_without_creating_home()
-> anyhow::Result<()> {
    let tempdir = tempdir()?;
    let paths = PvPaths::for_home(tempdir.path().join("home"));

    let result = run_blocking(paths.clone());

    assert!(matches!(
        result,
        Err(DaemonError::Platform(PlatformError::Unsupported {
            capability: PlatformCapability::DaemonIpc,
            ..
        }))
    ));
    assert!(!paths.home().exists());

    Ok(())
}

#[derive(Clone, Debug, Default)]
struct RecordingClient {
    request_count: Arc<AtomicUsize>,
}

impl RecordingClient {
    fn request_count(&self) -> usize {
        self.request_count.load(Ordering::Relaxed)
    }

    fn record_request(&self, url: &str) -> resources::Result<()> {
        self.request_count.fetch_add(1, Ordering::Relaxed);

        Err(resources::ResourcesError::HttpRequestFailed {
            url: url.to_string(),
            reason: "unexpected request during platform preflight".to_string(),
        })
    }
}

impl resources::ResourceHttpClient for RecordingClient {
    fn get_text(&self, url: &str) -> resources::Result<String> {
        self.record_request(url)?;

        Ok(String::new())
    }

    fn download(&self, url: &str, _writer: &mut dyn Write) -> resources::Result<()> {
        self.record_request(url)
    }
}
