use mysql_dump::{Frame, SessionError, SessionSetup, scan, session_setup};

#[test]
fn generated_connection_setup_is_accepted() -> Result<(), Box<dyn std::error::Error>> {
    for source in [
        "/*!40101 SET @OLD_CHARACTER_SET_CLIENT=@@CHARACTER_SET_CLIENT */;",
        "/*!50503 SET NAMES utf8mb4 */;",
        "/*!40103 SET TIME_ZONE='+00:00' */;",
        "/*!40014 SET @OLD_UNIQUE_CHECKS=@@UNIQUE_CHECKS, UNIQUE_CHECKS=0 */;",
        "/*!40014 SET @OLD_FOREIGN_KEY_CHECKS=@@FOREIGN_KEY_CHECKS, FOREIGN_KEY_CHECKS=0 */;",
        "/*!40101 SET @OLD_SQL_MODE=@@SQL_MODE, SQL_MODE='NO_AUTO_VALUE_ON_ZERO' */;",
        "SET time_zone = '+00:00';",
        "SET NAMES utf8mb4;",
        "SET character_set_client=latin1;",
        "SET character_set_results=utf8mb3;",
        "SET collation_connection=latin1_swedish_ci;",
        "SET collation_connection=utf8mb3_general_ci;",
        "SET @private_restore=@@character_set_client;",
        "SET sql_mode='STRICT_TRANS_TABLES,STRICT_ALL_TABLES,NO_ZERO_IN_DATE,NO_ZERO_DATE,ALLOW_INVALID_DATES,ERROR_FOR_DIVISION_BY_ZERO,TRADITIONAL,NO_ENGINE_SUBSTITUTION';",
        "/*!50003 SET sql_mode = 'ONLY_FULL_GROUP_BY,STRICT_TRANS_TABLES,NO_ZERO_IN_DATE,NO_ZERO_DATE,ERROR_FOR_DIVISION_BY_ZERO,NO_ENGINE_SUBSTITUTION' */;",
    ] {
        assert_eq!(session_setup(source)?, Some(SessionSetup::Keep), "{source}");
    }
    assert_eq!(
        session_setup("/*!40101 SET SQL_MODE=@OLD_SQL_MODE */;")?,
        Some(SessionSetup::Skip)
    );
    assert_eq!(session_setup("CREATE TABLE users (id int);")?, None);
    Ok(())
}

#[test]
fn server_scope_and_unsafe_expressions_fail_preflight() {
    for source in [
        "SET GLOBAL general_log=ON;",
        "/*!50606 SET GLOBAL INNODB_STATS_AUTO_RECALC=OFF */;",
        "SET @@global.foreign_key_checks=0;",
        "SET PERSIST sql_mode='';",
        "SET foreign_key_checks=(SELECT 0);",
        "SET @sql_text=CONCAT('DROP ', 'DATABASE admin');",
        "SET @sql_text='DROP DATABASE admin';",
        "SET sql_mode='ANSI_QUOTES';",
        "SET sql_mode='NO_BACKSLASH_ESCAPES';",
        "SET sql_mode='AN\\SI_QUOTES';",
        "SET sql_mode='NO_AUTO_VALUE_ON_ZERO,ANSI_QUOTES';",
        "SET sql_mode='ANSI';",
        "SET sql_mode=@@global.sql_mode;",
        "SET ROLE admin;",
        "SET character_set_client = utf8mb4_0900_ai_ci;",
        "SET collation_connection = utf8mb4;",
        "SET character_set_client = gbk;",
        "SET collation_connection = gbk_chinese_ci;",
        "SET sql_mode=@OLD_SQL_MODE, foreign_key_checks=0;",
    ] {
        assert!(
            matches!(session_setup(source), Err(SessionError::UnsafeSetup)),
            "{source}"
        );
    }
}

#[test]
fn generated_plain_dump_session_setup_is_covered() -> Result<(), Box<dyn std::error::Error>> {
    let source = include_str!("../../../it/fixtures/mysql-import/8.4.9/plain.sql");
    let mut checked = 0;
    for line in source.lines() {
        if line.starts_with("/*!") && line.contains(" SET ") && line.trim_end().ends_with(';') {
            let expected = if line.contains("SET SQL_MODE=@OLD_SQL_MODE")
                || line.contains("SET sql_mode              = @saved_sql_mode")
            {
                SessionSetup::Skip
            } else {
                SessionSetup::Keep
            };
            assert!(
                matches!(session_setup(line), Ok(Some(actual)) if actual == expected),
                "{line}"
            );
            checked += 1;
        }
    }
    assert!(checked > 25);
    Ok(())
}

#[test]
fn framed_dump_setup_uses_the_same_statement_ranges_as_preflight()
-> Result<(), Box<dyn std::error::Error>> {
    for source in [
        include_str!("../../../it/fixtures/mysql-import/8.0.46/plain.sql"),
        include_str!("../../../it/fixtures/mysql-import/8.4.9/plain.sql"),
        include_str!("../../../it/fixtures/mysql-import/9.7.0/plain.sql"),
        include_str!("../../../it/fixtures/mysql-import/phpmyadmin.sql"),
    ] {
        let mut checked = 0;
        scan(source.as_bytes(), |frame| {
            if let Frame::Sql { range, .. } = frame
                && let Some(statement) = source.get(range.start as usize..range.end as usize)
                && (statement.contains("SET @OLD_CHARACTER_SET_CLIENT")
                    || statement.contains("SET SQL_MODE = \"NO_AUTO_VALUE_ON_ZERO\""))
            {
                assert_eq!(session_setup(statement), Ok(Some(SessionSetup::Keep)));
                checked += 1;
            }
            Ok(())
        })?;
        assert!(checked > 0);
    }
    Ok(())
}
