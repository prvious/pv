use crate::error::{ResourcesError, Result};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ArtifactPlatform {
    Any,
    DarwinArm64,
    DarwinAmd64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
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
    pub fn current() -> Result<Self> {
        target_platform_for(std::env::consts::OS, std::env::consts::ARCH)
    }

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

fn target_platform_for(operating_system: &str, architecture: &str) -> Result<TargetPlatform> {
    match (operating_system, architecture) {
        ("macos", "aarch64") => Ok(TargetPlatform::DarwinArm64),
        ("macos", "x86_64") => Ok(TargetPlatform::DarwinAmd64),
        _ => Err(ResourcesError::UnsupportedPlatform {
            platform: format!("{operating_system}-{architecture}"),
        }),
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

#[cfg(test)]
mod tests {
    use super::{TargetPlatform, target_platform_for};
    use crate::ResourcesError;

    #[test]
    fn target_platform_for_selects_only_supported_darwin_targets() {
        let cases = [
            ("macos", "aarch64", Ok(TargetPlatform::DarwinArm64)),
            ("macos", "x86_64", Ok(TargetPlatform::DarwinAmd64)),
            (
                "linux",
                "x86_64",
                Err(ResourcesError::UnsupportedPlatform {
                    platform: "linux-x86_64".to_string(),
                }),
            ),
            (
                "windows",
                "x86_64",
                Err(ResourcesError::UnsupportedPlatform {
                    platform: "windows-x86_64".to_string(),
                }),
            ),
        ];

        for (operating_system, architecture, expected) in cases {
            assert_eq!(
                target_platform_for(operating_system, architecture),
                expected
            );
        }
    }
}
