use camino::{Utf8Path, Utf8PathBuf};
use self_update::{AppUpdateVersion, Sha256Digest};
use serde::{Deserialize, Serialize};

use crate::error::{CliError, ExecuteError};

const HELPER_METADATA_FILE_NAME: &str = "pv-helper.json";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HelperReleaseMetadata {
    version: String,
    protocol_version: u32,
    sha256: String,
}

impl HelperReleaseMetadata {
    pub(crate) fn new(
        version: impl Into<String>,
        protocol_version: u32,
        sha256: impl Into<String>,
    ) -> Result<Self, CliError> {
        let metadata = Self {
            version: version.into(),
            protocol_version,
            sha256: sha256.into(),
        };
        metadata.validate(Utf8Path::new(HELPER_METADATA_FILE_NAME))?;

        Ok(metadata)
    }

    pub(crate) fn read(helper_path: &Utf8Path) -> Result<Self, ExecuteError> {
        let metadata_path = metadata_path(helper_path);
        let content = state::fs::read_to_string(&metadata_path)?;
        let metadata = serde_json::from_str::<Self>(&content).map_err(|error| {
            CliError::InvalidPrivilegedHelperReleaseMetadata {
                path: metadata_path.to_string(),
                reason: error.to_string(),
            }
        })?;
        metadata.validate(&metadata_path)?;

        Ok(metadata)
    }

    pub(crate) fn write(&self, helper_path: &Utf8Path) -> Result<(), ExecuteError> {
        let metadata_path = metadata_path(helper_path);
        self.validate(&metadata_path)?;
        let mut content = serde_json::to_string_pretty(self)?;
        content.push('\n');
        state::fs::write_sensitive_file(&metadata_path, &content)?;

        Ok(())
    }

    pub(crate) fn version(&self) -> &str {
        &self.version
    }

    pub(crate) const fn protocol_version(&self) -> u32 {
        self.protocol_version
    }

    pub(crate) fn sha256(&self) -> &str {
        &self.sha256
    }

    fn validate(&self, path: &Utf8Path) -> Result<(), CliError> {
        AppUpdateVersion::parse(self.version.clone()).map_err(|error| {
            CliError::InvalidPrivilegedHelperReleaseMetadata {
                path: path.to_string(),
                reason: error.to_string(),
            }
        })?;
        if self.protocol_version == 0 {
            return Err(CliError::InvalidPrivilegedHelperReleaseMetadata {
                path: path.to_string(),
                reason: "protocol_version must be greater than zero".to_string(),
            });
        }
        Sha256Digest::parse(self.sha256.clone()).map_err(|error| {
            CliError::InvalidPrivilegedHelperReleaseMetadata {
                path: path.to_string(),
                reason: error.to_string(),
            }
        })?;

        Ok(())
    }
}

pub(crate) fn metadata_path(helper_path: &Utf8Path) -> Utf8PathBuf {
    helper_path.with_file_name(HELPER_METADATA_FILE_NAME)
}
