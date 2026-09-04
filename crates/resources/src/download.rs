use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use camino::{Utf8Path, Utf8PathBuf};
use sha2::{Digest, Sha256};

use crate::fs;
use crate::http::ResourceHttpClient;
use crate::{
    ArtifactVersion, ManifestArtifact, ResourceName, ResourcesError, Result, Sha256Digest,
};

const DOWNLOAD_ATTEMPTS: usize = 2;
const DOWNLOAD_RETRY_BACKOFF: Duration = Duration::from_millis(300);
pub const MAX_PARALLEL_ARTIFACT_DOWNLOADS: usize = 4;
static VERIFIED_DOWNLOAD_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactDownload {
    path: Utf8PathBuf,
    from_cache: bool,
    verified_file: Arc<VerifiedArtifactFile>,
    resource_name: ResourceName,
    artifact_version: ArtifactVersion,
    sha256: Sha256Digest,
}

#[derive(Debug, Eq, PartialEq)]
struct VerifiedArtifactFile {
    path: Utf8PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactDownloader {
    downloads_dir: Utf8PathBuf,
}

#[derive(Clone, Copy, Debug)]
pub enum DownloadProgressEvent<'artifact> {
    Started {
        artifact: &'artifact ManifestArtifact,
    },
    Advanced {
        artifact: &'artifact ManifestArtifact,
        downloaded_bytes: u64,
    },
    Finished {
        artifact: &'artifact ManifestArtifact,
        downloaded_bytes: u64,
    },
}

