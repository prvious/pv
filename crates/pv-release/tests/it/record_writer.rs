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
fn release_record_writer_serializes_record_metadata() -> Result<()> {
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
        source_url: "https://www.php.net/distributions/php-8.4.20.tar.gz?mirror=primary&fallback=1"
            .to_string(),
        source_sha256: "a2def5d534d57c6a0236f2265de7537608af871900a4f7955eff463e9e38247d"
            .to_string(),
        recipe: "release/artifacts/recipes/php/build.sh".to_string(),
        pv_commit: "0123456789abcdef0123456789abcdef01234567".to_string(),
        build_run_id: "run\"with\\escaping".to_string(),
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
        source_inputs: vec![SourceInputRequest {
            name: "php".to_string(),
            source_url:
                "https://www.php.net/distributions/php-8.4.20.tar.gz?mirror=primary&fallback=1"
                    .to_string(),
            source_sha256: "a2def5d534d57c6a0236f2265de7537608af871900a4f7955eff463e9e38247d"
                .to_string(),
        }],
    })?;

    let json = read_to_string(&record)?;
    let parsed = ReleaseRecord::from_json(&record, &json)?;

    assert_eq!(parsed.provenance().build_run_id(), "run\"with\\escaping");
    assert_eq!(parsed.provenance().source_inputs().len(), 1);
    assert_eq!(parsed.php_extensions().len(), 2);
    assert_snapshot!(json);
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
