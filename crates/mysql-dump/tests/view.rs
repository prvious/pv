use mysql_dump::{Patch, ViewError, view_statement_references, write_patched};

#[test]
fn fallback_finds_only_verified_database_names_and_explicit_definer()
-> Result<(), Box<dyn std::error::Error>> {
    let source = "CREATE ALGORITHM=UNDEFINED DEFINER=`prod`@`%` SQL SECURITY DEFINER VIEW `admin`.`v` AS SELECT `analytics`.`t`.`id`, t.id, 'admin.t' AS label FROM `analytics`.`t` t WHERE t.id IN (SELECT id FROM `other`.`records`);";
    let references = view_statement_references(source)?;
    let names: Vec<_> = references
        .databases
        .iter()
        .map(|reference| reference.name.as_str())
        .collect();
    assert_eq!(names, ["admin", "analytics", "analytics", "other"]);
    assert_eq!(references.definers.len(), 1);
    assert_eq!(
        source.get(references.definers[0].clone()),
        Some("DEFINER=`prod`@`%`")
    );
    let patches = references.databases.into_iter().map(|reference| Patch {
        range: reference.span.start as u64..reference.span.end as u64,
        expected: source[reference.span].as_bytes().to_vec(),
        replacement: format!("`project_{}`", reference.name).into_bytes(),
    });
    let mut output = Vec::new();
    write_patched(source.as_bytes(), &mut output, patches)?;
    let output = String::from_utf8(output)?;
    assert!(output.contains("VIEW `project_admin`.`v`"));
    assert!(output.contains("FROM `project_analytics`.`t`"));
    assert!(output.contains("'admin.t'"));
    Ok(())
}

#[test]
fn unsupported_view_effects_and_unverified_definers_fail() {
    for source in [
        "CREATE VIEW v AS SELECT LOAD_FILE('/etc/passwd');",
        "CREATE VIEW v AS SELECT id INTO OUTFILE '/tmp/x' FROM admin.t;",
        "CREATE DEFINER=CURRENT_USER VIEW v AS SELECT 1;",
        "CREATE MATERIALIZED VIEW v AS SELECT 1;",
        "CREATE VIEW v AS SELECT 1; DROP DATABASE other;",
    ] {
        assert!(matches!(
            view_statement_references(source),
            Err(ViewError::UnsupportedForm
                | ViewError::WrongStatement
                | ViewError::UnsupportedSyntax { .. })
        ));
    }
}

#[test]
fn generated_versioned_view_options_keep_original_identifier_spans()
-> Result<(), Box<dyn std::error::Error>> {
    let source = "CREATE /*!50001 ALGORITHM=UNDEFINED */\n/*!50013 DEFINER=`prod`@`localhost` SQL SECURITY DEFINER */\nVIEW `admin`.`v` AS SELECT t.id FROM `other`.`table` t;";
    let references = view_statement_references(source)?;
    assert_eq!(references.databases.len(), 2);
    assert_eq!(
        source.get(references.databases[0].span.clone()),
        Some("`admin`")
    );
    assert_eq!(
        source.get(references.databases[1].span.clone()),
        Some("`other`")
    );
    assert_eq!(
        source.get(references.definers[0].clone()),
        Some("DEFINER=`prod`@`localhost`")
    );
    Ok(())
}
