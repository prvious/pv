use mysql_dump::{Frame, dump_key_toggle, scan};

#[test]
fn generated_key_build_hints_are_exactly_recognized() -> Result<(), Box<dyn std::error::Error>> {
    for source in [
        include_str!("../../../it/fixtures/mysql-import/8.0.46/databases.sql"),
        include_str!("../../../it/fixtures/mysql-import/8.4.9/databases.sql"),
        include_str!("../../../it/fixtures/mysql-import/9.7.0/databases.sql"),
    ] {
        let mut count = 0;
        scan(source.as_bytes(), |frame| {
            if let Frame::Sql { range, .. } = frame
                && let Some(statement) = source.get(range.start as usize..range.end as usize)
                && (statement.contains("DISABLE KEYS") || statement.contains("ENABLE KEYS"))
            {
                assert!(matches!(dump_key_toggle(statement), Ok(true)));
                count += 1;
            }
            Ok(())
        })?;
        assert_eq!(count, 8);
    }
    Ok(())
}

#[test]
fn other_alter_statements_are_not_dropped() -> Result<(), Box<dyn std::error::Error>> {
    for source in [
        "ALTER TABLE t ADD COLUMN x int;",
        "ALTER TABLE t DISABLE KEYS; DROP DATABASE other;",
        "ALTER TABLE t DISABLE KEYS /* no terminator */",
        "ALTER TABLE t DISABLE KEYS, DROP COLUMN x;",
    ] {
        assert!(!dump_key_toggle(source)?, "{source}");
    }
    assert!(dump_key_toggle("ALTER TABLE `admin`.`users` ENABLE KEYS;")?);
    Ok(())
}
