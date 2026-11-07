#[cfg(target_os = "linux")]
use clap::{Parser, Subcommand};
#[cfg(target_os = "linux")]
use platform::run_privileged_helper_lifecycle;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use platform::serve_privileged_helper;
#[cfg(target_os = "linux")]
use std::io;

#[cfg(target_os = "linux")]
#[derive(Parser)]
struct Arguments {
    #[command(subcommand)]
    command: Option<Command>,
}

#[cfg(target_os = "linux")]
#[derive(Subcommand)]
enum Command {
    #[command(hide = true)]
    HelperLifecycle {
        #[arg(long)]
        owner_uid: u32,
    },
}

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    serve_privileged_helper()?;

    Ok(())
}

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    match Arguments::parse().command {
        Some(Command::HelperLifecycle { owner_uid }) => {
            let outcome = run_privileged_helper_lifecycle(owner_uid)?;
            serde_json::to_writer(io::stdout().lock(), &outcome)?;
        }
        None => serve_privileged_helper()?,
    }
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("the PV privileged helper is only supported on macOS and Linux")
}
