use mysql_dump::{Frame, Patch, TableError, scan, table_database_references, write_patched};

#[test]
fn table_and_foreign_key_qualifiers_have_verified_byte_spans()
-> Result<(), Box<dyn std::error::Error>> {
    let source = "-- heading\r\nCREATE TABLE /* André */ `admin`.`children` (\r\n  `id` int NOT NULL,\r\n  `parent_id` int REFERENCES `third`.`roots` (`id`),\r\n  CONSTRAINT `fk` FOREIGN KEY (`id`) REFERENCES `other`.`parents` (`id`)\r\n) ENGINE=InnoDB COMMENT='admin.other';";
    let references = table_database_references(source)?;
    assert_eq!(references.len(), 3);
    assert_eq!(references[0].name, "admin");
    assert_eq!(references[1].name, "third");
    assert_eq!(references[2].name, "other");
    assert_eq!(source.get(references[0].span.clone()), Some("`admin`"));
    assert_eq!(source.get(references[1].span.clone()), Some("`third`"));
    assert_eq!(source.get(references[2].span.clone()), Some("`other`"));
    let patches = references.into_iter().map(|reference| Patch {
        range: reference.span.start as u64..reference.span.end as u64,
        expected: format!("`{}`", reference.name).into_bytes(),
        replacement: format!("`project_{}`", reference.name).into_bytes(),
    });
    let mut output = Vec::new();
    write_patched(source.as_bytes(), &mut output, patches)?;
    let output = String::from_utf8(output)?;
    assert!(output.contains("CREATE TABLE /* André */ `project_admin`.`children`"));
    assert!(output.contains("REFERENCES `project_third`.`roots`"));
    assert!(output.contains("REFERENCES `project_other`.`parents`"));
    assert!(output.contains("COMMENT='admin.other'"));
    Ok(())
}

#[test]
fn generated_dump_table_definitions_are_supported() -> Result<(), Box<dyn std::error::Error>> {
    let source = include_str!("../../../it/fixtures/mysql-import/8.4.9/databases.sql");
    let mut statements = Vec::new();
    scan(source.as_bytes(), |frame| {
        if let Frame::Sql { range, .. } = frame
            && let Some(statement) = source.get(range.start as usize..range.end as usize)
            && statement.contains("CREATE TABLE")
        {
            statements.push(statement);
        }
        Ok(())
    })?;
    assert!(!statements.is_empty());
    for statement in statements {
        assert!(table_database_references(statement).is_ok());
    }
    Ok(())
}

#[test]
fn unquoted_and_escaped_database_identifiers_are_verified() -> Result<(), Box<dyn std::error::Error>>
{
    let source = "CREATE TABLE admin.children (id int, FOREIGN KEY (id) REFERENCES `other``db`.parents(id));";
    let references = table_database_references(source)?;
    assert_eq!(references.len(), 2);
    assert_eq!(references[0].name, "admin");
    assert_eq!(source.get(references[0].span.clone()), Some("admin"));
    assert_eq!(references[1].name, "other`db");
    assert_eq!(source.get(references[1].span.clone()), Some("`other``db`"));
    Ok(())
}

#[test]
fn temporary_table_uses_the_same_option_policy() -> Result<(), Box<dyn std::error::Error>> {
    let source = "CREATE TEMPORARY TABLE `admin`.`scratch` (id INT) ENGINE=InnoDB;";
    let references = table_database_references(source)?;
    assert_eq!(references.len(), 1);
    assert_eq!(references[0].name, "admin");
    Ok(())
}

#[test]
fn unsupported_table_forms_and_extra_statements_fail() {
    for source in [
        "CREATE TABLE t AS SELECT * FROM other.t;",
        "CREATE TABLE t (id int); DROP DATABASE other;",
        "CREATE TABLE t (id int CHECK (other.t.id > 0));",
        "CREATE TABLE t (id int CHECK (other.validate(id) > 0));",
        "CREATE TABLE t LIKE other.t;",
        "CREATE TABLE t (id INT) DATA DIRECTORY '/tmp/pv350-data';",
        "CREATE TABLE t (id INT) INDEX DIRECTORY '/tmp/pv350-index';",
        "CREATE TABLE t (id INT) ENGINE=FEDERATED CONNECTION='mysql://remote/db/t';",
        "CREATE TABLE t (id INT) TABLESPACE mysql;",
        "CREATE TABLE t (id INT) /*!50100 ENGINE=FEDERATED CONNECTION='mysql://remote/db/t' */;",
    ] {
        assert!(matches!(
            table_database_references(source),
            Err(TableError::UnsupportedForm | TableError::WrongStatement)
        ));
    }
}
