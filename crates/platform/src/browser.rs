use crate::error::PlatformError;

#[expect(
    clippy::disallowed_types,
    reason = "PV platform browser helper owns the macOS browser handoff for `pv open`"
)]
type StdCommand = std::process::Command;

pub fn open_url(url: &str) -> Result<(), PlatformError> {
    open_url_with_launcher(url, |url| {
        let status = StdCommand::new("open")
            .arg(url)
            .status()
            .map_err(PlatformError::BrowserOpen)?;

        if status.success() {
            return Ok(true);
        }

        Err(PlatformError::BrowserOpenStatus {
            status: status.to_string(),
        })
    })
}

pub(crate) fn open_url_with_launcher(
    url: &str,
    launch: impl FnOnce(&str) -> Result<bool, PlatformError>,
) -> Result<(), PlatformError> {
    if launch(url)? {
        return Ok(());
    }

    Err(PlatformError::BrowserOpenStatus {
        status: "exit status: unsuccessful".to_string(),
    })
}
