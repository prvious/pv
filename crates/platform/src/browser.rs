use crate::{PlatformCapability, PlatformError, require_capability};

pub fn open_url(url: &str) -> Result<(), PlatformError> {
    require_capability(PlatformCapability::BrowserHandoff)?;
    webbrowser::open(url).map_err(PlatformError::BrowserOpen)
}
