#[cfg(target_os = "macos")]
use platform::serve_privileged_helper;

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    serve_privileged_helper()?;

    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("the PV privileged helper is only supported on macOS")
}
