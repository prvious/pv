use mysql_dump::{
    StoredRoutinePolicyError, analyze_stored_routine, validate_stored_routine_effects,
};

#[test]
fn routine_effects_accept_database_local_sql() -> Result<(), Box<dyn std::error::Error>> {
    let source = "CREATE PROCEDURE p() BEGIN INSERT IGNORE t VALUES (1); UPDATE t SET id=2; DELETE t FROM t t WHERE t.id=3; END$$";
    let analysis = analyze_stored_routine(source, b"$$")?;
    validate_stored_routine_effects(&analysis.sql)?;
    Ok(())
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
