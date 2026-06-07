use anyhow::Result;
use camino::Utf8Path;
use insta::assert_debug_snapshot;
use pv_release::recipe::{ComposerRecipe, PhpRecipe};

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
fn recipe_metadata_rejects_invalid_shapes() -> Result<()> {
    let duplicate_track = VALID_PHP_TOML.replace("name = \"8.3\"", "name = \"8.4\"");
    let missing_extension = VALID_PHP_TOML.replace("\"pdo_mysql\",", "");
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
build_extensions = ["bcmath", "curl", "intl", "mbstring", "openssl", "pdo_mysql", "pdo_pgsql", "pdo_sqlite", "redis", "zip"]
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
