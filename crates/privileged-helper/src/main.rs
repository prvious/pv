#[cfg(target_os = "linux")]
use clap::{Parser, Subcommand};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use platform::serve_privileged_helper;
#[cfg(target_os = "linux")]
use platform::{PrivilegedHelperLifecycleAction, run_privileged_helper_lifecycle};

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
        action: PrivilegedHelperLifecycleAction,
        #[arg(long)]
        owner_uid: u32,
        #[arg(long)]
        sha256: String,
        #[arg(long)]
        helper_version: String,
        #[arg(long)]
        protocol_version: u32,
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
        Some(Command::HelperLifecycle {
            action,
            owner_uid,
            sha256,
            helper_version,
            protocol_version,
        }) => run_privileged_helper_lifecycle(
            action,
            owner_uid,
            &sha256,
            &helper_version,
            protocol_version,
        )?,
        None => serve_privileged_helper()?,
    }
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("the PV privileged helper is only supported on macOS and Linux")
}
