//! The profit and loss the report subcommand prints.
//!
//! The shape of the statement lives here: which rows it carries, which slice of the
//! cube it reads, which lines are taken off the result below them, and how a figure
//! is written in a printed statement.

use crate::decimal::{Decimal, Rounding};
use crate::error::{EXIT_USAGE, Error, Result, Uncaught};
use crate::model::{Cube, Model};
use crate::py;

pub const REPORT_CUBE: &str = "PnL";

/// The profit and loss the report prints, in statement order. Each name is an
/// element of the Account dimension, consolidated or not.
pub const REPORT_ROWS: [&str; 8] = [
    "Revenue",
    "Direct Costs",
    "Gross Margin",
    "Employment Costs",
    "Overheads",
    "EBITDA",
    "Depreciation",
    "EBIT",
];

/// The slice the report reads: the whole year, the whole group, every cost
/// centre. Year and Version come from the command line instead.
pub const REPORT_SLICE: [(&str, &str); 3] = [
    ("Period", "FY"),
    ("Entity", "Group"),
    ("CostCentre", "All Cost Centres"),
];
const REPORT_MEASURE: (&str, &str) = ("PnLMeasure", "Amount");

/// Which report dimensions the caller supplies.
const FROM_COMMAND_LINE: [&str; 2] = ["Year", "Version"];

/// The rows that reduce the result above them.
const DEDUCTION_ROWS: [&str; 4] = [
    "Direct Costs",
    "Employment Costs",
    "Overheads",
    "Depreciation",
];

/// The 3 dimensions without which the 8 rows would all read the same cell.
const REQUIRED_DIMENSIONS: [&str; 3] = ["Year", "Version", "Account"];

fn decimal_failure(signal: crate::decimal::Signal) -> Error {
    Error::Uncaught(Uncaught::decimal(signal))
}

/// Whole dollars with thousands separators, rounded the way a ledger rounds.
/// A negative prints in brackets.
pub fn money(value: &Decimal) -> Result<String> {
    let dollars = value
        .quantize(0, Rounding::HalfUp)
        .map_err(decimal_failure)?;
    if dollars.is_zero() {
        // Rounding a small negative gives minus zero, which reads as a figure
        // the statement does not hold.
        return Ok("0".into());
    }
    if dollars.is_negative() {
        let positive = dollars.neg().map_err(decimal_failure)?;
        return Ok(format!("({})", positive.to_grouped_integer()));
    }
    Ok(dollars.to_grouped_integer())
}

/// The slice the report is fixed to, spelt out for an error message.
fn fixed_slice() -> String {
    REPORT_SLICE
        .iter()
        .map(|(name, element)| format!("{name} {}", py::repr_str(element)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// One resolved report row, keyed by dimension in the cube's order.
pub type Selection = Vec<(String, String)>;

fn wanted(dimension: &str, year: &str, version: &str, account: &str) -> Option<String> {
    let fixed = REPORT_SLICE
        .iter()
        .chain(std::iter::once(&REPORT_MEASURE))
        .find(|(d, _)| *d == dimension)
        .map(|(_, e)| e.to_string());
    match dimension {
        "Year" => Some(year.into()),
        "Version" => Some(version.into()),
        "Account" => Some(account.into()),
        _ => fixed,
    }
}

fn selection(
    model: &Model,
    cube: &Cube,
    year: &str,
    version: &str,
    account: &str,
) -> Result<Selection> {
    let mut resolved: Selection = Vec::new();
    for dimension in &cube.dimensions {
        let Some(element) = wanted(dimension, year, version, account) else {
            return Err(Error::cli(
                EXIT_USAGE,
                format!(
                    "cube {} has a dimension {} that the report has no element for, so this \
                     report does not fit this model. Use the calculate subcommand, which takes \
                     the whole coordinate from you",
                    py::repr_str(&cube.name),
                    py::repr_str(dimension)
                ),
            ));
        };
        let canonical = if FROM_COMMAND_LINE.contains(&dimension.as_str()) {
            crate::cli::element_or_error(model, dimension, &element, EXIT_USAGE)?
        } else {
            match model
                .hierarchy(dimension)
                .and_then(|h| h.resolve(&element).map(str::to_string))
            {
                Ok(canonical) => canonical,
                Err(error) if error.is_model_error() => {
                    return Err(Error::cli(
                        EXIT_USAGE,
                        format!(
                            "cube {}: the report reads {dimension} {}, which this model does not \
                             hold ({}). The report is fixed to {} and to the rows of a direct \
                             profit and loss, so a model that rolls up differently needs the \
                             calculate subcommand instead",
                            py::repr_str(&cube.name),
                            py::repr_str(&element),
                            error.message(),
                            fixed_slice()
                        ),
                    ));
                }
                Err(error) => return Err(error),
            }
        };
        // A dict assignment: a dimension listed twice keeps its first place.
        match resolved.iter_mut().find(|(d, _)| d == dimension) {
            Some(slot) => slot.1 = canonical,
            None => resolved.push((dimension.clone(), canonical)),
        }
    }
    Ok(resolved)
}

pub fn lookup<'a>(selection: &'a Selection, dimension: &str) -> Option<&'a str> {
    selection
        .iter()
        .find(|(d, _)| d == dimension)
        .map(|(_, e)| e.as_str())
}

/// One resolved coordinate per statement row, in statement order.
pub fn rows(model: &Model, cube: &Cube, year: &str, version: &str) -> Result<Vec<Selection>> {
    let absent: Vec<&str> = REQUIRED_DIMENSIONS
        .iter()
        .copied()
        .filter(|name| !cube.dimensions.iter().any(|d| d == name))
        .collect();
    if !absent.is_empty() {
        return Err(Error::cli(
            EXIT_USAGE,
            format!(
                "cube {} has no {} dimension, so a profit and loss cannot be built from it. \
                 This report does not fit this model, which is a different thing from the \
                 model being wrong",
                py::repr_str(&cube.name),
                absent.join(", ")
            ),
        ));
    }
    REPORT_ROWS
        .iter()
        .map(|account| selection(model, cube, year, version, account))
        .collect()
}

/// The machine report's schema, carried in its JSON document.
pub const SCHEMA_VERSION: &str = "profit-and-loss.v1";

/// One statement row of the machine report: its account, the coordinate
/// actually read, the stored value and its signed effect on the result.
pub struct DocumentRow<'a> {
    pub account: &'a str,
    pub coordinate: &'a Selection,
    pub value: Decimal,
    pub effect: Decimal,
}

