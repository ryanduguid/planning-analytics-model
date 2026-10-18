//! The Python behaviours the original engine's output depended on.
//!
//! The engine was a Python program, and its messages quote Python's own
//! renderings: `repr` of strings and numbers, `str` of `OSError`s and
//! `UnicodeDecodeError`s, and `str` methods such as `casefold`, `upper` and
//! `isdigit`. Reproducing them here keeps every line byte for byte what a
//! user of the Python release saw. The tables come from CPython 3.15.0.

use std::fmt::Write as _;
use std::io;

use num_bigint::BigInt;

use crate::pyunicode as tables;

fn in_table(cp: u32, table: &[(u32, u32)]) -> bool {
    table
        .binary_search_by(|&(lo, hi)| {
            if hi < cp {
                std::cmp::Ordering::Less
            } else if lo > cp {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// `str.isspace` for one character.
pub fn is_space(c: char) -> bool {
    in_table(c as u32, tables::SPACE)
}

/// `str.isprintable` for one character.
pub fn is_printable(c: char) -> bool {
    !in_table(c as u32, tables::NOT_PRINTABLE)
}

/// `str.isdecimal` for one character: what `\d` matches in a `str` pattern.
pub fn is_decimal(c: char) -> bool {
    in_table(c as u32, tables::DECIMAL)
}

/// `str.isdigit` for one character.
pub fn is_digit(c: char) -> bool {
    in_table(c as u32, tables::DIGIT)
}

/// `str.isalpha` for one character.
pub fn is_alpha(c: char) -> bool {
    in_table(c as u32, tables::ALPHA)
}

/// `str.isalnum` for one character.
pub fn is_alnum(c: char) -> bool {
    in_table(c as u32, tables::ALNUM)
}

/// What `\w` matches in a `str` pattern.
pub fn is_word(c: char) -> bool {
    c == '_' || is_alnum(c)
}

/// The value `int()` gives a decimal digit, ASCII or not.
pub fn decimal_value(c: char) -> Option<u32> {
    if !is_decimal(c) {
        return None;
    }
    let cp = c as u32;
    let index = tables::DECIMAL_ZERO.partition_point(|&zero| zero <= cp);
    let zero = tables::DECIMAL_ZERO[index.checked_sub(1)?];
    (cp - zero < 10).then_some(cp - zero)
}

fn map_chars(s: &str, table: &[(u32, &str)]) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match table.binary_search_by_key(&(c as u32), |&(k, _)| k) {
            Ok(i) => out.push_str(table[i].1),
            Err(_) => out.push(c),
        }
    }
    out
}

/// `str.casefold()`.
pub fn casefold(s: &str) -> String {
    if s.is_ascii() {
        return s.to_ascii_lowercase();
    }
    map_chars(s, tables::CASEFOLD)
}

/// `str.upper()`.
pub fn upper(s: &str) -> String {
    if s.is_ascii() {
        return s.to_ascii_uppercase();
    }
    map_chars(s, tables::UPPER)
}

/// `str.lower()`, including the Final_Sigma rule for U+03A3.
pub fn lower(s: &str) -> String {
    if s.is_ascii() {
        return s.to_ascii_lowercase();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let cased = |c: char| in_table(c as u32, tables::CASED);
    let ignorable = |c: char| in_table(c as u32, tables::CASE_IGNORABLE);
    for (i, &c) in chars.iter().enumerate() {
        if c != '\u{3a3}' {
            match tables::LOWER.binary_search_by_key(&(c as u32), |&(k, _)| k) {
                Ok(j) => out.push_str(tables::LOWER[j].1),
                Err(_) => out.push(c),
            }
            continue;
        }
        let before = chars[..i].iter().rev().find(|c| !ignorable(**c));
        let mut final_sigma = before.is_some_and(|c| cased(*c));
        if final_sigma && i + 1 < chars.len() {
            let after = chars[i + 1..].iter().find(|c| !ignorable(**c));
            final_sigma = after.is_none_or(|c| !cased(*c));
        }
        out.push(if final_sigma { '\u{3c2}' } else { '\u{3c3}' });
    }
    out
}

/// `str.strip()`.
pub fn strip(s: &str) -> &str {
    s.trim_matches(is_space)
}

/// `str.lstrip()`.
pub fn lstrip(s: &str) -> &str {
    s.trim_start_matches(is_space)
}

/// `str.splitlines()`.
pub fn splitlines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let boundary = matches!(
            c,
            '\n' | '\r'
                | '\x0b'
                | '\x0c'
                | '\x1c'
                | '\x1d'
                | '\x1e'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        );
        if boundary {
            out.push(&s[start..i]);
            let mut next = i + c.len_utf8();
            if c == '\r'
                && let Some(&(j, '\n')) = chars.peek()
            {
                chars.next();
                next = j + 1;
            }
            start = next;
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

/// `len(s)`: a count of code points.
pub fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// The first `n` characters, as `s[:n]` takes them.
pub fn prefix_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// `repr(s)` for a `str`.
pub fn repr_str(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            _ if c == quote || c == '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ if (c as u32) < 0x20 || c as u32 == 0x7f => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            _ if (c as u32) < 0x7f => out.push(c),
            _ if is_printable(c) => out.push(c),
            _ if (c as u32) <= 0xff => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            _ if (c as u32) <= 0xffff => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            _ => {
                let _ = write!(out, "\\U{:08x}", c as u32);
            }
        }
    }
    out.push(quote);
    out
}

/// `repr` of a list of strings, as `f"{list(names)}"` prints it.
pub fn repr_list(items: &[String]) -> String {
    let parts: Vec<String> = items.iter().map(|s| repr_str(s)).collect();
    format!("[{}]", parts.join(", "))
}

/// `repr(f)` for a `float`.
pub fn float_repr(f: f64) -> String {
    if f.is_nan() {
        return "nan".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let sign = if f.is_sign_negative() { "-" } else { "" };
    if f == 0.0 {
        return format!("{sign}0.0");
    }
    let sci = format!("{:e}", f.abs());
    let (mantissa, exponent) = sci.split_once('e').expect("scientific notation");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let decpt: i64 = exponent.parse::<i64>().expect("exponent") + 1;
    let n = digits.len() as i64;
    let body = if decpt <= -4 || decpt > 16 {
        let mut m = digits[..1].to_string();
        if n > 1 {
            m.push('.');
            m.push_str(&digits[1..]);
        }
        let e = decpt - 1;
        format!("{m}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
    } else if decpt <= 0 {
        format!("0.{}{digits}", "0".repeat((-decpt) as usize))
    } else if decpt >= n {
        format!("{digits}{}.0", "0".repeat((decpt - n) as usize))
    } else {
        format!(
            "{}.{}",
            &digits[..decpt as usize],
            &digits[decpt as usize..]
        )
    };
    format!("{sign}{body}")
}

/// Python refuses to convert more than this many decimal digits to or from an `int`.
pub const INT_MAX_STR_DIGITS: usize = 4300;

/// The `ValueError` Python raises when parsing an over-long decimal `int`.
pub fn int_digits_error(digits: usize) -> String {
    format!(
        "Exceeds the limit ({INT_MAX_STR_DIGITS} digits) for integer string conversion: value has \
         {digits} digits; use sys.set_int_max_str_digits() to increase the limit"
    )
}

/// Unicode digits to ASCII and Unicode spaces to ' ', as `int()` first does.
fn number_text(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if is_space(c) {
            out.push(' ');
        } else if let Some(d) = decimal_value(c) {
            out.push(char::from_digit(d, 10).expect("digit"));
        } else if c.is_ascii() {
            out.push(c);
        } else {
            return None;
        }
    }
    Some(out)
}

/// `int(s)` in base 10; the error is the `ValueError` message.
pub fn parse_int(s: &str) -> Result<BigInt, String> {
    let invalid = || {
        let shown: String = if char_len(s) > 200 {
            prefix_chars(s, 200).to_string()
        } else {
            s.to_string()
        };
        format!(
            "invalid literal for int() with base 10: {}",
            repr_str(&shown)
        )
    };
    let text = number_text(s).ok_or_else(invalid)?;
    let text =
        text.trim_matches(|c: char| c.is_ascii_whitespace() || ('\x1c'..='\x1f').contains(&c));
    let (negative, body) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    // An underscore must sit between two digits.
    let mut digits = String::new();
    let mut previous_digit = false;
    let mut previous_underscore = false;
    for c in body.chars() {
        if c == '_' {
            if previous_underscore || !previous_digit {
                return Err(invalid());
            }
            previous_underscore = true;
            previous_digit = false;
            continue;
        }
        if !c.is_ascii_digit() {
            return Err(invalid());
        }
        digits.push(c);
        previous_digit = true;
        previous_underscore = false;
    }
    if digits.is_empty() || previous_underscore {
        return Err(invalid());
    }
    if digits.len() > INT_MAX_STR_DIGITS {
        return Err(int_digits_error(digits.len()));
    }
    let value = BigInt::parse_bytes(digits.as_bytes(), 10).ok_or_else(invalid)?;
    Ok(if negative { -value } else { value })
}

/// A UTF-8 decoding failure, rendered as Python's `UnicodeDecodeError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError {
    pub start: usize,
    pub end: usize,
    pub byte: u8,
    pub reason: &'static str,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.end == self.start + 1 {
            write!(
                f,
                "'utf-8' codec can't decode byte 0x{:02x} in position {}: {}",
                self.byte, self.start, self.reason
            )
        } else {
            write!(
                f,
                "'utf-8' codec can't decode bytes in position {}-{}: {}",
                self.start,
                self.end - 1,
                self.reason
            )
        }
    }
}

