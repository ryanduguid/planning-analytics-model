//! JSON read and written as the original engine's `json` module did.
//!
//! Model files are read with the C scanner's semantics, so a refusal names the
//! same position and a repeated key behaves as a `dict` does: the key keeps
//! its first position and takes its last value. Output matches
//! `json.dumps(..., indent=2)`, which escapes every non-ASCII character.

use std::fmt::Write as _;

use num_bigint::BigInt;
use num_traits::Zero;

use crate::py;

/// A decoded JSON value, as the Python object `json.loads` returns.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(BigInt),
    Float(f64),
    Str(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    /// `value.get(key)` on a `dict`.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// `key in value` on a `dict`.
    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    /// `type(value).__name__`.
    pub fn type_name(&self) -> &'static str {
        match self {
            Json::Null => "NoneType",
            Json::Bool(_) => "bool",
            Json::Int(_) => "int",
            Json::Float(_) => "float",
            Json::Str(_) => "str",
            Json::Array(_) => "list",
            Json::Object(_) => "dict",
        }
    }

    /// `repr(value)` of the Python object.
    pub fn repr(&self) -> String {
        match self {
            Json::Null => "None".into(),
            Json::Bool(true) => "True".into(),
            Json::Bool(false) => "False".into(),
            Json::Int(i) => i.to_string(),
            Json::Float(f) => py::float_repr(*f),
            Json::Str(s) => py::repr_str(s),
            Json::Array(items) => format!(
                "[{}]",
                items.iter().map(Json::repr).collect::<Vec<_>>().join(", ")
            ),
            Json::Object(pairs) => format!(
                "{{{}}}",
                pairs
                    .iter()
                    .map(|(k, v)| format!("{}: {}", py::repr_str(k), v.repr()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    /// `str(value)` of the Python object.
    pub fn py_str(&self) -> String {
        match self {
            Json::Str(s) => s.clone(),
            other => other.repr(),
        }
    }

    /// Python truthiness.
    pub fn truthy(&self) -> bool {
        match self {
            Json::Null => false,
            Json::Bool(b) => *b,
            Json::Int(i) => !i.is_zero(),
            Json::Float(f) => *f != 0.0,
            Json::Str(s) => !s.is_empty(),
            Json::Array(items) => !items.is_empty(),
            Json::Object(pairs) => !pairs.is_empty(),
        }
    }
}

/// Why a JSON text could not be read.
#[derive(Debug, Clone, PartialEq)]
pub enum DecodeError {
    /// A `ValueError`, `JSONDecodeError` included, with its message.
    Value(String),
    /// The scanner ran out of stack: Python raised `RecursionError`.
    TooDeep,
}

fn decode_error(msg: &str, doc: &[char], pos: usize) -> DecodeError {
    let lineno = doc[..pos].iter().filter(|c| **c == '\n').count() + 1;
    let colno = match doc[..pos].iter().rposition(|c| *c == '\n') {
        Some(i) => pos - i,
        None => pos + 1,
    };
    DecodeError::Value(format!("{msg}: line {lineno} column {colno} (char {pos})"))
}

fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

enum Frame {
    Array(Vec<Json>),
    Object(Vec<(String, Json)>, String),
}

// The stack the C scanner may use, in units of one array or object level.
// CPython 3.15 bounds C recursion by stack space, so the deepest document it
// reads is a property of the build and the platform. Measured through the
// 3.15.0 console script reading a manifest, whose own object is one level:
// on Windows, 13,855 nested arrays, 15,587 nested objects or 7,335
// array-and-object pairs inside it; on Linux 65,208 arrays, 65,216 objects or
// 32,602 to 32,607 pairs, which moved by a few levels between runs.
// shortcut: a cost per level stands in for the scanner's stack frames, so a
// document within a level (Windows) or about ten levels (Linux) of the limit
// may be judged differently from Python.
#[cfg(windows)]
const STACK_BUDGET: u64 = 1_385_630_000;
#[cfg(windows)]
const ARRAY_COST: u64 = 100_000;
#[cfg(windows)]
const OBJECT_COST: u64 = 88_888;
#[cfg(not(windows))]
const STACK_BUDGET: u64 = 6_520_950_000;
#[cfg(not(windows))]
const ARRAY_COST: u64 = 100_000;
#[cfg(not(windows))]
const OBJECT_COST: u64 = 100_000;

fn frame_cost(frame: &Frame) -> u64 {
    match frame {
        Frame::Array(_) => ARRAY_COST,
        Frame::Object(..) => OBJECT_COST,
    }
}

/// `json.loads(text)`.
pub fn loads(text: &str) -> Result<Json, DecodeError> {
    let s: Vec<char> = text.chars().collect();
    if s.first() == Some(&'\u{feff}') {
        return Err(decode_error(
            "Unexpected UTF-8 BOM (decode using utf-8-sig)",
            &s,
            0,
        ));
    }
    let len = s.len();
    let skip = |mut i: usize| {
        while i < len && is_ws(s[i]) {
            i += 1;
        }
        i
    };
    let mut idx = skip(0);
    let mut stack: Vec<Frame> = Vec::new();
    let mut used: u64 = 0;
    let expecting_value = |pos: usize| decode_error("Expecting value", &s, pos);

    // Reads a key and its delimiter, leaving idx at the value.
    let read_key = |idx: &mut usize| -> Result<String, DecodeError> {
        if *idx >= len || s[*idx] != '"' {
            return Err(decode_error(
                "Expecting property name enclosed in double quotes",
                &s,
                *idx,
            ));
        }
        let (key, next) = scanstring(&s, *idx + 1)?;
        *idx = skip(next);
        if *idx >= len || s[*idx] != ':' {
            return Err(decode_error("Expecting ':' delimiter", &s, *idx));
        }
        *idx = skip(*idx + 1);
        Ok(key)
    };

    fn push(stack: &mut Vec<Frame>, used: &mut u64, frame: Frame) -> Result<(), DecodeError> {
        *used += frame_cost(&frame);
        if *used > STACK_BUDGET {
            return Err(DecodeError::TooDeep);
        }
        stack.push(frame);
        Ok(())
    }

    'value: loop {
        if idx >= len {
            return Err(expecting_value(idx));
        }
        let mut value = match s[idx] {
            '"' => {
                let (string, next) = scanstring(&s, idx + 1)?;
                idx = next;
                Json::Str(string)
            }
            '{' => {
                push(
                    &mut stack,
                    &mut used,
                    Frame::Object(Vec::new(), String::new()),
                )?;
                idx = skip(idx + 1);
                if idx < len && s[idx] == '}' {
                    idx += 1;
                    stack.pop();
                    used -= OBJECT_COST;
                    Json::Object(Vec::new())
                } else {
                    let key = read_key(&mut idx)?;
                    if let Some(Frame::Object(_, slot)) = stack.last_mut() {
                        *slot = key;
                    }
                    continue 'value;
                }
            }
            '[' => {
                push(&mut stack, &mut used, Frame::Array(Vec::new()))?;
                idx = skip(idx + 1);
                if idx < len && s[idx] == ']' {
                    idx += 1;
                    stack.pop();
                    used -= ARRAY_COST;
                    Json::Array(Vec::new())
                } else {
                    continue 'value;
                }
            }
            _ => {
                let literal = |word: &str| {
                    s[idx..]
                        .iter()
                        .take(word.chars().count())
                        .copied()
                        .eq(word.chars())
                };
                if literal("null") {
                    idx += 4;
                    Json::Null
                } else if literal("true") {
                    idx += 4;
                    Json::Bool(true)
                } else if literal("false") {
                    idx += 5;
                    Json::Bool(false)
                } else if literal("NaN") {
                    idx += 3;
                    Json::Float(f64::NAN)
                } else if literal("Infinity") {
                    idx += 8;
                    Json::Float(f64::INFINITY)
                } else if literal("-Infinity") {
                    idx += 9;
                    Json::Float(f64::NEG_INFINITY)
                } else {
                    let (number, next) =
                        match_number(&s, idx).ok_or_else(|| expecting_value(idx))??;
                    idx = next;
                    number
                }
            }
        };
        // Attach the finished value to its container, closing any that end here.
        loop {
            let Some(frame) = stack.last_mut() else {
                let end = skip(idx);
                if end != len {
                    return Err(decode_error("Extra data", &s, end));
                }
                return Ok(value);
            };
            match frame {
                Frame::Array(items) => {
                    items.push(value);
                    idx = skip(idx);
                    if idx < len && s[idx] == ']' {
                        idx += 1;
                        let items = std::mem::take(items);
                        stack.pop();
                        used -= ARRAY_COST;
                        value = Json::Array(items);
                        continue;
                    }
                    if idx >= len || s[idx] != ',' {
                        return Err(decode_error("Expecting ',' delimiter", &s, idx));
                    }
                    let comma = idx;
                    idx = skip(idx + 1);
                    if idx < len && s[idx] == ']' {
                        return Err(decode_error(
                            "Illegal trailing comma before end of array",
                            &s,
                            comma,
                        ));
                    }
                    continue 'value;
                }
                Frame::Object(pairs, key) => {
                    let key = std::mem::take(key);
                    // A dict keeps a repeated key where it first appeared.
                    match pairs.iter_mut().find(|(k, _)| *k == key) {
                        Some(slot) => slot.1 = value,
                        None => pairs.push((key, value)),
                    }
                    idx = skip(idx);
                    if idx < len && s[idx] == '}' {
                        idx += 1;
                        let pairs = std::mem::take(pairs);
                        stack.pop();
                        used -= OBJECT_COST;
                        value = Json::Object(pairs);
                        continue;
                    }
                    if idx >= len || s[idx] != ',' {
                        return Err(decode_error("Expecting ',' delimiter", &s, idx));
                    }
                    let comma = idx;
                    idx = skip(idx + 1);
                    if idx < len && s[idx] == '}' {
                        return Err(decode_error(
                            "Illegal trailing comma before end of object",
                            &s,
                            comma,
                        ));
                    }
                    let next_key = read_key(&mut idx)?;
                    if let Some(Frame::Object(_, key)) = stack.last_mut() {
                        *key = next_key;
                    }
                    continue 'value;
                }
            }
        }
    }
}

/// The C scanner's number match; None where it raises StopIteration.
fn match_number(s: &[char], start: usize) -> Option<Result<(Json, usize), DecodeError>> {
    let len = s.len();
    let digit = |i: usize| i < len && s[i].is_ascii_digit();
    let mut idx = start;
    if s[idx] == '-' {
        idx += 1;
        if idx >= len {
            return None;
        }
    }
    if ('1'..='9').contains(&s[idx]) {
        idx += 1;
        while digit(idx) {
            idx += 1;
        }
    } else if s[idx] == '0' {
        idx += 1;
    } else {
        return None;
    }
    let mut is_float = false;
    if idx + 1 < len && s[idx] == '.' && s[idx + 1].is_ascii_digit() {
        is_float = true;
        idx += 2;
        while digit(idx) {
            idx += 1;
        }
    }
    if idx + 1 < len && (s[idx] == 'e' || s[idx] == 'E') {
        let e_start = idx;
        idx += 1;
        if idx + 1 < len && (s[idx] == '-' || s[idx] == '+') {
            idx += 1;
        }
        while digit(idx) {
            idx += 1;
        }
        if s[idx - 1].is_ascii_digit() {
            is_float = true;
        } else {
            idx = e_start;
        }
    }
    let text: String = s[start..idx].iter().collect();
    if is_float {
        let value: f64 = text.parse().expect("a JSON float literal");
        Some(Ok((Json::Float(value), idx)))
    } else {
        Some(
            py::parse_int(&text)
                .map(|i| (Json::Int(i), idx))
                .map_err(DecodeError::Value),
        )
    }
}

/// The C `scanstring`: `end` is the index after the opening quote.
fn scanstring(s: &[char], end: usize) -> Result<(String, usize), DecodeError> {
    let len = s.len();
    let begin = end - 1;
    let mut out = String::new();
    let mut end = end;
    loop {
        let mut next = end;
        let mut c = '\0';
        while next < len {
            c = s[next];
            if c == '"' || c == '\\' {
                break;
            }
            if (c as u32) <= 0x1f {
                return Err(decode_error("Invalid control character at", s, next));
            }
            next += 1;
        }
        if next >= len || c != '"' && c != '\\' {
            return Err(decode_error("Unterminated string starting at", s, begin));
        }
        out.extend(&s[end..next]);
        next += 1;
        if c == '"' {
            return Ok((out, next));
        }
        if next == len {
            return Err(decode_error("Unterminated string starting at", s, begin));
        }
        let c = s[next];
        if c != 'u' {
            end = next + 1;
            let replacement = match c {
                '"' => '"',
                '\\' => '\\',
                '/' => '/',
                'b' => '\x08',
                'f' => '\x0c',
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                _ => return Err(decode_error("Invalid \\escape", s, end - 2)),
            };
            out.push(replacement);
            continue;
        }
        next += 1;
        end = next + 4;
        if end >= len {
            return Err(decode_error("Invalid \\uXXXX escape", s, next - 1));
        }
        let hex = |from: usize, end: usize| -> Result<u32, DecodeError> {
            let mut value = 0;
            for &digit in &s[from..from + 4] {
                value = value << 4
                    | digit
                        .to_digit(16)
                        .ok_or_else(|| decode_error("Invalid \\uXXXX escape", s, end - 5))?;
            }
            Ok(value)
        };
        let mut code = hex(next, end)?;
        if (0xd800..=0xdbff).contains(&code) && end + 6 < len && s[end] == '\\' && s[end + 1] == 'u'
        {
            let low = hex(end + 2, end + 6)?;
            if (0xdc00..=0xdfff).contains(&low) {
                code = 0x10000 + (((code - 0xd800) << 10) | (low - 0xdc00));
                end += 6;
            }
        }
        match char::from_u32(code) {
            Some(ch) => out.push(ch),
            // shortcut: Python keeps a lone surrogate in the string; a Rust
            // string cannot hold one, so the escape is refused here.
            None => {
                return Err(decode_error(
                    "Unpaired surrogate in \\uXXXX escape",
                    s,
                    next - 2,
                ));
            }
        }
    }
}

/// A value for `json.dumps`: the shapes the explain and compare output use.
#[derive(Clone, Debug)]
pub enum Out {
    Null,
    Bool(bool),
    Int(i64),
    Str(String),
    List(Vec<Out>),
    Dict(Vec<(String, Out)>),
}

/// `json.dumps(s)` for one string, with `ensure_ascii`.
pub fn dump_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
        }
    }
    out.push('"');
}

