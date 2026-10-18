//! Replays tests/data/python_golden.json, which records how CPython 3.15.0's
//! libraries behaved for the base revision (written by gen_golden.py in the
//! port's evidence folder): Decimal parsing, arithmetic and formatting, the
//! JSON scanner, str methods, utf-8-sig decoding with the CSV reader, pathlib,
//! difflib, textwrap and argparse. The argparse rows were regenerated at main
//! 99a3704, whose parser adds `demo` and `report --format`.

mod common;

use std::sync::OnceLock;

use pacioliscube::argparse::{Outcome, close_match, wrap};
use pacioliscube::cli;
use pacioliscube::csvread::{CsvError, Reader};
use pacioliscube::decimal::{Decimal, Rounding, Signal};
use pacioliscube::py::{self, SigDecoder};
use pacioliscube::pyjson::{self, DecodeError, Json};
use pacioliscube::pypath::PyPath;
use pacioliscube::terminal::{Console, Terminal};
use sha2::{Digest, Sha256};

fn golden() -> &'static Json {
    static GOLDEN: OnceLock<Json> = OnceLock::new();
    GOLDEN.get_or_init(|| {
        let text = std::fs::read_to_string(
            common::repo()
                .join("tests")
                .join("data")
                .join("python_golden.json"),
        )
        .unwrap();
        pyjson::loads(&text).unwrap()
    })
}

fn section(path: &[&str]) -> &'static Json {
    let mut value = golden();
    for key in path {
        value = value.get(key).unwrap_or_else(|| panic!("{key}"));
    }
    value
}

fn rows(path: &[&str]) -> &'static [Json] {
    match section(path) {
        Json::Array(items) => items,
        _ => panic!("{path:?} is not a list"),
    }
}

fn at(row: &Json, i: usize) -> &Json {
    match row {
        Json::Array(items) => &items[i],
        _ => panic!("not a row"),
    }
}

fn text(value: &Json) -> &str {
    value
        .as_str()
        .unwrap_or_else(|| panic!("{value:?} is not text"))
}

fn strings(value: &Json) -> Vec<String> {
    match value {
        Json::Array(items) => items.iter().map(|i| text(i).to_string()).collect(),
        _ => panic!("not a list"),
    }
}

fn bools(value: &Json) -> Vec<bool> {
    match value {
        Json::Array(items) => items
            .iter()
            .map(|i| matches!(i, Json::Bool(true)))
            .collect(),
        _ => panic!("not a list"),
    }
}

/// An outcome recorded as `{"ok": ...}`, `{"signal": name}` or `{"error": ..., "message": ...}`.
fn expect_decimal(outcome: &Json, actual: Result<String, Signal>, context: &str) {
    match (outcome.get("ok"), outcome.get("signal"), actual) {
        (Some(Json::Null), _, _) => {}
        (Some(ok), _, Ok(found)) => assert_eq!(found, text(ok), "{context}"),
        (_, Some(signal), Err(found)) => assert_eq!(found.name(), text(signal), "{context}"),
        (_, _, found) => panic!("{context}: expected {outcome:?}, found {found:?}"),
    }
}

#[test]
fn decimal_parsing_matches_cpython() {
    for row in rows(&["decimal", "parse"]) {
        let source = text(at(row, 0));
        let actual = Decimal::parse(source).map(|d| d.to_sci_string());
        expect_decimal(at(row, 1), actual, source);
    }
}

#[test]
fn decimal_arithmetic_matches_cpython() {
    for row in rows(&["decimal", "ops"]) {
        let a = Decimal::parse(text(at(row, 0))).unwrap();
        let b = Decimal::parse(text(at(row, 2))).unwrap();
        let op = text(at(row, 1));
        let result = match op {
            "+" => a.add(&b),
            "-" => a.sub(&b),
            "*" => a.mul(&b),
            _ => a.div(&b),
        };
        let context = format!("{} {op} {}", text(at(row, 0)), text(at(row, 2)));
        expect_decimal(at(row, 3), result.map(|d| d.to_sci_string()), &context);
    }
}

