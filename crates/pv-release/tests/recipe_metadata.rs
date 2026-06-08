use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::{assert_debug_snapshot, assert_snapshot};
use pv_release::ReleaseError;
use pv_release::defaults::ManifestDefaults;
use pv_release::recipe::{
    BackingRecipe, BackingRecipeKind, ComposerRecipe, PhpRecipe, backing_recipe_env,
    composer_recipe_env, php_recipe_env,
};
use resources::ResourceName;

#[test]
fn recipe_metadata_parses_php_tracks_and_composer() -> Result<()> {
    let php = PhpRecipe::from_toml(Utf8Path::new("tracks.toml"), VALID_PHP_TOML)?;
    let composer = ComposerRecipe::from_toml(Utf8Path::new("composer.toml"), VALID_COMPOSER_TOML)?;

    assert_debug_snapshot!((
        php_summary(&php),
        composer.track().as_str(),
        composer.upstream_version(),
        composer.platform().as_str(),
    ));
    Ok(())
}

#[test]
fn backing_recipe_metadata_parses_common_shape() -> Result<()> {
    let recipe = BackingRecipe::from_toml(
        Utf8Path::new("redis/recipe.toml"),
        BackingRecipeKind::Redis,
        VALID_REDIS_TOML,
    )?;

    assert_debug_snapshot!((
        recipe.kind(),
        recipe.resource().as_str(),
        recipe.default_track().as_str(),
        recipe
            .platforms()
            .iter()
            .map(|platform| platform.as_str())
            .collect::<Vec<_>>(),
        recipe
            .tracks()
            .iter()
            .map(|track| (
                track.name().as_str(),
                track.upstream_version(),
                track.source_url(),
                track.source_sha256().as_str(),
            ))
            .collect::<Vec<_>>(),
        recipe.payload_paths(),
    ));

    Ok(())
}

#[test]
fn backing_recipe_metadata_accepts_additional_legal_files() -> Result<()> {
    let redis_with_third_party_notices = VALID_REDIS_TOML.replace(
        "notice_files = [\"NOTICE\"]",
        "notice_files = [\"NOTICE\", \"THIRD-PARTY-NOTICES\"]",
    );

    let recipe = BackingRecipe::from_toml(
        Utf8Path::new("redis/recipe.toml"),
        BackingRecipeKind::Redis,
        &redis_with_third_party_notices,
    )?;

    assert_eq!(recipe.license_files(), ["LICENSE"]);
    assert_eq!(recipe.notice_files(), ["NOTICE", "THIRD-PARTY-NOTICES"]);
    Ok(())
}

#[test]
fn committed_recipe_metadata_parses() -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let php = PhpRecipe::load(&workspace_root.join("release/artifacts/recipes/php/tracks.toml"))?;
    let composer = ComposerRecipe::load(
        &workspace_root.join("release/artifacts/recipes/composer/composer.toml"),
    )?;
    let redis = BackingRecipe::load(
        &workspace_root.join("release/artifacts/recipes/redis/recipe.toml"),
        BackingRecipeKind::Redis,
    )?;
    let mailpit = BackingRecipe::load(
        &workspace_root.join("release/artifacts/recipes/mailpit/recipe.toml"),
        BackingRecipeKind::Mailpit,
    )?;
    let rustfs = BackingRecipe::load(
        &workspace_root.join("release/artifacts/recipes/rustfs/recipe.toml"),
        BackingRecipeKind::Rustfs,
    )?;
    let defaults =
        ManifestDefaults::load(&workspace_root.join("release/artifacts/default-tracks.toml"))?;

    assert_eq!(php.default_track().as_str(), "8.4");
    assert_eq!(php.tracks().len(), 3);
    assert_eq!(composer.track().as_str(), "2");
    assert_eq!(composer.platform().as_str(), "any");
    assert_eq!(redis.default_track().as_str(), "8.2");
    assert_eq!(redis.tracks().len(), 1);
    assert_eq!(redis.payload_paths(), ["bin/redis-server", "bin/redis-cli"]);
    assert_eq!(mailpit.default_track().as_str(), "1");
    assert_eq!(mailpit.payload_paths(), ["bin/mailpit"]);
    assert_eq!(rustfs.default_track().as_str(), "1");
    assert_eq!(rustfs.payload_paths(), ["bin/rustfs"]);
    assert_default_track(&defaults, "php", "8.4")?;
    assert_default_track(&defaults, "frankenphp", "8.4")?;
    assert_default_track(&defaults, "composer", "2")?;
    assert_default_track(&defaults, "redis", "8.2")?;
    assert_default_track(&defaults, "mailpit", "1")?;
    assert_default_track(&defaults, "rustfs", "1")?;

    Ok(())
}

