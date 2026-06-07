use anyhow::Context;
use camino::{Utf8Path, Utf8PathBuf};
use clap::{Parser, Subcommand};
use std::io::{self, Write};

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
    GenerateRecipeFixtures {
        #[arg(long)]
        php: Utf8PathBuf,
        #[arg(long)]
        composer: Utf8PathBuf,
        #[arg(long)]
        archives: Utf8PathBuf,
        #[arg(long)]
        records: Utf8PathBuf,
        #[arg(long)]
        pv_commit: String,
        #[arg(long)]
        build_run_id: String,
    },
    ValidateArchive {
        #[arg(long)]
        archive: Utf8PathBuf,
        #[arg(long)]
        record: Utf8PathBuf,
        #[arg(long)]
        smoke_hook: Option<Utf8PathBuf>,
    },
    PrintRecipeEnv {
        #[arg(long)]
        php: Option<Utf8PathBuf>,
        #[arg(long)]
        composer: Option<Utf8PathBuf>,
        #[arg(long)]
        resource: String,
        #[arg(long)]
        track: String,
        #[arg(long)]
        platform: String,
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
        Command::GenerateRecipeFixtures {
            php,
            composer,
            archives,
            records,
            pv_commit,
            build_run_id,
        } => crate::fixture::generate_recipe_fixtures(
            &php,
            &composer,
            &archives,
            &records,
            &pv_commit,
            &build_run_id,
        )
        .with_context(|| format!("failed to generate recipe fixtures under `{records}`")),
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
        Command::PrintRecipeEnv {
            php,
            composer,
            resource,
            track,
            platform,
        } => {
            let env = print_recipe_env(
                php.as_deref(),
                composer.as_deref(),
                &resource,
                &track,
                &platform,
            )?;
            let mut stdout = io::stdout().lock();
            stdout
                .write_all(env.as_bytes())
                .context("failed to write recipe environment to stdout")
        }
    }
}

fn print_recipe_env(
    php: Option<&Utf8Path>,
    composer: Option<&Utf8Path>,
    resource: &str,
    track: &str,
    platform: &str,
) -> anyhow::Result<String> {
    match (php, composer) {
        (Some(php), None) => {
            let context = format!("failed to print PHP recipe environment for `{php}`");
            crate::recipe::php_recipe_env(php, resource, track, platform).context(context)
        }
        (None, Some(composer)) => {
            let context = format!("failed to print Composer recipe environment for `{composer}`");
            crate::recipe::composer_recipe_env(composer, resource, track, platform).context(context)
        }
        (None, None) | (Some(_), Some(_)) => {
            anyhow::bail!("print-recipe-env requires exactly one of --php or --composer")
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::bail;

    use super::*;

    #[test]
    fn parses_generate_recipe_fixtures_arguments() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "pv-release",
            "generate-recipe-fixtures",
            "--php",
            "release/artifacts/recipes/php/tracks.toml",
            "--composer",
            "release/artifacts/recipes/composer/composer.toml",
            "--archives",
            "archives",
            "--records",
            "records",
            "--pv-commit",
            "0123456789abcdef0123456789abcdef01234567",
            "--build-run-id",
            "local-test",
        ])?;

        match args.command {
            Command::GenerateRecipeFixtures {
                php,
                composer,
                archives,
                records,
                pv_commit,
                build_run_id,
            } => {
                assert_eq!(
                    php,
                    Utf8PathBuf::from("release/artifacts/recipes/php/tracks.toml")
                );
                assert_eq!(
                    composer,
                    Utf8PathBuf::from("release/artifacts/recipes/composer/composer.toml")
                );
                assert_eq!(archives, Utf8PathBuf::from("archives"));
                assert_eq!(records, Utf8PathBuf::from("records"));
                assert_eq!(pv_commit, "0123456789abcdef0123456789abcdef01234567");
                assert_eq!(build_run_id, "local-test");
                Ok(())
            }
            command => bail!("parsed unexpected command: {command:?}"),
        }
    }

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

    #[test]
    fn parses_print_recipe_env_arguments() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "pv-release",
            "print-recipe-env",
            "--composer",
            "release/artifacts/recipes/composer/composer.toml",
            "--resource",
            "composer",
            "--track",
            "2",
            "--platform",
            "any",
        ])?;

        match args.command {
            Command::PrintRecipeEnv {
                php,
                composer,
                resource,
                track,
                platform,
            } => {
                assert_eq!(php, None);
                assert_eq!(
                    composer,
                    Some(Utf8PathBuf::from(
                        "release/artifacts/recipes/composer/composer.toml"
                    ))
                );
                assert_eq!(resource, "composer");
                assert_eq!(track, "2");
                assert_eq!(platform, "any");
                Ok(())
            }
            command => bail!("parsed unexpected command: {command:?}"),
        }
    }

    #[test]
    fn parses_print_recipe_env_php_arguments() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "pv-release",
            "print-recipe-env",
            "--php",
            "release/artifacts/recipes/php/tracks.toml",
            "--resource",
            "php",
            "--track",
            "8.4",
            "--platform",
            "darwin-arm64",
        ])?;

        match args.command {
            Command::PrintRecipeEnv {
                php,
                composer,
                resource,
                track,
                platform,
            } => {
                assert_eq!(
                    php,
                    Some(Utf8PathBuf::from(
                        "release/artifacts/recipes/php/tracks.toml"
                    ))
                );
                assert_eq!(composer, None);
                assert_eq!(resource, "php");
                assert_eq!(track, "8.4");
                assert_eq!(platform, "darwin-arm64");
                Ok(())
            }
            command => bail!("parsed unexpected command: {command:?}"),
        }
    }

    #[test]
    fn print_recipe_env_rejects_missing_or_multiple_metadata_paths() {
        let php = Utf8Path::new("release/artifacts/recipes/php/tracks.toml");
        let composer = Utf8Path::new("release/artifacts/recipes/composer/composer.toml");

        assert!(
            print_recipe_env(None, None, "php", "8.4", "darwin-arm64").is_err(),
            "missing metadata path must be rejected"
        );
        assert!(
            print_recipe_env(Some(php), Some(composer), "php", "8.4", "darwin-arm64").is_err(),
            "multiple metadata paths must be rejected"
        );
    }
}
