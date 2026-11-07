use crate::error::PlatformError;

#[expect(
    clippy::disallowed_types,
    reason = "PV platform browser helper owns the macOS browser handoff for `pv open`"
)]
type StdCommand = std::process::Command;

pub fn open_url(url: &str) -> Result<(), PlatformError> {
    let status = StdCommand::new("open")
        .arg(url)
        .status()
        .map_err(PlatformError::BrowserOpen)?;

    if status.success() {
        return Ok(());
    }

    Err(PlatformError::BrowserOpenStatus {
        status: status.to_string(),
    })
}