#[test]
fn backing_recipe_metadata_rejects_invalid_shapes() -> Result<()> {
    let wrong_resource =
        VALID_REDIS_TOML.replace("resources = [\"redis\"]", "resources = [\"mysql\"]");
    let missing_platform = VALID_REDIS_TOML.replace(
        "platforms = [\"darwin-arm64\", \"darwin-amd64\"]",
        "platforms = [\"darwin-arm64\"]",
    );
    let missing_default_track =
        VALID_REDIS_TOML.replace("default_track = \"8.2\"", "default_track = \"8.0\"");
    let empty_payload_paths = VALID_REDIS_TOML.replace(
        "payload_paths = [\"bin/redis-server\", \"bin/redis-cli\"]",
        "payload_paths = []",
    );
    let unsafe_payload_path = VALID_REDIS_TOML.replace(
        "\"bin/redis-server\", \"bin/redis-cli\"",
        "\"bin/redis-server\", \"../bin/redis-cli\"",
    );
    let insecure_source_url = VALID_REDIS_TOML.replace(
        "https://download.redis.io/releases/redis-8.2.1.tar.gz",
        "http://download.redis.io/releases/redis-8.2.1.tar.gz",
    );
    let bad_source_sha256 = VALID_REDIS_TOML.replace(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "bad",
    );

    assert_debug_snapshot!((
        BackingRecipe::from_toml(
            Utf8Path::new("wrong-resource.toml"),
            BackingRecipeKind::Redis,
            &wrong_resource,
        ),
        BackingRecipe::from_toml(
            Utf8Path::new("missing-platform.toml"),
            BackingRecipeKind::Redis,
            &missing_platform,
        ),
        BackingRecipe::from_toml(
            Utf8Path::new("missing-default-track.toml"),
            BackingRecipeKind::Redis,
            &missing_default_track,
        ),
        BackingRecipe::from_toml(
            Utf8Path::new("empty-payload-paths.toml"),
            BackingRecipeKind::Redis,
            &empty_payload_paths,
        ),
        BackingRecipe::from_toml(
            Utf8Path::new("unsafe-payload-path.toml"),
            BackingRecipeKind::Redis,
            &unsafe_payload_path,
        ),
        BackingRecipe::from_toml(
            Utf8Path::new("insecure-source-url.toml"),
            BackingRecipeKind::Redis,
            &insecure_source_url,
        ),
        BackingRecipe::from_toml(
            Utf8Path::new("bad-source-sha256.toml"),
            BackingRecipeKind::Redis,
            &bad_source_sha256,
        ),
    ));

    Ok(())
}

#[test]
fn recipe_metadata_rejects_invalid_shapes() -> Result<()> {
    let duplicate_track = VALID_PHP_TOML.replace("name = \"8.3\"", "name = \"8.4\"");
    let missing_extension = VALID_PHP_TOML.replace("\"phar\", \"posix\"", "\"posix\"");
    let bad_checksum = VALID_COMPOSER_TOML.replace(
        "345b9c6a98da5c30dcbd4b0d99fc8710bf0ae98a3898eea18f7b2ad9dec93f06",
        "bad",
    );

    assert_debug_snapshot!((
        PhpRecipe::from_toml(Utf8Path::new("duplicate.toml"), &duplicate_track),
        PhpRecipe::from_toml(Utf8Path::new("missing-extension.toml"), &missing_extension),
        ComposerRecipe::from_toml(Utf8Path::new("bad-composer.toml"), &bad_checksum),
    ));
    Ok(())
}

