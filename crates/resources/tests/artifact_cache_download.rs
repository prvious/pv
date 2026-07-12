use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{Error, Read, Write};
use std::net::TcpListener;

use anyhow::{Result, bail};
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::assert_debug_snapshot;
use resources::{
    ArtifactDownloader, ArtifactManifest, ArtifactManifestCache, ArtifactManifestSource,
    DownloadProgress, DownloadProgressEvent, ManifestArtifact, ResourceHttpClient, ResourceName,
    ResourcesError, TargetPlatform, TrackName, UreqResourceHttpClient,
};

#[test]
fn manifest_cache_fetches_latest_and_falls_back_to_cached_manifest() -> Result<()> {
    let tempdir = tempdir()?;
    let cache = ArtifactManifestCache::new(tempdir.path().join("downloads"));
    let client = ScriptedClient::new().with_text(VALID_MANIFEST);

    let manifest = cache.refresh(MANIFEST_URL, &client)?;
    assert_eq!(manifest.manifest().schema_version(), 1);
    assert_eq!(manifest.source(), &ArtifactManifestSource::Latest);

    let fallback_client =
        ScriptedClient::new().with_text_error(ResourcesError::HttpRequestFailed {
            url: MANIFEST_URL.to_string(),
            reason: "offline".to_string(),
        });
    let cached = cache.refresh(MANIFEST_URL, &fallback_client)?;

    assert!(cached.is_from_cache());
    assert_debug_snapshot!(cached.source());
    assert_debug_snapshot!(cached.manifest());

    Ok(())
}

#[test]
fn manifest_cache_refresh_latest_does_not_fall_back_to_cached_manifest() -> Result<()> {
    let tempdir = tempdir()?;
    let cache = ArtifactManifestCache::new(tempdir.path().join("downloads"));
    let client = ScriptedClient::new().with_text(VALID_MANIFEST);

    cache.refresh(MANIFEST_URL, &client)?;

    let offline_client = ScriptedClient::new().with_text_error(ResourcesError::HttpRequestFailed {
        url: MANIFEST_URL.to_string(),
        reason: "offline".to_string(),
    });
    let result = cache.refresh_latest(MANIFEST_URL, &offline_client);

    assert_debug_snapshot!(result);

    Ok(())
}

#[test]
fn manifest_cache_rejects_invalid_manifest_url_even_when_cached_manifest_exists() -> Result<()> {
    let tempdir = tempdir()?;
    let cache = ArtifactManifestCache::new(tempdir.path().join("downloads"));
    let client = ScriptedClient::new().with_text(VALID_MANIFEST);

    cache.refresh(MANIFEST_URL, &client)?;

    let result = cache.refresh(
        "http://artifacts.example.test/manifest.json",
        &ScriptedClient::new(),
    );

    assert_debug_snapshot!(result);

    Ok(())
}

#[test]
fn manifest_cache_rejects_invalid_manifest_url_without_cached_manifest() -> Result<()> {
    let tempdir = tempdir()?;
    let cache = ArtifactManifestCache::new(tempdir.path().join("downloads"));
    let result = cache.refresh(
        "http://artifacts.example.test/manifest.json",
        &ScriptedClient::new(),
    );

    assert_debug_snapshot!(result);

    Ok(())
}

#[test]
fn manifest_cache_rejects_http_status_errors_even_when_cached_manifest_exists() -> Result<()> {
    let tempdir = tempdir()?;
    let cache = ArtifactManifestCache::new(tempdir.path().join("downloads"));
    let client = ScriptedClient::new().with_text(VALID_MANIFEST);

    cache.refresh(MANIFEST_URL, &client)?;

    let results = [403, 404, 500]
        .into_iter()
        .map(|status_code| {
            let status_client =
                ScriptedClient::new().with_text_error(ResourcesError::HttpStatusFailed {
                    url: MANIFEST_URL.to_string(),
                    status_code,
                });

            (status_code, cache.refresh(MANIFEST_URL, &status_client))
        })
        .collect::<Vec<_>>();

    assert_debug_snapshot!(results);

    Ok(())
}

#[test]
fn manifest_cache_rejects_invalid_latest_payload_even_when_cached_manifest_exists() -> Result<()> {
    let tempdir = tempdir()?;
    let cache = ArtifactManifestCache::new(tempdir.path().join("downloads"));
    let client = ScriptedClient::new().with_text(VALID_MANIFEST);

    cache.refresh(MANIFEST_URL, &client)?;

    let invalid_client = ScriptedClient::new().with_text("{");
    let result = cache.refresh(MANIFEST_URL, &invalid_client);

    assert_debug_snapshot!(result);

    Ok(())
}

