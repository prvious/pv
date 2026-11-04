use std::io::{Write, copy};

use camino::Utf8PathBuf;
use resources::{ResourceHttpClient, ResourcesError, UreqResourceHttpClient};

pub struct RealArtifactClient {
    directory: Option<Utf8PathBuf>,
    remote: UreqResourceHttpClient,
}

impl RealArtifactClient {
    #[expect(
        clippy::disallowed_methods,
        reason = "ignored real-artifact tests select a local artifact directory"
    )]
    pub fn from_env() -> Self {
        Self {
            directory: std::env::var("PV_E2E_ARTIFACT_DIR")
                .ok()
                .map(Utf8PathBuf::from),
            remote: UreqResourceHttpClient::new(),
        }
    }
}

impl ResourceHttpClient for RealArtifactClient {
    fn get_text(&self, url: &str) -> resources::Result<String> {
        let Some(directory) = &self.directory else {
            return self.remote.get_text(url);
        };
        let path = directory.join(url.rsplit('/').next().unwrap_or_default());
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
        let path = directory.join(url.rsplit('/').next().unwrap_or_default());
        std::fs::File::open(&path)
            .and_then(|mut file| copy(&mut file, writer))
            .map(|_| ())
            .map_err(|error| ResourcesError::Filesystem {
                path: path.to_string(),
                reason: error.to_string(),
            })
    }
}
