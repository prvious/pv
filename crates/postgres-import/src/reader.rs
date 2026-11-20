use std::io::{BufReader, Read};

use pg_query::protobuf::Token;
use pg_query::protobuf::node::Node;
use thiserror::Error;

const MAX_SQL_STATEMENT_BYTES: usize = 8 * 1024 * 1024;
const MAX_META_COMMAND_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegularDumpFormat {
    Plain,
    Custom,
    Tar,
}

/// Detects a regular dump file by its leading bytes, independent of its name.
pub fn detect_regular_dump_format(mut input: impl Read) -> Result<RegularDumpFormat, ImportError> {
    let mut header = [0; 512];
    let mut length = 0;
    while length < header.len() {
        let count = input.read(&mut header[length..])?;
        if count == 0 {
            break;
        }
        length += count;
    }
    if header[..length].starts_with(b"PGDMP") {
        Ok(RegularDumpFormat::Custom)
    } else if length >= 262 && &header[257..262] == b"ustar" {
        Ok(RegularDumpFormat::Tar)
    } else {
        Ok(RegularDumpFormat::Plain)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct DumpSummary {
    pub statements: usize,
    pub meta_commands: usize,
    pub copy_rows: usize,
    pub connections: Vec<String>,
    pub encodings: Vec<String>,
}

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("failed to read PostgreSQL dump: {0}")]
    Io(#[from] std::io::Error),
    #[error("SQL statement beginning on line {line} exceeds {MAX_SQL_STATEMENT_BYTES} bytes")]
    StatementTooLarge { line: usize },
    #[error("psql command on line {line} exceeds {MAX_META_COMMAND_BYTES} bytes")]
    MetaCommandTooLarge { line: usize },
    #[error("dump contains non-UTF-8 SQL near line {line}")]
    NonUtf8Sql { line: usize },
    #[error("invalid PostgreSQL SQL on line {line}: {message}")]
    Sql { line: usize, message: String },
    #[error("unsupported psql command on line {line}: {command}")]
    MetaCommand { line: usize, command: String },
    #[error("psql command on line {line} interrupts an unfinished SQL statement")]
    MetaInsideStatement { line: usize },
    #[error("COPY FROM STDIN beginning on line {line} has no \\. terminator")]
    UnterminatedCopy { line: usize },
    #[error("SQL statement beginning on line {line} is incomplete")]
    IncompleteSql { line: usize },
    #[error("COPY from a server file or program is unsupported on line {line}")]
    UnsafeCopy { line: usize },
    #[error("unsupported COPY mode or options on line {line}")]
    UnsupportedCopy { line: usize },
    #[error("psql restriction key on line {line} is missing or mismatched")]
    RestrictionKey { line: usize },
    #[error("psql restriction beginning before line {line} is not closed")]
    UnterminatedRestriction { line: usize },
}

#[derive(Default)]
enum Lex {
    #[default]
    Normal,
    LineComment,
    BlockComment {
        depth: usize,
        previous: u8,
    },
    Single {
        escape: bool,
        escaped_next: bool,
    },
    SingleQuotePending {
        escape: bool,
    },
    Double,
    DoubleQuotePending,
    DollarOpening(Vec<u8>),
    Dollar(Vec<u8>),
}

#[derive(Default)]
struct CopyLine {
    length: usize,
    prefix: [u8; 4],
}

impl CopyLine {
    fn push(&mut self, byte: u8) -> bool {
        if self.length < self.prefix.len() {
            self.prefix[self.length] = byte;
        }
        self.length += 1;
        if byte != b'\n' {
            return false;
        }

        let terminator = (self.length == 3 && &self.prefix[..3] == b"\\.\n")
            || (self.length == 4 && &self.prefix == b"\\.\r\n");
        self.length = 0;
        self.prefix = [0; 4];
        terminator
    }
}

/// Inspects a plain `pg_dump` or `pg_dumpall` script without executing it.
///
/// This checks framing and syntax only. Callers must apply a separate SQL policy
/// and target mapping before treating the script as executable.
pub fn inspect_plain_dump(input: impl Read) -> Result<DumpSummary, ImportError> {
    let mut input = BufReader::new(input);
    let mut summary = DumpSummary::default();
    let mut statement = Vec::new();
    let mut meta = Vec::new();
    let mut lex = Lex::Normal;
    let mut line = 1;
    let mut statement_line = 1;
    let mut meta_line = 1;
    let mut at_line_start = true;
    let mut in_meta = false;
    let mut copy_header_end = false;
    let mut copy_line = None::<CopyLine>;
    let mut copy_start_line = 0;
    let mut restriction = None::<String>;

    loop {
        let mut byte = [0];
        if input.read(&mut byte)? == 0 {
            break;
        }
        let byte = byte[0];

        if copy_header_end {
            if byte == b'\n' {
                copy_header_end = false;
                copy_line = Some(CopyLine::default());
            } else if !byte.is_ascii_whitespace() {
                return Err(ImportError::IncompleteSql {
                    line: copy_start_line,
                });
            }
        } else if let Some(copy) = copy_line.as_mut() {
            if copy.push(byte) {
                copy_line = None;
            } else if byte == b'\n' {
                summary.copy_rows += 1;
            }
        } else if in_meta {
            meta.push(byte);
            if meta.len() > MAX_META_COMMAND_BYTES {
                return Err(ImportError::MetaCommandTooLarge { line: meta_line });
            }
            if byte == b'\n' {
                inspect_meta(&meta, meta_line, &mut summary, &mut restriction)?;
                meta.clear();
                in_meta = false;
            }
        } else if at_line_start && byte == b'\\' && matches!(lex, Lex::Normal) {
            if !parse_statement(&statement, statement_line)?
                .protobuf
                .stmts
                .is_empty()
            {
                return Err(ImportError::MetaInsideStatement { line });
            }
            statement.clear();
            meta_line = line;
            meta.push(byte);
            in_meta = true;
        } else {
            if statement.is_empty() {
                if byte.is_ascii_whitespace() {
                    if byte == b'\n' {
                        line += 1;
                        at_line_start = true;
                    }
                    continue;
                }
                statement_line = line;
            }
            statement.push(byte);
            if statement.len() > MAX_SQL_STATEMENT_BYTES {
                return Err(ImportError::StatementTooLarge {
                    line: statement_line,
                });
            }
            if scan_sql_byte(byte, &statement, &mut lex) {
                let sql = std::str::from_utf8(&statement).map_err(|_| ImportError::NonUtf8Sql {
                    line: statement_line,
                })?;
                let parsed = match pg_query::parse(sql, 0) {
                    Ok(parsed) => parsed,
                    // A semicolon can separate commands inside a SQL-standard function body.
                    Err(pg_query::Error::Parse(message))
                        if message == "syntax error at end of input" =>
                    {
                        at_line_start = false;
                        continue;
                    }
                    Err(error) => return Err(sql_error(error, statement_line)),
                };
                for raw in &parsed.protobuf.stmts {
                    summary.statements += 1;
                    if let Some(node) = &raw.stmt
                        && let Some(Node::CopyStmt(copy)) = &node.node
                    {
                        if copy.is_program || !copy.filename.is_empty() {
                            return Err(ImportError::UnsafeCopy {
                                line: statement_line,
                            });
                        }
                        if !copy.is_from
                            || copy.relation.is_none()
                            || copy.query.is_some()
                            || !copy.options.is_empty()
                            || copy.where_clause.is_some()
                        {
                            return Err(ImportError::UnsupportedCopy {
                                line: statement_line,
                            });
                        }
                        let tokens = pg_query::scan(sql)
                            .map_err(|error| sql_error(error, statement_line))?;
                        let mut tokens = tokens.tokens.iter().rev().filter_map(|token| {
                            Token::try_from(token.token).ok().filter(|kind| {
                                !matches!(
                                    kind,
                                    Token::Ascii59 | Token::SqlComment | Token::CComment
                                )
                            })
                        });
                        if tokens.next() != Some(Token::Stdin) {
                            return Err(ImportError::UnsafeCopy {
                                line: statement_line,
                            });
                        }
                        copy_header_end = true;
                        copy_start_line = statement_line;
                    }
                }
                statement.clear();
            }
        }

        if byte == b'\n' {
            line += 1;
            at_line_start = true;
        } else if at_line_start && !byte.is_ascii_whitespace() {
            at_line_start = false;
        }
    }

    if in_meta {
        inspect_meta(&meta, meta_line, &mut summary, &mut restriction)?;
    }
    if copy_header_end || copy_line.is_some() {
        return Err(ImportError::UnterminatedCopy {
            line: copy_start_line,
        });
    }
    if restriction.is_some() {
        return Err(ImportError::UnterminatedRestriction { line });
    }
    if !parse_statement(&statement, statement_line)?
        .protobuf
        .stmts
        .is_empty()
    {
        return Err(ImportError::IncompleteSql {
            line: statement_line,
        });
    }

    Ok(summary)
}

fn parse_statement(statement: &[u8], line: usize) -> Result<pg_query::ParseResult, ImportError> {
    let sql = std::str::from_utf8(statement).map_err(|_| ImportError::NonUtf8Sql { line })?;
    pg_query::parse(sql, 0).map_err(|error| sql_error(error, line))
}

fn sql_error(error: pg_query::Error, line: usize) -> ImportError {
    ImportError::Sql {
        line,
        message: error.to_string(),
    }
}

fn identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || !byte.is_ascii()
}