/// `codecs.utf_8_decode(bytes, "strict", final)`: the text and how many bytes
/// it consumed. Without `final`, an incomplete sequence at the end is left
/// unconsumed rather than refused.
pub fn utf8_decode(bytes: &[u8], last: bool) -> Result<(String, usize), DecodeError> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok((text.to_string(), bytes.len())),
        Err(e) => {
            let start = e.valid_up_to();
            let byte = bytes[start];
            // CPython leaves a trailing \xED and continuation byte undecided
            // until the next call, although no third byte can make it valid.
            let surrogate_prefix = bytes[start..].len() == 2
                && byte == 0xed
                && (0xa0..=0xbf).contains(&bytes[start + 1]);
            let (end, reason) = match e.error_len() {
                Some(_) if surrogate_prefix && !last => {
                    let text = std::str::from_utf8(&bytes[..start]).expect("valid prefix");
                    return Ok((text.to_string(), start));
                }
                None if !last => {
                    let text = std::str::from_utf8(&bytes[..start]).expect("valid prefix");
                    return Ok((text.to_string(), start));
                }
                None => (bytes.len(), "unexpected end of data"),
                Some(n) if (0xc2..=0xf4).contains(&byte) => {
                    (start + n, "invalid continuation byte")
                }
                Some(n) => (start + n, "invalid start byte"),
            };
            Err(DecodeError {
                start,
                end,
                byte,
                reason,
            })
        }
    }
}

