use mysql_dump::{RoutingAction, RoutingError, routing_reference};

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
fn generated_create_database_options_are_rejected() {
    let source = "CREATE DATABASE `admin` DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci DEFAULT ENCRYPTION='N';";
    assert!(matches!(
        routing_reference(source),
        Err(RoutingError::UnsupportedSyntax { .. })
    ));
}
