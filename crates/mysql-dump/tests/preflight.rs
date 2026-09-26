use std::collections::BTreeMap;
use std::io::Cursor;

use mysql_dump::{PreflightError, RoutineName, preflight_dump, write_transformed};

#[test]
fn generated_multi_database_dump_is_fully_checked_before_rewrite()
-> Result<(), Box<dyn std::error::Error>> {
    let source = include_bytes!("../../../it/fixtures/mysql-import/8.4.9/databases.sql");
    let targets = BTreeMap::from([
        ("admin".to_owned(), "project_admin".to_owned()),
        ("analytics".to_owned(), "project_analytics".to_owned()),
        ("Mixed-Name".to_owned(), "project_mixed".to_owned()),
    ]);
    let plan = preflight_dump(
        Cursor::new(source),
        &mut Cursor::new(source),
        &targets,
        Some("admin"),
        [RoutineName {
            database: "admin".to_owned(),
            name: "dynamic_probe".to_owned(),
        }],
    )?;
    assert_eq!(plan.skipped_routines.len(), 1);
    let mut output = Vec::new();
    write_transformed(Cursor::new(source), &mut output, plan.edits)?;
    let output = String::from_utf8(output)?;
    assert!(output.contains("USE `project_admin`"));
    assert!(output.contains("USE `project_analytics`"));
    assert!(!output.contains("CREATE DATABASE `admin`"));
    assert!(!output.contains("CREATE DEFINER=`root`@`localhost`"));
    assert!(!output.contains("PROCEDURE `dynamic_probe`"));
    Ok(())
}

#[test]
fn system_sections_are_removed_and_user_section_is_imported()
-> Result<(), Box<dyn std::error::Error>> {
    let source = include_bytes!("../../../it/fixtures/mysql-import/system-sections.sql");
    let targets = BTreeMap::from([("admin".to_owned(), "project_admin".to_owned())]);
    let plan = preflight_dump(
        Cursor::new(source),
        &mut Cursor::new(source),
        &targets,
        None,
        [],
    )?;
    assert_eq!(plan.skipped_system_databases.len(), 3);
    let mut output = Vec::new();
    write_transformed(Cursor::new(source), &mut output, plan.edits)?;
    let output = String::from_utf8(output)?;
    assert!(!output.contains("CREATE TABLE should_skip"));
    assert!(output.contains("CREATE TABLE users"));
    assert!(output.contains("USE `project_admin`"));
    Ok(())
}

#[test]
fn phpmyadmin_dump_with_transactions_is_preflighted() -> Result<(), Box<dyn std::error::Error>> {
    let source = include_bytes!("../../../it/fixtures/mysql-import/phpmyadmin.sql");
    let targets = BTreeMap::from([("admin".to_owned(), "project_admin".to_owned())]);
    let plan = preflight_dump(
        Cursor::new(source),
        &mut Cursor::new(source),
        &targets,
        Some("admin"),
        [],
    )?;
    let mut output = Vec::new();
    write_transformed(Cursor::new(source), &mut output, plan.edits)?;
    let output = String::from_utf8(output)?;
    assert!(output.contains("CREATE TABLE `project_admin`.`users`"));
    assert!(output.contains("COMMIT"));
    Ok(())
}

#[test]
fn server_effects_and_unsafe_targets_fail_complete_preflight() {
    let source = include_bytes!("../../../it/fixtures/mysql-import/hostile.sql");
    let targets = BTreeMap::from([("admin".to_owned(), "project_admin".to_owned())]);
    assert!(matches!(
        preflight_dump(
            Cursor::new(source),
            &mut Cursor::new(source),
            &targets,
            None,
            []
        ),
        Err(PreflightError::Unsupported { .. })
    ));
    let invalid = BTreeMap::from([(
        "admin".to_owned(),
        "project_admin; DROP DATABASE mysql".to_owned(),
    )]);
    assert!(matches!(
        preflight_dump(
            Cursor::new(source),
            &mut Cursor::new(source),
            &invalid,
            None,
            []
        ),
        Err(PreflightError::InvalidTarget { .. })
    ));
    let collision = BTreeMap::from([
        ("admin".to_owned(), "project_admin".to_owned()),
        ("other".to_owned(), "PROJECT_ADMIN".to_owned()),
    ]);
    assert!(matches!(
        preflight_dump(
            Cursor::new(source),
            &mut Cursor::new(source),
            &collision,
            None,
            []
        ),
        Err(PreflightError::TargetCollision { .. })
    ));
}

