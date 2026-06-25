use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::assert_snapshot;
use pv_release::record::ReleaseRecord;
use pv_release::record_writer::{
    PhpExtensionRecordRequest, SourceInputRequest, WriteReleaseRecordRequest, write_release_record,
};
use std::fs;

#[test]
fn release_record_writer_serializes_metadata_and_source_inputs() -> Result<()> {
    let tempdir = tempdir()?;
    let archive = tempdir.path().join("composer-2.10.1-pv1-any.tar.gz");
    let record = tempdir
        .path()
        .join("records/composer/2/2.10.1-pv1/any/composer-2.10.1-pv1-any.json");
    write_file(&archive, b"artifact bytes")?;

    write_release_record(&WriteReleaseRecordRequest {
        record: record.clone(),
        archive,
        resource: "composer".to_string(),
        track: "2".to_string(),
        upstream_version: "2.10.1".to_string(),
        pv_build_revision: "pv1".to_string(),
        platform: "any".to_string(),
        object_key: "resources/composer/2/2.10.1-pv1/any/composer-2.10.1-pv1-any.tar.gz"
            .to_string(),
        source_url:
            "https://getcomposer.org/download/2.10.1/composer.phar?mirror=primary&fallback=1"
                .to_string(),
        source_sha256: "345b9c6a98da5c30dcbd4b0d99fc8710bf0ae98a3898eea18f7b2ad9dec93f06"
            .to_string(),
        recipe: "release/artifacts/recipes/composer/build.sh".to_string(),
        pv_commit: "0123456789abcdef0123456789abcdef01234567".to_string(),
        build_run_id: "run\"with\\escaping".to_string(),
        minimum_pv_version: "0.1.0".to_string(),
        published_at: "2026-06-08T12:00:00Z".to_string(),
        license_files: vec!["LICENSE".to_string()],
        notice_files: vec!["NOTICE".to_string()],
        php_extensions: Vec::new(),
        source_inputs: vec![SourceInputRequest {
            name: "composer".to_string(),
            source_url:
                "https://getcomposer.org/download/2.10.1/composer.phar?mirror=primary&fallback=1"
                    .to_string(),
            source_sha256: "345b9c6a98da5c30dcbd4b0d99fc8710bf0ae98a3898eea18f7b2ad9dec93f06"
                .to_string(),
        }],
    })?;

    let json = read_to_string(&record)?;
    let parsed = ReleaseRecord::from_json(&record, &json)?;

    assert_eq!(parsed.provenance().build_run_id(), "run\"with\\escaping");
    assert_eq!(parsed.provenance().source_inputs().len(), 1);
    assert_snapshot!(json);
    Ok(())
}

#[test]
fn release_record_writer_serializes_php_extension_metadata() -> Result<()> {
    let tempdir = tempdir()?;
    let archive = tempdir.path().join("php-8.4.20-pv1-darwin-arm64.tar.gz");
    let record = tempdir
        .path()
        .join("records/php/8.4/8.4.20-pv1/darwin-arm64/php-8.4.20-pv1-darwin-arm64.json");
    write_file(&archive, b"artifact bytes")?;

    write_release_record(&WriteReleaseRecordRequest {
        record: record.clone(),
        archive,
        resource: "php".to_string(),
        track: "8.4".to_string(),
        upstream_version: "8.4.20".to_string(),
        pv_build_revision: "pv1".to_string(),
        platform: "darwin-arm64".to_string(),
        object_key: "resources/php/8.4/8.4.20-pv1/darwin-arm64/php-8.4.20-pv1-darwin-arm64.tar.gz"
            .to_string(),
        source_url: "https://www.php.net/distributions/php-8.4.20.tar.gz".to_string(),
        source_sha256: "a2def5d534d57c6a0236f2265de7537608af871900a4f7955eff463e9e38247d"
            .to_string(),
        recipe: "release/artifacts/recipes/php/build.sh".to_string(),
        pv_commit: "0123456789abcdef0123456789abcdef01234567".to_string(),
        build_run_id: "local-test".to_string(),
        minimum_pv_version: "0.1.0".to_string(),
        published_at: "2026-06-08T12:00:00Z".to_string(),
        license_files: vec!["LICENSE".to_string()],
        notice_files: vec!["NOTICE".to_string()],
        php_extensions: vec![
            PhpExtensionRecordRequest {
                name: "redis".to_string(),
                load_kind: "extension".to_string(),
                path: "lib/php/extensions/redis.so".to_string(),
            },
            PhpExtensionRecordRequest {
                name: "xdebug".to_string(),
                load_kind: "zend_extension".to_string(),
                path: "lib/php/extensions/xdebug.so".to_string(),
            },
        ],
        source_inputs: Vec::new(),
    })?;

    let json = read_to_string(&record)?;
    let parsed = ReleaseRecord::from_json(&record, &json)?;

    assert_eq!(parsed.php_extensions().len(), 2);
    assert_snapshot!(json);
    Ok(())
}

#[test]
fn release_record_writer_serializes_custom_legal_files() -> Result<()> {
    let tempdir = tempdir()?;
    let archive = tempdir.path().join("redis-8.2.7-pv1-darwin-arm64.tar.gz");
    let record = tempdir
        .path()
        .join("records/redis/8.2/8.2.7-pv1/darwin-arm64/redis-8.2.7-pv1-darwin-arm64.json");
    write_file(&archive, b"artifact bytes")?;

    write_release_record(&WriteReleaseRecordRequest {
        record: record.clone(),
        archive,
        resource: "redis".to_string(),
        track: "8.2".to_string(),
        upstream_version: "8.2.7".to_string(),
        pv_build_revision: "pv1".to_string(),
        platform: "darwin-arm64".to_string(),
        object_key:
            "resources/redis/8.2/8.2.7-pv1/darwin-arm64/redis-8.2.7-pv1-darwin-arm64.tar.gz"
                .to_string(),
        source_url: "https://download.redis.io/releases/redis-8.2.7.tar.gz".to_string(),
        source_sha256: "afaae66030c193b06720a714ba7a558136b82689027536e0e24f53908c18cbe9"
            .to_string(),
        recipe: "release/artifacts/recipes/redis/build.sh".to_string(),
        pv_commit: "0123456789abcdef0123456789abcdef01234567".to_string(),
        build_run_id: "local-test".to_string(),
        minimum_pv_version: "0.1.0".to_string(),
        published_at: "2026-06-08T12:00:00Z".to_string(),
        license_files: vec!["LICENSE".to_string()],
        notice_files: vec!["NOTICE".to_string(), "THIRD-PARTY-NOTICES".to_string()],
        php_extensions: Vec::new(),
        source_inputs: Vec::new(),
    })?;

    let json = read_to_string(&record)?;
    let parsed = ReleaseRecord::from_json(&record, &json)?;

    assert_eq!(parsed.license_files(), ["LICENSE"]);
    assert_eq!(parsed.notice_files(), ["NOTICE", "THIRD-PARTY-NOTICES"]);
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests write local fixture archives"
)]
fn write_file(path: &Utf8Path, content: &[u8]) -> Result<()> {
    fs::write(path, content)?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "release tooling tests read generated records"
)]
fn read_to_string(path: &Utf8Path) -> Result<String> {
    Ok(fs::read_to_string(path)?)
}