#[test]
fn manifest_cache_rejects_incompatible_latest_manifest_even_when_cached_manifest_exists()
-> Result<()> {
    let tempdir = tempdir()?;
    let cache = ArtifactManifestCache::new(tempdir.path().join("downloads"));
    let client = ScriptedClient::new().with_text(VALID_MANIFEST);

    cache.refresh(MANIFEST_URL, &client)?;

    let newer_manifest = VALID_MANIFEST.replacen(
        "\"minimum_pv_version\": \"0.1.0\"",
        "\"minimum_pv_version\": \"999.0.0\"",
        1,
    );
    let newer_client = ScriptedClient::new().with_text(&newer_manifest);
    assert_debug_snapshot!(cache.refresh(MANIFEST_URL, &newer_client));

    let unsupported_manifest =
        VALID_MANIFEST.replacen("\"schema_version\": 1", "\"schema_version\": 2", 1);
    let unsupported_client = ScriptedClient::new().with_text(&unsupported_manifest);
    assert_debug_snapshot!(cache.refresh(MANIFEST_URL, &unsupported_client));

    Ok(())
}

#[test]
fn artifact_downloader_caches_verified_artifacts_by_manifest_checksum() -> Result<()> {
    let tempdir = tempdir()?;
    let downloader = ArtifactDownloader::new(tempdir.path().join("downloads"));
    let artifact = redis_artifact()?;
    let client = ScriptedClient::new().with_bytes(ARTIFACT_BYTES);

    let downloaded = downloader.download(&artifact, &client)?;
    assert!(!downloaded.is_from_cache());

    let cache_only_client =
        ScriptedClient::new().with_download_error(ResourcesError::HttpRequestFailed {
            url: artifact.url().to_string(),
            reason: "network should not be used".to_string(),
        });
    let cached = downloader.download(&artifact, &cache_only_client)?;

    assert!(cached.is_from_cache());
    assert_eq!(downloaded.path(), cached.path());
    assert_debug_snapshot!(cached.path().file_name());

    Ok(())
}

#[test]
fn artifact_downloader_replaces_corrupt_cached_artifact_with_verified_download() -> Result<()> {
    let tempdir = tempdir()?;
    let downloads_dir = tempdir.path().join("downloads");
    let downloader = ArtifactDownloader::new(downloads_dir.clone());
    let artifact = redis_artifact()?;
    let cached_path = downloads_dir
        .join("87698b18df0047a6404165a79250f5728ecc25b65fed27077ed9dff23e1232a9-redis-7.2.5-pv1-darwin-arm64.tar.gz");
    write_test_file(&cached_path, b"old corrupt cache")?;

    let client = ScriptedClient::new().with_bytes(ARTIFACT_BYTES);
    let downloaded = downloader.download(&artifact, &client)?;

    assert!(!downloaded.is_from_cache());
    assert_eq!(downloaded.path(), cached_path);
    assert_eq!(read_test_file(&cached_path)?, ARTIFACT_BYTES);

    Ok(())
}

#[test]
fn artifact_downloader_reports_download_progress_events() -> Result<()> {
    let tempdir = tempdir()?;
    let downloader = ArtifactDownloader::new(tempdir.path().join("downloads"));
    let artifact = redis_artifact()?;
    let client = ScriptedClient::new().with_bytes(ARTIFACT_BYTES);
    let progress = RecordingDownloadProgress::default();

    let downloaded = downloader.download_with_progress(&artifact, &client, &progress)?;

    assert!(!downloaded.is_from_cache());
    assert_debug_snapshot!(progress.events());

    Ok(())
}

#[test]
fn ureq_client_reports_destination_write_failures_separately() -> Result<()> {
    serve_once(ARTIFACT_BYTES, |url| {
        let client = UreqResourceHttpClient::default();
        let mut writer = FailingWriter;
        let result = client.download(&url, &mut writer);

        let Err(ResourcesError::DownloadWriteFailed {
            url: error_url,
            reason,
        }) = result
        else {
            assert_debug_snapshot!(result);
            return Ok(());
        };

        assert_eq!(error_url, url);
        assert_eq!(reason, "disk full");

        Ok(())
    })?;

    Ok(())
}

#[test]
fn scripted_client_reports_destination_write_failures_separately() -> Result<()> {
    let client = ScriptedClient::new().with_bytes(ARTIFACT_BYTES);
    let mut writer = FailingWriter;
    let url = "https://artifacts.example.test/redis.tar.gz";
    let result = client.download(url, &mut writer);

    let Err(ResourcesError::DownloadWriteFailed {
        url: error_url,
        reason,
    }) = result
    else {
        bail!("expected DownloadWriteFailed, got {result:?}");
    };

    assert_eq!(error_url, url);
    assert_eq!(reason, "disk full");

    Ok(())
}

