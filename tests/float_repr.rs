//! Numeric JSON weights pass through Python's `float` and `Decimal(str(value))`,
//! so the binary must spell every double exactly as CPython's `repr` does.
//! tests/data/float_repr.json holds CPython 3.15's `repr` for 8,007 doubles,
//! many of them ties between two equally short spellings.

use pacioliscube::py::float_repr;
use pacioliscube::pyjson::{self, Json};

#[test]
fn every_double_is_spelt_as_cpython_spells_it() {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/float_repr.json"),
    )
    .unwrap();
    let corpus = pyjson::loads(&text).unwrap();
    let Some(Json::Array(cases)) = corpus.get("cases") else {
        panic!("a cases array")
    };
    let mut wrong = Vec::new();
    for case in cases {
        let Json::Array(pair) = case else { panic!() };
        let (Json::Str(bits), Json::Str(expected)) = (&pair[0], &pair[1]) else {
            panic!()
        };
        let f = f64::from_bits(u64::from_str_radix(bits, 16).unwrap());
        let actual = float_repr(f);
        if &actual != expected {
            wrong.push(format!("{bits}: python {expected}, rust {actual}"));
        }
    }
    assert!(cases.len() > 8000);
    assert!(
        wrong.is_empty(),
        "{} of {}: {:?}",
        wrong.len(),
        cases.len(),
        &wrong[..wrong.len().min(10)]
    );
}

#[test]
fn a_tie_between_two_shortest_spellings_goes_to_the_even_digit() {
    // 1 + 2**-17 is exactly 1.00000762939453125.
    assert_eq!(float_repr(1.0 + 2f64.powi(-17)), "1.0000076293945312");
}
