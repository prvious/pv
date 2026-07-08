use std::io::{Read, Write};
use std::thread;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use sha2::{Digest, Sha256};

use crate::fs;
use crate::http::ResourceHttpClient;
use crate::{ManifestArtifact, ResourcesError, Result};

const DOWNLOAD_ATTEMPTS: usize = 2;
const DOWNLOAD_RETRY_BACKOFF: Duration = Duration::from_millis(300);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactDownload {
    path: Utf8PathBuf,
    from_cache: bool,
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

pub trait DownloadProgress {
    fn report(&self, event: DownloadProgressEvent<'_>);
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
        let path = self.cache_path(artifact)?;

        if let Some(cached) = self.cached_download(artifact, &path)? {
            return Ok(cached);
        }

        self.download_with_retry(artifact, client, &path, progress)?;

        Ok(ArtifactDownload {
            path,
            from_cache: false,
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

        if hash_cached_file(path)? == artifact.sha256().as_str() {
            return Ok(Some(ArtifactDownload {
                path: path.to_path_buf(),
                from_cache: true,
            }));
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
    pub fn path(&self) -> &Utf8Path {
        &self.path
    }

    pub fn is_from_cache(&self) -> bool {
        self.from_cache
    }
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

fn hash_cached_file(path: &Utf8Path) -> Result<String> {
    fs::read_with(path, |reader| sha256_reader(path, reader))
}

fn sha256_reader(path: &Utf8Path, reader: &mut dyn Read) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];

    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|source| ResourcesError::Filesystem {
                path: path.to_string(),
                reason: source.to_string(),
            })?;
        if read == 0 {
            return Ok(sha256_digest_hex(hasher.finalize()));
        }

        hasher.update(&buffer[..read]);
    }
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