#[test]
fn each_server_scoped_statement_fails_before_rewrite() {
    let targets = BTreeMap::from([("admin".to_owned(), "project_admin".to_owned())]);
    for source in [
        "CREATE USER intruder IDENTIFIED BY 'pw';",
        "GRANT ALL ON *.* TO intruder;",
        "SET GLOBAL general_log=ON;",
        "INSTALL PLUGIN audit SONAME 'audit.so';",
        "SELECT id INTO OUTFILE '/tmp/export' FROM users;",
        "LOAD DATA INFILE '/tmp/import' INTO TABLE users;",
        "ALTER TABLE users DISCARD TABLESPACE;",
        "ALTER TABLE users IMPORT TABLESPACE;",
        "ALTER TABLE users ENGINE=FEDERATED;",
        "CREATE TABLE users (id INT) DATA DIRECTORY='/tmp';",
        "CREATE TEMPORARY TABLE users (id INT) ENGINE=FEDERATED CONNECTION='mysql://remote/db/t';",
        "INSERT INTO users VALUES (1) /*!50000 ON DUPLICATE KEY UPDATE id=LOAD_FILE('/tmp/x') */;",
    ] {
        assert!(
            matches!(
                preflight_dump(
                    Cursor::new(source.as_bytes()),
                    &mut Cursor::new(source.as_bytes()),
                    &targets,
                    Some("admin"),
                    [],
                ),
                Err(PreflightError::Unsupported { .. })
            ),
            "{source}"
        );
    }
}

#[test]
fn drop_database_is_removed_without_dropping_its_target() -> Result<(), Box<dyn std::error::Error>>
{
    let source =
        b"CREATE DATABASE admin;\nUSE admin;\nCREATE TABLE users (id INT);\nDROP DATABASE admin;\n";
    let targets = BTreeMap::from([("admin".to_owned(), "project_admin".to_owned())]);
    let plan = preflight_dump(
        Cursor::new(source),
        &mut Cursor::new(source),
        &targets,
        None,
        [],
    )?;
    let mut output = Vec::new();
    write_transformed(Cursor::new(source), &mut output, plan.edits)?;
    let output = String::from_utf8(output)?;
    assert!(output.contains("CREATE TABLE users"));
    assert!(!output.contains("DROP DATABASE"));
    Ok(())
}

#[test]
fn trigger_drop_is_rewritten_to_its_mapped_database() -> Result<(), Box<dyn std::error::Error>> {
    let source = b"USE admin; DROP TRIGGER IF EXISTS `admin`.`audit_trigger`;";
    let targets = BTreeMap::from([("admin".to_owned(), "project_admin".to_owned())]);
    let plan = preflight_dump(
        Cursor::new(source),
        &mut Cursor::new(source),
        &targets,
        None,
        [],
    )?;
    let mut output = Vec::new();
    write_transformed(Cursor::new(source), &mut output, plan.edits)?;
    assert!(String::from_utf8(output)?.contains("DROP TRIGGER IF EXISTS `project_admin`"));
    Ok(())
}

#[test]
fn large_literal_insert_streams_and_rejects_expressions() -> Result<(), Box<dyn std::error::Error>>
{
    let targets = BTreeMap::from([("admin".to_owned(), "project_admin".to_owned())]);
    let source = format!(
        "INSERT INTO `admin`.`events` VALUES (1, '{}'),(2, NULL);",
        "a".repeat(9 * 1024 * 1024)
    );
    let plan = preflight_dump(
        Cursor::new(source.as_bytes()),
        &mut Cursor::new(source.as_bytes()),
        &targets,
        Some("admin"),
        [],
    )?;
    let mut output = Vec::new();
    write_transformed(Cursor::new(source.as_bytes()), &mut output, plan.edits)?;
    assert!(output.starts_with(b"INSERT INTO `project_admin`.`events` VALUES"));
    let unsafe_source = format!(
        "INSERT INTO `admin`.`events` VALUES (1, '{}'),(2, LOAD_FILE('/tmp/x'));",
        "a".repeat(9 * 1024 * 1024)
    );
    assert!(matches!(
        preflight_dump(
            Cursor::new(unsafe_source.as_bytes()),
            &mut Cursor::new(unsafe_source.as_bytes()),
            &targets,
            Some("admin"),
            [],
        ),
        Err(PreflightError::Unsupported { .. })
    ));
    Ok(())
}
