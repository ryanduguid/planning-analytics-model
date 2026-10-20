//! The command line, being the way a shell script or a CI job drives the engine.
//!
//! `validate` reports structural findings, `calculate` prints cell values,
//! `explain` shows their calculation evidence, `compare` sets two data
//! snapshots side by side, and `report` prints a small profit and loss. Every
//! run reads: nothing here writes a file, touches a network, or changes the
//! model it is pointed at.
//!
//! Exit codes, which a shell script or a CI job can rely on:
//!
//! - 0: the command finished and found nothing wrong
//! - 1: a usage or input error, being a directory that is not there, a CSV that
//!   cannot be read, a cell reference that cannot be parsed, or a model the
//!   report subcommand does not fit
//! - 2: the model is not sound: it does not load, validation reports at least
//!   one error severity finding, or a fault the structural checks do not reach
//!   surfaces while a figure is being calculated
//! - 3: a calculation failed, which is a division by zero, a circular
//!   reference, or any other evaluation error
//!
//! Warnings never change the exit code.

use std::io::Write;

use crate::argparse::{Action, Choice, Kind, Namespace, Nargs, Outcome, Parser, Value};
use crate::data::load_data;
use crate::decimal::Decimal;
use crate::error::{
    EXIT_CALCULATION, EXIT_INVALID_MODEL, EXIT_OK, EXIT_USAGE, Error, Result, Uncaught,
};
use crate::evaluate::{self, CellStore, Evidence, Explanation, Item};
use crate::model::{Cube, Model, load_model};
use crate::py;
use crate::pyjson::{self, Out};
use crate::pypath::PyPath;
use crate::report;
use crate::terminal::Terminal;
use crate::validate::{ERROR, Finding, validate_model};

pub const DEFAULT_MODEL_ROOT: &str = "model";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where a run's output goes.
pub struct Streams<'a> {
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
}

impl Streams<'_> {
    /// `print(line)`: a failed write raises, as Python's OSError would.
    fn print(&mut self, line: &str) -> Result<()> {
        writeln!(self.out, "{line}").map_err(stream_failure)
    }

    fn eprint(&mut self, line: &str) -> Result<()> {
        writeln!(self.err, "{line}").map_err(stream_failure)
    }
}

fn stream_failure(error: std::io::Error) -> Error {
    Error::Os(py::stream_oserror(&error).1)
}

/// Resolve an element name, taking the exit code that fits who supplied it.
pub fn element_or_error(model: &Model, dimension: &str, name: &str, code: i32) -> Result<String> {
    match model
        .hierarchy(dimension)
        .and_then(|h| h.resolve(name).map(str::to_string))
    {
        Ok(found) => Ok(found),
        Err(error) if error.is_model_error() => Err(Error::cli(code, error.message())),
        Err(error) => Err(error),
    }
}

pub(crate) fn directory(argument: &str, what: &str) -> Result<PyPath> {
    let path = PyPath::new(argument);
    if !path.is_dir() {
        return Err(Error::cli(
            EXIT_USAGE,
            format!("{path}: there is no {what} directory there"),
        ));
    }
    Ok(path)
}

fn model_root(args: &Namespace) -> Result<PyPath> {
    // `args.model_dir or DEFAULT_MODEL_ROOT`: an empty argument also means the default.
    let argument = args.str("model_dir").filter(|s| !s.is_empty());
    directory(argument.unwrap_or(DEFAULT_MODEL_ROOT), "model")
}

pub(crate) fn load(root: &PyPath) -> Result<Model> {
    match load_model(root) {
        Ok(model) => Ok(model),
        Err(Error::Model(m) | Error::Evaluation(m) | Error::RuleSyntax(m)) => {
            Err(Error::cli(EXIT_INVALID_MODEL, m))
        }
        Err(Error::Decode(m)) => Err(Error::cli(
            EXIT_INVALID_MODEL,
            format!(
                "{root}: a file in the model tree is not UTF-8 text ({m}). Every file of a model \
                 tree is read as UTF-8"
            ),
        )),
        Err(Error::Os(m)) => Err(Error::cli(
            EXIT_INVALID_MODEL,
            format!("{root}: the model could not be read, {m}"),
        )),
        Err(other) => Err(other),
    }
}

