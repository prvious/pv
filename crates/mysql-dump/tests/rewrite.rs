use mysql_dump::{Patch, RewriteError, write_patched};

#[test]
fn patches_only_verified_identifier_bytes() -> Result<(), Box<dyn std::error::Error>> {
    let source = b"USE `admin`; INSERT INTO `admin`.`users` VALUES ('admin');";
    let mut output = Vec::new();
    write_patched(
        source.as_slice(),
        &mut output,
        [
            Patch {
                range: 4..11,
                expected: b"`admin`".to_vec(),
                replacement: b"`google_admin`".to_vec(),
            },
            Patch {
                range: 25..32,
                expected: b"`admin`".to_vec(),
                replacement: b"`google_admin`".to_vec(),
            },
        ],
    )?;
    assert_eq!(
        output,
        b"USE `google_admin`; INSERT INTO `google_admin`.`users` VALUES ('admin');"
    );
    Ok(())
}

#[test]
fn changed_source_is_rejected_before_success() {
    let result = write_patched(
        b"USE `other`;".as_slice(),
        Vec::new(),
        [Patch {
            range: 4..11,
            expected: b"`admin`".to_vec(),
            replacement: b"`google_admin`".to_vec(),
        }],
    );
    assert!(matches!(
        result,
        Err(RewriteError::SourceMismatch { start: 4 })
    ));
}

#[test]
fn overlapping_patches_are_rejected() {
    let result = write_patched(
        b"USE `admin`;".as_slice(),
        Vec::new(),
        [
            Patch {
                range: 4..11,
                expected: b"`admin`".to_vec(),
                replacement: b"`google_admin`".to_vec(),
            },
            Patch {
                range: 5..10,
                expected: b"admin".to_vec(),
                replacement: b"x".to_vec(),
            },
        ],
    );
    assert!(matches!(
        result,
        Err(RewriteError::InvalidRange { start: 5 })
    ));
}