#[test]
fn ureq_client_reports_http_status_responses_separately() -> Result<()> {
    serve_once_status(404, "Not Found", b"missing", |url| {
        let client = UreqResourceHttpClient::default();
        let result = client.get_text(&url);

        let Err(ResourcesError::HttpStatusFailed {
            url: error_url,
            status_code,
        }) = result
        else {
            assert_debug_snapshot!(result);
            return Ok(());
        };

        assert_eq!(error_url, url);
        assert_eq!(status_code, 404);

        Ok(())
    })?;

    Ok(())
}

#[test]
fn artifact_downloader_deletes_bad_downloads_and_reports_checksum_mismatch() -> Result<()> {
    let tempdir = tempdir()?;
    let downloads_dir = tempdir.path().join("downloads");
    let downloader = ArtifactDownloader::new(downloads_dir.clone());
    let artifact = redis_artifact()?;
    let client = ScriptedClient::new().with_bytes(b"tampered");
    let cached_path = downloads_dir
        .join("87698b18df0047a6404165a79250f5728ecc25b65fed27077ed9dff23e1232a9-redis-7.2.5-pv1-darwin-arm64.tar.gz");

    assert_debug_snapshot!(downloader.download(&artifact, &client));
    assert!(!cached_path.exists());

    Ok(())
}

#[test]
fn artifact_downloader_retries_transient_download_failures() -> Result<()> {
    let tempdir = tempdir()?;
    let downloader = ArtifactDownloader::new(tempdir.path().join("downloads"));
    let artifact = redis_artifact()?;
    let client = ScriptedClient::new()
        .with_download_error(ResourcesError::HttpRequestFailed {
            url: artifact.url().to_string(),
            reason: "connection reset".to_string(),
        })
        .with_bytes(ARTIFACT_BYTES);

    let downloaded = downloader.download(&artifact, &client)?;

    assert!(!downloaded.is_from_cache());
    assert_debug_snapshot!(downloaded.path().file_name());

    Ok(())
}

#[test]
fn artifact_downloader_retries_transient_http_status_failures() -> Result<()> {
    let tempdir = tempdir()?;
    let downloader = ArtifactDownloader::new(tempdir.path().join("downloads"));
    let artifact = redis_artifact()?;
    let client = ScriptedClient::new()
        .with_download_error(ResourcesError::HttpStatusFailed {
            url: artifact.url().to_string(),
            status_code: 500,
        })
        .with_bytes(ARTIFACT_BYTES);

    let downloaded = downloader.download(&artifact, &client)?;

    assert!(!downloaded.is_from_cache());
    assert_debug_snapshot!(downloaded.path().file_name());

    Ok(())
}

#[test]
fn artifact_downloader_does_not_retry_permanent_http_status_failures() -> Result<()> {
    let tempdir = tempdir()?;
    let downloader = ArtifactDownloader::new(tempdir.path().join("downloads"));
    let artifact = redis_artifact()?;
    let client = ScriptedClient::new()
        .with_download_error(ResourcesError::HttpStatusFailed {
            url: artifact.url().to_string(),
            status_code: 404,
        })
        .with_bytes(ARTIFACT_BYTES);

    assert_debug_snapshot!(downloader.download(&artifact, &client));

    Ok(())
}

fn redis_artifact() -> Result<ManifestArtifact> {
    let manifest = ArtifactManifest::parse(VALID_MANIFEST)?;
    let resource = ResourceName::new("redis")?;
    let track = TrackName::new("7.2")?;
    let selected =
        manifest.select_latest(&resource, &track, TargetPlatform::new("darwin-arm64")?)?;

    Ok(selected.artifact().clone())
}

#[derive(Debug)]
struct ScriptedClient {
    text_responses: RefCell<VecDeque<Result<String, ResourcesError>>>,
    byte_responses: RefCell<VecDeque<Result<Vec<u8>, ResourcesError>>>,
}

impl ScriptedClient {
    fn new() -> Self {
        Self {
            text_responses: RefCell::new(VecDeque::new()),
            byte_responses: RefCell::new(VecDeque::new()),
        }
    }

    fn with_text(self, text: &str) -> Self {
        self.text_responses
            .borrow_mut()
            .push_back(Ok(text.to_string()));
        self
    }

