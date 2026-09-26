use std::io::{self, Read};

use mysql_dump::{Frame, ReaderError, scan, scan_with};

fn frames(source: &[u8]) -> Result<Vec<Frame>, mysql_dump::ReaderError> {
    let mut found = Vec::new();
    scan(source, |frame| {
        found.push(frame);
        Ok(())
    })?;
    Ok(found)
}

#[derive(Debug, thiserror::Error)]
enum PreflightFailure {
    #[error(transparent)]
    Reader(#[from] ReaderError),
    #[error("unsafe SQL")]
    Unsafe,
}

#[test]
fn scanner_preserves_preflight_callback_error() {
    let result = scan_with("DROP DATABASE other;".as_bytes(), |_| {
        Err(PreflightFailure::Unsafe)
    });
    assert!(matches!(result, Err(PreflightFailure::Unsafe)));
}

#[test]
fn delimiters_inside_literals_comments_and_routines_do_not_split()
-> Result<(), Box<dyn std::error::Error>> {
    let source = b"-- header ;\r\nDELIMITER $$\r\nCREATE PROCEDURE p() BEGIN SELECT 'a;$$', `a$$`; /* $$ */ SELECT 2; END$$\r\nDELIMITER ;\r\nINSERT INTO t VALUES ('a\\'; b', 'x'';y');\r\n";
    let found = frames(source)?;
    let sql: Vec<&[u8]> = found
        .iter()
        .filter_map(|frame| match frame {
            Frame::Sql { range, .. } => source.get(range.start as usize..range.end as usize),
            _ => None,
        })
        .collect();
    assert_eq!(sql.len(), 2);
    assert!(sql[0].ends_with(b"END$$"));
    assert!(sql[1].ends_with(b"'x'';y');"));
    assert_eq!(
        found
            .iter()
            .filter(|frame| matches!(frame, Frame::Delimiter { .. }))
            .count(),
        2
    );
    Ok(())
}

#[test]
fn generated_dump_frames_cover_every_source_byte() -> Result<(), Box<dyn std::error::Error>> {
    let source = include_bytes!("../../../it/fixtures/mysql-import/8.4.9/databases.sql");
    let found = frames(source)?;
    let mut previous_end = 0;
    let mut sql_count = 0;
    for frame in &found {
        let range = match frame {
            Frame::Sql { range, .. } => {
                sql_count += 1;
                range
            }
            Frame::Delimiter { range, .. } | Frame::Trivia { range } => range,
        };
        assert_eq!(range.start, previous_end);
        previous_end = range.end;
    }
    assert_eq!(previous_end as usize, source.len());
    assert!(sql_count > 100);
    Ok(())
}

struct RepeatingInsert {
    cursor: usize,
    values: usize,
}

impl Read for RepeatingInsert {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let prefix = b"INSERT INTO t VALUES (1, X'";
        let suffix = b"');";
        let total = prefix.len() + self.values + suffix.len();
        if self.cursor >= total {
            return Ok(0);
        }
        let count = output.len().min(total - self.cursor);
        for (index, byte) in output[..count].iter_mut().enumerate() {
            let position = self.cursor + index;
            *byte = if position < prefix.len() {
                prefix[position]
            } else if position < prefix.len() + self.values {
                b'a'
            } else {
                suffix[position - prefix.len() - self.values]
            };
        }
        self.cursor += count;
        Ok(count)
    }
}

#[test]
fn large_insert_is_framed_without_retaining_values() -> Result<(), Box<dyn std::error::Error>> {
    let values = 8 * 1024 * 1024;
    let mut found = Vec::new();
    scan(RepeatingInsert { cursor: 0, values }, |frame| {
        found.push(frame);
        Ok(())
    })?;
    assert_eq!(found.len(), 1);
    assert!(matches!(&found[0], Frame::Sql { range, .. } if range.end > values as u64));
    Ok(())
}

#[test]
fn unterminated_quote_is_rejected() {
    for source in [
        b"INSERT INTO t VALUES ('unfinished;".as_slice(),
        b"SELECT \"unfinished".as_slice(),
        b"SELECT `unfinished".as_slice(),
        b"/* unfinished".as_slice(),
        b"SELECT 'escaped\\".as_slice(),
    ] {
        assert!(matches!(frames(source), Err(ReaderError::Unterminated)));
    }
}

#[test]
fn slash_delimiter_at_end_of_file_is_recognized() -> Result<(), Box<dyn std::error::Error>> {
    let source = b"DELIMITER //\nCREATE PROCEDURE p() BEGIN SELECT 1; END//";
    let found = frames(source)?;
    assert_eq!(found.len(), 2);
    assert!(
        matches!(&found[1], Frame::Sql { range, delimiter } if range.end as usize == source.len() && delimiter == b"//")
    );
    Ok(())
}

