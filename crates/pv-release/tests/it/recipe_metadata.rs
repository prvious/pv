use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::{assert_debug_snapshot, assert_snapshot};
use pv_release::defaults::ManifestDefaults;
use pv_release::recipe::{
    BackingRecipe, BackingRecipeKind, ComposerRecipe, PhpRecipe, backing_recipe_env,
    composer_recipe_env, php_recipe_env,
};
use resources::BUILT_IN_PHP_EXTENSIONS;
use std::collections::BTreeSet;

#[test]
fn committed_recipe_metadata_parses() -> Result<()> {
    let artifacts = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../../release/artifacts");
    let php = PhpRecipe::load(&artifacts.join("recipes/php/tracks.toml"))?;
    ComposerRecipe::load(&artifacts.join("recipes/composer/composer.toml"))?;
    for (resource, kind) in [
        ("redis", BackingRecipeKind::Redis),
        ("mysql", BackingRecipeKind::Mysql),
        ("postgres", BackingRecipeKind::Postgres),
        ("mailpit", BackingRecipeKind::Mailpit),
        ("caddy", BackingRecipeKind::Caddy),
        ("rustfs", BackingRecipeKind::Rustfs),
    ] {
        BackingRecipe::load(
            &artifacts.join(format!("recipes/{resource}/recipe.toml")),
            kind,
        )?;
    }
    ManifestDefaults::load(&artifacts.join("default-tracks.toml"))?;

    assert_php_staticphp_build_extensions(&php);

    Ok(())
}

