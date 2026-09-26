use mysql_dump::{
    Edit, Frame, RoutineAction, RoutineKind, RoutineName, RoutineSkipError, RoutineSkips,
    RoutingAction, routine_reference, routing_reference, scan, source_database_header,
    write_transformed,
};

#[test]
fn generated_unparseable_routine_has_a_reference() -> Result<(), Box<dyn std::error::Error>> {
    let source = include_str!("../../../it/fixtures/mysql-import/8.4.9/plain.sql");
    let mut found_create = false;
    let mut found_drop = false;
    scan(source.as_bytes(), |frame| {
        if let Frame::Sql { range, delimiter } = frame
            && let Some(statement) = source.get(range.start as usize..range.end as usize)
            && statement.contains("dynamic_probe")
        {
            let reference = routine_reference(statement, &delimiter);
            assert!(reference.is_ok());
            if let Ok(Some(reference)) = reference {
                assert_eq!(reference.kind, RoutineKind::Procedure);
                assert_eq!(reference.name, "dynamic_probe");
                assert_eq!(statement.get(reference.name_span), Some("`dynamic_probe`"));
                match reference.action {
                    RoutineAction::Create => found_create = true,
                    RoutineAction::Drop => found_drop = true,
                }
            }
        }
        Ok(())
    })?;
    assert!(found_create && found_drop);
    Ok(())
}

#[test]
fn qualified_routine_names_and_quoted_definers_are_identified()
-> Result<(), Box<dyn std::error::Error>> {
    let create = "CREATE DEFINER='prod'@'%' PROCEDURE `admin`.`do``work`() BEGIN PREPARE stmt FROM @sql; EXECUTE stmt; END;;";
    let reference = routine_reference(create, b";;")?.ok_or("missing routine header")?;
    assert_eq!(reference.action, RoutineAction::Create);
    assert_eq!(reference.database.as_deref(), Some("admin"));
    assert_eq!(reference.name, "do`work");
    assert_eq!(create.get(reference.name_span), Some("`do``work`"));

    let drop = "DROP PROCEDURE IF EXISTS `admin`.`do``work`;";
    let reference = routine_reference(drop, b";")?.ok_or("missing routine drop")?;
    assert_eq!(reference.action, RoutineAction::Drop);
    assert_eq!(reference.database.as_deref(), Some("admin"));
    assert_eq!(reference.name, "do`work");
    Ok(())
}

#[test]
fn unrelated_sql_is_not_misidentified_as_a_routine() -> Result<(), Box<dyn std::error::Error>> {
    for statement in [
        "CREATE TABLE t (body varchar(100) DEFAULT 'CREATE PROCEDURE p');",
        "INSERT INTO log VALUES ('CREATE PROCEDURE p');",
        "CREATE VIEW v AS SELECT 'FUNCTION f' AS label;",
    ] {
        assert!(routine_reference(statement, b";")?.is_none());
    }
    Ok(())
}

#[test]
fn definer_objects_other_than_routines_are_not_routines() -> Result<(), Box<dyn std::error::Error>>
{
    for statement in [
        "CREATE DEFINER='prod'@'%' VIEW v AS SELECT 1;",
        "CREATE DEFINER='prod'@'%' TRIGGER tr BEFORE INSERT ON t FOR EACH ROW SET @a = 1;",
        "CREATE DEFINER='prod'@'%' EVENT ev ON SCHEDULE EVERY 1 DAY DO DELETE FROM t;",
    ] {
        assert!(routine_reference(statement, b";")?.is_none());
    }
    Ok(())
}

#[test]
fn unquoted_definer_account_named_like_an_object_is_not_the_object_kind()
-> Result<(), Box<dyn std::error::Error>> {
    for account in ["event@localhost", "view@localhost", "trigger@localhost"] {
        let statement = format!("CREATE DEFINER={account} PROCEDURE p() BEGIN SELECT 1; END;");
        let reference = routine_reference(&statement, b";")?.ok_or("missing procedure")?;
        assert_eq!(reference.kind, RoutineKind::Procedure);
        assert_eq!(reference.name, "p");
    }
    let statement = "CREATE DEFINER=CURRENT_USER() FUNCTION f() RETURNS int RETURN 1;";
    let reference = routine_reference(statement, b";")?.ok_or("missing function")?;
    assert_eq!(reference.kind, RoutineKind::Function);
    Ok(())
}

#[test]
fn routine_create_if_not_exists_identifies_name() -> Result<(), Box<dyn std::error::Error>> {
    let create = "CREATE PROCEDURE IF NOT EXISTS p() BEGIN SELECT 1; END $$";
    let reference = routine_reference(create, b"$$")?.ok_or("missing procedure")?;
    assert_eq!(reference.name, "p");
    assert_eq!(create.get(reference.name_span), Some("p"));
    Ok(())
}

#[test]
fn custom_delimiter_and_function_kind_are_supported() -> Result<(), Box<dyn std::error::Error>> {
    let create = "CREATE FUNCTION `f`() RETURNS int BEGIN RETURN 1; END //";
    let reference = routine_reference(create, b"//")?.ok_or("missing function")?;
    assert_eq!(reference.kind, RoutineKind::Function);
    assert_eq!(reference.name, "f");
    let drop = "DROP FUNCTION IF EXISTS `f`$$";
    let reference = routine_reference(drop, b"$$")?.ok_or("missing function drop")?;
    assert_eq!(reference.kind, RoutineKind::Function);
    assert_eq!(reference.action, RoutineAction::Drop);
    Ok(())
}