#[test]
fn recipe_metadata_rejects_strict_php_metadata() -> Result<()> {
    let invalid_deployment_target = VALID_PHP_TOML.replace(
        "deployment_target = \"13.0\"",
        "deployment_target = \"14.0\"",
    );
    let php_version_without_patch =
        VALID_PHP_TOML.replace("php_version = \"8.4.20\"", "php_version = \"8.4\"");
    let unexpected_expected_extension =
        VALID_PHP_TOML.replace("\"zlib\"]", "\"zlib\", \"xdebug\"]");
    let empty_license_files =
        VALID_PHP_TOML.replace("license_files = [\"LICENSE\"]", "license_files = []");
    let unsafe_license_file = VALID_PHP_TOML.replace(
        "license_files = [\"LICENSE\"]",
        "license_files = [\"../LICENSE\"]",
    );
    let unsafe_notice_file = VALID_PHP_TOML.replace(
        "notice_files = [\"NOTICE\"]",
        "notice_files = [\"../NOTICE\"]",
    );

    assert_debug_snapshot!((
        PhpRecipe::from_toml(
            Utf8Path::new("invalid-deployment-target.toml"),
            &invalid_deployment_target,
        ),
        PhpRecipe::from_toml(
            Utf8Path::new("php-version-without-patch.toml"),
            &php_version_without_patch,
        ),
        PhpRecipe::from_toml(
            Utf8Path::new("unexpected-expected-extension.toml"),
            &unexpected_expected_extension,
        ),
        PhpRecipe::from_toml(
            Utf8Path::new("empty-license-files.toml"),
            &empty_license_files
        ),
        PhpRecipe::from_toml(
            Utf8Path::new("unsafe-license-file.toml"),
            &unsafe_license_file
        ),
        PhpRecipe::from_toml(
            Utf8Path::new("unsafe-notice-file.toml"),
            &unsafe_notice_file
        ),
    ));
    Ok(())
}

#[test]
fn recipe_metadata_rejects_unknown_fields() -> Result<()> {
    let unknown_root = VALID_PHP_TOML.replacen(
        "[recipe]",
        "unknown_recipe_metadata = \"ignored\"\n\n[recipe]",
        1,
    );
    let unknown_php_settings =
        VALID_PHP_TOML.replacen("[php]", "[php]\nunknown_php_metadata = \"ignored\"", 1);
    let unknown_composer_track = VALID_COMPOSER_TOML.replacen(
        "source_sha256 = \"345b9c6a98da5c30dcbd4b0d99fc8710bf0ae98a3898eea18f7b2ad9dec93f06\"",
        "source_sha256 = \"345b9c6a98da5c30dcbd4b0d99fc8710bf0ae98a3898eea18f7b2ad9dec93f06\"\nunknown_track_metadata = \"ignored\"",
        1,
    );
    let unknown_backing_artifact = VALID_REDIS_TOML.replacen(
        "[artifact]",
        "[artifact]\nunknown_artifact_metadata = \"ignored\"",
        1,
    );

    assert_invalid_recipe_metadata(PhpRecipe::from_toml(
        Utf8Path::new("unknown-root.toml"),
        &unknown_root,
    ));
    assert_invalid_recipe_metadata(PhpRecipe::from_toml(
        Utf8Path::new("unknown-php.toml"),
        &unknown_php_settings,
    ));
    assert_invalid_recipe_metadata(ComposerRecipe::from_toml(
        Utf8Path::new("unknown-composer-track.toml"),
        &unknown_composer_track,
    ));
    assert_invalid_recipe_metadata(BackingRecipe::from_toml(
        Utf8Path::new("unknown-backing-artifact.toml"),
        BackingRecipeKind::Redis,
        &unknown_backing_artifact,
    ));

    Ok(())
}

#[test]
fn recipe_metadata_rejects_unsupported_license_notice_files() -> Result<()> {
    let extra_license_file = VALID_PHP_TOML.replace(
        "license_files = [\"LICENSE\"]",
        "license_files = [\"LICENSE\", \"COPYING\"]",
    );
    let alternate_notice_file = VALID_COMPOSER_TOML.replace(
        "notice_files = [\"NOTICE\"]",
        "notice_files = [\"THIRD_PARTY_NOTICES\"]",
    );

    assert_invalid_recipe_metadata(PhpRecipe::from_toml(
        Utf8Path::new("extra-license.toml"),
        &extra_license_file,
    ));
    assert_invalid_recipe_metadata(ComposerRecipe::from_toml(
        Utf8Path::new("alternate-notice.toml"),
        &alternate_notice_file,
    ));

    Ok(())
}

