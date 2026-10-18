//! `csv.reader(open(path, encoding="utf-8-sig", newline=""))`, record by record.
//!
//! The order of the engine's errors depends on when Python decoded each part
//! of a file: `TextIOWrapper` reads 8,192 bytes at a time and decodes each
//! chunk as it is read, so an invalid byte stops the load as soon as its
//! chunk is reached, before or after rows that would themselves be refused,
//! and the reported position is relative to that chunk. The reader here
//! reproduces that timing, then parses records with the `excel` dialect state
//! machine of CPython's `_csv` module.

use std::io::Read;

use crate::py::{DecodeError, SigDecoder};

const CHUNK: usize = 8192;
/// `csv.field_size_limit()`'s default.
pub const FIELD_LIMIT: usize = 128 * 1024;

/// Why the next record could not be read.
#[derive(Debug)]
pub enum CsvError {
    Decode(DecodeError),
    Io(std::io::Error),
    /// `_csv.Error`, which the Python engine did not catch.
    Csv(String),
}

/// Lines as text-mode iteration with `newline=""` yields them, terminators kept.
struct Lines<R> {
    source: R,
    decoder: SigDecoder,
    pending_cr: bool,
    decoded: Vec<char>,
    pos: usize,
    eof: bool,
}

impl<R: Read> Lines<R> {
    fn read_chunk(&mut self) -> Result<bool, CsvError> {
        if self.eof {
            return Ok(false);
        }
        let mut buf = vec![0u8; CHUNK];
        let mut filled = 0;
        // A regular file returns the whole request unless it ends first.
        while filled < CHUNK {
            match self.source.read(&mut buf[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(CsvError::Io(e)),
            }
        }
        let last = filled == 0;
        let mut output = self
            .decoder
            .decode(&buf[..filled], last)
            .map_err(CsvError::Decode)?;
        // IncrementalNewlineDecoder holds back a trailing '\r' until it knows
        // whether '\n' follows.
        if self.pending_cr && (!output.is_empty() || last) {
            output.insert(0, '\r');
            self.pending_cr = false;
        }
        if output.ends_with('\r') && !last {
            output.pop();
            self.pending_cr = true;
        }
        if self.pos > 0 {
            self.decoded.drain(..self.pos);
            self.pos = 0;
        }
        self.decoded.extend(output.chars());
        if last {
            self.eof = true;
        }
        Ok(true)
    }

    fn next_line(&mut self) -> Result<Option<Vec<char>>, CsvError> {
        let mut search = self.pos;
        loop {
            let found = (search..self.decoded.len())
                .find(|&i| self.decoded[i] == '\n' || self.decoded[i] == '\r');
            if let Some(i) = found {
                let mut end = i + 1;
                if self.decoded[i] == '\r' && self.decoded.get(i + 1) == Some(&'\n') {
                    end += 1;
                }
                let line = self.decoded[self.pos..end].to_vec();
                self.pos = end;
                return Ok(Some(line));
            }
            let consumed = self.pos;
            search = self.decoded.len() - consumed;
            if !self.read_chunk()? {
                let line = self.decoded[self.pos..].to_vec();
                self.pos = self.decoded.len();
                return Ok((!line.is_empty()).then_some(line));
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    StartRecord,
    StartField,
    InField,
    InQuotedField,
    QuoteInQuotedField,
    EatCrnl,
}

/// The records of one CSV file.
pub struct Reader<R> {
    lines: Lines<R>,
    state: State,
    field: String,
    field_len: usize,
    fields: Vec<String>,
}

impl<R: Read> Reader<R> {
    pub fn new(source: R) -> Reader<R> {
        Reader {
            lines: Lines {
                source,
                decoder: SigDecoder::default(),
                pending_cr: false,
                decoded: Vec::new(),
                pos: 0,
                eof: false,
            },
            state: State::StartRecord,
            field: String::new(),
            field_len: 0,
            fields: Vec::new(),
        }
    }

    fn save_field(&mut self) {
        self.fields.push(std::mem::take(&mut self.field));
        self.field_len = 0;
    }

    fn add_char(&mut self, c: char) -> Result<(), CsvError> {
        if self.field_len >= FIELD_LIMIT {
            return Err(CsvError::Csv(format!(
                "field larger than field limit ({FIELD_LIMIT})"
            )));
        }
        self.field.push(c);
        self.field_len += 1;
        Ok(())
    }

    /// One character, or the end of a line when `c` is None.
    fn process(&mut self, c: Option<char>) -> Result<(), CsvError> {
        let newline = matches!(c, Some('\n' | '\r'));
        let eol = c.is_none();
        match self.state {
            State::StartRecord if eol => {}
            State::StartRecord if newline => self.state = State::EatCrnl,
            State::StartRecord | State::StartField => {
                if eol || newline {
                    self.save_field();
                    self.state = if eol {
                        State::StartRecord
                    } else {
                        State::EatCrnl
                    };
                } else if c == Some('"') {
                    self.state = State::InQuotedField;
                } else if c == Some(',') {
                    self.save_field();
                    self.state = State::StartField;
                } else {
                    self.add_char(c.expect("a character"))?;
                    self.state = State::InField;
                }
            }
            State::InField => {
                if eol || newline {
                    self.save_field();
                    self.state = if eol {
                        State::StartRecord
                    } else {
                        State::EatCrnl
                    };
                } else if c == Some(',') {
                    self.save_field();
                    self.state = State::StartField;
                } else {
                    self.add_char(c.expect("a character"))?;
                }
            }
            State::InQuotedField => {
                if eol {
                } else if c == Some('"') {
                    self.state = State::QuoteInQuotedField;
                } else {
                    self.add_char(c.expect("a character"))?;
                }
            }
            State::QuoteInQuotedField => {
                if c == Some('"') {
                    self.add_char('"')?;
                    self.state = State::InQuotedField;
                } else if c == Some(',') {
                    self.save_field();
                    self.state = State::StartField;
                } else if eol || newline {
                    self.save_field();
                    self.state = if eol {
                        State::StartRecord
                    } else {
                        State::EatCrnl
                    };
                } else {
                    self.add_char(c.expect("a character"))?;
                    self.state = State::InField;
                }
            }
            State::EatCrnl => {
                if newline {
                } else if eol {
                    self.state = State::StartRecord;
                } else {
                    return Err(CsvError::Csv(
                        "new-line character seen in unquoted field - do you need to open the \
                         file with newline=''?"
                            .into(),
                    ));
                }
            }
        }
        Ok(())
    }

    /// `next(reader)`: the next record, or None at the end of the file.
    pub fn next_record(&mut self) -> Result<Option<Vec<String>>, CsvError> {
        self.fields.clear();
        loop {
            let Some(line) = self.lines.next_line()? else {
                if self.field_len != 0 || self.state == State::InQuotedField {
                    self.save_field();
                    self.state = State::StartRecord;
                    return Ok(Some(std::mem::take(&mut self.fields)));
                }
                return Ok(None);
            };
            for c in line {
                self.process(Some(c))?;
            }
            self.process(None)?;
            if self.state == State::StartRecord {
                return Ok(Some(std::mem::take(&mut self.fields)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records(data: &[u8]) -> Vec<Vec<String>> {
        let mut reader = Reader::new(data);
        let mut out = Vec::new();
        while let Some(record) = reader.next_record().unwrap() {
            out.push(record);
        }
        out
    }

    #[test]
    fn records_follow_the_excel_dialect() {
        assert_eq!(
            records(b"\xef\xbb\xbfa,\"b,c\"\r\n\r\nx\"y,\"q\"\"\"z\n\"multi\nline\",\n"),
            vec![
                vec!["a".to_string(), "b,c".into()],
                vec![],
                vec!["x\"y".into(), "q\"z".into()],
                vec!["multi\nline".into(), "".into()],
            ]
        );
        assert_eq!(records(b"\"open"), vec![vec!["open".to_string()]]);
        assert_eq!(
            records(b"a\rb"),
            vec![vec!["a".to_string()], vec!["b".into()]]
        );
        assert!(records(b"").is_empty());
    }

    #[test]
    fn decoding_stops_at_the_chunk_python_read() {
        let mut data = b"a,b,c\n".repeat(3000);
        data[8192] = 0xff;
        let mut reader = Reader::new(&data[..]);
        let mut rows = 0;
        let error = loop {
            match reader.next_record() {
                Ok(Some(_)) => rows += 1,
                Ok(None) => panic!("the invalid byte was not reached"),
                Err(CsvError::Decode(e)) => break e,
                Err(_) => panic!("unexpected error"),
            }
        };
        assert_eq!(rows, 1365);
        assert_eq!(
            error.to_string(),
            "'utf-8' codec can't decode byte 0xff in position 0: invalid start byte"
        );
    }
}