/// Exact cell values and their signed effects, with the coordinates actually read.
pub fn document<'a>(selections: &'a [Selection], values: &[Decimal]) -> Vec<DocumentRow<'a>> {
    assert_eq!(
        selections.len(),
        values.len(),
        "one value per statement row"
    );
    REPORT_ROWS
        .iter()
        .zip(selections)
        .zip(values)
        .map(|((account, selection), value)| DocumentRow {
            account: lookup(selection, "Account").expect("an Account element"),
            coordinate: selection,
            value: value.clone(),
            // A zero cost stays 0, not -0, matching the text report.
            effect: if DEDUCTION_ROWS.contains(account) && !value.is_zero() {
                value.copy_negate()
            } else {
                value.clone()
            },
        })
        .collect()
}

/// The statement as printed, one string per line, blank lines included.
pub fn lines(cube: &Cube, selections: &[Selection], values: &[Decimal]) -> Result<Vec<String>> {
    let mut amounts = Vec::new();
    for (account, value) in REPORT_ROWS.iter().zip(values) {
        let signed = if DEDUCTION_ROWS.contains(account) {
            value.neg().map_err(decimal_failure)?
        } else {
            value.clone()
        };
        amounts.push(money(&signed)?);
    }
    let first = &selections[0];
    let labels: Vec<&str> = selections
        .iter()
        .map(|s| lookup(s, "Account").expect("an Account element"))
        .collect();
    let applied: Vec<&str> = REPORT_SLICE
        .iter()
        .filter_map(|(name, _)| lookup(first, name))
        .collect();
    let mut printed = vec![format!(
        "Profit and loss for {}, {}",
        lookup(first, "Year").expect("a Year element"),
        lookup(first, "Version").expect("a Version element")
    )];
    if applied.is_empty() {
        printed.push(format!(
            "{}, whole cube: it carries none of the usual reporting dimensions",
            cube.name
        ));
    } else {
        printed.push(format!("{} at {}", cube.name, applied.join(", ")));
    }
    printed.push(String::new());
    let label_width = labels.iter().map(|l| py::char_len(l)).max().unwrap_or(0);
    let amount_width = amounts.iter().map(|a| py::char_len(a)).max().unwrap_or(0);
    for (label, amount) in labels.iter().zip(&amounts) {
        let label_pad = label_width - py::char_len(label);
        let amount_pad = amount_width - py::char_len(amount);
        printed.push(format!(
            "{label}{}  {}{amount}",
            " ".repeat(label_pad),
            " ".repeat(amount_pad)
        ));
    }
    Ok(printed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn money_of(text: &str) -> String {
        money(&Decimal::parse(text).unwrap()).unwrap()
    }

    #[test]
    fn money_brackets_a_negative_and_prints_a_rounded_zero_plain() {
        assert_eq!(money_of("1234.5"), "1,235");
        assert_eq!(money_of("-1234.5"), "(1,235)");
        assert_eq!(money_of("-0.4"), "0");
        assert_eq!(money_of("999.5"), "1,000");
        assert!(money(&Decimal::parse("1E+30").unwrap()).is_err());
    }
}
