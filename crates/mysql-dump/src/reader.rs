use std::io::{self, BufReader, Read};
use std::ops::Range;

use thiserror::Error;

const MAX_DIRECTIVE_LINE: usize = 128;

/// Byte range of a SQL statement or MySQL client delimiter directive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    Sql {
        range: Range<u64>,
        delimiter: Vec<u8>,
    },
    Delimiter {
        range: Range<u64>,
        value: Vec<u8>,
    },
    Trivia {
        range: Range<u64>,
    },
}

/// Dump framing error. No SQL is safe to execute after this error.
#[derive(Debug, Error)]
pub enum ReaderError {
    #[error("could not read dump: {0}")]
    Io(#[from] io::Error),
    #[error("invalid DELIMITER directive at byte {offset}")]
    InvalidDelimiter { offset: u64 },
    #[error("unterminated quoted string or comment at end of dump")]
    Unterminated,
    #[error("SQL mode changing dump lexer rules is unsupported at byte {offset}")]
    UnsupportedSqlMode { offset: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Normal,
    Single,
    Double,
    Backtick,
    QuotedAfter(u8),
    Escape(u8),
    Dash,
    DoubleDash,
    Slash,
    LineComment,
    BlockFirst,
    BlockComment,
    BlockStar,
}

struct Scanner<F> {
    callback: F,
    state: State,
    delimiter: Vec<u8>,
    tail: Vec<u8>,
    start: u64,
    has_code: bool,
    line_start: bool,
}

/// Frame a dump without retaining a complete statement in memory.
///
/// The callback receives source byte ranges. Callers can parse small statement
/// ranges and stream long `INSERT` values from the original file in a second
/// pass. The callback must reject `SET` statements that change string/identifier
/// lexing (notably `NO_BACKSLASH_ESCAPES` and `ANSI_QUOTES`) before scanning the
/// next statement. This reader does not classify SQL effects or authorize execution.
pub fn scan<R: Read, F: FnMut(Frame) -> Result<(), ReaderError>>(
    input: R,
    callback: F,
) -> Result<(), ReaderError> {
    let mut scanner = Scanner {
        callback,
        state: State::Normal,
        delimiter: vec![b';'],
        tail: Vec::new(),
        start: 0,
        has_code: false,
        line_start: true,
    };
    let mut reader = BufReader::new(input);
    let mut byte = [0_u8; 1];
    let mut offset = 0_u64;
    let mut directive_candidate = Vec::with_capacity(MAX_DIRECTIVE_LINE);
    let mut candidate_start = 0_u64;
    let mut candidate_active = false;
    let mut skip_candidate_until_newline = false;

    loop {
        if reader.read(&mut byte)? == 0 {
            break;
        }
        let current = byte[0];
        if !candidate_active
            && !skip_candidate_until_newline
            && scanner.line_start
            && scanner.state == State::Normal
            && !scanner.has_code
        {
            candidate_active = true;
            candidate_start = offset;
        }
        if candidate_active {
            directive_candidate.push(current);
            if current == b'\n' || directive_candidate.len() == MAX_DIRECTIVE_LINE {
                if let Some(value) = parse_delimiter(&directive_candidate, candidate_start)? {
                    if scanner.start < candidate_start {
                        (scanner.callback)(Frame::Trivia {
                            range: scanner.start..candidate_start,
                        })?;
                    }
                    scanner.delimiter = value.clone();
                    scanner.start = offset + 1;
                    scanner.line_start = true;
                    scanner.tail.clear();
                    (scanner.callback)(Frame::Delimiter {
                        range: candidate_start..offset + 1,
                        value,
                    })?;
                } else {
                    for (index, candidate_byte) in directive_candidate.iter().enumerate() {
                        scanner.feed(*candidate_byte, candidate_start + index as u64)?;
                    }
                    skip_candidate_until_newline = current != b'\n';
                }
                directive_candidate.clear();
                candidate_active = false;
            }
        } else {
            scanner.feed(current, offset)?;
            if current == b'\n' {
                skip_candidate_until_newline = false;
            }
        }
        offset += 1;
    }

    if candidate_active {
        if let Some(value) = parse_delimiter(&directive_candidate, candidate_start)? {
            if scanner.start < candidate_start {
                (scanner.callback)(Frame::Trivia {
                    range: scanner.start..candidate_start,
                })?;
            }
            scanner.delimiter = value.clone();
            scanner.start = offset;
            (scanner.callback)(Frame::Delimiter {
                range: candidate_start..offset,
                value,
            })?;
        } else {
            for (index, candidate_byte) in directive_candidate.iter().enumerate() {
                scanner.feed(*candidate_byte, candidate_start + index as u64)?;
            }
        }
    }
    match scanner.state {
        State::Dash | State::Slash => {
            scanner.push_tail(
                if scanner.state == State::Dash {
                    b'-'
                } else {
                    b'/'
                },
                offset - 1,
            )?;
            scanner.state = State::Normal;
        }
        State::DoubleDash => {
            scanner.push_tail(b'-', offset - 2)?;
            scanner.push_tail(b'-', offset - 1)?;
            scanner.state = State::Normal;
        }
        _ => {}
    }
    match scanner.state {
        State::Single
        | State::Double
        | State::Backtick
        | State::Escape(_)
        | State::BlockFirst
        | State::BlockComment
        | State::BlockStar => return Err(ReaderError::Unterminated),
        _ => {}
    }
    if scanner.start < offset {
        let range = scanner.start..offset;
        if scanner.has_code {
            (scanner.callback)(Frame::Sql {
                range,
                delimiter: scanner.delimiter,
            })?;
        } else {
            (scanner.callback)(Frame::Trivia { range })?;
        }
    }
    Ok(())
}

fn parse_delimiter(line: &[u8], offset: u64) -> Result<Option<Vec<u8>>, ReaderError> {
    let trimmed = line.strip_suffix(b"\n").unwrap_or(line);
    let trimmed = trimmed.strip_suffix(b"\r").unwrap_or(trimmed);
    let trimmed = trimmed
        .iter()
        .copied()
        .skip_while(|byte| *byte == b' ' || *byte == b'\t');
    let content: Vec<u8> = trimmed.collect();
    if content.len() < 10 || !content[..9].eq_ignore_ascii_case(b"DELIMITER") {
        return Ok(None);
    }
    if content[9] != b' ' && content[9] != b'\t' {
        return Ok(None);
    }
    let mut value: Vec<u8> = content[10..]
        .iter()
        .copied()
        .skip_while(|byte| *byte == b' ' || *byte == b'\t')
        .collect();
    while value
        .last()
        .is_some_and(|byte| *byte == b' ' || *byte == b'\t')
    {
        value.pop();
    }
    if value.is_empty()
        || value.len() > 32
        || value.iter().any(|byte| {
            !byte.is_ascii_graphic() || matches!(*byte, b'\\' | b'#' | b'\'' | b'"' | b'`')
        })
        || value.windows(2).any(|pair| pair == b"--" || pair == b"/*")
    {
        return Err(ReaderError::InvalidDelimiter { offset });
    }
    Ok(Some(value))
}

impl<F: FnMut(Frame) -> Result<(), ReaderError>> Scanner<F> {
    fn feed(&mut self, byte: u8, offset: u64) -> Result<(), ReaderError> {
        let mut again = true;
        while again {
            again = false;
            match self.state {
                State::Normal => match byte {
                    b'\'' => self.open_quote(State::Single),
                    b'"' => self.open_quote(State::Double),
                    b'`' => self.open_quote(State::Backtick),
                    b'-' => self.state = State::Dash,
                    b'/' => self.state = State::Slash,
                    b'#' => self.open_comment(State::LineComment),
                    _ => {
                        if !byte.is_ascii_whitespace() {
                            self.has_code = true;
                        }
                        self.push_tail(byte, offset)?;
                    }
                },
                State::Dash => {
                    if byte == b'-' {
                        self.state = State::DoubleDash;
                    } else {
                        self.has_code = true;
                        self.state = State::Normal;
                        self.push_tail(b'-', offset - 1)?;
                        again = true;
                    }
                }
                State::DoubleDash => {
                    if byte.is_ascii_whitespace() {
                        self.open_comment(State::LineComment);
                        if byte == b'\n' {
                            self.state = State::Normal;
                        }
                    } else {
                        self.has_code = true;
                        self.state = State::Normal;
                        self.push_tail(b'-', offset - 2)?;
                        self.push_tail(b'-', offset - 1)?;
                        again = true;
                    }
                }
                State::Slash => {
                    if byte == b'*' {
                        self.open_comment(State::BlockFirst);
                    } else {
                        self.has_code = true;
                        self.state = State::Normal;
                        self.push_tail(b'/', offset - 1)?;
                        again = true;
                    }
                }
                State::LineComment => {
                    if byte == b'\n' {
                        self.state = State::Normal;
                    }
                }
                State::BlockFirst => {
                    if byte == b'!' {
                        self.has_code = true;
                    }
                    self.state = if byte == b'*' {
                        State::BlockStar
                    } else {
                        State::BlockComment
                    };
                }
                State::BlockComment => {
                    if byte == b'*' {
                        self.state = State::BlockStar;
                    }
                }
                State::BlockStar => {
                    if byte == b'/' {
                        self.state = State::Normal;
                    } else if byte != b'*' {
                        self.state = State::BlockComment;
                    }
                }
                State::Single | State::Double | State::Backtick => {
                    let marker = match self.state {
                        State::Single => b'\'',
                        State::Double => b'"',
                        _ => b'`',
                    };
                    if byte == b'\\' && marker != b'`' {
                        self.state = State::Escape(marker);
                    } else if byte == marker {
                        self.state = State::QuotedAfter(marker);
                    }
                }
                State::Escape(marker) => {
                    self.state = match marker {
                        b'\'' => State::Single,
                        b'"' => State::Double,
                        _ => State::Backtick,
                    };
                }
                State::QuotedAfter(marker) => {
                    if byte == marker {
                        self.state = match marker {
                            b'\'' => State::Single,
                            b'"' => State::Double,
                            _ => State::Backtick,
                        };
                    } else {
                        self.state = State::Normal;
                        again = true;
                    }
                }
            }
        }
        self.line_start = byte == b'\n';
        Ok(())
    }

    fn open_quote(&mut self, state: State) {
        self.has_code = true;
        self.tail.clear();
        self.state = state;
    }

    fn open_comment(&mut self, state: State) {
        self.tail.clear();
        self.state = state;
    }

    fn push_tail(&mut self, byte: u8, offset: u64) -> Result<(), ReaderError> {
        self.tail.push(byte);
        if self.tail.len() > self.delimiter.len() {
            self.tail.remove(0);
        }
        if self.tail == self.delimiter {
            (self.callback)(Frame::Sql {
                range: self.start..offset + 1,
                delimiter: self.delimiter.clone(),
            })?;
            self.start = offset + 1;
            self.has_code = false;
            self.tail.clear();
        }
        Ok(())
    }
}
