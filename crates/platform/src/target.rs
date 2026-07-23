use std::fmt;

use crate::PlatformError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformTarget {
    Macos,
    Linux,
    Windows,
}

impl PlatformTarget {
    pub fn current() -> Result<Self, PlatformError> {
        current_target()
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Macos => "macos",
            Self::Linux => "linux",
            Self::Windows => "windows",
        }
    }
}

impl fmt::Display for PlatformTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(target_os = "macos")]
fn current_target() -> Result<PlatformTarget, PlatformError> {
    Ok(PlatformTarget::Macos)
}

#[cfg(target_os = "linux")]
fn current_target() -> Result<PlatformTarget, PlatformError> {
    Ok(PlatformTarget::Linux)
}

#[cfg(target_os = "windows")]
fn current_target() -> Result<PlatformTarget, PlatformError> {
    Ok(PlatformTarget::Windows)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn current_target() -> Result<PlatformTarget, PlatformError> {
    Err(PlatformError::UnsupportedTarget {
        target: std::env::consts::OS,
    })
}
