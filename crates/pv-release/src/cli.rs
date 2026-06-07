use anyhow::Context;
use camino::Utf8PathBuf;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "pv-release")]
#[command(about = "PV internal artifact release tooling")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    GenerateManifest {
        #[arg(long)]
        records: Utf8PathBuf,
        #[arg(long)]
        revocations: Utf8PathBuf,
        #[arg(long)]
        defaults: Option<Utf8PathBuf>,
        #[arg(long)]
        output: Utf8PathBuf,
        #[arg(long)]
        base_url: String,
    },
    ValidateArchive {
        #[arg(long)]
        archive: Utf8PathBuf,
        #[arg(long)]
        record: Utf8PathBuf,
        #[arg(long)]
        smoke_hook: Option<Utf8PathBuf>,
    },
}

pub fn run() -> anyhow::Result<()> {
    let args = Args::parse();
    match args.command {
        Command::GenerateManifest {
            records,
            revocations,
            defaults,
            output,
            base_url,
        } => crate::manifest::generate_manifest_file_with_defaults(
            &records,
            &revocations,
            defaults.as_deref(),
            &output,
            &base_url,
        )
        .with_context(|| format!("failed to generate manifest at `{output}`")),
        Command::ValidateArchive {
            archive,
            record,
            smoke_hook,
        } => crate::archive::validate_archive_for_record_file_with_smoke_hook(
            &archive,
            &record,
            smoke_hook.as_deref(),
        )
        .with_context(|| format!("failed to validate archive `{archive}`")),
    }
}

#[cfg(test)]
mod tests {
    use anyhow::bail;

    use super::*;

    #[test]
    fn parses_generate_manifest_arguments() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "pv-release",
            "generate-manifest",
            "--records",
            "records",
            "--revocations",
            "revocations",
            "--output",
            "manifest.json",
            "--base-url",
            "https://artifacts.test",
        ])?;

        match args.command {
            Command::GenerateManifest {
                records,
                revocations,
                defaults,
                output,
                base_url,
            } => {
                assert_eq!(records, Utf8PathBuf::from("records"));
                assert_eq!(revocations, Utf8PathBuf::from("revocations"));
                assert_eq!(defaults, None);
                assert_eq!(output, Utf8PathBuf::from("manifest.json"));
                assert_eq!(base_url, "https://artifacts.test");
                Ok(())
            }
            command => bail!("parsed unexpected command: {command:?}"),
        }
    }

    #[test]
    fn parses_generate_manifest_defaults_argument() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "pv-release",
            "generate-manifest",
            "--records",
            "records",
            "--revocations",
            "revocations",
            "--defaults",
            "release/artifacts/default-tracks.toml",
            "--output",
            "manifest.json",
            "--base-url",
            "https://artifacts.test",
        ])?;

        match args.command {
            Command::GenerateManifest {
                records,
                revocations,
                defaults,
                output,
                base_url,
            } => {
                assert_eq!(records, Utf8PathBuf::from("records"));
                assert_eq!(revocations, Utf8PathBuf::from("revocations"));
                assert_eq!(
                    defaults,
                    Some(Utf8PathBuf::from("release/artifacts/default-tracks.toml"))
                );
                assert_eq!(output, Utf8PathBuf::from("manifest.json"));
                assert_eq!(base_url, "https://artifacts.test");
                Ok(())
            }
            command => bail!("parsed unexpected command: {command:?}"),
        }
    }

    #[test]
    fn parses_validate_archive_arguments() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "pv-release",
            "validate-archive",
            "--archive",
            "artifact.tar.gz",
            "--record",
            "release.json",
        ])?;

        match args.command {
            Command::ValidateArchive {
                archive,
                record,
                smoke_hook,
            } => {
                assert_eq!(archive, Utf8PathBuf::from("artifact.tar.gz"));
                assert_eq!(record, Utf8PathBuf::from("release.json"));
                assert_eq!(smoke_hook, None);
                Ok(())
            }
            command => bail!("parsed unexpected command: {command:?}"),
        }
    }

    #[test]
    fn parses_validate_archive_smoke_hook_argument() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "pv-release",
            "validate-archive",
            "--archive",
            "artifact.tar.gz",
            "--record",
            "release.json",
            "--smoke-hook",
            "smoke.sh",
        ])?;

        match args.command {
            Command::ValidateArchive {
                archive,
                record,
                smoke_hook,
            } => {
                assert_eq!(archive, Utf8PathBuf::from("artifact.tar.gz"));
                assert_eq!(record, Utf8PathBuf::from("release.json"));
                assert_eq!(smoke_hook, Some(Utf8PathBuf::from("smoke.sh")));
                Ok(())
            }
            command => bail!("parsed unexpected command: {command:?}"),
        }
    }
}
