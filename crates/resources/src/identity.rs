use crate::error::{ResourcesError, Result};
use std::fmt;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResourceName(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TrackName(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ArtifactVersion(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Sha256Digest(String);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PublishedAt(OffsetDateTime);

impl ResourceName {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        validate_identity("resource name", value.into()).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TrackName {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        validate_identity("track", value.into()).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl ArtifactVersion {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        validate_identity("artifact version", value.into()).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Sha256Digest {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        let is_valid = value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit());

        if is_valid {
            Ok(Self(value.to_ascii_lowercase()))
        } else {
            Err(ResourcesError::InvalidIdentity {
                kind: "sha256",
                value,
            })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl PublishedAt {
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        let parsed = OffsetDateTime::parse(&value, &Rfc3339)
            .map_err(|_| ResourcesError::InvalidPublishedAt { value })?;

        Ok(Self(parsed))
    }

    pub fn as_rfc3339(&self) -> String {
        match self.0.format(&Rfc3339) {
            Ok(value) => value,
            Err(_error) => self.0.to_string(),
        }
    }
}

impl fmt::Display for ResourceName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl fmt::Display for TrackName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl fmt::Display for ArtifactVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

fn validate_identity(kind: &'static str, value: String) -> Result<String> {
    let is_valid = !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'));

    if is_valid {
        Ok(value)
    } else {
        Err(ResourcesError::InvalidIdentity { kind, value })
    }
}