pub(crate) fn cube<'a>(model: &'a Model, name: &str) -> Result<&'a Cube> {
    let key = py::casefold(name);
    if let Some(cube) = model.cubes.iter().find(|c| py::casefold(&c.name) == key) {
        return Ok(cube);
    }
    let names = model.cube_names().join(", ");
    Err(Error::cli(
        EXIT_USAGE,
        format!(
            "no cube named {} in model {}. The model holds {}",
            py::repr_str(name),
            model.name_repr(),
            if names.is_empty() { "no cubes" } else { &names }
        ),
    ))
}

fn format_finding(finding: &Finding) -> String {
    format!(
        "{} {} {}: {}",
        finding.severity, finding.code, finding.location, finding.message
    )
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

/// Stop calculate and report before they read a model that cannot be right.
pub(crate) fn refuse_broken_model(model: &Model, streams: &mut Streams) -> Result<()> {
    let errors: Vec<Finding> = validate_model(model)?
        .into_iter()
        .filter(|f| f.severity == ERROR)
        .collect();
    if errors.is_empty() {
        return Ok(());
    }
    for finding in &errors {
        streams.eprint(&format_finding(finding))?;
    }
    Err(Error::cli(
        EXIT_INVALID_MODEL,
        format!(
            "the model has {}, so nothing was calculated. Run the validate subcommand for the \
             findings in full",
            plural(errors.len(), "error")
        ),
    ))
}

pub(crate) fn calculation(error: Error) -> Error {
    match error {
        Error::Evaluation(m) => Error::cli(EXIT_CALCULATION, m),
        // Validation has already passed, so a model error at this point is a
        // defect the structural checks do not reach rather than a bad argument.
        Error::Model(m) => Error::cli(EXIT_INVALID_MODEL, m),
        other => other,
    }
}

pub(crate) fn evaluated(model: &Model, store: &CellStore) -> Result<CellStore> {
    evaluate::evaluate(model, store).map_err(calculation)
}

/// Turn CUBE:element,element into a cube name and a canonical coordinate.
fn parse_cell(model: &Model, text: &str) -> Result<(String, Vec<String>)> {
    let shape = || {
        Error::cli(
            EXIT_USAGE,
            format!(
                "cell reference {} is not in the form CUBE:element,element,...",
                py::repr_str(text)
            ),
        )
    };
    let Some((cube_name, coordinate_text)) = text.split_once(':') else {
        return Err(shape());
    };
    if py::strip(cube_name).is_empty() || py::strip(coordinate_text).is_empty() {
        return Err(shape());
    }
    let cube = cube(model, py::strip(cube_name))?;
    let names: Vec<&str> = coordinate_text.split(',').map(py::strip).collect();
    if names.iter().any(|n| n.is_empty()) {
        return Err(Error::cli(
            EXIT_USAGE,
            format!(
                "cell reference {} has an empty element name",
                py::repr_str(text)
            ),
        ));
    }
    if names.len() != cube.dimensions.len() {
        return Err(Error::cli(
            EXIT_USAGE,
            format!(
                "cell reference {} gives {} elements for cube {}, which takes {}, being {}",
                py::repr_str(text),
                names.len(),
                py::repr_str(&cube.name),
                cube.dimensions.len(),
                cube.dimensions.join(", ")
            ),
        ));
    }
    let mut coordinate = Vec::new();
    for (dimension, name) in cube.dimensions.iter().zip(names) {
        coordinate.push(element_or_error(model, dimension, name, EXIT_USAGE)?);
    }
    Ok((cube.name.clone(), coordinate))
}

/// A Decimal in full, never in exponent form and never through a float.
pub fn plain(value: &Decimal) -> Result<String> {
    let Some(mut text) = value.to_fixed_string() else {
        return Err(Error::Uncaught(Uncaught::new("MemoryError", "")));
    };
    if text.contains('.') {
        let trimmed = text.trim_end_matches('0').trim_end_matches('.').len();
        text.truncate(trimmed);
    }
    Ok(text)
}

fn validate_command(args: &Namespace, streams: &mut Streams) -> Result<i32> {
    let model = load(&model_root(args)?)?;
    let findings = validate_model(&model)?;
    let (errors, warnings): (Vec<&Finding>, Vec<&Finding>) =
        findings.iter().partition(|f| f.severity == ERROR);
    for finding in errors.iter().chain(&warnings) {
        streams.print(&format_finding(finding))?;
    }
    streams.print(&format!(
        "{}, {}",
        plural(errors.len(), "error"),
        plural(warnings.len(), "warning")
    ))?;
    Ok(if errors.is_empty() {
        EXIT_OK
    } else {
        EXIT_INVALID_MODEL
    })
}

fn parse_cells(model: &Model, args: &Namespace) -> Result<Vec<(String, Vec<String>)>> {
    args.list("cell")
        .iter()
        .map(|text| parse_cell(model, text))
        .collect()
}

fn calculate_command(args: &Namespace, streams: &mut Streams) -> Result<i32> {
    let model = load(&model_root(args)?)?;
    refuse_broken_model(&model, streams)?;
    let data = directory(args.str("data").unwrap_or_default(), "data")?;
    // Cell references are parsed before the CSVs are read so that a typo costs
    // a second rather than a full load and evaluation.
    let cells = parse_cells(&model, args)?;
    let store = evaluated(&model, &load_data(&model, &data)?)?;
    // Python's loop body raises out of the loop, so a failed format or print
    // stops the remaining cells from being evaluated, and keeps its own error.
    let mut body: Result<()> = Ok(());
    evaluate::consolidate_each(&model, &store, &cells, |i, value| {
        let (cube, coordinate) = &cells[i];
        let printed = plain(&value)
            .and_then(|text| streams.print(&format!("{cube}:{} = {text}", coordinate.join(","))));
        let go_on = printed.is_ok();
        body = printed;
        go_on
    })
    .map_err(calculation)?;
    body?;
    Ok(EXIT_OK)
}

pub(crate) fn report_command(args: &Namespace, streams: &mut Streams) -> Result<i32> {
    let model = load(&model_root(args)?)?;
    refuse_broken_model(&model, streams)?;
    let data = directory(args.str("data").unwrap_or_default(), "data")?;
    let cube = cube(&model, report::REPORT_CUBE)?;
    let selections = report::rows(
        &model,
        cube,
        args.str("year").unwrap_or_default(),
        args.str("version").unwrap_or_default(),
    )?;
    let store = evaluated(&model, &load_data(&model, &data)?)?;
    let cells: Vec<(String, Vec<String>)> = selections
        .iter()
        .map(|selection| {
            let coordinate = cube
                .dimensions
                .iter()
                .map(|d| {
                    report::lookup(selection, d)
                        .expect("a resolved element")
                        .to_string()
                })
                .collect();
            (cube.name.clone(), coordinate)
        })
        .collect();
    let mut values = Vec::new();
    evaluate::consolidate_many(&model, &store, &cells, |_, value| values.push(value))
        .map_err(calculation)?;
    match args.str("format").unwrap_or("text") {
        "text" => {
            for line in report::lines(cube, &selections, &values)? {
                streams.print(&line)?;
            }
        }
        "json" => streams.print(&pyjson::dumps(&report_json(cube, &selections, &values)?))?,
        _ => {
            let csv = report_csv(cube, &selections, &values)?;
            // csv.writer ends every row with its line terminator, and print adds none.
            streams.print(csv.strip_suffix('\n').unwrap_or(&csv))?;
        }
    }
    Ok(EXIT_OK)
}

fn report_json(cube: &Cube, selections: &[report::Selection], values: &[Decimal]) -> Result<Out> {
    let mut rows = Vec::new();
    for row in report::document(selections, values) {
        let coordinate = row
            .coordinate
            .iter()
            .map(|(dimension, element)| (dimension.clone(), Out::Str(element.clone())))
            .collect();
        rows.push(Out::Dict(vec![
            ("account".into(), Out::Str(row.account.to_string())),
            ("coordinate".into(), Out::Dict(coordinate)),
            ("value".into(), decimal_out(&row.value)?),
            ("effect".into(), decimal_out(&row.effect)?),
        ]));
    }
    Ok(Out::Dict(vec![
        (
            "schema_version".into(),
            Out::Str(report::SCHEMA_VERSION.into()),
        ),
        ("cube".into(), Out::Str(cube.name.clone())),
        ("rows".into(), Out::List(rows)),
    ]))
}

/// Keep spreadsheet formula prefixes literal when CSV labels are opened in Excel.
fn csv_label(value: &str) -> String {
    let formula = py::lstrip(value).starts_with(['=', '+', '-', '@']);
    if formula || value.starts_with(['\t', '\r', '\n']) {
        format!("'{value}")
    } else {
        value.to_string()
    }
}

/// One `csv.writer` row with the default dialect's minimal quoting.
fn csv_row(fields: &[String], out: &mut String) {
    let cells: Vec<String> = fields
        .iter()
        .map(|field| {
            if field.contains([',', '"', '\r', '\n']) {
                format!("\"{}\"", field.replace('"', "\"\""))
            } else {
                field.clone()
            }
        })
        .collect();
    out.push_str(&cells.join(","));
    out.push('\n');
}

fn report_csv(cube: &Cube, selections: &[report::Selection], values: &[Decimal]) -> Result<String> {
    // Build the whole document before emitting it, as the JSON report does.
    let mut out = String::new();
    let mut header = vec!["cube".to_string()];
    header.extend(cube.dimensions.iter().cloned());
    header.extend(["value".to_string(), "effect".to_string()]);
    csv_row(&header, &mut out);
    for row in report::document(selections, values) {
        let mut fields = vec![csv_label(&cube.name)];
        for dimension in &cube.dimensions {
            let element = report::lookup(row.coordinate, dimension).expect("a resolved element");
            fields.push(csv_label(element));
        }
        fields.push(plain(&row.value)?);
        fields.push(plain(&row.effect)?);
        csv_row(&fields, &mut out);
    }
    Ok(out)
}

/// Read the bundled synthetic model, without depending on a source checkout.
fn demo_command(streams: &mut Streams) -> Result<i32> {
    let unpacked = crate::demo::Unpacked::new().map_err(|error| {
        Error::cli(
            EXIT_USAGE,
            format!("the demo inputs could not be written to a temporary directory, {error}"),
        )
    })?;
    let path = |p: std::path::PathBuf| -> Result<Value> {
        Ok(Value::Str(crate::pypath::fs_text(p.as_os_str())?))
    };
    let mut args = Namespace::default();
    args.values.insert("model_dir", path(unpacked.model())?);
    args.values.insert("data", path(unpacked.data())?);
    args.values
        .insert("cell", Value::List(vec![crate::demo::CELL.to_string()]));
    calculate_command(&args, streams)
}

fn decimal_out(value: &Decimal) -> Result<Out> {
    Ok(Out::Str(plain(value)?))
}

fn item_out(item: &Item) -> Result<Out> {
    Ok(match item {
        Item::Input { cell, value } => Out::Dict(vec![
            ("cell".into(), Out::Str(cell.clone())),
            ("value".into(), decimal_out(value)?),
        ]),
        Item::Step {
            operator,
            left,
            right,
            result,
        } => Out::Dict(vec![
            ("operator".into(), Out::Str(operator.clone())),
            ("left".into(), decimal_out(left)?),
            ("right".into(), decimal_out(right)?),
            ("result".into(), decimal_out(result)?),
        ]),
        Item::Comparison {
            operator,
            left,
            right,
            result,
        } => Out::Dict(vec![
            ("operator".into(), Out::Str(operator.clone())),
            ("left".into(), decimal_out(left)?),
            ("right".into(), decimal_out(right)?),
            ("result".into(), Out::Bool(*result)),
        ]),
        Item::Branch { then } => Out::Dict(vec![
            ("operator".into(), Out::Str("IF".into())),
            (
                "branch".into(),
                Out::Str(if *then { "then" } else { "else" }.into()),
            ),
        ]),
        Item::Contribution {
            cell,
            value,
            weight,
            contribution,
        } => Out::Dict(vec![
            ("cell".into(), Out::Str(cell.clone())),
            ("value".into(), decimal_out(value)?),
            ("weight".into(), decimal_out(weight)?),
            ("contribution".into(), decimal_out(contribution)?),
        ]),
    })
}

fn evidence_out(evidence: &Evidence) -> Result<Out> {
    let mut pairs = vec![
        ("cube".to_string(), Out::Str(evidence.cube.clone())),
        (
            "coordinate".to_string(),
            Out::List(evidence.coordinate.iter().cloned().map(Out::Str).collect()),
        ),
    ];
    if let Some(kind) = evidence.kind {
        pairs.push(("kind".into(), Out::Str(kind.into())));
    }
    if let Some((file, line)) = &evidence.rule {
        pairs.push((
            "rule".into(),
            Out::Dict(vec![
                ("file".into(), file.clone().map_or(Out::Null, Out::Str)),
                ("line".into(), Out::Int(*line as i64)),
            ]),
        ));
    }
    for (field, items) in &evidence.lists {
        let items = items.iter().map(item_out).collect::<Result<Vec<_>>>()?;
        pairs.push((field.to_string(), Out::List(items)));
    }
    if let Some(value) = &evidence.value {
        pairs.push(("value".into(), decimal_out(value)?));
    }
    Ok(Out::Dict(pairs))
}

fn explanation_out(explanation: &Explanation) -> Result<Out> {
    let cells = explanation
        .cells
        .iter()
        .map(|(label, evidence)| Ok((label.clone(), evidence_out(evidence)?)))
        .collect::<Result<Vec<_>>>()?;
    Ok(Out::Dict(vec![
        (
            "requested".into(),
            Out::List(
                explanation
                    .requested
                    .iter()
                    .cloned()
                    .map(Out::Str)
                    .collect(),
            ),
        ),
        ("cells".into(), Out::Dict(cells)),
    ]))
}

pub(crate) fn explain_command(args: &Namespace, streams: &mut Streams) -> Result<i32> {
    let model = load(&model_root(args)?)?;
    refuse_broken_model(&model, streams)?;
    let data = directory(args.str("data").unwrap_or_default(), "data")?;
    let cells = parse_cells(&model, args)?;
    let explanation = evaluate::explain(&model, &load_data(&model, &data)?, &cells)?;
    // Finish the entire request before printing, so failures leave no partial JSON.
    let text = pyjson::dumps(&explanation_out(&explanation)?);
    streams.print(&text)?;
    Ok(EXIT_OK)
}

/// `item == other` for two evidence items as Python compared their dicts.
fn items_equal(a: &Item, b: &Item) -> bool {
    match (a, b) {
        (
            Item::Input {
                cell: c1,
                value: v1,
            },
            Item::Input {
                cell: c2,
                value: v2,
            },
        ) => c1 == c2 && v1 == v2,
        (
            Item::Step {
                operator: o1,
                left: l1,
                right: r1,
                result: x1,
            },
            Item::Step {
                operator: o2,
                left: l2,
                right: r2,
                result: x2,
            },
        ) => o1 == o2 && l1 == l2 && r1 == r2 && x1 == x2,
        (
            Item::Comparison {
                operator: o1,
                left: l1,
                right: r1,
                result: x1,
            },
            Item::Comparison {
                operator: o2,
                left: l2,
                right: r2,
                result: x2,
            },
        ) => o1 == o2 && l1 == l2 && r1 == r2 && x1 == x2,
        (Item::Branch { then: t1 }, Item::Branch { then: t2 }) => t1 == t2,
        (
            Item::Contribution {
                cell: c1,
                value: v1,
                weight: w1,
                contribution: x1,
            },
            Item::Contribution {
                cell: c2,
                value: v2,
                weight: w2,
                contribution: x2,
            },
        ) => c1 == c2 && v1 == v2 && w1 == w2 && x1 == x2,
        _ => false,
    }
}

/// `before == after` for two evidence dicts: key order does not matter.
fn evidence_equal(a: &Evidence, b: &Evidence) -> bool {
    let lists_equal = a.lists.len() == b.lists.len()
        && a.lists.iter().all(|(field, items)| {
            b.lists.iter().any(|(other_field, other)| {
                field == other_field
                    && items.len() == other.len()
                    && items.iter().zip(other).all(|(x, y)| items_equal(x, y))
            })
        });
    a.cube == b.cube
        && a.coordinate == b.coordinate
        && a.kind == b.kind
        && a.rule == b.rule
        && a.value == b.value
        && lists_equal
}

fn compare_command(args: &Namespace, streams: &mut Streams) -> Result<i32> {
    let model = load(&model_root(args)?)?;
    refuse_broken_model(&model, streams)?;
    let cells = parse_cells(&model, args)?;
    let previous_dir = directory(
        args.str("previous_data").unwrap_or_default(),
        "previous data",
    )?;
    let previous = evaluate::explain(&model, &load_data(&model, &previous_dir)?, &cells)?;
    let current_dir = directory(args.str("current_data").unwrap_or_default(), "current data")?;
    let current = evaluate::explain(&model, &load_data(&model, &current_dir)?, &cells)?;
    let mut changes = Vec::new();
    let mut named: Vec<&String> = Vec::new();
    for name in &previous.requested {
        if !named.contains(&name) {
            named.push(name);
        }
    }
    for name in named {
        let before = previous
            .cell(name)
            .and_then(|e| e.value.clone())
            .expect("a value");
        let after = current
            .cell(name)
            .and_then(|e| e.value.clone())
            .expect("a value");
        let difference = after
            .sub(&before)
            .map_err(|signal| Error::Uncaught(Uncaught::decimal(signal)))?;
        changes.push(Out::Dict(vec![
            ("cell".into(), Out::Str(name.clone())),
            ("previous".into(), decimal_out(&before)?),
            ("current".into(), decimal_out(&after)?),
            ("difference".into(), decimal_out(&difference)?),
        ]));
    }
    let mut labels: Vec<&String> = previous
        .cells
        .iter()
        .chain(&current.cells)
        .map(|(label, _)| label)
        .collect();
    labels.sort();
    labels.dedup();
    let mut evidence_changes = Vec::new();
    for label in labels {
        let before = previous.cell(label);
        let after = current.cell(label);
        let same = match (before, after) {
            (Some(x), Some(y)) => evidence_equal(x, y),
            _ => false,
        };
        if !same {
            let side = |e: Option<&Evidence>| e.map_or(Ok(Out::Null), evidence_out);
            evidence_changes.push(Out::Dict(vec![
                ("cell".into(), Out::Str(label.clone())),
                ("previous".into(), side(before)?),
                ("current".into(), side(after)?),
            ]));
        }
    }
    let document = Out::Dict(vec![
        (
            "schema_version".into(),
            Out::Str("scenario-comparison.v1".into()),
        ),
        ("changes".into(), Out::List(changes)),
        ("evidence_changes".into(), Out::List(evidence_changes)),
        ("previous_explanation".into(), explanation_out(&previous)?),
        ("current_explanation".into(), explanation_out(&current)?),
        (
            "scope".into(),
            Out::Str(
                "Same model, selected cells, offline evaluation only. Overlapping \
                 consolidations are not added together. Evidence changes do not attribute \
                 causation or prove TM1 agreement."
                    .into(),
            ),
        ),
    ]);
    streams.print(&pyjson::dumps(&document))?;
    Ok(EXIT_OK)
}

fn model_dir() -> Action {
    Action {
        options: vec![],
        dest: "model_dir",
        nargs: Nargs::Optional,
        required: false,
        help: Some("the directory holding tm1project.json, model by default"),
        metavar: Some("MODEL_DIR"),
        kind: Kind::Store,
    }
}

fn option(
    flag: &'static str,
    dest: &'static str,
    metavar: &'static str,
    help: Option<&'static str>,
    kind: Kind,
) -> Action {
    Action {
        options: vec![flag],
        dest,
        nargs: Nargs::One,
        required: true,
        help,
        metavar: Some(metavar),
        kind,
    }
}

const DATA_HELP: &str = "directory of long format CSV input";
const CELL_HELP: &str = "a cell to print, given once per cell, as CUBE:element,element,...";
const REPORT_DESCRIPTION: &str = "Print the group profit and loss for one year and version, at \
the FY period and every cost centre. A line that is taken off the result below it, being direct \
costs, employment costs, overheads and depreciation, prints in brackets, as does any figure that \
comes out negative, so the column reads in the direction it adds. Each line is rounded to whole \
dollars on its own, so a subtotal can sit a dollar away from the lines above it: in the shipped \
budget, EBITDA less depreciation prints as 18,684,639 while EBIT prints as 18,684,640.";

fn subparser(name: &str, description: Option<&'static str>, extra: Vec<Action>) -> Parser {
    let mut actions = vec![Action::help(), model_dir()];
    actions.extend(extra);
    Parser {
        prog: format!("pacioliscube {name}"),
        description,
        actions,
        choices: vec![],
    }
}

/// The parser `build_parser()` built, with every subcommand.
pub fn build_parser() -> Parser {
    let cell_parser = |name: &str| {
        subparser(
            name,
            None,
            vec![
                option("--data", "data", "DIR", Some(DATA_HELP), Kind::Store),
                option(
                    "--cell",
                    "cell",
                    "CUBE:ELEMENT,...",
                    Some(CELL_HELP),
                    Kind::Append,
                ),
            ],
        )
    };
    let choices = vec![
        Choice {
            name: "demo",
            help: "calculate gross margin from packaged synthetic inputs",
            parser: Parser {
                prog: "pacioliscube demo".into(),
                description: None,
                actions: vec![Action::help()],
                choices: vec![],
            },
        },
        Choice {
            name: "validate",
            help: "report structural findings in a model",
            parser: subparser("validate", None, vec![]),
        },
        Choice {
            name: "compare",
            help: "compare two data snapshots using the same model and requested cells",
            parser: subparser(
                "compare",
                None,
                vec![
                    option("--previous-data", "previous_data", "DIR", None, Kind::Store),
                    option("--current-data", "current_data", "DIR", None, Kind::Store),
                    option("--cell", "cell", "CUBE:ELEMENT,...", None, Kind::Append),
                ],
            ),
        },
        Choice {
            name: "calculate",
            help: "print the value at one or more cells",
            parser: cell_parser("calculate"),
        },
        Choice {
            name: "explain",
            help: "print JSON calculation evidence for one or more cells",
            parser: cell_parser("explain"),
        },
        Choice {
            name: "report",
            help: "print a profit and loss for a year and version",
            parser: subparser(
                "report",
                Some(REPORT_DESCRIPTION),
                vec![
                    option("--data", "data", "DIR", Some(DATA_HELP), Kind::Store),
                    option(
                        "--year",
                        "year",
                        "Y",
                        Some("an element of the Year dimension"),
                        Kind::Store,
                    ),
                    option(
                        "--version",
                        "version",
                        "V",
                        Some("an element of the Version dimension"),
                        Kind::Store,
                    ),
                    Action {
                        options: vec!["--format"],
                        dest: "format",
                        nargs: Nargs::One,
                        required: false,
                        help: Some(
                            "output format: rounded text by default; JSON and CSV keep exact \
                             decimal strings",
                        ),
                        metavar: None,
                        kind: Kind::Choice(&["text", "json", "csv"]),
                    },
                ],
            ),
        },
    ];
    Parser {
        prog: "pacioliscube".into(),
        description: Some(
            "Validate, calculate, explain and report on a Planning Analytics model tree. Reads \
             the model and its data, never writes.",
        ),
        actions: vec![
            Action::help(),
            Action {
                options: vec!["--version"],
                dest: "version",
                nargs: Nargs::Zero,
                required: false,
                help: Some("show program's version number and exit"),
                metavar: None,
                kind: Kind::Version(format!("pacioliscube {VERSION}")),
            },
            Action {
                options: vec![],
                dest: "command",
                nargs: Nargs::Parser,
                required: true,
                help: None,
                metavar: Some("SUBCOMMAND"),
                kind: Kind::Parsers,
            },
        ],
        choices,
    }
}

fn dispatch(args: &Namespace, streams: &mut Streams) -> Result<i32> {
    match args.command.as_deref() {
        Some("demo") => demo_command(streams),
        Some("validate") => validate_command(args, streams),
        Some("calculate") => calculate_command(args, streams),
        Some("explain") => explain_command(args, streams),
        Some("compare") => compare_command(args, streams),
        Some("report") => report_command(args, streams),
        _ => unreachable!("argparse requires a subcommand"),
    }
}

/// Run the command line on `argv` (without the program name) and return its exit code.
pub fn main(argv: &[String], terminal: &Terminal, streams: &mut Streams) -> i32 {
    let parser = build_parser();
    let args = match parser.parse_args(argv, terminal) {
        Outcome::Parsed(args) => args,
        Outcome::Exit {
            code,
            stdout,
            stderr,
        } => {
            // argparse has already written its own message. It exits 0 for --help
            // and --version and 2 for a bad argument, so only the codes change here.
            // argparse swallows a failed write; on buffered stderr the message
            // then fails again at the shutdown flush (exit 120). stdout's own
            // buffer gives that result through the final flush in main.
            let _ = streams.out.write_all(stdout.as_bytes());
            if streams.err.write_all(stderr.as_bytes()).is_err() && !python_unbuffered() {
                return EXIT_FLUSH_FAILED;
            }
            return if code == 0 { EXIT_OK } else { EXIT_USAGE };
        }
    };
    let (diagnostic, code) = match dispatch(&args, streams) {
        Ok(code) => return code,
        Err(error) => diagnostic(error),
    };
    match streams.eprint(&diagnostic) {
        Ok(()) => code,
        // Python's handler printed to stderr before returning its code. When
        // that print failed, the OSError escaped the handler (exit 1); with
        // stderr buffered it failed instead when stderr was flushed at
        // shutdown (exit 120). Either way the original code was lost.
        Err(_) if python_unbuffered() => EXIT_USAGE,
        Err(_) => EXIT_FLUSH_FAILED,
    }
}

/// The message and exit code a failed command reports.
pub(crate) fn diagnostic(error: Error) -> (String, i32) {
    match error {
        Error::Cli(code, message) => (format!("pacioliscube: {message}"), code),
        Error::Evaluation(message) => (format!("pacioliscube: {message}"), EXIT_CALCULATION),
        Error::Model(message) => (format!("pacioliscube: {message}"), EXIT_INVALID_MODEL),
        // A backstop: the places that read a file map their own failures.
        Error::Os(message) | Error::Decode(message) => {
            (format!("pacioliscube: {message}"), EXIT_USAGE)
        }
        Error::RuleSyntax(message) => (
            uncaught(&Uncaught::new(
                "pacioliscube.rules.RuleSyntaxError",
                message,
            )),
            EXIT_USAGE,
        ),
        Error::Uncaught(error) => (uncaught(&error), EXIT_USAGE),
    }
}

/// Python's exit status when flushing a standard stream fails at shutdown.
pub const EXIT_FLUSH_FAILED: i32 = 120;

/// `PYTHONUNBUFFERED`: a non-empty value made Python's standard streams unbuffered.
pub fn python_unbuffered() -> bool {
    std::env::var_os("PYTHONUNBUFFERED").is_some_and(|v| !v.is_empty())
}

/// Where the Python engine ended in a traceback, the binary says what was raised.
fn uncaught(error: &Uncaught) -> String {
    if error.message.is_empty() {
        format!("pacioliscube: unexpected error: {}", error.name)
    } else {
        format!(
            "pacioliscube: unexpected error: {}: {}",
            error.name, error.message
        )
    }
}
