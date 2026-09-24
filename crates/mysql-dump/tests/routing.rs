use mysql_dump::{Frame, RoutingAction, RoutingError, routing_reference, scan};

#[test]
fn routing_names_have_verified_source_spans() -> Result<(), Box<dyn std::error::Error>> {
    for (source, action, name, bytes) in [
        ("USE `admin`;", RoutingAction::Use, "admin", "`admin`"),
        (
            "CREATE DATABASE IF NOT EXISTS `Mixed-Name`;",
            RoutingAction::Create,
            "Mixed-Name",
            "`Mixed-Name`",
        ),
        (
            "ALTER DATABASE `analytics` DEFAULT CHARACTER SET utf8mb4;",
            RoutingAction::Alter,
            "analytics",
            "`analytics`",
        ),
        (
            "DROP DATABASE IF EXISTS `admin`;",
            RoutingAction::Drop,
            "admin",
            "`admin`",
        ),
    ] {
        let Some(reference) = routing_reference(source)? else {
            return Err("expected routing reference".into());
        };
        assert_eq!(reference.action, action);
        assert_eq!(reference.name, name);
        assert_eq!(&source[reference.span], bytes);
    }
    Ok(())
}

#[test]
fn generated_create_database_options_are_routing_only() -> Result<(), Box<dyn std::error::Error>> {
    let source = "CREATE DATABASE /*!32312 IF NOT EXISTS*/ `admin` /*!40100 DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci */ /*!80016 DEFAULT ENCRYPTION='N' */;";
    let Some(reference) = routing_reference(source)? else {
        return Err("expected database marker".into());
    };
    assert_eq!(reference.action, RoutingAction::Create);
    assert_eq!(reference.name, "admin");
    assert_eq!(&source[reference.span], "`admin`");

    let source = "CREATE DATABASE `admin` DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci DEFAULT ENCRYPTION='N';";
    assert!(matches!(
        routing_reference(source),
        Err(RoutingError::UnsupportedSyntax { .. })
    ));
    Ok(())
}

#[test]
fn generated_dump_database_markers_are_identified() -> Result<(), Box<dyn std::error::Error>> {
    let source = include_str!("../../../it/fixtures/mysql-import/8.4.9/databases.sql");
    let mut database_names = Vec::new();
    scan(source.as_bytes(), |frame| {
        if let Frame::Sql { range, .. } = frame
            && let Some(statement) = source.get(range.start as usize..range.end as usize)
            && let Ok(Some(reference)) = routing_reference(statement)
            && reference.action == RoutingAction::Create
        {
            database_names.push(reference.name);
        }
        Ok(())
    })?;
    assert_eq!(database_names, ["admin", "analytics", "Mixed-Name"]);
    Ok(())
}
