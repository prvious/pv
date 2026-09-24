use std::collections::BTreeMap;
use std::env;
use std::fs;

use qusql_parse::{Issues, ParseOptions, SQLDialect};
use sqlparser::dialect::MySqlDialect;
use sqlparser::parser::Parser;
use squonk::dialect::MySql;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args().nth(1).ok_or("expected dump path")?;
    if path == "--chunks" {
        let source = fs::read_to_string(env::args().nth(2).ok_or("expected dump path")?)?;
        let mut delimiter = ";";
        let mut chunk = String::new();
        let mut accepted = 0;
        let mut rejected = 0;
        let mut kinds = BTreeMap::<String, usize>::new();
        for (line_number, line) in source.lines().enumerate() {
            if let Some(next) = line.strip_prefix("DELIMITER ") {
                delimiter = next.trim();
                continue;
            }
            chunk.push_str(line);
            chunk.push('\n');
            if line.trim_end().ends_with(delimiter) {
                let candidate = if delimiter == ";" {
                    chunk.as_str()
                } else {
                    chunk.trim_end().strip_suffix(delimiter).unwrap_or(&chunk)
                };
                match squonk::parse_with(candidate, squonk::ParseConfig::new(MySql)) {
                    Ok(parsed) => {
                        accepted += parsed.statements().len();
                        for statement in parsed.statements() {
                            let debug = format!("{statement:?}");
                            let kind = debug.split(['(', '{']).next().unwrap_or("?");
                            *kinds.entry(kind.to_owned()).or_default() += 1;
                        }
                    }
                    Err(error) => {
                        rejected += 1;
                        if rejected <= 20 {
                            println!("chunk ending at line {}: {error}", line_number + 1);
                        }
                    }
                }
                chunk.clear();
            }
        }
        println!(
            "accepted={accepted} rejected={rejected} trailing_bytes={}",
            chunk.len()
        );
        println!("kinds={kinds:?}");
        return Ok(());
    }
    if path == "--samples" {
        for (label, source) in [
            (
                "database",
                "CREATE DATABASE IF NOT EXISTS `admin` DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci DEFAULT ENCRYPTION='N';",
            ),
            ("use", "USE `admin`;"),
            (
                "table",
                "CREATE TABLE `admin`.`users` (`id` bigint NOT NULL PRIMARY KEY, `name` varchar(255) NOT NULL);",
            ),
            (
                "view",
                "CREATE VIEW `analytics`.`admin_names` AS SELECT `admin`.`users`.`name` FROM `admin`.`users`;",
            ),
            (
                "trigger",
                "CREATE DEFINER=`root`@`localhost` TRIGGER `users_after_insert` AFTER INSERT ON `admin`.`users` FOR EACH ROW BEGIN INSERT INTO `analytics`.`audit`(message) VALUES (NEW.name); END;",
            ),
            (
                "procedure",
                "CREATE DEFINER=`root`@`localhost` PROCEDURE `admin`.`write_audit`(IN entry varchar(255)) BEGIN INSERT INTO `analytics`.`audit`(message) VALUES (entry); END;",
            ),
            (
                "event",
                "CREATE DEFINER=`root`@`localhost` EVENT `analytics`.`daily_audit` ON SCHEDULE EVERY 1 DAY DISABLE DO INSERT INTO `analytics`.`audit`(message) VALUES ('event');",
            ),
            (
                "executable_comment",
                "/*!50003 CREATE*/ /*!50017 DEFINER=`root`@`localhost`*/ /*!50003 TRIGGER `users_after_insert` AFTER INSERT ON `users` FOR EACH ROW BEGIN INSERT INTO analytics.audit(message) VALUES (NEW.name); END */;",
            ),
        ] {
            println!("{label}:");
            probe(source);
        }
        return Ok(());
    }
    if path == "--squonk" {
        let source = fs::read_to_string(env::args().nth(2).ok_or("expected dump path")?)?;
        let parsed = squonk::parse_with(&source, squonk::ParseConfig::new(MySql))?;
        println!("squonk: ok {} statements", parsed.statements().len());
        return Ok(());
    }
    let source = fs::read_to_string(path)?;
    println!("bytes={}", source.len());
    probe(&source);
    Ok(())
}

fn probe(source: &str) {
    match Parser::parse_sql(&MySqlDialect {}, source) {
        Ok(statements) => {
            println!("sqlparser: ok {} statements", statements.len());
            if source.starts_with("/*!") {
                println!("sqlparser AST: {statements:?}");
            }
        }
        Err(error) => println!("sqlparser: {error}"),
    }

    let options = ParseOptions::new().dialect(SQLDialect::MariaDB);
    let mut issues = Issues::new(source);
    let statements = qusql_parse::parse_statements(source, &mut issues, &options);
    println!(
        "qusql: {} statements; ok={}",
        statements.len(),
        issues.is_ok()
    );

    match sqlglot_rust::parse(source, sqlglot_rust::Dialect::Mysql) {
        Ok(_) => println!("sqlglot: parsed one statement"),
        Err(error) => println!("sqlglot: {error}"),
    }

    match squonk::parse_with(source, squonk::ParseConfig::new(MySql)) {
        Ok(parsed) => {
            println!("squonk: ok {} statements", parsed.statements().len());
            if source.starts_with("/*!") {
                println!("squonk AST: {:?}", parsed.statements());
            }
        }
        Err(error) => println!("squonk: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use squonk::ast::Ident;
    use squonk::ast::generated::Visit;

    use super::*;

    #[test]
    fn executable_comment_identifiers_have_exact_source_spans()
    -> Result<(), Box<dyn std::error::Error>> {
        let source = "/*!50003 CREATE*/ /*!50017 DEFINER=`root`@`localhost`*/ /*!50003 TRIGGER `users_after_insert` AFTER INSERT ON `admin`.`users` FOR EACH ROW BEGIN INSERT INTO `analytics`.`audit`(message) VALUES (NEW.name); END */;";
        let parsed = squonk::parse_with(source, squonk::ParseConfig::new(MySql))?;
        struct Identifiers<'a> {
            source: &'a str,
            slices: Vec<&'a str>,
        }
        impl<'ast> Visit<'ast> for Identifiers<'_> {
            fn visit_ident(&mut self, ident: &'ast Ident) {
                let span = ident.meta.span;
                if let Some(slice) = self.source.get(span.start() as usize..span.end() as usize) {
                    self.slices.push(slice);
                }
            }
        }
        let mut identifiers = Identifiers {
            source,
            slices: Vec::new(),
        };
        for statement in parsed.statements() {
            identifiers.visit_statement(statement);
        }
        assert!(identifiers.slices.contains(&"`admin`"));
        assert!(identifiers.slices.contains(&"`analytics`"));
        assert!(identifiers.slices.contains(&"`root`"));
        assert!(identifiers.slices.contains(&"NEW"));
        Ok(())
    }
}