    fn with_bytes(self, bytes: &[u8]) -> Self {
        self.byte_responses
            .borrow_mut()
            .push_back(Ok(bytes.to_vec()));
        self
    }

    fn with_text_error(self, error: ResourcesError) -> Self {
        self.text_responses.borrow_mut().push_back(Err(error));
        self
    }

    fn with_download_error(self, error: ResourcesError) -> Self {
        self.byte_responses.borrow_mut().push_back(Err(error));
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
            .byte_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| {
                Err(ResourcesError::HttpRequestFailed {
                    url: url.to_string(),
                    reason: "no scripted byte response".to_string(),
                })
            })?;
        writer
            .write_all(&bytes)
            .map_err(|source| ResourcesError::DownloadWriteFailed {
                url: url.to_string(),
                reason: source.to_string(),
            })
    }
}

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
        Err(Error::other("disk full"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct RecordingDownloadProgress {
    events: RefCell<Vec<String>>,
}

impl RecordingDownloadProgress {
    fn events(&self) -> Vec<String> {
        self.events.borrow().clone()
    }
}

impl DownloadProgress for RecordingDownloadProgress {
    fn report(&self, event: DownloadProgressEvent<'_>) {
        self.events.borrow_mut().push(match event {
            DownloadProgressEvent::Started { artifact } => {
                format!(
                    "started {} {} {} total={}",
                    artifact.resource_name(),
                    artifact.track(),
                    artifact.artifact_version(),
                    artifact.size()
                )
            }
            DownloadProgressEvent::Advanced {
                artifact,
                downloaded_bytes,
            } => {
                format!(
                    "advanced {} {} {} downloaded={}/{}",
                    artifact.resource_name(),
                    artifact.track(),
                    artifact.artifact_version(),
                    downloaded_bytes,
                    artifact.size()
                )
            }
            DownloadProgressEvent::Finished {
                artifact,
                downloaded_bytes,
            } => {
                format!(
                    "finished {} {} {} downloaded={}/{}",
                    artifact.resource_name(),
                    artifact.track(),
                    artifact.artifact_version(),
                    downloaded_bytes,
                    artifact.size()
                )
            }
        });
    }
}

fn serve_once(body: &[u8], operation: impl FnOnce(String) -> Result<()>) -> Result<()> {
    serve_once_status(200, "OK", body, operation)
}

fn serve_once_status(
    status_code: u16,
    reason_phrase: &str,
    body: &[u8],
    operation: impl FnOnce(String) -> Result<()>,
) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let url = format!("http://{}/artifact.tar.gz", listener.local_addr()?);

    std::thread::scope(|scope| {
        let server = scope.spawn(move || -> Result<()> {
            let (mut stream, _address) = listener.accept()?;
            read_http_request(&mut stream)?;

            let response = format!(
                "HTTP/1.1 {status_code} {reason_phrase}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _header_result = stream.write_all(response.as_bytes());
            let _body_result = stream.write_all(body);

            Ok(())
        });

        let operation_result = operation(url);
        let server_result = server
            .join()
            .map_err(|_panic| anyhow::anyhow!("test HTTP server thread panicked"))?;
        operation_result?;
        server_result?;

        Ok(())
    })
}

fn read_http_request(stream: &mut std::net::TcpStream) -> Result<()> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];

    while !request.windows(4).any(|window| window == b"\r\n\r\n") && request.len() < 8192 {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
    }

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "resource integration tests seed cache fixtures directly"
)]
fn write_test_file(path: &Utf8Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "resource integration tests inspect cache fixture bytes directly"
)]
fn read_test_file(path: &Utf8Path) -> Result<Vec<u8>> {
    Ok(std::fs::read(path)?)
}

const MANIFEST_URL: &str = "https://artifacts.example.test/manifest.json";
const ARTIFACT_BYTES: &[u8] = b"redis artifact fixture";

const VALID_MANIFEST: &str = r#"
{
  "schema_version": 1,
  "minimum_pv_version": "0.1.0",
  "resources": [
    {
      "name": "redis",
      "default_track": "7.2",
      "tracks": [
        {
          "name": "7.2",
          "artifacts": [
            {
              "artifact_version": "7.2.5-pv1",
              "upstream_version": "7.2.5",
              "pv_build_revision": "1",
              "platform": "darwin-arm64",
              "url": "https://artifacts.example.test/redis-7.2.5-pv1-darwin-arm64.tar.gz",
              "sha256": "87698b18df0047a6404165a79250f5728ecc25b65fed27077ed9dff23e1232a9",
              "size": 22,
              "published_at": "2026-05-26T14:30:00Z"
            }
          ]
        }
      ]
    }
  ]
}
"#;
