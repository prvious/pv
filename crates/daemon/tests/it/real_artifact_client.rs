use std::env::VarError;
use std::io::{Error, Write, copy};

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use resources::{ResourceHttpClient, ResourcesError, UreqResourceHttpClient};

pub struct RealArtifactClient {
    directory: Option<Utf8PathBuf>,
    remote: UreqResourceHttpClient,
}

impl RealArtifactClient {
    #[expect(
        clippy::disallowed_methods,
        reason = "ignored real-artifact tests explicitly select their local artifact directory"
    )]
    pub fn from_env() -> Result<Self> {
        let directory = match std::env::var("PV_E2E_ARTIFACT_DIR") {
            Ok(directory) => Some(Utf8PathBuf::from(directory)),
            Err(VarError::NotPresent) => None,
            Err(error) => return Err(error).context("PV_E2E_ARTIFACT_DIR must be UTF-8"),
        };
        Self::new(directory)
    }

    pub fn new(directory: Option<Utf8PathBuf>) -> Result<Self> {
        if let Some(directory) = &directory
            && !directory.is_dir()
        {
            bail!("local artifact directory does not exist: {directory}");
        }
        Ok(Self {
            directory,
            remote: UreqResourceHttpClient::new(),
        })
    }
}

impl ResourceHttpClient for RealArtifactClient {
    fn get_text(&self, url: &str) -> resources::Result<String> {
        let Some(directory) = &self.directory else {
            return self.remote.get_text(url);
        };
        let path = local_path(directory, url)?;
        state::fs::read_to_string(&path).map_err(|error| ResourcesError::Filesystem {
            path: path.to_string(),
            reason: error.to_string(),
        })
    }

    #[expect(
        clippy::disallowed_types,
        reason = "the test transport streams local archive inputs into the real downloader"
    )]
    fn download(&self, url: &str, writer: &mut dyn Write) -> resources::Result<()> {
        let Some(directory) = &self.directory else {
            return self.remote.download(url, writer);
        };
        let path = local_path(directory, url)?;
        let filesystem_error = |error: Error| ResourcesError::Filesystem {
            path: path.to_string(),
            reason: error.to_string(),
        };
        let mut file = std::fs::File::open(&path).map_err(filesystem_error)?;
        copy(&mut file, writer)
            .map(|_| ())
            .map_err(|error| ResourcesError::DownloadWriteFailed {
                url: url.to_string(),
                reason: error.to_string(),
            })
    }
}

fn local_path(directory: &Utf8Path, url: &str) -> resources::Result<Utf8PathBuf> {
    let name = url
        .split(['?', '#'])
        .next()
        .and_then(|url| url.rsplit('/').next())
        .filter(|name| !name.is_empty() && *name != "." && *name != ".." && !name.contains('\\'))
        .ok_or_else(|| ResourcesError::InvalidArtifactUrl {
            url: url.to_string(),
        })?;
    Ok(directory.join(name))
}
