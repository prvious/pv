use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::ops::Range;

use thiserror::Error;

use crate::policy::validate_statement_effects;
use crate::reference::{StatementReferences, statement_references};

const MAX_PREFIX_BYTES: u64 = 64 * 1024;

#[derive(Debug, Error)]
pub enum InsertError {
    #[error("could not inspect large INSERT: {0}")]
    Io(#[from] io::Error),
    #[error("large statement is not a supported literal-only INSERT")]
    Unsupported,
}

/// Validate a large `INSERT ... VALUES` without retaining its data. The caller
/// must check every returned database reference against the target map.
pub(crate) fn large_insert_references<S: Read + Seek>(
    source: &mut S,
    frame: Range<u64>,
    delimiter: &[u8],
) -> Result<(String, StatementReferences), InsertError> {
    if delimiter != b";" {
        return Err(InsertError::Unsupported);
    }
    let length = frame
        .end
        .checked_sub(frame.start)
        .ok_or(InsertError::Unsupported)?;
    source.seek(SeekFrom::Start(frame.start))?;
    let mut preview = Vec::new();
    source
        .take(length.min(MAX_PREFIX_BYTES))
        .read_to_end(&mut preview)?;
    let mut selected = None;
    for (index, keyword) in preview.windows(6).enumerate() {
        if !keyword.eq_ignore_ascii_case(b"VALUES") {
            continue;
        }
        if index > 0 && identifier_byte(preview[index - 1]) {
            continue;
        }
        if preview
            .get(index + 6)
            .is_some_and(|byte| identifier_byte(*byte))
        {
            continue;
        }
        let Some(prefix) = preview
            .get(..index + 6)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
        else {
            continue;
        };
        let synthetic = format!("{prefix} (NULL);");
        if validate_statement_effects(&synthetic).is_ok()
            && let Ok(references) = statement_references(&synthetic)
        {
            selected = Some((prefix.to_owned(), references));
            break;
        }
    }
    let (prefix, references) = selected.ok_or(InsertError::Unsupported)?;
    let data_start = frame.start + prefix.len() as u64;
    source.seek(SeekFrom::Start(data_start))?;
    let remaining = frame.end - data_start;
    let mut values = ValueReader::new(source.take(remaining));
    values.validate()?;
    Ok((prefix, references))
}

fn identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

struct ValueReader<R: Read> {
    input: BufReader<R>,
    pending: Option<u8>,
}

impl<R: Read> ValueReader<R> {
    fn new(input: R) -> Self {
        Self {
            input: BufReader::new(input),
            pending: None,
        }
    }

    fn peek(&mut self) -> Result<Option<u8>, InsertError> {
        if self.pending.is_none() {
            let mut byte = [0];
            if self.input.read(&mut byte)? != 0 {
                self.pending = Some(byte[0]);
            }
        }
        Ok(self.pending)
    }

    fn next(&mut self) -> Result<Option<u8>, InsertError> {
        let value = self.peek()?;
        self.pending = None;
        Ok(value)
    }

    fn whitespace(&mut self) -> Result<(), InsertError> {
        while self.peek()?.is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.next()?;
        }
        Ok(())
    }

    fn expect(&mut self, expected: u8) -> Result<(), InsertError> {
        if self.next()? == Some(expected) {
            Ok(())
        } else {
            Err(InsertError::Unsupported)
        }
    }

    fn validate(&mut self) -> Result<(), InsertError> {
        self.whitespace()?;
        loop {
            self.expect(b'(')?;
            self.whitespace()?;
            loop {
                self.literal()?;
                self.whitespace()?;
                match self.next()? {
                    Some(b',') => self.whitespace()?,
                    Some(b')') => break,
                    _ => return Err(InsertError::Unsupported),
                }
            }
            self.whitespace()?;
            match self.next()? {
                Some(b',') => self.whitespace()?,
                Some(b';') => {
                    return if self.next()?.is_none() {
                        Ok(())
                    } else {
                        Err(InsertError::Unsupported)
                    };
                }
                _ => return Err(InsertError::Unsupported),
            }
        }
    }

    fn literal(&mut self) -> Result<(), InsertError> {
        match self.peek()? {
            Some(b'\'' | b'"') => {
                let quote = self.next()?.ok_or(InsertError::Unsupported)?;
                self.quoted(quote)
            }
            Some(b'+' | b'-' | b'.' | b'0'..=b'9') => self.number(),
            Some(byte) if byte.is_ascii_alphabetic() || byte == b'_' => self.word(),
            _ => Err(InsertError::Unsupported),
        }
    }

    fn quoted(&mut self, quote: u8) -> Result<(), InsertError> {
        loop {
            match self.next()? {
                Some(b'\\') => {
                    self.next()?.ok_or(InsertError::Unsupported)?;
                }
                Some(byte) if byte == quote => {
                    if self.peek()? == Some(quote) {
                        self.next()?;
                    } else {
                        return Ok(());
                    }
                }
                Some(0) | None => return Err(InsertError::Unsupported),
                Some(_) => {}
            }
        }
    }

    fn word(&mut self) -> Result<(), InsertError> {
        let mut word = Vec::new();
        while self.peek()?.is_some_and(identifier_byte) {
            if word.len() >= 32 {
                return Err(InsertError::Unsupported);
            }
            word.push(self.next()?.ok_or(InsertError::Unsupported)?);
        }
        if [b"NULL".as_slice(), b"TRUE", b"FALSE", b"DEFAULT"]
            .iter()
            .any(|candidate| word.eq_ignore_ascii_case(candidate))
        {
            return Ok(());
        }
        if [
            b"_binary".as_slice(),
            b"_utf8mb4",
            b"_utf8mb3",
            b"_latin1",
            b"x",
            b"b",
        ]
        .iter()
        .any(|candidate| word.eq_ignore_ascii_case(candidate))
            && let Some(quote @ (b'\'' | b'"')) = self.next()?
        {
            return self.quoted(quote);
        }
        Err(InsertError::Unsupported)
    }

    fn digits(&mut self) -> Result<usize, InsertError> {
        let mut count = 0;
        while self.peek()?.is_some_and(|byte| byte.is_ascii_digit()) {
            self.next()?;
            count += 1;
        }
        Ok(count)
    }

    fn number(&mut self) -> Result<(), InsertError> {
        if self
            .peek()?
            .is_some_and(|byte| byte == b'+' || byte == b'-')
        {
            self.next()?;
        }
        let first_digit = self.peek()?;
        let integer_digits = self.digits()?;
        if integer_digits == 1
            && first_digit == Some(b'0')
            && self
                .peek()?
                .is_some_and(|byte| byte == b'x' || byte == b'X')
        {
            self.next()?;
            let mut digits = 0;
            while self.peek()?.is_some_and(|byte| byte.is_ascii_hexdigit()) {
                self.next()?;
                digits += 1;
            }
            return if digits > 0 {
                Ok(())
            } else {
                Err(InsertError::Unsupported)
            };
        }
        let fraction_digits = if self.peek()? == Some(b'.') {
            self.next()?;
            self.digits()?
        } else {
            0
        };
        if integer_digits + fraction_digits == 0 {
            return Err(InsertError::Unsupported);
        }
        if self
            .peek()?
            .is_some_and(|byte| byte == b'e' || byte == b'E')
        {
            self.next()?;
            if self
                .peek()?
                .is_some_and(|byte| byte == b'+' || byte == b'-')
            {
                self.next()?;
            }
            if self.digits()? == 0 {
                return Err(InsertError::Unsupported);
            }
        }
        Ok(())
    }
}
