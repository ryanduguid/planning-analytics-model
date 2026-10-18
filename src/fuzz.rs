//! The fuzz target's checks, shared by the cargo-fuzz target and the tests
//! that replay its seed corpus.
//!
//! Fabricated bytes go to the TM1 rules parser. Every input must either parse
//! or be refused as a rules syntax error; parsing the same text twice must give
//! the same rule set; and every rule's qualifier must be one the grammar allows.

use crate::error::Error;
use crate::rules::{RuleSet, parse_rules};

/// Check one input; the error says which invariant failed.
pub fn check_input(data: &[u8]) -> Result<(), String> {
    check_with(data, |text| parse_rules(text, "Fuzz.rules"))
}

/// `check_input` with the parser supplied, so the tests can prove each check bites.
pub fn check_with(
    data: &[u8],
    parse: impl Fn(&str) -> Result<RuleSet, Error>,
) -> Result<(), String> {
    let text = String::from_utf8_lossy(data);
    let parsed = match parse(&text) {
        Ok(parsed) => parsed,
        Err(Error::RuleSyntax(_)) => return Ok(()),
        Err(other) => return Err(format!("the parser failed unexpectedly: {other:?}")),
    };
    let again = parse(&text).map_err(|e| format!("the second parse failed: {e:?}"))?;
    if format!("{again:?}") != format!("{parsed:?}") {
        return Err("parsing the same rules twice gave different results".into());
    }
    for rule in &parsed.rules {
        if !["", "N", "C"].contains(&rule.area.qualifier) {
            return Err(format!(
                "unexpected rule qualifier {:?}",
                rule.area.qualifier
            ));
        }
    }
    Ok(())
}