#[test]
fn print_composer_recipe_env() -> Result<()> {
    let tempdir = tempdir()?;
    let composer = tempdir.path().join("composer.toml");
    write_file(&composer, VALID_COMPOSER_TOML)?;

    let env = composer_recipe_env(&composer, "composer", "2", "any")?;

    assert_snapshot!(env);
    Ok(())
}

#[test]
fn print_recipe_env_php() -> Result<()> {
    let tempdir = tempdir()?;
    let php = tempdir.path().join("tracks.toml");
    write_file(&php, VALID_PHP_TOML)?;

    let env = php_recipe_env(&php, "php", "8.4", "darwin-arm64")?;

    assert!(!env.lines().any(|line| line.starts_with("PV_PHP_SOURCE_")));
    assert_snapshot!(env);
    Ok(())
}

#[test]
fn print_recipe_env_frankenphp() -> Result<()> {
    let tempdir = tempdir()?;
    let php = tempdir.path().join("tracks.toml");
    write_file(&php, VALID_PHP_TOML)?;

    let env = php_recipe_env(&php, "frankenphp", "8.4", "darwin-arm64")?;
    let php_source_env = env
        .lines()
        .filter(|line| line.starts_with("PV_PHP_SOURCE_"))
        .collect::<Vec<_>>();

    assert_eq!(
        php_source_env,
        vec![
            "PV_PHP_SOURCE_URL='https://www.php.net/distributions/php-8.4.20.tar.gz'",
            "PV_PHP_SOURCE_SHA256='a2def5d534d57c6a0236f2265de7537608af871900a4f7955eff463e9e38247d'",
        ]
    );
    assert_snapshot!(env);
    Ok(())
}

#[test]
fn print_recipe_env_redis() -> Result<()> {
    let tempdir = tempdir()?;
    let redis = tempdir.path().join("recipe.toml");
    write_file(&redis, VALID_REDIS_TOML)?;

    let env = backing_recipe_env(
        &redis,
        BackingRecipeKind::Redis,
        "redis",
        "8.2",
        "darwin-arm64",
    )?;

    assert_snapshot!(env);
    Ok(())
}

#[test]
fn print_backing_recipe_env_mailpit() -> Result<()> {
    let tempdir = tempdir()?;
    let mailpit = tempdir.path().join("recipe.toml");
    write_file(&mailpit, VALID_MAILPIT_TOML)?;

    let env = backing_recipe_env(
        &mailpit,
        BackingRecipeKind::Mailpit,
        "mailpit",
        "1",
        "darwin-arm64",
    )?;

    assert_snapshot!(env);
    Ok(())
}

#[test]
fn print_composer_recipe_env_quotes_shell_values() -> Result<()> {
    let tempdir = tempdir()?;
    let composer = tempdir.path().join("composer.toml");
    let source_url_with_query = VALID_COMPOSER_TOML.replace(
        "https://getcomposer.org/download/2.10.1/composer.phar",
        "https://getcomposer.org/download/2.10.1/composer.phar?mirror=primary&fallback=1",
    );
    write_file(&composer, &source_url_with_query)?;

    let env = composer_recipe_env(&composer, "composer", "2", "any")?;

    assert_snapshot!(env);
    Ok(())
}

fn assert_invalid_recipe_metadata(result: pv_release::Result<impl std::fmt::Debug>) {
    assert!(
        matches!(result, Err(ReleaseError::InvalidRecipeMetadata { .. })),
        "recipe metadata should be rejected, got {result:?}"
    );
}

fn php_summary(recipe: &PhpRecipe) -> Vec<(String, String, String)> {
    recipe
        .tracks()
        .iter()
        .map(|track| {
            (
                track.name().as_str().to_string(),
                track.php_version().to_string(),
                track.php_source_url().to_string(),
            )
        })
        .collect()
}

fn assert_default_track(
    defaults: &ManifestDefaults,
    resource_name: &str,
    expected_track: &str,
) -> Result<()> {
    let resource_name = ResourceName::new(resource_name)?;
    assert_eq!(
        defaults
            .default_track_for(&resource_name)
            .map(|track| track.as_str()),
        Some(expected_track)
    );
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests write local recipe metadata"
)]
fn write_file(path: &Utf8Path, content: &str) -> Result<()> {
    std::fs::write(path, content)?;
    Ok(())
}

