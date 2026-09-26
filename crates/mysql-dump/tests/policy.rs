use std::collections::BTreeSet;

use mysql_dump::{
    StoredRoutinePolicyError, analyze_stored_routine, validate_statement_effects,
    validate_stored_object_effects, validate_stored_routine_effects,
    validate_stored_routine_effects_for_targets,
};

#[test]
fn routine_effects_accept_database_local_sql() -> Result<(), Box<dyn std::error::Error>> {
    let source = "CREATE PROCEDURE p() BEGIN INSERT IGNORE t VALUES (1); UPDATE t SET id=2; DELETE t FROM t t WHERE t.id=3; END$$";
    let analysis = analyze_stored_routine(source, b"$$")?;
    validate_stored_routine_effects(&analysis.sql)?;
    Ok(())
}

#[test]
fn truncate_requires_every_table_to_resolve_to_a_mapped_target()
-> Result<(), Box<dyn std::error::Error>> {
    let source = "CREATE PROCEDURE p() BEGIN TRUNCATE TABLE local_data; TRUNCATE TABLE `other`.`audit`; END$$";
    let analysis = analyze_stored_routine(source, b"$$")?;
    let mapped = BTreeSet::from(["admin".to_owned(), "other".to_owned()]);
    assert!(matches!(
        validate_stored_routine_effects(&analysis.sql),
        Err(StoredRoutinePolicyError::UnmappedTruncate)
    ));
    assert!(validate_stored_routine_effects_for_targets(
        &analysis.sql,
        Some("admin"),
        &mapped
    )?);
    assert!(matches!(
        validate_stored_routine_effects_for_targets(&analysis.sql, None, &mapped),
        Err(StoredRoutinePolicyError::UnmappedTruncate)
    ));
    let unmapped = BTreeSet::from(["admin".to_owned()]);
    assert!(matches!(
        validate_stored_routine_effects_for_targets(&analysis.sql, Some("admin"), &unmapped),
        Err(StoredRoutinePolicyError::UnmappedTruncate)
    ));
    Ok(())
}

#[test]
fn ordinary_statement_policy_rejects_server_effects() {
    for source in [
        "CREATE USER thief IDENTIFIED BY 'pw';",
        "GRANT ALL ON *.* TO thief;",
        "SET GLOBAL general_log=ON;",
        "SELECT LOAD_FILE('/tmp/secret');",
        "INSERT INTO t VALUES (LOAD_FILE('/tmp/secret'));",
        "ALTER TABLE t DISCARD TABLESPACE;",
        "ALTER TABLE t IMPORT TABLESPACE;",
    ] {
        assert!(validate_statement_effects(source).is_err(), "{source}");
    }
    for source in [
        "DROP TABLE IF EXISTS t;",
        "CREATE INDEX idx ON t (id);",
        "LOCK TABLES t WRITE;",
        "UNLOCK TABLES;",
        "DROP TRIGGER IF EXISTS `admin`.`audit_trigger`;",
    ] {
        assert!(validate_statement_effects(source).is_ok(), "{source}");
    }
}

#[test]
fn stored_trigger_and_event_bodies_reject_dynamic_sql_and_server_files() {
    for source in [
        "CREATE TRIGGER t AFTER INSERT ON users FOR EACH ROW INSERT INTO audit VALUES (NEW.id);",
        "CREATE EVENT e ON SCHEDULE EVERY 1 DAY DO DELETE FROM expired WHERE id < 10;",
    ] {
        assert!(
            validate_stored_object_effects(source).is_ok(),
            "{source}: {:?}",
            validate_stored_object_effects(source)
        );
    }
    for source in [
        "CREATE TRIGGER t AFTER INSERT ON users FOR EACH ROW SET @x=LOAD_FILE('/tmp/x');",
        "CREATE EVENT e ON SCHEDULE EVERY 1 DAY DO TRUNCATE TABLE other.t;",
        "CREATE EVENT e ON SCHEDULE EVERY 1 DAY DO CREATE TABLE t (id INT) ENGINE=FEDERATED CONNECTION='mysql://remote/db/t';",
    ] {
        assert!(validate_stored_object_effects(source).is_err(), "{source}");
    }
}

#[test]
fn routine_effects_reject_dynamic_sql_and_server_files() -> Result<(), Box<dyn std::error::Error>> {
    for source in [
        "CREATE PROCEDURE p() BEGIN SET @sql='DROP DATABASE other'; PREPARE q FROM @sql; EXECUTE q; END$$",
        "CREATE PROCEDURE p() BEGIN SELECT LOAD_FILE('/tmp/secret'); END$$",
        "CREATE PROCEDURE p() BEGIN SET GLOBAL max_connections=1; END$$",
    ] {
        let analysis = analyze_stored_routine(source, b"$$")?;
        assert!(matches!(
            validate_stored_routine_effects(&analysis.sql),
            Err(StoredRoutinePolicyError::UnsafeStatement { .. }
                | StoredRoutinePolicyError::ServerFileFunction)
        ));
    }
    Ok(())
}
