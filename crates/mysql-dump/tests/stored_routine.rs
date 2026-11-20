use mysql_dump::{
    Frame, StoredRoutineError, analyze_stored_routine, normalize_stored_routine_for_analysis, scan,
    statement_references,
};
use squonk::dialect::MySql;

#[test]
fn generated_stored_function_has_parseable_byte_aligned_analysis()
-> Result<(), Box<dyn std::error::Error>> {
    for source in [
        include_str!("../../../it/fixtures/mysql-import/8.0.46/plain.sql"),
        include_str!("../../../it/fixtures/mysql-import/8.4.9/plain.sql"),
        include_str!("../../../it/fixtures/mysql-import/9.7.0/plain.sql"),
    ] {
        let mut checked = false;
        scan(source.as_bytes(), |frame| {
            if let Frame::Sql { range, delimiter } = frame
                && let Some(statement) = source.get(range.start as usize..range.end as usize)
                && statement.contains("FUNCTION `user_total`")
            {
                let normalized = normalize_stored_routine_for_analysis(statement, &delimiter);
                assert!(normalized.is_ok());
                if let Ok(normalized) = normalized {
                    assert_eq!(normalized.len(), statement.len());
                    assert!(!normalized.contains("INTO user_count"));
                    assert!(normalized.contains("FROM admin.users"));
                    assert!(
                        squonk::parse_with(&normalized, squonk::ParseConfig::new(MySql)).is_ok()
                    );
                }
                checked = true;
            }
            Ok(())
        })?;
        assert!(checked);
    }
    Ok(())
}

#[test]
fn generated_procedures_have_parseable_byte_aligned_analysis()
-> Result<(), Box<dyn std::error::Error>> {
    for source in [
        include_str!("../../../it/fixtures/mysql-import/8.0.46/plain.sql"),
        include_str!("../../../it/fixtures/mysql-import/8.4.9/plain.sql"),
        include_str!("../../../it/fixtures/mysql-import/9.7.0/plain.sql"),
    ] {
        let mut found = 0;
        scan(source.as_bytes(), |frame| {
            if let Frame::Sql { range, delimiter } = frame
                && let Some(statement) = source.get(range.start as usize..range.end as usize)
                && (statement.contains("PROCEDURE `write_audit`")
                    || statement.contains("PROCEDURE `dynamic_probe`"))
            {
                let normalized = normalize_stored_routine_for_analysis(statement, &delimiter);
                assert!(normalized.is_ok());
                if let Ok(normalized) = normalized {
                    assert_eq!(normalized.len(), statement.len());
                    assert!(
                        squonk::parse_with(&normalized, squonk::ParseConfig::new(MySql)).is_ok()
                    );
                }
                found += 1;
            }
            Ok(())
        })?;
        assert_eq!(found, 2);
    }
    Ok(())
}

#[test]
fn procedure_select_into_retains_source_references() -> Result<(), Box<dyn std::error::Error>> {
    let source = "CREATE PROCEDURE admin.count_users() BEGIN DECLARE total int; SELECT COUNT(*) INTO total FROM admin.users; SELECT total; END$$";
    let normalized = normalize_stored_routine_for_analysis(source, b"$$")?;
    assert_eq!(normalized.len(), source.len());
    assert!(normalized.contains("FROM admin.users"));
    assert!(!normalized.contains("INTO total"));
    assert!(squonk::parse_with(&normalized, squonk::ParseConfig::new(MySql)).is_ok());
    Ok(())
}

#[test]
fn procedure_parameter_is_a_valid_select_into_target() -> Result<(), Box<dyn std::error::Error>> {
    let source = "CREATE PROCEDURE p(OUT total int) BEGIN SELECT COUNT(*) INTO total FROM admin.users; END$$";
    let normalized = normalize_stored_routine_for_analysis(source, b"$$")?;
    assert!(normalized.contains("SELECT COUNT(*)"));
    assert!(!normalized.contains("INTO total"));
    assert!(squonk::parse_with(&normalized, squonk::ParseConfig::new(MySql)).is_ok());
    Ok(())
}

#[test]
fn multiple_declared_variables_can_receive_select_into() -> Result<(), Box<dyn std::error::Error>> {
    let source = "CREATE PROCEDURE p() BEGIN DECLARE first_count, next_count int; SELECT COUNT(*), MAX(id) INTO first_count, next_count FROM admin.users; END$$";
    let normalized = normalize_stored_routine_for_analysis(source, b"$$")?;
    assert!(!normalized.contains("INTO first_count, next_count"));
    assert!(squonk::parse_with(&normalized, squonk::ParseConfig::new(MySql)).is_ok());
    Ok(())
}

#[test]
fn server_file_output_is_never_masked_as_a_local_variable() {
    let source = "CREATE FUNCTION f() RETURNS int BEGIN DECLARE result int; SELECT 1 INTO OUTFILE '/tmp/leak'; RETURN result; END;;";
    assert!(matches!(
        normalize_stored_routine_for_analysis(source, b";;"),
        Err(StoredRoutineError::UnsupportedSelectInto)
    ));
    assert!(matches!(
        normalize_stored_routine_for_analysis("SELECT 1 INTO result;", b";"),
        Err(StoredRoutineError::WrongStatement)
    ));
}