#[derive(Clone, Copy, Debug)]
pub enum ResourceOperation<'artifact> {
    Manifest,
    Download(&'artifact ManifestArtifact),
    Install(&'artifact ManifestArtifact),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceOperationOutcome<'reason> {
    Succeeded,
    Failed,
    Fallback { reason: &'reason str },
}

#[derive(Clone, Copy, Debug)]
pub struct ResourceOperationEvent<'artifact, 'reason> {
    pub operation: ResourceOperation<'artifact>,
    pub elapsed: Duration,
    pub outcome: ResourceOperationOutcome<'reason>,
}

pub trait DownloadProgress {
    /// Receives synchronous download updates on the calling thread.
    ///
    /// Cache hits emit no [`DownloadProgressEvent`] through this callback. Retried downloads emit a
    /// new [`DownloadProgressEvent::Started`] event for each attempt.
    fn report(&self, event: DownloadProgressEvent<'_>);

    /// Receives one synchronous completion event for each overall manifest, download, or install
    /// operation, including cache hits and after all download retries.
    fn operation_finished(&self, _event: ResourceOperationEvent<'_, '_>) {}
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NoDownloadProgress;

impl DownloadProgress for NoDownloadProgress {
    fn report(&self, _event: DownloadProgressEvent<'_>) {}
}

impl ArtifactDownloader {
    pub fn new(downloads_dir: impl Into<Utf8PathBuf>) -> Self {
        Self {
            downloads_dir: downloads_dir.into(),
        }
    }

    pub fn download(
        &self,
        artifact: &ManifestArtifact,
        client: &(impl ResourceHttpClient + ?Sized),
    ) -> Result<ArtifactDownload> {
        self.download_with_progress(artifact, client, &NoDownloadProgress)
    }

    pub fn download_with_progress(
        &self,
        artifact: &ManifestArtifact,
        client: &(impl ResourceHttpClient + ?Sized),
        progress: &(impl DownloadProgress + ?Sized),
    ) -> Result<ArtifactDownload> {
        let started_at = Instant::now();
        let result = (|| {
            let path = self.cache_path(artifact)?;

            if let Some(cached) = self.cached_download(artifact, &path)? {
                return Ok(cached);
            }

            let verified_file = VerifiedArtifactFile::new_for(&path)?;
            self.download_with_retry(artifact, client, &verified_file.path, progress)?;
            copy_file_atomically(&verified_file.path, &path)?;

            Ok(ArtifactDownload::new(path, false, verified_file, artifact))
        })();
        progress.operation_finished(ResourceOperationEvent {
            operation: ResourceOperation::Download(artifact),
            elapsed: started_at.elapsed(),
            outcome: ResourceOperationOutcome::from_succeeded(result.is_ok()),
        });

        result
    }

    pub fn download_many_with_progress(
        &self,
        artifacts: &[ManifestArtifact],
        client: &(impl ResourceHttpClient + Sync + ?Sized),
        progress: &(impl DownloadProgress + Sync + ?Sized),
    ) -> Vec<Result<ArtifactDownload>> {
        let next_artifact = AtomicUsize::new(0);
        let (result_sender, result_receiver) = mpsc::channel();
        let worker_count = artifacts.len().min(MAX_PARALLEL_ARTIFACT_DOWNLOADS);

        thread::scope(|scope| {
            for _worker in 0..worker_count {
                let result_sender = result_sender.clone();
                let next_artifact = &next_artifact;
                scope.spawn(move || {
                    loop {
                        let index = next_artifact.fetch_add(1, Ordering::Relaxed);
                        let Some(artifact) = artifacts.get(index) else {
                            break;
                        };
                        let result = self.download_with_progress(artifact, client, progress);
                        if result_sender.send((index, result)).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(result_sender);

            let mut results = result_receiver.into_iter().collect::<Vec<_>>();
            results.sort_by_key(|(index, _result)| *index);
            results.into_iter().map(|(_index, result)| result).collect()
        })
    }

    fn cached_download(
        &self,
        artifact: &ManifestArtifact,
        path: &Utf8Path,
    ) -> Result<Option<ArtifactDownload>> {
        if !fs::path_exists(path) {
            return Ok(None);
        }

        let (verified_file, actual) = VerifiedArtifactFile::copy_and_hash(path)?;
        if actual == artifact.sha256().as_str() {
            return Ok(Some(ArtifactDownload::new(
                path.to_path_buf(),
                true,
                verified_file,
                artifact,
            )));
        }

        fs::remove_file_if_exists(path)?;
        Ok(None)
    }

    fn download_with_retry(
        &self,
        artifact: &ManifestArtifact,
        client: &(impl ResourceHttpClient + ?Sized),
        path: &Utf8Path,
        progress: &(impl DownloadProgress + ?Sized),
    ) -> Result<()> {
        for _ in 1..DOWNLOAD_ATTEMPTS {
            match write_download(artifact, client, path, progress) {
                Err(error) if is_retriable_download_error(&error) => {
                    thread::sleep(DOWNLOAD_RETRY_BACKOFF)
                }
                result => return result,
            }
        }

        write_download(artifact, client, path, progress)
    }

    fn cache_path(&self, artifact: &ManifestArtifact) -> Result<Utf8PathBuf> {
        let file_name = artifact_file_name(artifact.url())?;
        let cached_file_name = format!("{}-{file_name}", artifact.sha256().as_str());

        Ok(self.downloads_dir.join(cached_file_name))
    }
}

impl ResourceOperationOutcome<'_> {
    pub fn from_succeeded(succeeded: bool) -> Self {
        if succeeded {
            Self::Succeeded
        } else {
            Self::Failed
        }
    }
}

fn is_retriable_download_error(error: &ResourcesError) -> bool {
    match error {
        ResourcesError::HttpRequestFailed { .. } => true,
        ResourcesError::HttpStatusFailed { status_code, .. } => {
            *status_code == 429 || (500..=599).contains(status_code)
        }
        _error => false,
    }
}

impl ArtifactDownload {
    fn new(
        path: Utf8PathBuf,
        from_cache: bool,
        verified_file: Arc<VerifiedArtifactFile>,
        artifact: &ManifestArtifact,
    ) -> Self {
        Self {
            path,
            from_cache,
            verified_file,
            resource_name: artifact.resource_name().clone(),
            artifact_version: artifact.artifact_version().clone(),
            sha256: artifact.sha256().clone(),
        }
    }

    pub(crate) fn validate_for(&self, artifact: &ManifestArtifact) -> Result<()> {
        if self.resource_name == *artifact.resource_name()
            && self.artifact_version == *artifact.artifact_version()
            && self.sha256 == *artifact.sha256()
        {
            return Ok(());
        }

        Err(ResourcesError::InvalidArtifactLayout {
            resource: artifact.resource_name().as_str().to_string(),
            reason: format!(
                "prefetched download belongs to {} artifact {} checksum {}, not {} artifact {} checksum {}",
                self.resource_name,
                self.artifact_version,
                self.sha256.as_str(),
                artifact.resource_name(),
                artifact.artifact_version(),
                artifact.sha256().as_str()
            ),
        })
    }

    pub fn path(&self) -> &Utf8Path {
        &self.path
    }

    pub(crate) fn install_path(&self) -> &Utf8Path {
        &self.verified_file.path
    }

    pub fn is_from_cache(&self) -> bool {
        self.from_cache
    }
}

impl VerifiedArtifactFile {
    fn new_for(cache_path: &Utf8Path) -> Result<Arc<Self>> {
        let Some(parent) = cache_path.parent() else {
            return Err(ResourcesError::Filesystem {
                path: cache_path.to_string(),
                reason: "artifact download has no parent directory".to_string(),
            });
        };
        let counter = VERIFIED_DOWNLOAD_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(".verified-{}-{counter}.tar.gz", std::process::id()));

        Ok(Arc::new(Self { path }))
    }

    fn copy_and_hash(source_path: &Utf8Path) -> Result<(Arc<Self>, String)> {
        let verified_file = Self::new_for(source_path)?;
        let path = &verified_file.path;
        let mut hasher = Sha256::new();
        fs::write_atomically_with(path, |writer| {
            fs::read_with(source_path, |reader| {
                let mut buffer = [0_u8; 8192];
                loop {
                    let read =
                        reader
                            .read(&mut buffer)
                            .map_err(|source| ResourcesError::Filesystem {
                                path: source_path.to_string(),
                                reason: source.to_string(),
                            })?;
                    if read == 0 {
                        return Ok(());
                    }
                    writer.write_all(&buffer[..read]).map_err(|source| {
                        ResourcesError::Filesystem {
                            path: path.to_string(),
                            reason: source.to_string(),
                        }
                    })?;
                    hasher.update(&buffer[..read]);
                }
            })
        })?;
        let actual = sha256_digest_hex(hasher.finalize());

        Ok((verified_file, actual))
    }
}

impl Drop for VerifiedArtifactFile {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file_if_exists(&self.path) {
            let _fallback_result = writeln!(
                std::io::stderr().lock(),
                "PV could not remove verified artifact snapshot `{}`: {error}",
                self.path
            );
        }
    }
}

fn copy_file_atomically(source_path: &Utf8Path, destination_path: &Utf8Path) -> Result<()> {
    fs::write_atomically_with(destination_path, |writer| {
        fs::read_with(source_path, |reader| {
            std::io::copy(reader, writer)
                .map(|_copied| ())
                .map_err(|source| ResourcesError::Filesystem {
                    path: destination_path.to_string(),
                    reason: source.to_string(),
                })
        })
    })
}

fn artifact_file_name(url: &str) -> Result<&str> {
    let without_fragment = url.split('#').next().unwrap_or(url);
    let without_query = without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment);
    let file_name = without_query.rsplit('/').next().unwrap_or("");

    if file_name.is_empty()
        || file_name == "."
        || file_name == ".."
        || file_name.contains('/')
        || file_name.contains('\\')
    {
        return Err(ResourcesError::InvalidArtifactUrl {
            url: url.to_string(),
        });
    }

    Ok(file_name)
}

fn write_download(
    artifact: &ManifestArtifact,
    client: &(impl ResourceHttpClient + ?Sized),
    path: &Utf8Path,
    progress: &(impl DownloadProgress + ?Sized),
) -> Result<()> {
    progress.report(DownloadProgressEvent::Started { artifact });
    let mut downloaded_bytes = 0;
    fs::write_atomically_with(path, |writer| {
        let mut writer = ProgressWriter::new(writer, artifact, progress);
        let actual = {
            let mut hashing_writer = HashingWriter::new(&mut writer);
            client.download(artifact.url(), &mut hashing_writer)?;

            hashing_writer.finish()
        };
        downloaded_bytes = writer.downloaded_bytes();

        verify_checksum(artifact, &actual)
    })?;
    progress.report(DownloadProgressEvent::Finished {
        artifact,
        downloaded_bytes,
    });

    Ok(())
}

fn verify_checksum(artifact: &ManifestArtifact, actual: &str) -> Result<()> {
    if actual == artifact.sha256().as_str() {
        return Ok(());
    }

    Err(ResourcesError::ArtifactChecksumMismatch {
        url: artifact.url().to_string(),
        expected: artifact.sha256().as_str().to_string(),
        actual: actual.to_string(),
    })
}

fn sha256_digest_hex(digest: impl IntoIterator<Item = u8>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(64);

    for byte in digest {
        hex.push(HEX[(byte >> 4) as usize] as char);
        hex.push(HEX[(byte & 0x0f) as usize] as char);
    }

    hex
}

struct HashingWriter<'a> {
    inner: &'a mut dyn Write,
    hasher: Sha256,
}

impl<'a> HashingWriter<'a> {
    fn new(inner: &'a mut dyn Write) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
        }
    }

    fn finish(self) -> String {
        sha256_digest_hex(self.hasher.finalize())
    }
}

impl Write for HashingWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buffer)?;
        self.hasher.update(&buffer[..written]);

        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

struct ProgressWriter<'a, Progress>
where
    Progress: DownloadProgress + ?Sized,
{
    inner: &'a mut dyn Write,
    artifact: &'a ManifestArtifact,
    progress: &'a Progress,
    downloaded_bytes: u64,
}

impl<'a, Progress> ProgressWriter<'a, Progress>
where
    Progress: DownloadProgress + ?Sized,
{
    fn new(
        inner: &'a mut dyn Write,
        artifact: &'a ManifestArtifact,
        progress: &'a Progress,
    ) -> Self {
        Self {
            inner,
            artifact,
            progress,
            downloaded_bytes: 0,
        }
    }

    fn downloaded_bytes(&self) -> u64 {
        self.downloaded_bytes
    }
}

impl<Progress> Write for ProgressWriter<'_, Progress>
where
    Progress: DownloadProgress + ?Sized,
{
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buffer)?;
        let written_bytes =
            u64::try_from(written).map_err(|_| std::io::Error::other("download size overflow"))?;
        self.downloaded_bytes = self
            .downloaded_bytes
            .checked_add(written_bytes)
            .ok_or_else(|| std::io::Error::other("download size overflow"))?;
        self.progress.report(DownloadProgressEvent::Advanced {
            artifact: self.artifact,
            downloaded_bytes: self.downloaded_bytes,
        });

        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