const BOM: &[u8] = b"\xef\xbb\xbf";

/// The `utf-8-sig` incremental decoder: `codecs.BufferedIncrementalDecoder`
/// around a first call that drops one byte order mark.
#[derive(Default)]
pub struct SigDecoder {
    buffer: Vec<u8>,
    decided: bool,
}

impl SigDecoder {
    /// `decoder.decode(input, final)`. Error positions are relative to the
    /// pending bytes plus `input`, after a byte order mark, as Python's are.
    pub fn decode(&mut self, input: &[u8], last: bool) -> Result<String, DecodeError> {
        let mut data = std::mem::take(&mut self.buffer);
        data.extend_from_slice(input);
        let (text, consumed) = if !self.decided {
            if data.len() < 3 {
                if BOM.starts_with(&data) {
                    // Not enough data to decide; even a final call yields nothing.
                    self.buffer = data;
                    return Ok(String::new());
                }
                self.decided = true;
                utf8_decode(&data, last)?
            } else {
                self.decided = true;
                if data.starts_with(BOM) {
                    let (text, consumed) = utf8_decode(&data[3..], last)?;
                    (text, consumed + 3)
                } else {
                    utf8_decode(&data, last)?
                }
            }
        } else {
            utf8_decode(&data, last)?
        };
        self.buffer = data[consumed..].to_vec();
        Ok(text)
    }
}

/// Text mode's universal newlines: `\r\n` and `\r` both read as `\n`.
pub fn universal_newlines(text: &str) -> String {
    if text.contains('\r') {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text.to_string()
    }
}

/// How the platform reported an `OSError`, which decides how Python renders it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OsStyle {
    /// `open()`: the C runtime's errno, on every platform.
    Errno,
    /// Directory and metadata calls, which on Windows report a Win32 error.
    Native,
}

fn strip_os_suffix(message: &str) -> &str {
    match message.rfind(" (os error ") {
        Some(i) => &message[..i],
        None => message,
    }
}

#[cfg(windows)]
fn crt_errno(winerror: i32) -> (i32, &'static str) {
    match winerror {
        2 | 3 | 15 | 206 => (2, "No such file or directory"),
        4 => (24, "Too many open files"),
        5 | 32 | 33 | 1920 => (13, "Permission denied"),
        6 => (9, "Bad file descriptor"),
        8 => (12, "Not enough space"),
        80 | 183 => (17, "File exists"),
        _ => (22, "Invalid argument"),
    }
}

