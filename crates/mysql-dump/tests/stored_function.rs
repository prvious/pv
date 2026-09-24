use mysql_dump::{Frame, StoredFunctionError, normalize_stored_function_for_analysis, scan};
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
                let normalized = normalize_stored_function_for_analysis(statement, &delimiter);
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
fn server_file_output_is_never_masked_as_a_local_variable() {
    let source = "CREATE FUNCTION f() RETURNS int BEGIN DECLARE result int; SELECT 1 INTO OUTFILE '/tmp/leak'; RETURN result; END;;";
    assert!(matches!(
        normalize_stored_function_for_analysis(source, b";;"),
        Err(StoredFunctionError::UnsupportedSelectInto)
    ));
    assert!(matches!(
        normalize_stored_function_for_analysis("SELECT 1 INTO result;", b";"),
        Err(StoredFunctionError::WrongStatement)
    ));
}

#[test]
fn dollar_delimiter_and_select_expression_remain_visible() -> Result<(), Box<dyn std::error::Error>>
{
    let source = "CREATE FUNCTION f() RETURNS int BEGIN DECLARE user_count int; SELECT LOAD_FILE('/tmp/secret') INTO user_count; RETURN user_count; END$$";
    let normalized = normalize_stored_function_for_analysis(source, b"$$")?;
    assert_eq!(normalized.len(), source.len());
    assert!(normalized.contains("LOAD_FILE('/tmp/secret')"));
    assert!(normalized.contains("SELECT"));
    assert!(!normalized.contains("INTO user_count"));
    assert!(squonk::parse_with(&normalized, squonk::ParseConfig::new(MySql)).is_ok());
    Ok(())
}