#[test]
fn decimal_negation_rounding_and_formatting_match_cpython() {
    for row in rows(&["decimal", "unary"]) {
        let source = text(at(row, 0));
        let value = Decimal::parse(source).unwrap();
        expect_decimal(at(row, 1), value.neg().map(|d| d.to_sci_string()), source);
        expect_decimal(
            at(row, 2),
            value
                .quantize(0, Rounding::HalfUp)
                .map(|d| d.to_sci_string()),
            source,
        );
        expect_decimal(
            at(row, 3),
            Ok(value.to_fixed_string().unwrap_or_default()),
            source,
        );
        expect_decimal(
            at(row, 4),
            value
                .quantize(-2, Rounding::HalfUp)
                .map(|d| d.to_sci_string()),
            source,
        );
    }
    for row in rows(&["decimal", "compare"]) {
        let a = Decimal::parse(text(at(row, 0))).unwrap();
        let b = Decimal::parse(text(at(row, 1))).unwrap();
        assert_eq!(bools(at(row, 2)), [a == b, a < b, a > b], "{a:?} {b:?}");
    }
}

#[test]
fn json_scanning_matches_cpython() {
    for row in rows(&["json", "loads"]) {
        let source = text(at(row, 0));
        let outcome = at(row, 1);
        match (
            pyjson::loads(source),
            outcome.get("ok"),
            outcome.get("message"),
        ) {
            (Ok(value), Some(ok), _) => assert_eq!(value.repr(), text(ok), "{source:?}"),
            (Err(DecodeError::Value(message)), _, Some(expected)) => {
                assert_eq!(message, text(expected), "{source:?}")
            }
            (found, _, _) => panic!("{source:?}: expected {outcome:?}, found {found:?}"),
        }
    }
    for row in rows(&["json", "dumps"]) {
        let mut out = String::new();
        pyjson::dump_str(text(at(row, 0)), &mut out);
        assert_eq!(out, text(at(row, 1)));
    }
    for row in rows(&["json", "floats"]) {
        let Json::Float(value) = pyjson::loads(text(at(row, 0))).unwrap() else {
            panic!()
        };
        let shown = py::float_repr(value);
        assert_eq!(shown, text(at(row, 1)));
        assert_eq!(
            Decimal::parse(&shown).unwrap().to_sci_string(),
            text(at(row, 2))
        );
    }
}