#[test]
fn ordinary_bang_comment_does_not_hide_delimiter_change() -> Result<(), Box<dyn std::error::Error>>
{
    let source = b"/* generated! */\nDELIMITER $$   \nCREATE PROCEDURE p() BEGIN SELECT 1; END$$";
    let found = frames(source)?;
    assert!(matches!(&found[1], Frame::Delimiter { value, .. } if value == b"$$"));
    assert!(matches!(&found[2], Frame::Sql { range, .. } if range.end as usize == source.len()));
    Ok(())
}

#[test]
fn ambiguous_delimiters_fail_preflight() {
    for delimiter in ["--", "/*", "#", "'", "\"", "`", "\\"] {
        let source = format!("DELIMITER {delimiter}\nSELECT 1{delimiter}\n");
        assert!(
            matches!(
                frames(source.as_bytes()),
                Err(ReaderError::InvalidDelimiter { offset: 0 })
            ),
            "{source:?}"
        );
    }
}

#[test]
fn lexer_changing_sql_modes_fail_preflight() {
    for sql_mode in ["NO_BACKSLASH_ESCAPES", "ANSI_QUOTES"] {
        let source = format!("SET sql_mode='{sql_mode}'; SELECT '\\'; DROP DATABASE other;");
        let result = scan(source.as_bytes(), |frame| {
            if let Frame::Sql { range, .. } = frame
                && source[range.start as usize..range.end as usize].contains(sql_mode)
            {
                return Err(ReaderError::UnsupportedSqlMode {
                    offset: range.start,
                });
            }
            Ok(())
        });
        assert!(matches!(
            result,
            Err(ReaderError::UnsupportedSqlMode { offset: 0 })
        ));
    }
}

#[test]
fn sql_mode_words_in_application_data_are_framed_normally() -> Result<(), Box<dyn std::error::Error>>
{
    let source = b"INSERT INTO messages(body) VALUES ('NO_BACKSLASH_ESCAPES');\n-- ANSI_QUOTES\n";
    let found = frames(source)?;
    assert!(matches!(&found[0], Frame::Sql { range, .. } if range.end == 59));
    Ok(())
}

#[test]
fn dash_comment_requires_following_whitespace() -> Result<(), Box<dyn std::error::Error>> {
    let found = frames(b"-- comment;\nSELECT 1;--not-a-comment;\n")?;
    let sql_count = found
        .iter()
        .filter(|frame| matches!(frame, Frame::Sql { .. }))
        .count();
    assert_eq!(sql_count, 2);
    Ok(())
}

#[test]
fn malformed_delimiters_fail_preflight() {
    for source in [
        "DELIMITER \n".to_owned(),
        "DELIMITER a b\n".to_owned(),
        "DELIMITER \\n".to_owned(),
        format!("DELIMITER {}\n", "a".repeat(33)),
        "DELIMITER a\tb\n".to_owned(),
    ] {
        assert!(
            matches!(
                frames(source.as_bytes()),
                Err(ReaderError::InvalidDelimiter { offset: 0 })
            ),
            "{source:?}"
        );
    }
}

#[test]
fn final_sql_without_delimiter_is_framed() -> Result<(), Box<dyn std::error::Error>> {
    let source = b"SELECT 1 -- trailing comment";
    let found = frames(source)?;
    assert!(
        matches!(&found[..], [Frame::Sql { range, delimiter }] if range == &(0..source.len() as u64) && delimiter == b";")
    );
    Ok(())
}

#[test]
fn phpmyadmin_crlf_dump_has_contiguous_frames() -> Result<(), Box<dyn std::error::Error>> {
    let source = include_bytes!("../../../it/fixtures/mysql-import/phpmyadmin-crlf.sql");
    let mut end = 0;
    for frame in frames(source)? {
        let range = match frame {
            Frame::Sql { range, .. } | Frame::Delimiter { range, .. } | Frame::Trivia { range } => {
                range
            }
        };
        assert_eq!(range.start, end);
        end = range.end;
    }
    assert_eq!(end as usize, source.len());
    Ok(())
}

#[test]
fn every_track_dump_has_covered_ranges() -> Result<(), Box<dyn std::error::Error>> {
    for (version, source) in [
        (
            "8.0.46",
            include_bytes!("../../../it/fixtures/mysql-import/8.0.46/databases.sql").as_slice(),
        ),
        (
            "8.4.9",
            include_bytes!("../../../it/fixtures/mysql-import/8.4.9/databases.sql").as_slice(),
        ),
        (
            "9.7.0",
            include_bytes!("../../../it/fixtures/mysql-import/9.7.0/databases.sql").as_slice(),
        ),
    ] {
        let mut last_end = 0;
        scan(source, |frame| {
            let range = match frame {
                Frame::Sql { range, .. }
                | Frame::Delimiter { range, .. }
                | Frame::Trivia { range } => range,
            };
            assert_eq!(range.start, last_end, "{version}");
            last_end = range.end;
            Ok(())
        })?;
        assert_eq!(last_end as usize, source.len());
    }
    Ok(())
}