fn dump_into(value: &Out, level: usize, out: &mut String) {
    let pad = |out: &mut String, n: usize| {
        for _ in 0..n {
            out.push_str("  ");
        }
    };
    match value {
        Out::Null => out.push_str("null"),
        Out::Bool(true) => out.push_str("true"),
        Out::Bool(false) => out.push_str("false"),
        Out::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Out::Str(s) => dump_str(s, out),
        Out::List(items) if items.is_empty() => out.push_str("[]"),
        Out::List(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('\n');
                pad(out, level + 1);
                dump_into(item, level + 1, out);
            }
            out.push('\n');
            pad(out, level);
            out.push(']');
        }
        Out::Dict(pairs) if pairs.is_empty() => out.push_str("{}"),
        Out::Dict(pairs) => {
            out.push('{');
            for (i, (k, v)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('\n');
                pad(out, level + 1);
                dump_str(k, out);
                out.push_str(": ");
                dump_into(v, level + 1, out);
            }
            out.push('\n');
            pad(out, level);
            out.push('}');
        }
    }
}

/// `json.dumps(value, indent=2)`.
pub fn dumps(value: &Out) -> String {
    let mut out = String::new();
    dump_into(value, 0, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(text: &str) -> String {
        match loads(text).unwrap_err() {
            DecodeError::Value(message) => message,
            DecodeError::TooDeep => "too deep".into(),
        }
    }

    #[test]
    fn errors_match_the_c_scanner() {
        assert_eq!(
            err("{\"a\":1,}"),
            "Illegal trailing comma before end of object: line 1 column 7 (char 6)"
        );
        assert_eq!(
            err("[1,]"),
            "Illegal trailing comma before end of array: line 1 column 3 (char 2)"
        );
        assert_eq!(
            err("\"\u{1}\""),
            "Invalid control character at: line 1 column 2 (char 1)"
        );
        assert_eq!(err("\"\\q\""), "Invalid \\escape: line 1 column 2 (char 1)");
        assert_eq!(
            err("\"\\u12\""),
            "Invalid \\uXXXX escape: line 1 column 3 (char 2)"
        );
        assert_eq!(
            err("{\"a\" 1}"),
            "Expecting ':' delimiter: line 1 column 6 (char 5)"
        );
        assert_eq!(err("1 2"), "Extra data: line 1 column 3 (char 2)");
        assert_eq!(err(""), "Expecting value: line 1 column 1 (char 0)");
        assert_eq!(
            err("\u{feff}{}"),
            "Unexpected UTF-8 BOM (decode using utf-8-sig): line 1 column 1 (char 0)"
        );
        assert_eq!(
            err("[1 2]"),
            "Expecting ',' delimiter: line 1 column 4 (char 3)"
        );
        assert_eq!(err("01"), "Extra data: line 1 column 2 (char 1)");
        assert_eq!(err("[".repeat(100_000).as_str()), "too deep");
    }

    /// The manifest-in-object depths measured through the 3.15.0 console script.
    #[test]
    fn nesting_stops_where_python_stopped() {
        // Parsing is iterative, but dropping the resulting nested value is not.
        // Match the binary's large-stack execution instead of the test harness's
        // small default stack, including both accepted values and refusal paths.
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn(nesting_limits)
            .unwrap()
            .join()
            .unwrap();
    }

    fn nesting_limits() {
        let arrays = |n: usize| format!("{{\"Name\": {}{}}}", "[".repeat(n), "]".repeat(n));
        let objects =
            |n: usize| format!("{{\"Name\": {}1{}}}", "{\"a\": ".repeat(n), "}".repeat(n));
        #[cfg(windows)]
        let (array_limit, object_limit) = (13_855, 15_587);
        #[cfg(not(windows))]
        let (array_limit, object_limit) = (65_208, 65_208);
        assert!(loads(&arrays(array_limit)).is_ok());
        assert_eq!(loads(&arrays(array_limit + 1)), Err(DecodeError::TooDeep));
        assert!(loads(&objects(object_limit)).is_ok());
        assert_eq!(loads(&objects(object_limit + 1)), Err(DecodeError::TooDeep));
    }

    #[test]
    fn a_repeated_key_keeps_its_place_and_its_last_value() {
        let value = loads("{\"a\": 1, \"b\": 2, \"a\": 3}").unwrap();
        assert_eq!(value.repr(), "{'a': 3, 'b': 2}");
    }

    #[test]
    fn dumps_matches_python() {
        let value = Out::Dict(vec![
            (
                "b".into(),
                Out::List(vec![Out::Int(1), Out::Str("\u{e9}\n\u{1f600}".into())]),
            ),
            ("a".into(), Out::Dict(vec![])),
            ("c".into(), Out::List(vec![])),
            ("d".into(), Out::Null),
        ]);
        assert_eq!(
            dumps(&value),
            "{\n  \"b\": [\n    1,\n    \"\\u00e9\\n\\ud83d\\ude00\"\n  ],\n  \"a\": {},\n  \"c\": [],\n  \"d\": null\n}"
        );
    }

    #[test]
    fn dumps_writes_booleans_and_short_escapes() {
        let value = Out::List(vec![
            Out::Bool(true),
            Out::Bool(false),
            Out::Str("\x08\x0c".into()),
        ]);
        assert_eq!(dumps(&value), "[\n  true,\n  false,\n  \"\\b\\f\"\n]");
    }

    #[test]
    fn accessors_answer_as_the_python_object_would() {
        let value =
            loads("{\"s\": \"x\", \"t\": true, \"f\": false, \"r\": 0.0, \"o\": {}}").unwrap();
        assert_eq!(value.get("s").and_then(Json::as_str), Some("x"));
        assert_eq!(value.get("t").and_then(Json::as_str), None);
        assert_eq!(value.get("s").unwrap().get("s"), None);
        assert_eq!(value.get("t").unwrap().type_name(), "bool");
        assert_eq!(value.get("r").unwrap().type_name(), "float");
        assert_eq!(value.get("o").unwrap().type_name(), "dict");
        assert_eq!(value.get("f").unwrap().repr(), "False");
        assert!(!value.get("r").unwrap().truthy());
        assert!(Json::Float(0.5).truthy());
    }

    #[test]
    fn an_exponent_without_digits_is_left_unread() {
        // json.loads("[1e]") stops the number before the "e".
        assert_eq!(
            err("[1e]"),
            "Expecting ',' delimiter: line 1 column 3 (char 2)"
        );
        assert_eq!(
            err("[1e+]"),
            "Expecting ',' delimiter: line 1 column 3 (char 2)"
        );
    }

    #[test]
    fn a_lone_surrogate_escape_is_refused() {
        assert_eq!(
            err("\"\\ud800\""),
            "Unpaired surrogate in \\uXXXX escape: line 1 column 2 (char 1)"
        );
    }
}
