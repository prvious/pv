use camino::Utf8Path;
use resources::{ResourceName, TrackName};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

#[derive(Clone, Debug, Default)]
pub struct ManifestDefaults {
    default_tracks: BTreeMap<ResourceName, TrackName>,
}

#[derive(Debug, Deserialize)]
struct RawManifestDefaults {
    #[serde(default)]
    resource: Vec<RawResourceDefault>,
}

#[derive(Debug, Deserialize)]
struct RawResourceDefault {
    name: String,
    default_track: String,
}

impl ManifestDefaults {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn load(path: &Utf8Path) -> crate::Result<Self> {
        let content = read_to_string(path)?;
        Self::from_toml(path, &content)
    }

    pub fn default_track_for(&self, resource: &ResourceName) -> Option<&TrackName> {
        self.default_tracks.get(resource)
    }

    pub(crate) fn entries(&self) -> impl Iterator<Item = (&ResourceName, &TrackName)> + '_ {
        self.default_tracks.iter()
    }

    pub fn from_toml(path: &Utf8Path, content: &str) -> crate::Result<Self> {
        let raw: RawManifestDefaults =
            toml::from_str(content).map_err(|error| invalid_default_tracks(path, error))?;
        let mut default_tracks = BTreeMap::new();

        for resource in raw.resource {
            let name = ResourceName::new(resource.name)
                .map_err(|error| invalid_default_tracks(path, error))?;
            let default_track = TrackName::new(resource.default_track)
                .map_err(|error| invalid_default_tracks(path, error))?;

            match default_tracks.entry(name) {
                Entry::Vacant(entry) => {
                    entry.insert(default_track);
                }
                Entry::Occupied(entry) => {
                    return Err(invalid_default_tracks(
                        path,
                        format!("duplicate default metadata for resource `{}`", entry.key()),
                    ));
                }
            }
        }

        Ok(Self { default_tracks })
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "PV release tooling reads repository-local manifest default metadata"
)]
fn read_to_string(path: &Utf8Path) -> crate::Result<String> {
    std::fs::read_to_string(path).map_err(|error| invalid_default_tracks(path, error))
}

fn invalid_default_tracks(path: &Utf8Path, reason: impl ToString) -> crate::ReleaseError {
    crate::ReleaseError::InvalidDefaultTracks {
        path: path.to_string(),
        reason: reason.to_string(),
    }
}