#[test]
fn requested_dynamic_routine_omits_drop_and_create_frames() -> Result<(), Box<dyn std::error::Error>>
{
    let source = "USE `admin`;\nDROP PROCEDURE IF EXISTS `dynamic_probe`;\nDELIMITER $$\nCREATE PROCEDURE `dynamic_probe`() BEGIN PREPARE s FROM @sql; EXECUTE s; END $$\nDELIMITER ;\nCREATE TABLE `ok` (`id` int);\n";
    let mut frames = Vec::new();
    scan(source.as_bytes(), |frame| {
        frames.push(frame);
        Ok(())
    })?;
    let mut skips = RoutineSkips::new([RoutineName {
        database: "admin".to_owned(),
        name: "dynamic_probe".to_owned(),
    }])?;
    let mut edits = Vec::new();
    for frame in frames {
        let range = match &frame {
            Frame::Sql { range, .. } | Frame::Delimiter { range, .. } | Frame::Trivia { range } => {
                range.clone()
            }
        };
        let bytes = &source.as_bytes()[range.start as usize..range.end as usize];
        let should_skip = if let Frame::Sql { delimiter, .. } = frame {
            let statement = std::str::from_utf8(bytes)?;
            if let Some(reference) = routine_reference(statement, &delimiter)? {
                skips.observe(&reference, Some("admin"))?
            } else {
                false
            }
        } else {
            false
        };
        if should_skip {
            edits.push(Edit::Skip(range));
        }
    }
    assert_eq!(edits.len(), 2);
    let mut output = Vec::new();
    write_transformed(source.as_bytes(), &mut output, edits)?;
    assert_eq!(
        String::from_utf8(output)?,
        "USE `admin`;\nDELIMITER $$\n\nDELIMITER ;\nCREATE TABLE `ok` (`id` int);\n"
    );
    assert_eq!(
        skips.finish()?,
        [RoutineName {
            database: "admin".to_owned(),
            name: "dynamic_probe".to_owned()
        }]
    );
    Ok(())
}

#[test]
fn routine_skips_match_decoded_source_names_exactly() -> Result<(), Box<dyn std::error::Error>> {
    let statement = "DROP FUNCTION IF EXISTS `f`;";
    let reference = routine_reference(statement, b";")?.ok_or("missing function")?;
    for (database, name, expected) in [
        ("admin", "f", true),
        ("Admin", "f", false),
        ("admin", "F", false),
        ("other", "f", false),
        ("admin", "ff", false),
    ] {
        let mut skips = RoutineSkips::new([RoutineName {
            database: database.to_owned(),
            name: name.to_owned(),
        }])?;
        assert_eq!(skips.observe(&reference, Some("admin"))?, expected);
        if expected {
            assert_eq!(skips.finish()?.len(), 1);
        } else {
            assert!(matches!(
                skips.finish(),
                Err(RoutineSkipError::Absent { .. })
            ));
        }
    }
    Ok(())
}

#[test]
fn absent_and_duplicate_skip_requests_fail() -> Result<(), Box<dyn std::error::Error>> {
    let name = RoutineName {
        database: "admin".to_owned(),
        name: "missing".to_owned(),
    };
    let skips = RoutineSkips::new([name.clone()])?;
    assert!(matches!(
        skips.finish(),
        Err(RoutineSkipError::Absent { .. })
    ));
    assert!(matches!(
        RoutineSkips::new([name.clone(), name]),
        Err(RoutineSkipError::Duplicate { .. })
    ));
    Ok(())
}

#[test]
fn unqualified_skips_follow_use_and_plain_dump_header() -> Result<(), Box<dyn std::error::Error>> {
    let source = "-- Host: localhost    Database: admin\nDROP PROCEDURE IF EXISTS `p`;\nUSE `other`;\nDROP PROCEDURE IF EXISTS `p`;";
    let mut active_database = source_database_header(source.as_bytes())?;
    let mut skips = RoutineSkips::new([RoutineName {
        database: "admin".to_owned(),
        name: "p".to_owned(),
    }])?;
    let mut skipped = Vec::new();
    let mut frames = Vec::new();
    scan(source.as_bytes(), |frame| {
        frames.push(frame);
        Ok(())
    })?;
    let mut routines = 0;
    for frame in frames {
        let Frame::Sql { range, delimiter } = frame else {
            continue;
        };
        let statement = source
            .get(range.start as usize..range.end as usize)
            .ok_or("invalid frame")?;
        if let Some(reference) = routing_reference(statement)?
            && reference.action == RoutingAction::Use
        {
            active_database = Some(reference.name);
        }
        if let Some(reference) = routine_reference(statement, &delimiter)? {
            routines += 1;
            if skips.observe(&reference, active_database.as_deref())? {
                skipped.push(range);
            }
        }
    }
    assert_eq!(routines, 2);
    assert_eq!(skipped.len(), 1);
    assert_eq!(
        source.get(skipped[0].start as usize..skipped[0].end as usize),
        Some("-- Host: localhost    Database: admin\nDROP PROCEDURE IF EXISTS `p`;")
    );
    assert_eq!(skips.finish()?.len(), 1);
    Ok(())
}