fn identifier_continue(byte: u8) -> bool {
    identifier_start(byte) || byte.is_ascii_digit() || byte == b'$'
}

fn scan_sql_byte(byte: u8, statement: &[u8], lex: &mut Lex) -> bool {
    loop {
        match lex {
            Lex::Normal => {
                let previous = statement.iter().rev().nth(1).copied();
                match (previous, byte) {
                    (Some(b'-'), b'-') => *lex = Lex::LineComment,
                    (Some(b'/'), b'*') => {
                        *lex = Lex::BlockComment {
                            depth: 1,
                            previous: 0,
                        };
                    }
                    (_, b'\'') => {
                        let escape = matches!(previous, Some(b'E' | b'e'))
                            && !statement
                                .iter()
                                .rev()
                                .nth(2)
                                .copied()
                                .is_some_and(identifier_continue);
                        *lex = Lex::Single {
                            escape,
                            escaped_next: false,
                        };
                    }
                    (_, b'"') => *lex = Lex::Double,
                    (_, b'$') if !previous.is_some_and(identifier_continue) => {
                        *lex = Lex::DollarOpening(vec![b'$']);
                    }
                    (_, b';') => return true,
                    _ => {}
                }
            }
            Lex::LineComment => {
                if byte == b'\n' {
                    *lex = Lex::Normal;
                }
            }
            Lex::BlockComment { depth, previous } => {
                if *previous == b'/' && byte == b'*' {
                    *depth += 1;
                    *previous = 0;
                } else if *previous == b'*' && byte == b'/' {
                    *depth -= 1;
                    if *depth == 0 {
                        *lex = Lex::Normal;
                    } else {
                        *previous = 0;
                    }
                } else {
                    *previous = byte;
                }
            }
            Lex::Single {
                escape,
                escaped_next,
            } => {
                if *escaped_next {
                    *escaped_next = false;
                } else if *escape && byte == b'\\' {
                    *escaped_next = true;
                } else if byte == b'\'' {
                    *lex = Lex::SingleQuotePending { escape: *escape };
                }
            }
            Lex::SingleQuotePending { escape } => {
                if byte == b'\'' {
                    *lex = Lex::Single {
                        escape: *escape,
                        escaped_next: false,
                    };
                } else {
                    *lex = Lex::Normal;
                    continue;
                }
            }
            Lex::Double => {
                if byte == b'"' {
                    *lex = Lex::DoubleQuotePending;
                }
            }
            Lex::DoubleQuotePending => {
                if byte == b'"' {
                    *lex = Lex::Double;
                } else {
                    *lex = Lex::Normal;
                    continue;
                }
            }
            Lex::DollarOpening(delimiter) => {
                if byte == b'$' {
                    delimiter.push(byte);
                    *lex = Lex::Dollar(std::mem::take(delimiter));
                } else if identifier_start(byte) || (delimiter.len() > 1 && byte.is_ascii_digit()) {
                    delimiter.push(byte);
                } else {
                    *lex = Lex::Normal;
                    continue;
                }
            }
            Lex::Dollar(delimiter) => {
                if byte == b'$' && statement.ends_with(delimiter) {
                    *lex = Lex::Normal;
                }
            }
        }
        return false;
    }
}