#[test]
fn dollar_delimiter_and_select_expression_remain_visible() -> Result<(), Box<dyn std::error::Error>>
{
    let source = "CREATE FUNCTION f() RETURNS int BEGIN DECLARE user_count int; SELECT LOAD_FILE('/tmp/secret') INTO user_count; RETURN user_count; END$$";
    let normalized = normalize_stored_routine_for_analysis(source, b"$$")?;
    assert_eq!(normalized.len(), source.len());
    assert!(normalized.contains("LOAD_FILE('/tmp/secret')"));
    assert!(normalized.contains("SELECT"));
    assert!(!normalized.contains("INTO user_count"));
    assert!(squonk::parse_with(&normalized, squonk::ParseConfig::new(MySql)).is_ok());
    Ok(())
}

#[test]
fn extra_statement_in_routine_frame_is_rejected() {
    let source = "CREATE FUNCTION f() RETURNS int RETURN 1; DROP DATABASE other;;";
    assert!(normalize_stored_routine_for_analysis(source, b";;").is_err());
}

#[test]
fn comma_update_analysis_maps_qualified_references_to_source()
-> Result<(), Box<dyn std::error::Error>> {
    let source = "CREATE PROCEDURE p() BEGIN UPDATE `admin`.`users` u, (SELECT id FROM `other`.`users`) x SET u.flag=1 WHERE u.id=x.id; END$$";
    let analysis = analyze_stored_routine(source, b"$$")?;
    assert!(analysis.sql.contains(" CROSS JOIN "));
    let references = statement_references(&analysis.sql)?;
    let mapped: Option<Vec<_>> = references
        .databases
        .iter()
        .map(|reference| {
            let span = analysis.source_span(reference.span.clone())?;
            Some((reference.name.as_str(), source.get(span)?))
        })
        .collect();
    assert_eq!(
        mapped,
        Some(vec![("admin", "`admin`"), ("other", "`other`")])
    );
    let join = analysis.sql.find("CROSS JOIN").ok_or("missing join")?;
    assert!(analysis.source_span(join..join + 5).is_none());
    Ok(())
}

#[test]
fn optional_into_and_delete_target_analysis_preserve_source_references()
-> Result<(), Box<dyn std::error::Error>> {
    let source = "CREATE PROCEDURE p() BEGIN INSERT `admin`.`users` (id) VALUES (1); DELETE u FROM `admin`.`users` u WHERE u.id=1; END$$";
    let analysis = analyze_stored_routine(source, b"$$")?;
    assert!(analysis.sql.contains("INSERT INTO `admin`"));
    assert!(analysis.sql.contains("DELETE   FROM `admin`"));
    let references = statement_references(&analysis.sql)?;
    let mapped: Option<Vec<_>> = references
        .databases
        .iter()
        .map(|reference| {
            let span = analysis.source_span(reference.span.clone())?;
            Some((reference.name.as_str(), source.get(span)?))
        })
        .collect();
    assert_eq!(
        mapped,
        Some(vec![("admin", "`admin`"), ("admin", "`admin`")])
    );
    Ok(())
}

#[test]
fn temporary_table_drop_is_analyzed_without_changing_output()
-> Result<(), Box<dyn std::error::Error>> {
    let source = "CREATE PROCEDURE p() BEGIN DROP TEMPORARY TABLE IF EXISTS `admin`.`scratch`; CREATE TEMPORARY TABLE `admin`.`scratch` (id INT); END$$";
    let analysis = analyze_stored_routine(source, b"$$")?;
    assert!(analysis.sql.contains("DROP           TABLE"));
    let references = statement_references(&analysis.sql)?;
    let mapped: Option<Vec<_>> = references
        .databases
        .iter()
        .map(|reference| analysis.source_span(reference.span.clone()))
        .collect();
    assert!(mapped.is_some());
    assert_eq!(mapped.map(|spans| spans.len()), Some(2));
    Ok(())
}

#[test]
fn keyword_token_table_aliases_are_supported() -> Result<(), Box<dyn std::error::Error>> {
    for source in [
        "CREATE PROCEDURE p() BEGIN DELETE t FROM admin.t t; END$$",
        "CREATE PROCEDURE p() BEGIN INSERT t VALUES (1); END$$",
        "CREATE PROCEDURE p() BEGIN INSERT IGNORE t VALUES (1); END$$",
        "CREATE PROCEDURE p() BEGIN INSERT LOW_PRIORITY t VALUES (1); END$$",
        "CREATE PROCEDURE p() BEGIN INSERT HIGH_PRIORITY t VALUES (1); END$$",
        "CREATE PROCEDURE p() BEGIN INSERT DELAYED t VALUES (1); END$$",
    ] {
        assert!(analyze_stored_routine(source, b"$$").is_ok());
    }
    Ok(())
}
