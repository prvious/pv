use mysql_dump::{Edit, Patch, RewriteError, write_patched, write_transformed};

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

#[test]
fn whole_frame_omission_streams_past_patch_size_limit() -> Result<(), Box<dyn std::error::Error>> {
    let body = "x".repeat(16_000);
    let source = format!("before;{body}after;");
    let mut output = Vec::new();
    write_transformed(
        source.as_bytes(),
        &mut output,
        [Edit::Skip(7..(7 + body.len() as u64))],
    )?;
    assert_eq!(output, b"before;after;");
    Ok(())
}

#[test]
fn overlapping_omission_and_patch_are_rejected() {
    let result = write_transformed(
        b"USE `admin`;".as_slice(),
        Vec::new(),
        [
            Edit::Skip(0..4),
            Edit::Patch(Patch {
                range: 3..10,
                expected: b" `admin".to_vec(),
                replacement: b"x".to_vec(),
            }),
        ],
    );
    assert!(matches!(
        result,
        Err(RewriteError::InvalidRange { start: 3 })
    ));
}