fn inspect_meta(
    meta: &[u8],
    line: usize,
    summary: &mut DumpSummary,
    restriction: &mut Option<String>,
) -> Result<(), ImportError> {
    let command = std::str::from_utf8(meta)
        .map_err(|_| ImportError::NonUtf8Sql { line })?
        .trim_end_matches(['\r', '\n']);
    let mut parts = command.split_whitespace();
    let Some(name) = parts.next() else {
        return Err(ImportError::MetaCommand {
            line,
            command: command.to_string(),
        });
    };
    match name {
        "\\restrict" => {
            let key = parts.next().filter(|key| {
                !key.is_empty() && key.bytes().all(|byte| byte.is_ascii_alphanumeric())
            });
            if key.is_none() || parts.next().is_some() || restriction.is_some() {
                return Err(ImportError::RestrictionKey { line });
            }
            *restriction = key.map(str::to_string);
        }
        "\\unrestrict" => {
            let key = parts.next();
            if key.is_none() || parts.next().is_some() || key != restriction.as_deref() {
                return Err(ImportError::RestrictionKey { line });
            }
            *restriction = None;
        }
        "\\encoding" => {
            if restriction.is_some() {
                return Err(ImportError::MetaCommand {
                    line,
                    command: command.to_string(),
                });
            }
            let Some(encoding) = parts.next() else {
                return Err(ImportError::MetaCommand {
                    line,
                    command: command.to_string(),
                });
            };
            if !matches!(encoding, "SQL_ASCII" | "UTF8") || parts.next().is_some() {
                return Err(ImportError::MetaCommand {
                    line,
                    command: command.to_string(),
                });
            }
            summary.encodings.push(encoding.to_string());
        }
        "\\connect" => {
            if restriction.is_some() {
                return Err(ImportError::MetaCommand {
                    line,
                    command: command.to_string(),
                });
            }
            let rest = command.strip_prefix(name).unwrap_or_default().trim();
            let Some(database) = connect_database(rest) else {
                return Err(ImportError::MetaCommand {
                    line,
                    command: command.to_string(),
                });
            };
            summary.connections.push(database);
        }
        _ => {
            return Err(ImportError::MetaCommand {
                line,
                command: command.to_string(),
            });
        }
    }
    summary.meta_commands += 1;
    Ok(())
}