/// `str(OSError)` for an operation on `filename` (already rendered as `str(path)`).
pub fn oserror_str(error: &io::Error, filename: &str, style: OsStyle) -> String {
    let Some(code) = error.raw_os_error() else {
        return error.to_string();
    };
    let message = error.to_string();
    #[cfg(windows)]
    {
        if style == OsStyle::Errno {
            let (errno, text) = crt_errno(code);
            return format!("[Errno {errno}] {text}: {}", repr_str(filename));
        }
        let text = strip_os_suffix(&message).trim_end_matches(|c: char| c == '.' || c <= ' ');
        format!("[WinError {code}] {text}: {}", repr_str(filename))
    }
    #[cfg(not(windows))]
    {
        let _ = style;
        format!(
            "[Errno {code}] {}: {}",
            strip_os_suffix(&message),
            repr_str(filename)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_mappings_follow_cpython() {
        assert_eq!(casefold("Stra\u{df}e"), "strasse");
        assert_eq!(upper("\u{131}f"), "IF");
        assert_eq!(upper("stra\u{df}e"), "STRASSE");
        assert_eq!(lower("A\u{3a3}"), "a\u{3c2}");
        assert_eq!(lower("A\u{3a3}b"), "a\u{3c3}b");
        assert_eq!(lower("\u{3a3}"), "\u{3c3}");
        assert_eq!(lower("\u{130}"), "i\u{307}");
    }

    #[test]
    fn predicates_follow_cpython() {
        assert!(is_digit('\u{b2}') && !is_decimal('\u{b2}'));
        assert!(is_alpha('\u{e9}') && !is_alpha('1'));
        assert!(is_alnum('\u{2167}'));
        assert_eq!(decimal_value('\u{663}'), Some(3));
        assert_eq!(strip("\u{2003} a \u{1c}"), "a");
    }

    #[test]
    fn repr_matches_python() {
        assert_eq!(repr_str("it's"), "\"it's\"");
        assert_eq!(repr_str("a'\""), "'a\\'\"'");
        assert_eq!(
            repr_str("\u{1}\u{85}\u{e9}\u{ffff}\u{1f600}"),
            "'\\x01\\x85\u{e9}\\uffff\u{1f600}'"
        );
        assert_eq!(float_repr(1e16), "1e+16");
        assert_eq!(float_repr(0.1), "0.1");
        assert_eq!(float_repr(-0.0), "-0.0");
        assert_eq!(float_repr(1e-5), "1e-05");
        assert_eq!(float_repr(123.0), "123.0");
    }

    #[test]
    fn int_parsing_matches_python() {
        assert_eq!(parse_int(" 1_000 ").unwrap(), BigInt::from(1000));
        assert_eq!(parse_int("-\u{663}").unwrap(), BigInt::from(-3));
        assert!(parse_int("1__0").is_err());
        assert!(parse_int("_1").is_err());
        assert_eq!(
            parse_int(&"9".repeat(4301)).unwrap_err(),
            int_digits_error(4301)
        );
    }

    #[test]
    fn sig_decoder_matches_the_chunked_reader() {
        let mut decoder = SigDecoder::default();
        assert_eq!(decoder.decode(b"\xef\xbb", false).unwrap(), "");
        assert_eq!(decoder.decode(b"", true).unwrap(), "");
        let mut decoder = SigDecoder::default();
        assert_eq!(
            decoder.decode(b"\xef\xbb\xbfa\xe2\x82", false).unwrap(),
            "a"
        );
        let error = decoder.decode(b"x", false).unwrap_err();
        assert_eq!(
            error.to_string(),
            "'utf-8' codec can't decode bytes in position 0-1: invalid continuation byte"
        );
        let mut decoder = SigDecoder::default();
        assert_eq!(decoder.decode(b"a\xed\xa0", false).unwrap(), "a");
        assert_eq!(
            decoder.decode(b"\x80", false).unwrap_err().to_string(),
            "'utf-8' codec can't decode byte 0xed in position 0: invalid continuation byte"
        );
        assert_eq!(
            utf8_decode(b"a\xed\xa0", true).unwrap_err().to_string(),
            "'utf-8' codec can't decode byte 0xed in position 1: invalid continuation byte"
        );
        let mut decoder = SigDecoder::default();
        assert_eq!(decoder.decode(b"a\xe2\x82", false).unwrap(), "a");
        assert_eq!(
            decoder.decode(b"", true).unwrap_err().to_string(),
            "'utf-8' codec can't decode bytes in position 0-1: unexpected end of data"
        );
    }
}
