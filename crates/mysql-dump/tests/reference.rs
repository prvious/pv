use mysql_dump::{
    database_references, normalize_stored_routine_for_analysis, statement_references,
};

#[test]
fn object_qualifiers_are_found_without_column_aliases_or_strings()
-> Result<(), Box<dyn std::error::Error>> {
    for (source, expected) in [
        (
            "CREATE VIEW analytics.admin_names AS SELECT name FROM admin.users;",
            vec!["analytics", "admin"],
        ),
        (
            "SELECT admin.users.name FROM admin.users AS u WHERE u.name = 'analytics.users';",
            vec!["admin", "admin"],
        ),
        (
            "CREATE TRIGGER admin.t AFTER INSERT ON admin.users FOR EACH ROW INSERT INTO analytics.audit(message) VALUES (NEW.name);",
            vec!["admin", "admin", "analytics"],
        ),
    ] {
        let references = database_references(source)?;
        let names: Vec<&str> = references
            .iter()
            .map(|reference| reference.name.as_str())
            .collect();
        assert_eq!(names, expected, "{source}");
        for reference in references {
            assert!(source.get(reference.span).is_some());
        }
    }
    Ok(())
}

#[test]
fn named_definer_span_can_be_replaced_without_changing_data()
-> Result<(), Box<dyn std::error::Error>> {
    let source = "CREATE DEFINER=`prod`@`%` VIEW admin.users_view AS SELECT 'prod@%' AS label FROM admin.users;";
    let references = statement_references(source)?;
    assert_eq!(references.definers.len(), 1);
    assert_eq!(
        source.get(references.definers[0].clone()),
        Some("DEFINER=`prod`@`%`")
    );
    assert_eq!(references.databases.len(), 2);
    Ok(())
}

#[test]
fn normalized_function_keeps_qualified_reference_spans() -> Result<(), Box<dyn std::error::Error>> {
    let source = "CREATE FUNCTION admin.total() RETURNS int BEGIN DECLARE n int; SELECT COUNT(*) INTO n FROM admin.users; RETURN n; END;;";
    let normalized = normalize_stored_routine_for_analysis(source, b";;")?;
    let references = database_references(&normalized)?;
    assert_eq!(references.len(), 2);
    for reference in references {
        assert_eq!(reference.name, "admin");
        assert_eq!(source.get(reference.span), Some("admin"));
    }
    Ok(())
}

#[test]
fn normalized_procedure_keeps_qualified_reference_spans() -> Result<(), Box<dyn std::error::Error>>
{
    let source = "CREATE PROCEDURE admin.total_users(OUT total int) BEGIN SELECT COUNT(*) INTO total FROM admin.users; INSERT INTO analytics.audit(message) VALUES ('counted'); END$$";
    let normalized = normalize_stored_routine_for_analysis(source, b"$$")?;
    let references = database_references(&normalized)?;
    let names: Vec<&str> = references
        .iter()
        .map(|reference| reference.name.as_str())
        .collect();
    assert_eq!(names, ["admin", "admin", "analytics"]);
    for reference in references {
        assert_eq!(source.get(reference.span), Some(reference.name.as_str()));
    }
    Ok(())
}