#[test]
fn built_in_php_extensions_match_the_committed_recipe() -> Result<()> {
    let workspace_root = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let php = PhpRecipe::load(&workspace_root.join("release/artifacts/recipes/php/tracks.toml"))?;

    // PV accepts these in Project config without a diagnostic, so each build must load them.
    assert_eq!(php.expected_extensions(), BUILT_IN_PHP_EXTENSIONS);

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
    let missing_platform_source = VALID_MAILPIT_TOML.replace(
        r#"
[[tracks.sources]]
platform = "darwin-amd64"
source_url = "https://github.com/axllent/mailpit/releases/download/v1.30.1/mailpit-darwin-amd64.tar.gz"
source_sha256 = "a2d7df1b34967e51604b42136260789b75a45233a03c9dc773b98024e1390974"
"#,
        "",
    );
    let duplicate_platform_source =
        VALID_MAILPIT_TOML.replace("platform = \"darwin-amd64\"", "platform = \"darwin-arm64\"");
    let unsupported_platform_source =
        VALID_MAILPIT_TOML.replace("platform = \"darwin-amd64\"", "platform = \"any\"");
    let mixed_common_and_platform_sources = VALID_MAILPIT_TOML.replace(
        "upstream_version = \"1.30.1\"\n\n[[tracks.sources]]",
        "upstream_version = \"1.30.1\"\nsource_url = \"https://github.com/axllent/mailpit/releases/download/v1.30.1/mailpit-darwin-arm64.tar.gz\"\nsource_sha256 = \"1cce392a19a6093fcc859aeb87d9999671fed9a0a7a1c227a7f7df6307741be2\"\n\n[[tracks.sources]]",
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
        BackingRecipe::from_toml(
            Utf8Path::new("missing-platform-source.toml"),
            BackingRecipeKind::Mailpit,
            &missing_platform_source,
        ),
        BackingRecipe::from_toml(
            Utf8Path::new("duplicate-platform-source.toml"),
            BackingRecipeKind::Mailpit,
            &duplicate_platform_source,
        ),
        BackingRecipe::from_toml(
            Utf8Path::new("unsupported-platform-source.toml"),
            BackingRecipeKind::Mailpit,
            &unsupported_platform_source,
        ),
        BackingRecipe::from_toml(
            Utf8Path::new("mixed-common-and-platform-sources.toml"),
            BackingRecipeKind::Mailpit,
            &mixed_common_and_platform_sources,
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
    let extra_license_file = VALID_PHP_TOML.replace(
        "license_files = [\"LICENSE\"]",
        "license_files = [\"LICENSE\", \"COPYING\"]",
    );
    let alternate_notice_file = VALID_COMPOSER_TOML.replace(
        "notice_files = [\"NOTICE\"]",
        "notice_files = [\"THIRD_PARTY_NOTICES\"]",
    );

    assert_debug_snapshot!((
        PhpRecipe::from_toml(Utf8Path::new("duplicate.toml"), &duplicate_track),
        PhpRecipe::from_toml(Utf8Path::new("missing-extension.toml"), &missing_extension),
        ComposerRecipe::from_toml(Utf8Path::new("bad-composer.toml"), &bad_checksum),
        PhpRecipe::from_toml(Utf8Path::new("extra-license.toml"), &extra_license_file),
        ComposerRecipe::from_toml(
            Utf8Path::new("alternate-notice.toml"),
            &alternate_notice_file
        ),
    ));
    Ok(())
}

#[test]
fn recipe_metadata_rejects_strict_php_metadata() -> Result<()> {
    let invalid_deployment_target = VALID_PHP_TOML.replacen(
        "deployment_target = \"13.0\"",
        "deployment_target = \"14.0\"",
        1,
    );
    let php_version_without_patch =
        VALID_PHP_TOML.replacen("php_version = \"8.4.20\"", "php_version = \"8.4\"", 1);
    let unexpected_expected_extension =
        VALID_PHP_TOML.replacen("\"zlib\"]", "\"zlib\", \"xdebug\"]", 1);
    let duplicate_default_extension = VALID_PHP_TOML.replacen(
        "\"bcmath\", \"curl\"",
        "\"bcmath\", \"bcmath\", \"curl\"",
        1,
    );
    let invalid_default_extension =
        VALID_PHP_TOML.replacen("\"bcmath\", \"curl\"", "\"bad-extension\", \"curl\"", 1);
    let overlapping_optional_extension =
        VALID_PHP_TOML.replacen("\"redis\", \"sqlsrv\"", "\"bcmath\", \"sqlsrv\"", 1);
    let empty_license_files =
        VALID_PHP_TOML.replacen("license_files = [\"LICENSE\"]", "license_files = []", 1);
    let unsafe_license_file = VALID_PHP_TOML.replacen(
        "license_files = [\"LICENSE\"]",
        "license_files = [\"../LICENSE\"]",
        1,
    );
    let unsafe_notice_file = VALID_PHP_TOML.replacen(
        "notice_files = [\"NOTICE\"]",
        "notice_files = [\"../NOTICE\"]",
        1,
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
            Utf8Path::new("duplicate-default-extension.toml"),
            &duplicate_default_extension,
        ),
        PhpRecipe::from_toml(
            Utf8Path::new("invalid-default-extension.toml"),
            &invalid_default_extension,
        ),
        PhpRecipe::from_toml(
            Utf8Path::new("overlapping-optional-extension.toml"),
            &overlapping_optional_extension,
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

fn assert_php_staticphp_build_extensions(php: &PhpRecipe) {
    let actual = php
        .default_extensions()
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let required = [
        "bcmath",
        "ctype",
        "curl",
        "dom",
        "exif",
        "fileinfo",
        "filter",
        "ftp",
        "gd",
        "iconv",
        "intl",
        "libxml",
        "mbstring",
        "openssl",
        "pcntl",
        "pdo",
        "pdo_mysql",
        "pdo_pgsql",
        "pdo_sqlite",
        "phar",
        "posix",
        "session",
        "simplexml",
        "sockets",
        "sodium",
        "sqlite3",
        "tokenizer",
        "xml",
        "xmlreader",
        "xmlwriter",
        "zip",
        "zlib",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    let missing = required.difference(&actual).collect::<Vec<_>>();

    assert!(
        missing.is_empty(),
        "PHP build_extensions missing StaticPHP inputs for PV required extensions: {missing:?}"
    );
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
pv_build_revision = "pv2"
license_files = ["LICENSE"]
notice_files = ["NOTICE"]

[php]
deployment_target = "13.0"
rar_source_revision = "9c8fcd9ebc9feaf36f945d6d7407fdcd57b7136f"
rar_source_url = "https://github.com/static-php/php-rar/archive/9c8fcd9ebc9feaf36f945d6d7407fdcd57b7136f.tar.gz"
rar_source_sha256 = "5198de458e029950fb3462d83262345f0a3862c4e39125b3abca5d5c9b0b166b"
default_extensions = ["bcmath", "curl", "exif", "ftp", "gd", "intl", "mbstring", "openssl", "pcntl", "pdo_mysql", "pdo_pgsql", "pdo_sqlite", "sockets", "sodium", "zip"]
optional_extensions = ["redis", "sqlsrv", "pdo_sqlsrv", "xdebug", "apcu", "pcov", "imagick", "mongodb", "yaml", "rar"]
expected_extensions = ["bcmath", "ctype", "curl", "dom", "exif", "fileinfo", "filter", "ftp", "gd", "hash", "iconv", "intl", "json", "libxml", "mbstring", "openssl", "pcntl", "pcre", "pdo", "pdo_mysql", "pdo_pgsql", "pdo_sqlite", "phar", "posix", "session", "simplexml", "sockets", "sodium", "sqlite3", "tokenizer", "xml", "xmlreader", "xmlwriter", "zip", "zlib"]

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