#[test]
fn str_methods_match_cpython() {
    for row in rows(&["str", "strings"]) {
        let s = text(row.get("s").unwrap());
        assert_eq!(py::casefold(s), text(row.get("casefold").unwrap()), "{s:?}");
        assert_eq!(py::upper(s), text(row.get("upper").unwrap()), "{s:?}");
        assert_eq!(py::lower(s), text(row.get("lower").unwrap()), "{s:?}");
        assert_eq!(py::strip(s), text(row.get("strip").unwrap()), "{s:?}");
        assert_eq!(py::repr_str(s), text(row.get("repr").unwrap()), "{s:?}");
        assert_eq!(
            py::splitlines(s),
            strings(row.get("splitlines").unwrap()),
            "{s:?}"
        );
        for (name, predicate) in [
            ("isdigit", py::is_digit as fn(char) -> bool),
            ("isalpha", py::is_alpha),
            ("isalnum", py::is_alnum),
            ("isspace", py::is_space),
            ("isdecimal", py::is_decimal),
        ] {
            let found: Vec<bool> = s.chars().map(predicate).collect();
            assert_eq!(found, bools(row.get(name).unwrap()), "{name} {s:?}");
        }
    }
    for row in rows(&["str", "ints"]) {
        let source = text(at(row, 0));
        let outcome = at(row, 1);
        match (
            py::parse_int(source),
            outcome.get("ok"),
            outcome.get("message"),
        ) {
            (Ok(value), Some(ok), _) => assert_eq!(value.to_string(), text(ok)),
            (Err(message), _, Some(expected)) => assert_eq!(message, text(expected)),
            (found, _, _) => panic!("{source:?}: {found:?} vs {outcome:?}"),
        }
    }
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn dumps_strings(value: &[Vec<String>]) -> String {
    let rows: Vec<String> = value
        .iter()
        .map(|row| {
            let cells: Vec<String> = row
                .iter()
                .map(|c| {
                    let mut out = String::new();
                    pyjson::dump_str(c, &mut out);
                    out
                })
                .collect();
            format!("[{}]", cells.join(", "))
        })
        .collect();
    format!("[{}]", rows.join(", "))
}

fn digest(json: &str) -> String {
    let hash = Sha256::digest(json.as_bytes());
    format!(
        "sha256:{}",
        hash.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
}

#[test]
fn chunked_decoding_and_csv_records_match_cpython() {
    for row in rows(&["csv"]) {
        let data = match at(row, 0) {
            Json::Str(hex) => unhex(hex),
            spec => {
                let mut data = unhex(text(spec.get("prefix").unwrap()));
                let repeat = unhex(text(spec.get("repeat").unwrap()));
                let Json::Int(count) = spec.get("count").unwrap() else {
                    panic!()
                };
                for _ in 0..count.to_string().parse::<usize>().unwrap() {
                    data.extend(&repeat);
                }
                data.extend(unhex(text(spec.get("tail").unwrap())));
                data
            }
        };
        let expected = at(row, 1);
        let mut reader = Reader::new(&data[..]);
        let mut records = Vec::new();
        let error = loop {
            match reader.next_record() {
                Ok(Some(record)) => records.push(record),
                Ok(None) => break Json::Null,
                Err(CsvError::Decode(e)) => {
                    break Json::Array(vec![
                        Json::Str("UnicodeDecodeError".into()),
                        Json::Str(e.to_string()),
                    ]);
                }
                Err(CsvError::Csv(m)) => {
                    break Json::Array(vec![Json::Str("csv.Error".into()), Json::Str(m)]);
                }
                Err(CsvError::Io(e)) => panic!("{e}"),
            }
        };
        let context = format!("{:?}", &data[..data.len().min(60)]);
        assert_eq!(&error, expected.get("error").unwrap(), "{context}");
        match expected.get("records").unwrap() {
            Json::Str(hash) => assert_eq!(&digest(&dumps_strings(&records)), hash, "{context}"),
            Json::Array(expected_rows) => {
                let expected_rows: Vec<Vec<String>> = expected_rows.iter().map(strings).collect();
                assert_eq!(records, expected_rows, "{context}");
            }
            other => panic!("{other:?}"),
        }
        let whole = SigDecoder::default()
            .decode(&data, true)
            .map(|t| py::universal_newlines(&t));
        match (
            whole,
            expected.get("whole").unwrap(),
            expected.get("whole_error").unwrap(),
        ) {
            (Ok(found), Json::Str(text), _) if text.starts_with("sha256:") && data.len() > 2000 => {
                let mut quoted = String::new();
                pyjson::dump_str(&found, &mut quoted);
                assert_eq!(&digest(&quoted), text, "{context}");
            }
            (Ok(found), Json::Str(text), _) => assert_eq!(&found, text, "{context}"),
            (Err(e), Json::Null, Json::Str(message)) => {
                assert_eq!(&e.to_string(), message, "{context}")
            }
            (found, w, m) => panic!("{context}: {found:?} vs {w:?} {m:?}"),
        }
    }
}

#[test]
fn pathlib_matches_cpython_for_this_platform() {
    let flavour = if cfg!(windows) { "windows" } else { "posix" };
    for row in rows(&["paths", flavour, "paths"]) {
        let source = text(row.get("text").unwrap());
        let path = PyPath::new(source);
        assert_eq!(path.display(), text(row.get("str").unwrap()), "{source:?}");
        assert_eq!(path.name(), text(row.get("name").unwrap()), "{source:?}");
        assert_eq!(path.stem(), text(row.get("stem").unwrap()), "{source:?}");
        assert_eq!(
            path.suffix(),
            text(row.get("suffix").unwrap()),
            "{source:?}"
        );
        assert_eq!(
            path.parent().display(),
            text(row.get("parent").unwrap()),
            "{source:?}"
        );
        assert_eq!(
            path.as_posix(),
            text(row.get("posix").unwrap()),
            "{source:?}"
        );
        let Json::Object(joins) = row.get("join").unwrap() else {
            panic!()
        };
        for (part, expected) in joins {
            assert_eq!(
                path.join(part).display(),
                text(expected),
                "{source:?} / {part:?}"
            );
        }
    }
    let texts: Vec<String> = rows(&["paths", flavour, "paths"])
        .iter()
        .map(|r| text(r.get("text").unwrap()).to_string())
        .collect();
    let mut sorted: Vec<PyPath> = texts.iter().map(|t| PyPath::new(t)).collect();
    sorted.sort();
    let shown: Vec<String> = sorted.iter().map(PyPath::display).collect();
    assert_eq!(shown, strings(section(&["paths", flavour, "sorted"])));
    for row in rows(&["paths", flavour, "relative"]) {
        let a = PyPath::new(text(at(row, 0)));
        let b = PyPath::new(text(at(row, 1)));
        assert_eq!(
            a.is_relative_to(&b),
            matches!(at(row, 2), Json::Bool(true)),
            "{a} {b}"
        );
        assert_eq!(a == b, matches!(at(row, 3), Json::Bool(true)), "{a} {b}");
    }
}

#[test]
fn difflib_and_textwrap_match_cpython() {
    let names = ["validate", "compare", "calculate", "explain", "report"];
    for row in rows(&["misc", "close"]) {
        let word = text(at(row, 0));
        let expected = strings(at(row, 1));
        assert_eq!(
            close_match(word, &names)
                .map(str::to_string)
                .into_iter()
                .collect::<Vec<_>>(),
            expected,
            "{word:?}"
        );
    }
    for row in rows(&["misc", "wrap"]) {
        let source = text(at(row, 0));
        let Json::Int(width) = at(row, 1) else {
            panic!()
        };
        let indent = text(at(row, 2));
        let width: i64 = width.to_string().parse().unwrap();
        assert_eq!(
            wrap(source, width, indent, indent),
            strings(at(row, 3)),
            "{source:?} {width}"
        );
    }
}

#[test]
fn argparse_matches_cpython() {
    let parser = cli::build_parser();
    for row in rows(&["argparse"]) {
        let Json::Object(env) = row.get("env").unwrap() else {
            panic!()
        };
        let lookup = |name: &str| {
            env.iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| text(v).to_string())
        };
        let terminal = Terminal::from(&lookup, &Console::pipes());
        let argv = strings(row.get("argv").unwrap());
        let expected_code = match row.get("code").unwrap() {
            Json::Int(code) => Some(code.to_string().parse::<i32>().unwrap()),
            _ => None,
        };
        let version = |s: &str| s.replace("0.3.0", cli::VERSION);
        let context = format!("{argv:?} {env:?}");
        match parser.parse_args(&argv, &terminal) {
            Outcome::Parsed(_) => assert_eq!(expected_code, None, "{context}"),
            Outcome::Exit {
                code,
                stdout,
                stderr,
            } => {
                assert_eq!(Some(code), expected_code, "{context}");
                assert_eq!(
                    stdout,
                    version(text(row.get("stdout").unwrap())),
                    "{context}"
                );
                assert_eq!(stderr, text(row.get("stderr").unwrap()), "{context}");
            }
        }
    }
}
