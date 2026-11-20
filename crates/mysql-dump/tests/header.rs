use mysql_dump::{HeaderError, source_database_header};

#[test]
fn plain_dump_header_selects_its_source_database() -> Result<(), Box<dyn std::error::Error>> {
    for source in [
        include_bytes!("../../../it/fixtures/mysql-import/8.0.46/plain.sql").as_slice(),
        include_bytes!("../../../it/fixtures/mysql-import/8.4.9/plain.sql").as_slice(),
        include_bytes!("../../../it/fixtures/mysql-import/9.7.0/plain.sql").as_slice(),
        include_bytes!("../../../it/fixtures/mysql-import/phpmyadmin.sql").as_slice(),
    ] {
        assert_eq!(source_database_header(source)?, Some("admin".to_owned()));
    }
    // A multi-database dump can still name the first database in its header.
    // Its executable CREATE/USE sections take precedence during full preflight.
    assert_eq!(
        source_database_header(
            include_bytes!("../../../it/fixtures/mysql-import/8.4.9/databases.sql").as_slice()
        )?,
        Some("admin".to_owned())
    );
    Ok(())
}

#[test]
fn conflicting_headers_fail_and_sql_body_headers_are_ignored() {
    assert!(matches!(
        source_database_header(b"-- Database: admin\n-- Database: analytics\n".as_slice()),
        Err(HeaderError::ConflictingDatabases)
    ));
    assert_eq!(
        source_database_header(b"CREATE TABLE t (id int);\n-- Database: admin\n".as_slice()).ok(),
        Some(None)
    );
    assert_eq!(
        source_database_header(
            b"-- Database: admin\nINSERT INTO t VALUES (X'FF');\xFF\n".as_slice()
        )
        .ok(),
        Some(Some("admin".to_owned()))
    );
    assert_eq!(
        source_database_header(b"-- Database: `Mixed-Name`\r\n".as_slice()).ok(),
        Some(Some("Mixed-Name".to_owned()))
    );
    assert_eq!(
        source_database_header(b"\xef\xbb\xbf-- Database: admin\n".as_slice()).ok(),
        Some(Some("admin".to_owned()))
    );
    let mut beyond_limit = vec![b'\n'; 64 * 1024];
    beyond_limit.extend_from_slice(b"-- Database: admin\n");
    assert_eq!(
        source_database_header(beyond_limit.as_slice()).ok(),
        Some(None)
    );
}