const VALID_PHP_TOML: &str = r#"
[recipe]
resources = ["php", "frankenphp"]
default_track = "8.4"
platforms = ["darwin-arm64", "darwin-amd64"]
minimum_pv_version = "0.1.0"
pv_build_revision = "pv1"
license_files = ["LICENSE"]
notice_files = ["NOTICE"]

[php]
deployment_target = "13.0"
build_extensions = ["bcmath", "curl", "intl", "mbstring", "openssl", "pcntl", "pdo_mysql", "pdo_pgsql", "pdo_sqlite", "redis", "sodium", "zip"]
expected_extensions = ["bcmath", "ctype", "curl", "dom", "fileinfo", "filter", "hash", "iconv", "intl", "json", "libxml", "mbstring", "openssl", "pcntl", "pcre", "pdo", "pdo_mysql", "pdo_pgsql", "pdo_sqlite", "phar", "posix", "redis", "session", "simplexml", "sodium", "sqlite3", "tokenizer", "xml", "xmlreader", "xmlwriter", "zip", "zlib"]

[frankenphp]
version = "1.12.3"
source_url = "https://github.com/php/frankenphp/archive/refs/tags/v1.12.3.tar.gz"
source_sha256 = "2996fb95bbdf8410847fdcd59df04cd2e297568f6472ebe488af5fb5f3c79363"

[[tracks]]
name = "8.3"
php_version = "8.3.31"
php_source_url = "https://www.php.net/distributions/php-8.3.31.tar.gz"
php_source_sha256 = "4e7baaf0a690e954a20e7ced3dd633ce8cb8094e2b6b612a55e703ecbbdcbf4f"

[[tracks]]
name = "8.4"
php_version = "8.4.20"
php_source_url = "https://www.php.net/distributions/php-8.4.20.tar.gz"
php_source_sha256 = "a2def5d534d57c6a0236f2265de7537608af871900a4f7955eff463e9e38247d"
"#;

const VALID_COMPOSER_TOML: &str = r#"
[recipe]
resources = ["composer"]
default_track = "2"
platforms = ["any"]
pv_build_revision = "pv1"
minimum_pv_version = "0.1.0"
license_files = ["LICENSE"]
notice_files = ["NOTICE"]

[[tracks]]
name = "2"
upstream_version = "2.10.1"
source_url = "https://getcomposer.org/download/2.10.1/composer.phar"
source_sha256 = "345b9c6a98da5c30dcbd4b0d99fc8710bf0ae98a3898eea18f7b2ad9dec93f06"
"#;

const VALID_REDIS_TOML: &str = r#"
[recipe]
resources = ["redis"]
default_track = "8.2"
platforms = ["darwin-arm64", "darwin-amd64"]
minimum_pv_version = "0.1.0"
pv_build_revision = "pv1"
license_files = ["LICENSE"]
notice_files = ["NOTICE"]

[artifact]
payload_paths = ["bin/redis-server", "bin/redis-cli"]

[[tracks]]
name = "8.2"
upstream_version = "8.2.1"
source_url = "https://download.redis.io/releases/redis-8.2.1.tar.gz"
source_sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
"#;

const VALID_MAILPIT_TOML: &str = r#"
[recipe]
resources = ["mailpit"]
default_track = "1"
platforms = ["darwin-arm64", "darwin-amd64"]
minimum_pv_version = "0.1.0"
pv_build_revision = "pv1"
license_files = ["LICENSE"]
notice_files = ["NOTICE"]

[artifact]
payload_paths = ["bin/mailpit"]

[[tracks]]
name = "1"
upstream_version = "1.30.1"

[[tracks.sources]]
platform = "darwin-arm64"
source_url = "https://github.com/axllent/mailpit/releases/download/v1.30.1/mailpit-darwin-arm64.tar.gz"
source_sha256 = "1cce392a19a6093fcc859aeb87d9999671fed9a0a7a1c227a7f7df6307741be2"

[[tracks.sources]]
platform = "darwin-amd64"
source_url = "https://github.com/axllent/mailpit/releases/download/v1.30.1/mailpit-darwin-amd64.tar.gz"
source_sha256 = "a2d7df1b34967e51604b42136260789b75a45233a03c9dc773b98024e1390974"
"#;
