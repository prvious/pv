#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    platform::serve_privileged_helper()?;

    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("the PV privileged helper is only supported on macOS")
}
