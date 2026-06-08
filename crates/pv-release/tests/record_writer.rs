use anyhow::Result;
use camino::Utf8Path;
use camino_tempfile::tempdir;
use insta::assert_snapshot;
use pv_release::record::ReleaseRecord;
use pv_release::record_writer::{
    SourceInputRequest, WriteReleaseRecordRequest, write_release_record,
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
