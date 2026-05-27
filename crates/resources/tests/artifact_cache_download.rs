use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::Error;
use std::io::Write;
use std::net::TcpListener;

use anyhow::Result;
use camino_tempfile::tempdir;
use insta::assert_debug_snapshot;
use resources::{
    ArtifactDownloader, ArtifactManifest, ArtifactManifestCache, ManifestArtifact,
    ResourceHttpClient, ResourceName, ResourcesError, TargetPlatform, TrackName,
    UreqResourceHttpClient,
};

#[test]
fn manifest_cache_fetches_latest_and_falls_back_to_cached_manifest() -> Result<()> {
    let tempdir = tempdir()?;
    let cache = ArtifactManifestCache::new(tempdir.path().join("downloads"));
    let client = ScriptedClient::new().with_text(VALID_MANIFEST);

    let manifest = cache.refresh(MANIFEST_URL, &client)?;
    assert_eq!(manifest.schema_version(), 1);

    let fallback_client = ScriptedClient::new().with_error(ResourcesError::HttpRequestFailed {
        url: MANIFEST_URL.to_string(),
        reason: "offline".to_string(),
    });
    let cached = cache.refresh(MANIFEST_URL, &fallback_client)?;

    assert_debug_snapshot!(cached);

    Ok(())
}

#[test]
fn manifest_cache_uses_cached_manifest_when_latest_payload_is_invalid() -> Result<()> {
    let tempdir = tempdir()?;
    let cache = ArtifactManifestCache::new(tempdir.path().join("downloads"));
    let client = ScriptedClient::new().with_text(VALID_MANIFEST);

    cache.refresh(MANIFEST_URL, &client)?;

    let invalid_client = ScriptedClient::new().with_text("{");
    let cached = cache.refresh(MANIFEST_URL, &invalid_client)?;

    assert_debug_snapshot!(cached);

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

    let cache_only_client = ScriptedClient::new().with_error(ResourcesError::HttpRequestFailed {
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
fn ureq_client_reports_destination_write_failures_separately() -> Result<()> {
    serve_once(ARTIFACT_BYTES, |url| {
        let client = UreqResourceHttpClient;
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
fn artifact_downloader_deletes_bad_downloads_and_reports_checksum_mismatch() -> Result<()> {
    let tempdir = tempdir()?;
    let downloader = ArtifactDownloader::new(tempdir.path().join("downloads"));
    let artifact = redis_artifact()?;
    let client = ScriptedClient::new().with_bytes(b"tampered");

    assert_debug_snapshot!(downloader.download(&artifact, &client));

    Ok(())
}

#[test]
fn artifact_downloader_retries_transient_download_failures() -> Result<()> {
    let tempdir = tempdir()?;
    let downloader = ArtifactDownloader::new(tempdir.path().join("downloads"));
    let artifact = redis_artifact()?;
    let client = ScriptedClient::new()
        .with_error(ResourcesError::HttpRequestFailed {
            url: artifact.url().to_string(),
            reason: "connection reset".to_string(),
        })
        .with_bytes(ARTIFACT_BYTES);

    let downloaded = downloader.download(&artifact, &client)?;

    assert!(!downloaded.is_from_cache());
    assert_debug_snapshot!(downloaded.path().file_name());

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

    fn with_error(self, error: ResourcesError) -> Self {
        self.text_responses
            .borrow_mut()
            .push_back(Err(error.clone()));
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
            .map_err(|source| ResourcesError::HttpRequestFailed {
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

fn serve_once(body: &[u8], operation: impl FnOnce(String) -> Result<()>) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let url = format!("http://{}/artifact.tar.gz", listener.local_addr()?);

    std::thread::scope(|scope| {
        let server = scope.spawn(move || -> Result<()> {
            let (mut stream, _address) = listener.accept()?;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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
