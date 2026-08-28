use anyhow::Context;
use camino::{Utf8Path, Utf8PathBuf};
use clap::{Parser, Subcommand};
use resources::ArtifactPlatform;
use std::io::{self, Write};

use crate::app::WriteAppReleaseRecordRequest;
use crate::app_publication::AppPublicationRequest;
use crate::publication::PublicationRequest;
use crate::recipe::BackingRecipeKind;
use crate::record_writer::{
    PhpExtensionRecordRequest, SourceInputRequest, WriteReleaseRecordRequest,
};

#[derive(Debug, Parser)]
#[command(name = "pv-release")]
#[command(about = "PV internal artifact release tooling")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
#[expect(
    clippy::large_enum_variant,
    reason = "CLI subcommands intentionally keep parsed clap arguments inline"
)]
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
    GenerateAppManifest {
        #[arg(long)]
        records: Utf8PathBuf,
        #[arg(long)]
        output: Utf8PathBuf,
        #[arg(long)]
        base_url: String,
    },
    GenerateAppInstaller {
        #[arg(long)]
        records: Utf8PathBuf,
        #[arg(long)]
        output: Utf8PathBuf,
        #[arg(long)]
        base_url: String,
    },
    StagePublication {
        #[arg(long)]
        source_archives: Utf8PathBuf,
        #[arg(long)]
        candidate_records: Utf8PathBuf,
        #[arg(long)]
        published_records: Utf8PathBuf,
        #[arg(long)]
        published_revocations: Utf8PathBuf,
        #[arg(long)]
        defaults: Utf8PathBuf,
        #[arg(long)]
        stage: Utf8PathBuf,
        #[arg(long)]
        base_url: String,
        #[arg(long)]
        versioned_manifest_key: String,
        #[arg(long)]
        stable_manifest_key: String,
        #[arg(long = "required-native-platform")]
        required_native_platforms: Vec<String>,
    },
    StageAppPublication {
        #[arg(long)]
        source_binaries: Utf8PathBuf,
        #[arg(long)]
        candidate_records: Utf8PathBuf,
        #[arg(long)]
        stage: Utf8PathBuf,
        #[arg(long)]
        source_run_id: String,
        #[arg(long)]
        base_url: String,
        #[arg(long)]
        current_app_manifest: Option<Utf8PathBuf>,
        #[arg(long)]
        current_app_installer: Option<Utf8PathBuf>,
    },
    GenerateRecipeFixtures {
        #[arg(long)]
        php: Utf8PathBuf,
        #[arg(long)]
        composer: Utf8PathBuf,
        #[arg(long)]
        redis: Option<Utf8PathBuf>,
        #[arg(long)]
        mysql: Option<Utf8PathBuf>,
        #[arg(long)]
        postgres: Option<Utf8PathBuf>,
        #[arg(long)]
        mailpit: Option<Utf8PathBuf>,
        #[arg(long)]
        caddy: Option<Utf8PathBuf>,
        #[arg(long)]
        rustfs: Option<Utf8PathBuf>,
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
    WriteReleaseRecord {
        #[arg(long)]
        record: Utf8PathBuf,
        #[arg(long)]
        archive: Utf8PathBuf,
        #[arg(long)]
        resource: String,
        #[arg(long)]
        track: String,
        #[arg(long)]
        upstream_version: String,
        #[arg(long)]
        pv_build_revision: String,
        #[arg(long)]
        platform: String,
        #[arg(long)]
        object_key: String,
        #[arg(long)]
        source_url: String,
        #[arg(long)]
        source_sha256: String,
        #[arg(long)]
        recipe: String,
        #[arg(long)]
        pv_commit: String,
        #[arg(long)]
        build_run_id: String,
        #[arg(long)]
        minimum_pv_version: String,
        #[arg(long)]
        published_at: String,
        #[arg(long = "license-file")]
        license_files: Vec<String>,
        #[arg(long = "notice-file")]
        notice_files: Vec<String>,
        #[arg(long = "php-extension", num_args = 3, value_names = ["NAME", "LOAD_KIND", "PATH"])]
        php_extensions: Vec<String>,
        #[arg(long = "source-input", num_args = 3, value_names = ["NAME", "URL", "SHA256"])]
        source_inputs: Vec<String>,
    },
    WriteAppReleaseRecord {
        #[arg(long)]
        record: Utf8PathBuf,
        #[arg(long)]
        binary: Utf8PathBuf,
        #[arg(long)]
        version: String,
        #[arg(long)]
        minimum_pv_version: String,
        #[arg(long)]
        published_at: String,
        #[arg(long)]
        platform: String,
        #[arg(long)]
        object_key: String,
        #[arg(long)]
        source_url: String,
        #[arg(long)]
        source_sha256: String,
        #[arg(long)]
        recipe: String,
        #[arg(long)]
        pv_commit: String,
        #[arg(long)]
        build_run_id: String,
    },
    PrintRecipeEnv {
        #[arg(long)]
        php: Option<Utf8PathBuf>,
        #[arg(long)]
        composer: Option<Utf8PathBuf>,
        #[arg(long)]
        redis: Option<Utf8PathBuf>,
        #[arg(long)]
        mysql: Option<Utf8PathBuf>,
        #[arg(long)]
        postgres: Option<Utf8PathBuf>,
        #[arg(long)]
        mailpit: Option<Utf8PathBuf>,
        #[arg(long)]
        caddy: Option<Utf8PathBuf>,
        #[arg(long)]
        rustfs: Option<Utf8PathBuf>,
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
        Command::GenerateAppManifest {
            records,
            output,
            base_url,
        } => crate::app::generate_app_manifest_file(&records, &output, &base_url)
            .map_err(anyhow::Error::from),
        Command::GenerateAppInstaller {
            records,
            output,
            base_url,
        } => crate::app::generate_app_installer_file(&records, &output, &base_url)
            .map_err(anyhow::Error::from),
        Command::StagePublication {
            source_archives,
            candidate_records,
            published_records,
            published_revocations,
            defaults,
            stage,
            base_url,
            versioned_manifest_key,
            stable_manifest_key,
            required_native_platforms,
        } => crate::publication::prepare_publication(&PublicationRequest {
            source_archives,
            candidate_records,
            published_records,
            published_revocations,
            defaults,
            stage,
            base_url,
            versioned_manifest_key,
            stable_manifest_key,
            required_native_platforms: parse_required_native_platforms(required_native_platforms)?,
        })
        .context("failed to stage publication"),
        Command::StageAppPublication {
            source_binaries,
            candidate_records,
            stage,
            source_run_id,
            base_url,
            current_app_manifest,
            current_app_installer,
        } => crate::app_publication::stage_app_publication(&AppPublicationRequest {
            source_binaries,
            candidate_records,
            stage,
            source_run_id,
            base_url,
            current_app_manifest,
            current_app_installer,
        })
        .context("failed to stage app publication"),
        Command::GenerateRecipeFixtures {
            php,
            composer,
            redis,
            mysql,
            postgres,
            mailpit,
            caddy,
            rustfs,
            archives,
            records,
            pv_commit,
            build_run_id,
        } => crate::fixture::generate_recipe_fixtures_with_backing(
            &php,
            &composer,
            &backing_recipe_paths(redis, mysql, postgres, mailpit, caddy, rustfs),
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
        Command::WriteReleaseRecord {
            record,
            archive,
            resource,
            track,
            upstream_version,
            pv_build_revision,
            platform,
            object_key,
            source_url,
            source_sha256,
            recipe,
            pv_commit,
            build_run_id,
            minimum_pv_version,
            published_at,
            license_files,
            notice_files,
            php_extensions,
            source_inputs,
        } => {
            let context = format!("failed to write release record `{record}`");
            let php_extensions = parse_php_extensions(&php_extensions)?;
            let source_inputs = parse_source_inputs(&source_inputs)?;
            let license_files = default_legal_files(license_files, "LICENSE");
            let notice_files = default_legal_files(notice_files, "NOTICE");
            crate::record_writer::write_release_record(&WriteReleaseRecordRequest {
                record,
                archive,
                resource,
                track,
                upstream_version,
                pv_build_revision,
                platform,
                object_key,
                source_url,
                source_sha256,
                recipe,
                pv_commit,
                build_run_id,
                minimum_pv_version,
                published_at,
                license_files,
                notice_files,
                php_extensions,
                source_inputs,
            })
            .context(context)
        }
        Command::WriteAppReleaseRecord {
            record,
            binary,
            version,
            minimum_pv_version,
            published_at,
            platform,
            object_key,
            source_url,
            source_sha256,
            recipe,
            pv_commit,
            build_run_id,
        } => crate::app::write_app_release_record(&WriteAppReleaseRecordRequest {
            record,
            binary,
            version,
            minimum_pv_version,
            published_at,
            platform,
            object_key,
            source_url,
            source_sha256,
            recipe,
            pv_commit,
            build_run_id,
        })
        .map_err(anyhow::Error::from),
        Command::PrintRecipeEnv {
            php,
            composer,
            redis,
            mysql,
            postgres,
            mailpit,
            caddy,
            rustfs,
            resource,
            track,
            platform,
        } => {
            let env = print_recipe_env(
                RecipeEnvPaths {
                    php: php.as_deref(),
                    composer: composer.as_deref(),
                    redis: redis.as_deref(),
                    mysql: mysql.as_deref(),
                    postgres: postgres.as_deref(),
                    mailpit: mailpit.as_deref(),
                    caddy: caddy.as_deref(),
                    rustfs: rustfs.as_deref(),
                },
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

#[derive(Default)]
struct RecipeEnvPaths<'a> {
    php: Option<&'a Utf8Path>,
    composer: Option<&'a Utf8Path>,
    redis: Option<&'a Utf8Path>,
    mysql: Option<&'a Utf8Path>,
    postgres: Option<&'a Utf8Path>,
    mailpit: Option<&'a Utf8Path>,
    caddy: Option<&'a Utf8Path>,
    rustfs: Option<&'a Utf8Path>,
}

#[derive(Clone, Copy)]
enum RecipeEnvKind {
    Php,
    Composer,
    Backing(BackingRecipeKind),
}

impl RecipeEnvKind {
    fn display_name(self) -> &'static str {
        match self {
            Self::Php => "PHP",
            Self::Composer => "Composer",
            Self::Backing(BackingRecipeKind::Redis) => "Redis",
            Self::Backing(BackingRecipeKind::Mysql) => "MySQL",
            Self::Backing(BackingRecipeKind::Postgres) => "Postgres",
            Self::Backing(BackingRecipeKind::Mailpit) => "Mailpit",
            Self::Backing(BackingRecipeKind::Caddy) => "Caddy",
            Self::Backing(BackingRecipeKind::Rustfs) => "RustFS",
        }
    }
}

fn parse_source_inputs(values: &[String]) -> anyhow::Result<Vec<SourceInputRequest>> {
    let (chunks, remainder) = values.as_chunks::<3>();
    let source_inputs = chunks
        .iter()
        .map(|[name, source_url, source_sha256]| SourceInputRequest {
            name: name.clone(),
            source_url: source_url.clone(),
            source_sha256: source_sha256.clone(),
        })
        .collect::<Vec<_>>();

    if !remainder.is_empty() {
        anyhow::bail!("each --source-input requires NAME URL SHA256");
    }

    Ok(source_inputs)
}

fn parse_php_extensions(values: &[String]) -> anyhow::Result<Vec<PhpExtensionRecordRequest>> {
    let (chunks, remainder) = values.as_chunks::<3>();
    let php_extensions = chunks
        .iter()
        .map(|[name, load_kind, path]| PhpExtensionRecordRequest {
            name: name.clone(),
            load_kind: load_kind.clone(),
            path: path.clone(),
        })
        .collect::<Vec<_>>();

    if !remainder.is_empty() {
        anyhow::bail!("each --php-extension requires NAME LOAD_KIND PATH");
    }

    Ok(php_extensions)
}

fn default_legal_files(values: Vec<String>, default: &str) -> Vec<String> {
    if values.is_empty() {
        vec![default.to_string()]
    } else {
        values
    }
}

fn parse_required_native_platforms(values: Vec<String>) -> anyhow::Result<Vec<ArtifactPlatform>> {
    let values = if values.is_empty() {
        vec!["darwin-arm64".to_string(), "darwin-amd64".to_string()]
    } else {
        values
    };

    values
        .into_iter()
        .map(|value| {
            ArtifactPlatform::new(&value)
                .with_context(|| format!("invalid required native platform `{value}`"))
        })
        .collect()
}

fn backing_recipe_paths(
    redis: Option<Utf8PathBuf>,
    mysql: Option<Utf8PathBuf>,
    postgres: Option<Utf8PathBuf>,
    mailpit: Option<Utf8PathBuf>,
    caddy: Option<Utf8PathBuf>,
    rustfs: Option<Utf8PathBuf>,
) -> Vec<(BackingRecipeKind, Utf8PathBuf)> {
    [
        redis.map(|path| (BackingRecipeKind::Redis, path)),
        mysql.map(|path| (BackingRecipeKind::Mysql, path)),
        postgres.map(|path| (BackingRecipeKind::Postgres, path)),
        mailpit.map(|path| (BackingRecipeKind::Mailpit, path)),
        caddy.map(|path| (BackingRecipeKind::Caddy, path)),
        rustfs.map(|path| (BackingRecipeKind::Rustfs, path)),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn print_recipe_env(
    paths: RecipeEnvPaths<'_>,
    resource: &str,
    track: &str,
    platform: &str,
) -> anyhow::Result<String> {
    let metadata_paths = [
        paths.php.map(|path| (RecipeEnvKind::Php, path)),
        paths.composer.map(|path| (RecipeEnvKind::Composer, path)),
        paths
            .redis
            .map(|path| (RecipeEnvKind::Backing(BackingRecipeKind::Redis), path)),
        paths
            .mysql
            .map(|path| (RecipeEnvKind::Backing(BackingRecipeKind::Mysql), path)),
        paths
            .postgres
            .map(|path| (RecipeEnvKind::Backing(BackingRecipeKind::Postgres), path)),
        paths
            .mailpit
            .map(|path| (RecipeEnvKind::Backing(BackingRecipeKind::Mailpit), path)),
        paths
            .caddy
            .map(|path| (RecipeEnvKind::Backing(BackingRecipeKind::Caddy), path)),
        paths
            .rustfs
            .map(|path| (RecipeEnvKind::Backing(BackingRecipeKind::Rustfs), path)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();

    let [(kind, path)] = metadata_paths.as_slice() else {
        anyhow::bail!("print-recipe-env requires exactly one recipe metadata path");
    };
    let context = format!(
        "failed to print {} recipe environment for `{path}`",
        kind.display_name()
    );

    match kind {
        RecipeEnvKind::Php => {
            crate::recipe::php_recipe_env(path, resource, track, platform).context(context)
        }
        RecipeEnvKind::Composer => {
            crate::recipe::composer_recipe_env(path, resource, track, platform).context(context)
        }
        RecipeEnvKind::Backing(kind) => {
            crate::recipe::backing_recipe_env(path, *kind, resource, track, platform)
                .context(context)
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
                redis,
                mysql,
                postgres,
                mailpit,
                caddy,
                rustfs,
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
                assert_eq!(redis, None);
                assert_eq!(mysql, None);
                assert_eq!(postgres, None);
                assert_eq!(mailpit, None);
                assert_eq!(caddy, None);
                assert_eq!(rustfs, None);
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
    fn parses_generate_recipe_fixtures_arguments_with_backing_paths() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "pv-release",
            "generate-recipe-fixtures",
            "--php",
            "release/artifacts/recipes/php/tracks.toml",
            "--composer",
            "release/artifacts/recipes/composer/composer.toml",
            "--redis",
            "release/artifacts/recipes/redis/recipe.toml",
            "--mysql",
            "release/artifacts/recipes/mysql/recipe.toml",
            "--postgres",
            "release/artifacts/recipes/postgres/recipe.toml",
            "--mailpit",
            "release/artifacts/recipes/mailpit/recipe.toml",
            "--caddy",
            "release/artifacts/recipes/caddy/recipe.toml",
            "--rustfs",
            "release/artifacts/recipes/rustfs/recipe.toml",
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
                redis,
                mysql,
                postgres,
                mailpit,
                caddy,
                rustfs,
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
                assert_eq!(
                    redis,
                    Some(Utf8PathBuf::from(
                        "release/artifacts/recipes/redis/recipe.toml"
                    ))
                );
                assert_eq!(
                    mysql,
                    Some(Utf8PathBuf::from(
                        "release/artifacts/recipes/mysql/recipe.toml"
                    ))
                );
                assert_eq!(
                    postgres,
                    Some(Utf8PathBuf::from(
                        "release/artifacts/recipes/postgres/recipe.toml"
                    ))
                );
                assert_eq!(
                    mailpit,
                    Some(Utf8PathBuf::from(
                        "release/artifacts/recipes/mailpit/recipe.toml"
                    ))
                );
                assert_eq!(
                    caddy,
                    Some(Utf8PathBuf::from(
                        "release/artifacts/recipes/caddy/recipe.toml"
                    ))
                );
                assert_eq!(
                    rustfs,
                    Some(Utf8PathBuf::from(
                        "release/artifacts/recipes/rustfs/recipe.toml"
                    ))
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
    fn parses_stage_publication_arguments() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "pv-release",
            "stage-publication",
            "--source-archives",
            "source-archives",
            "--candidate-records",
            "candidate-records",
            "--published-records",
            "published-records",
            "--published-revocations",
            "published-revocations",
            "--defaults",
            "release/artifacts/default-tracks.toml",
            "--stage",
            "stage",
            "--base-url",
            "https://artifacts.example.test",
            "--versioned-manifest-key",
            "manifests/runs/123456789/manifest.json",
            "--stable-manifest-key",
            "manifest.json",
        ])?;

        match args.command {
            Command::StagePublication {
                source_archives,
                candidate_records,
                published_records,
                published_revocations,
                defaults,
                stage,
                base_url,
                versioned_manifest_key,
                stable_manifest_key,
                required_native_platforms,
            } => {
                assert_eq!(source_archives, Utf8PathBuf::from("source-archives"));
                assert_eq!(candidate_records, Utf8PathBuf::from("candidate-records"));
                assert_eq!(published_records, Utf8PathBuf::from("published-records"));
                assert_eq!(
                    published_revocations,
                    Utf8PathBuf::from("published-revocations")
                );
                assert_eq!(
                    defaults,
                    Utf8PathBuf::from("release/artifacts/default-tracks.toml")
                );
                assert_eq!(stage, Utf8PathBuf::from("stage"));
                assert_eq!(base_url, "https://artifacts.example.test");
                assert_eq!(
                    versioned_manifest_key,
                    "manifests/runs/123456789/manifest.json"
                );
                assert_eq!(stable_manifest_key, "manifest.json");
                assert_eq!(required_native_platforms, Vec::<String>::new());
                assert_eq!(
                    parse_required_native_platforms(required_native_platforms)?,
                    vec![ArtifactPlatform::DarwinArm64, ArtifactPlatform::DarwinAmd64]
                );
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
    fn parses_generate_app_installer_arguments() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "pv-release",
            "generate-app-installer",
            "--records",
            "records",
            "--output",
            "install.sh",
            "--base-url",
            "https://artifacts.test",
        ])?;

        match args.command {
            Command::GenerateAppInstaller {
                records,
                output,
                base_url,
            } => {
                assert_eq!(records, Utf8PathBuf::from("records"));
                assert_eq!(output, Utf8PathBuf::from("install.sh"));
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
    fn parses_write_release_record_arguments() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "pv-release",
            "write-release-record",
            "--record",
            "record.json",
            "--archive",
            "artifact.tar.gz",
            "--resource",
            "frankenphp",
            "--track",
            "8.4",
            "--upstream-version",
            "8.4.20-frankenphp1.12.3",
            "--pv-build-revision",
            "pv1",
            "--platform",
            "darwin-arm64",
            "--object-key",
            "resources/frankenphp/8.4/8.4.20-frankenphp1.12.3-pv1/darwin-arm64/frankenphp-8.4.20-frankenphp1.12.3-pv1-darwin-arm64.tar.gz",
            "--source-url",
            "https://github.com/php/frankenphp/archive/refs/tags/v1.12.3.tar.gz",
            "--source-sha256",
            "2996fb95bbdf8410847fdcd59df04cd2e297568f6472ebe488af5fb5f3c79363",
            "--recipe",
            "release/artifacts/recipes/php/build.sh",
            "--pv-commit",
            "0123456789abcdef0123456789abcdef01234567",
            "--build-run-id",
            "local-test",
            "--minimum-pv-version",
            "0.1.0",
            "--published-at",
            "2026-06-08T12:00:00Z",
            "--license-file",
            "LICENSE",
            "--notice-file",
            "NOTICE",
            "--notice-file",
            "THIRD-PARTY-NOTICES",
            "--php-extension",
            "redis",
            "extension",
            "lib/php/extensions/redis.so",
            "--php-extension",
            "xdebug",
            "zend_extension",
            "lib/php/extensions/xdebug.so",
            "--source-input",
            "frankenphp",
            "https://github.com/php/frankenphp/archive/refs/tags/v1.12.3.tar.gz",
            "2996fb95bbdf8410847fdcd59df04cd2e297568f6472ebe488af5fb5f3c79363",
            "--source-input",
            "php",
            "https://www.php.net/distributions/php-8.4.20.tar.gz",
            "a2def5d534d57c6a0236f2265de7537608af871900a4f7955eff463e9e38247d",
        ])?;

        match args.command {
            Command::WriteReleaseRecord {
                record,
                archive,
                resource,
                track,
                upstream_version,
                pv_build_revision,
                platform,
                object_key,
                source_url,
                source_sha256,
                recipe,
                pv_commit,
                build_run_id,
                minimum_pv_version,
                published_at,
                license_files,
                notice_files,
                php_extensions,
                source_inputs,
            } => {
                assert_eq!(record, Utf8PathBuf::from("record.json"));
                assert_eq!(archive, Utf8PathBuf::from("artifact.tar.gz"));
                assert_eq!(resource, "frankenphp");
                assert_eq!(track, "8.4");
                assert_eq!(upstream_version, "8.4.20-frankenphp1.12.3");
                assert_eq!(pv_build_revision, "pv1");
                assert_eq!(platform, "darwin-arm64");
                assert_eq!(
                    object_key,
                    "resources/frankenphp/8.4/8.4.20-frankenphp1.12.3-pv1/darwin-arm64/frankenphp-8.4.20-frankenphp1.12.3-pv1-darwin-arm64.tar.gz"
                );
                assert_eq!(
                    source_url,
                    "https://github.com/php/frankenphp/archive/refs/tags/v1.12.3.tar.gz"
                );
                assert_eq!(
                    source_sha256,
                    "2996fb95bbdf8410847fdcd59df04cd2e297568f6472ebe488af5fb5f3c79363"
                );
                assert_eq!(recipe, "release/artifacts/recipes/php/build.sh");
                assert_eq!(pv_commit, "0123456789abcdef0123456789abcdef01234567");
                assert_eq!(build_run_id, "local-test");
                assert_eq!(minimum_pv_version, "0.1.0");
                assert_eq!(published_at, "2026-06-08T12:00:00Z");
                assert_eq!(license_files, vec!["LICENSE"]);
                assert_eq!(notice_files, vec!["NOTICE", "THIRD-PARTY-NOTICES"]);
                assert_eq!(
                    php_extensions,
                    vec![
                        "redis",
                        "extension",
                        "lib/php/extensions/redis.so",
                        "xdebug",
                        "zend_extension",
                        "lib/php/extensions/xdebug.so",
                    ]
                );
                assert_eq!(
                    source_inputs,
                    vec![
                        "frankenphp",
                        "https://github.com/php/frankenphp/archive/refs/tags/v1.12.3.tar.gz",
                        "2996fb95bbdf8410847fdcd59df04cd2e297568f6472ebe488af5fb5f3c79363",
                        "php",
                        "https://www.php.net/distributions/php-8.4.20.tar.gz",
                        "a2def5d534d57c6a0236f2265de7537608af871900a4f7955eff463e9e38247d",
                    ]
                );
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
                redis,
                mysql,
                postgres,
                mailpit,
                caddy,
                rustfs,
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
                assert_eq!(redis, None);
                assert_eq!(mysql, None);
                assert_eq!(postgres, None);
                assert_eq!(mailpit, None);
                assert_eq!(caddy, None);
                assert_eq!(rustfs, None);
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
                redis,
                mysql,
                postgres,
                mailpit,
                caddy,
                rustfs,
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
                assert_eq!(redis, None);
                assert_eq!(mysql, None);
                assert_eq!(postgres, None);
                assert_eq!(mailpit, None);
                assert_eq!(caddy, None);
                assert_eq!(rustfs, None);
                assert_eq!(resource, "php");
                assert_eq!(track, "8.4");
                assert_eq!(platform, "darwin-arm64");
                Ok(())
            }
            command => bail!("parsed unexpected command: {command:?}"),
        }
    }

    #[test]
    fn parses_print_recipe_env_redis_arguments() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "pv-release",
            "print-recipe-env",
            "--redis",
            "release/artifacts/recipes/redis/recipe.toml",
            "--resource",
            "redis",
            "--track",
            "8.2",
            "--platform",
            "darwin-arm64",
        ])?;

        match args.command {
            Command::PrintRecipeEnv {
                php,
                composer,
                redis,
                mysql,
                postgres,
                mailpit,
                caddy,
                rustfs,
                resource,
                track,
                platform,
            } => {
                assert_eq!(php, None);
                assert_eq!(composer, None);
                assert_eq!(
                    redis,
                    Some(Utf8PathBuf::from(
                        "release/artifacts/recipes/redis/recipe.toml"
                    ))
                );
                assert_eq!(mysql, None);
                assert_eq!(postgres, None);
                assert_eq!(mailpit, None);
                assert_eq!(caddy, None);
                assert_eq!(rustfs, None);
                assert_eq!(resource, "redis");
                assert_eq!(track, "8.2");
                assert_eq!(platform, "darwin-arm64");
                Ok(())
            }
            command => bail!("parsed unexpected command: {command:?}"),
        }
    }

    #[test]
    fn parses_print_recipe_env_mysql_arguments() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "pv-release",
            "print-recipe-env",
            "--mysql",
            "release/artifacts/recipes/mysql/recipe.toml",
            "--resource",
            "mysql",
            "--track",
            "8.4",
            "--platform",
            "darwin-arm64",
        ])?;

        match args.command {
            Command::PrintRecipeEnv {
                php,
                composer,
                redis,
                mysql,
                postgres,
                mailpit,
                caddy,
                rustfs,
                resource,
                track,
                platform,
            } => {
                assert_eq!(php, None);
                assert_eq!(composer, None);
                assert_eq!(redis, None);
                assert_eq!(
                    mysql,
                    Some(Utf8PathBuf::from(
                        "release/artifacts/recipes/mysql/recipe.toml"
                    ))
                );
                assert_eq!(postgres, None);
                assert_eq!(mailpit, None);
                assert_eq!(caddy, None);
                assert_eq!(rustfs, None);
                assert_eq!(resource, "mysql");
                assert_eq!(track, "8.4");
                assert_eq!(platform, "darwin-arm64");
                Ok(())
            }
            command => bail!("parsed unexpected command: {command:?}"),
        }
    }

    #[test]
    fn parses_print_recipe_env_postgres_arguments() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "pv-release",
            "print-recipe-env",
            "--postgres",
            "release/artifacts/recipes/postgres/recipe.toml",
            "--resource",
            "postgres",
            "--track",
            "18",
            "--platform",
            "darwin-arm64",
        ])?;

        match args.command {
            Command::PrintRecipeEnv {
                php,
                composer,
                redis,
                mysql,
                postgres,
                mailpit,
                caddy,
                rustfs,
                resource,
                track,
                platform,
            } => {
                assert_eq!(php, None);
                assert_eq!(composer, None);
                assert_eq!(redis, None);
                assert_eq!(mysql, None);
                assert_eq!(
                    postgres,
                    Some(Utf8PathBuf::from(
                        "release/artifacts/recipes/postgres/recipe.toml"
                    ))
                );
                assert_eq!(mailpit, None);
                assert_eq!(caddy, None);
                assert_eq!(rustfs, None);
                assert_eq!(resource, "postgres");
                assert_eq!(track, "18");
                assert_eq!(platform, "darwin-arm64");
                Ok(())
            }
            command => bail!("parsed unexpected command: {command:?}"),
        }
    }

    #[test]
    fn parses_print_recipe_env_mailpit_arguments() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "pv-release",
            "print-recipe-env",
            "--mailpit",
            "release/artifacts/recipes/mailpit/recipe.toml",
            "--resource",
            "mailpit",
            "--track",
            "1",
            "--platform",
            "darwin-arm64",
        ])?;

        match args.command {
            Command::PrintRecipeEnv {
                php,
                composer,
                redis,
                mysql,
                postgres,
                mailpit,
                caddy,
                rustfs,
                resource,
                track,
                platform,
            } => {
                assert_eq!(php, None);
                assert_eq!(composer, None);
                assert_eq!(redis, None);
                assert_eq!(mysql, None);
                assert_eq!(postgres, None);
                assert_eq!(
                    mailpit,
                    Some(Utf8PathBuf::from(
                        "release/artifacts/recipes/mailpit/recipe.toml"
                    ))
                );
                assert_eq!(caddy, None);
                assert_eq!(rustfs, None);
                assert_eq!(resource, "mailpit");
                assert_eq!(track, "1");
                assert_eq!(platform, "darwin-arm64");
                Ok(())
            }
            command => bail!("parsed unexpected command: {command:?}"),
        }
    }

    #[test]
    fn parses_print_recipe_env_rustfs_arguments() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "pv-release",
            "print-recipe-env",
            "--rustfs",
            "release/artifacts/recipes/rustfs/recipe.toml",
            "--resource",
            "rustfs",
            "--track",
            "1",
            "--platform",
            "darwin-arm64",
        ])?;

        match args.command {
            Command::PrintRecipeEnv {
                php,
                composer,
                redis,
                mysql,
                postgres,
                mailpit,
                caddy,
                rustfs,
                resource,
                track,
                platform,
            } => {
                assert_eq!(php, None);
                assert_eq!(composer, None);
                assert_eq!(redis, None);
                assert_eq!(mysql, None);
                assert_eq!(postgres, None);
                assert_eq!(mailpit, None);
                assert_eq!(caddy, None);
                assert_eq!(
                    rustfs,
                    Some(Utf8PathBuf::from(
                        "release/artifacts/recipes/rustfs/recipe.toml"
                    ))
                );
                assert_eq!(resource, "rustfs");
                assert_eq!(track, "1");
                assert_eq!(platform, "darwin-arm64");
                Ok(())
            }
            command => bail!("parsed unexpected command: {command:?}"),
        }
    }

    #[test]
    fn parses_print_recipe_env_caddy_arguments() -> anyhow::Result<()> {
        let args = Args::try_parse_from([
            "pv-release",
            "print-recipe-env",
            "--caddy",
            "release/artifacts/recipes/caddy/recipe.toml",
            "--resource",
            "caddy",
            "--track",
            "2",
            "--platform",
            "darwin-arm64",
        ])?;

        match args.command {
            Command::PrintRecipeEnv {
                php,
                composer,
                redis,
                mysql,
                postgres,
                mailpit,
                caddy,
                rustfs,
                resource,
                track,
                platform,
            } => {
                assert_eq!(php, None);
                assert_eq!(composer, None);
                assert_eq!(redis, None);
                assert_eq!(mysql, None);
                assert_eq!(postgres, None);
                assert_eq!(mailpit, None);
                assert_eq!(
                    caddy,
                    Some(Utf8PathBuf::from(
                        "release/artifacts/recipes/caddy/recipe.toml"
                    ))
                );
                assert_eq!(rustfs, None);
                assert_eq!(resource, "caddy");
                assert_eq!(track, "2");
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
        let redis = Utf8Path::new("release/artifacts/recipes/redis/recipe.toml");
        let mysql = Utf8Path::new("release/artifacts/recipes/mysql/recipe.toml");
        let mailpit = Utf8Path::new("release/artifacts/recipes/mailpit/recipe.toml");

        assert!(
            print_recipe_env(RecipeEnvPaths::default(), "php", "8.4", "darwin-arm64").is_err(),
            "missing metadata path must be rejected"
        );
        assert!(
            print_recipe_env(
                RecipeEnvPaths {
                    php: Some(php),
                    composer: Some(composer),
                    ..RecipeEnvPaths::default()
                },
                "php",
                "8.4",
                "darwin-arm64"
            )
            .is_err(),
            "multiple metadata paths must be rejected"
        );
        assert!(
            print_recipe_env(
                RecipeEnvPaths {
                    php: Some(php),
                    redis: Some(redis),
                    ..RecipeEnvPaths::default()
                },
                "redis",
                "8.2",
                "darwin-arm64"
            )
            .is_err(),
            "multiple metadata paths must be rejected"
        );
        assert!(
            print_recipe_env(
                RecipeEnvPaths {
                    redis: Some(redis),
                    mysql: Some(mysql),
                    mailpit: Some(mailpit),
                    ..RecipeEnvPaths::default()
                },
                "redis",
                "8.2",
                "darwin-arm64"
            )
            .is_err(),
            "multiple metadata paths must be rejected"
        );
    }
}
