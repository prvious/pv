#[cfg(target_os = "linux")]
use clap::{Parser, Subcommand};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use platform::serve_privileged_helper;
#[cfg(target_os = "linux")]
use platform::{PrivilegedHelperLifecycleAction, run_privileged_helper_lifecycle};
#[cfg(target_os = "linux")]
use platform::{start_resolver_link, stop_resolver_link};
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
    DnsLink {
        #[command(subcommand)]
        action: DnsLinkCommand,
    },
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

#[cfg(target_os = "linux")]
#[derive(Subcommand)]
enum DnsLinkCommand {
    Up {
        #[arg(long, value_parser = clap::value_parser!(u16).range(1024..))]
        port: u16,
    },
    Down,
}

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    serve_privileged_helper()?;

    Ok(())
}

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    match Arguments::parse().command {
        Some(Command::DnsLink { action }) => match action {
            DnsLinkCommand::Up { port } => start_resolver_link(port)?,
            DnsLinkCommand::Down => stop_resolver_link()?,
        },
        Some(Command::HelperLifecycle {
            action,
            owner_uid,
            sha256,
            helper_version,
            protocol_version,
        }) => {
            let warning = run_privileged_helper_lifecycle(
                action,
                owner_uid,
                &sha256,
                &helper_version,
                protocol_version,
            )?;
            serde_json::to_writer(io::stdout().lock(), &warning)?;
        }
        None => serve_privileged_helper()?,
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::{Arguments, Command, DnsLinkCommand, Parser};

    #[test]
    fn dns_link_accepts_only_fixed_actions_and_high_up_ports() -> Result<(), clap::Error> {
        for (port, expected_port) in [("1024", 1024), ("35353", 35353), ("65535", 65535)] {
            let arguments =
                Arguments::try_parse_from(["pv-helper", "dns-link", "up", "--port", port])?;
            assert!(
                matches!(arguments.command, Some(Command::DnsLink { action: DnsLinkCommand::Up { port } }) if port == expected_port)
            );
        }
        let arguments = Arguments::try_parse_from(["pv-helper", "dns-link", "down"])?;
        assert!(matches!(
            arguments.command,
            Some(Command::DnsLink {
                action: DnsLinkCommand::Down
            })
        ));
        for arguments in [
            vec!["dns-link"],
            vec!["dns-link", "up"],
            vec!["dns-link", "up", "--port", "53"],
            vec!["dns-link", "up", "--port", "1023"],
            vec!["dns-link", "up", "--port", "65536"],
            vec!["dns-link", "up", "--port", "35353", "--interface", "eth0"],
            vec!["dns-link", "up", "--port", "35353", "--path", "/tmp/unit"],
            vec!["dns-link", "down", "--port", "35353"],
            vec!["dns-link", "replace"],
        ] {
            assert!(Arguments::try_parse_from(["pv-helper"].into_iter().chain(arguments)).is_err());
        }
        Ok(())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("the PV privileged helper is only supported on macOS and Linux")
}
