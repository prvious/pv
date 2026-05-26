use crate::error::{ResourcesError, Result};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArtifactPlatform {
    Any,
    DarwinArm64,
    DarwinAmd64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TargetPlatform {
    DarwinArm64,
    DarwinAmd64,
}

impl ArtifactPlatform {
    pub fn new(value: &str) -> Result<Self> {
        match value {
            "any" => Ok(Self::Any),
            "darwin-arm64" => Ok(Self::DarwinArm64),
            "darwin-amd64" => Ok(Self::DarwinAmd64),
            _ => Err(ResourcesError::UnsupportedPlatform {
                platform: value.to_string(),
            }),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::DarwinArm64 => "darwin-arm64",
            Self::DarwinAmd64 => "darwin-amd64",
        }
    }

    pub fn matches(self, target: TargetPlatform) -> bool {
        match self {
            Self::Any => true,
            Self::DarwinArm64 => target == TargetPlatform::DarwinArm64,
            Self::DarwinAmd64 => target == TargetPlatform::DarwinAmd64,
        }
    }

    pub fn is_exact(self) -> bool {
        self != Self::Any
    }
}

impl TargetPlatform {
    pub fn new(value: &str) -> Result<Self> {
        match value {
            "darwin-arm64" => Ok(Self::DarwinArm64),
            "darwin-amd64" => Ok(Self::DarwinAmd64),
            _ => Err(ResourcesError::UnsupportedPlatform {
                platform: value.to_string(),
            }),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::DarwinArm64 => "darwin-arm64",
            Self::DarwinAmd64 => "darwin-amd64",
        }
    }
}

impl fmt::Display for ArtifactPlatform {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl fmt::Display for TargetPlatform {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