// pg_dump quotes complex names twice: as a single libpq dbname value and
// then as a psql identifier. Accept only that generated connection shape.
fn connect_database(target: &str) -> Option<String> {
    let database = if let Some(argument) = target.strip_prefix("-reuse-previous=on ") {
        let conninfo = unquote_psql_identifier(argument)?;
        let name = conninfo.strip_prefix("dbname='")?.strip_suffix('\'')?;
        let mut chars = name.chars();
        let mut database = String::new();
        while let Some(character) = chars.next() {
            database.push(match character {
                '\\' => chars.next().filter(|next| matches!(next, '\\' | '\''))?,
                '\'' => return None,
                other => other,
            });
        }
        database
    } else {
        let name = if target.starts_with('"') {
            unquote_psql_identifier(target)?
        } else {
            target.to_owned()
        };
        if !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.'))
        {
            return None;
        }
        name
    };
    (!database.is_empty() && !database.contains(['\n', '\r'])).then_some(database)
}

fn unquote_psql_identifier(argument: &str) -> Option<String> {
    let mut chars = argument.strip_prefix('"')?.strip_suffix('"')?.chars();
    let mut value = String::new();
    while let Some(character) = chars.next() {
        if character == '"' && chars.next() != Some('"') {
            return None;
        }
        value.push(character);
    }
    Some(value)
}
