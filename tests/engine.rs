//! The engine through its library calls: rules parsing, CSV loading,
//! evaluation, consolidation and explanation, as the Python suite tested them.

mod common;

use std::path::Path;

use common::*;
use pacioliscube::data::{load_into_store, read_csv};
use pacioliscube::decimal::Decimal;
use pacioliscube::error::Error;
use pacioliscube::evaluate::{
    CellStore, Evidence, Explanation, Item, consolidate_many, evaluate, explain,
};
use pacioliscube::model::{Model, load_model};
use pacioliscube::pypath::PyPath;
use pacioliscube::rules::{Expr, parse_rules};

fn load(root: &Path) -> Model {
    load_model(&PyPath::new(&s(root))).unwrap()
}

fn mini_model() -> Model {
    load(&mini())
}

/// The batch entry point, collected; an error after some values keeps them.
fn batch(
    model: &Model,
    store: &CellStore,
    cells: &[(&str, &[&str])],
) -> (Vec<Decimal>, Option<Error>) {
    let cells: Vec<(String, Vec<String>)> = cells
        .iter()
        .map(|(c, coordinate)| (c.to_string(), coord(coordinate)))
        .collect();
    let mut values = Vec::new();
    let error = consolidate_many(model, store, &cells, |_, v| values.push(v)).err();
    (values, error)
}

fn consolidate(model: &Model, store: &CellStore, cube: &str, coordinate: &[&str]) -> Decimal {
    let (values, error) = batch(model, store, &[(cube, coordinate)]);
    assert!(error.is_none(), "{error:?}");
    values[0].clone()
}

fn get(store: &CellStore, cube: &str, coordinate: &[&str]) -> Decimal {
    store.get(cube, &coord(coordinate))
}

fn set(store: &mut CellStore, cube: &str, coordinate: &[&str], value: &str) {
    store.set(cube, &coord(coordinate), d(value));
}

/// The mini model and a store seeded with `(cube, colour, measure, value)` inputs.
fn loaded_store(cells: &[(&str, &str, &str, &str)]) -> (Model, CellStore) {
    let model = mini_model();
    let mut store = CellStore::new();
    for (cube, colour, measure, value) in cells {
        set(&mut store, cube, &[colour, measure], value);
    }
    (model, store)
}

const MEASURES_WITH_MARGIN: &str = r#"{"Name": "Units", "Type": "Numeric"}, {"Name": "Price", "Type": "Numeric"}, {"Name": "Amount", "Type": "Numeric"}, {"Name": "Margin", "Type": "Numeric"}"#;

/// A 2 dimension Sales cube with the given rules, loaded.
fn build_model(root: &Path, rules_text: &str) -> Model {
    write_model(
        root,
        &Spec {
            rules: format!("SKIPCHECK;\n{rules_text}\nFEEDERS;\n['Units'] => ['Amount'];\n"),
            measures: MEASURES_WITH_MARGIN.into(),
            ..Spec::default()
        },
    );
    load(root)
}

fn model_with(root: &Path, rules: &str, measures: &str) -> Model {
    write_model(
        root,
        &Spec {
            rules: rules.into(),
            measures: measures.into(),
            ..Spec::default()
        },
    );
    load(root)
}

fn eval_err(model: &Model, store: &CellStore) -> Error {
    evaluate(model, store).unwrap_err()
}

// ---------------------------------------------------------------- rules parser

fn parse(text: &str) -> pacioliscube::rules::RuleSet {
    big_stack_parse(text).unwrap()
}

fn big_stack_parse(text: &str) -> Result<pacioliscube::rules::RuleSet, Error> {
    let text = text.to_string();
    big_stack(move || parse_rules(&text, "inline.rules"))
}

fn mini_rules() -> pacioliscube::rules::RuleSet {
    let path = mini().join("cubes").join("Sales.rules");
    parse_rules(&std::fs::read_to_string(&path).unwrap(), &s(&path)).unwrap()
}

#[test]
fn skipcheck_rules_and_feeders_are_read() {
    let rules = mini_rules();
    assert!(rules.skipcheck);
    assert_eq!(rules.rules.len(), 2);
    assert_eq!(rules.feeders.len(), 2);
    assert!(!parse("['A'] = N: 1;").skipcheck);
}

#[test]
fn doubled_quotes_decode_as_one_apostrophe_in_names() {
    let rule = &parse("['O''Brien'] = N: DB('Owner''s cube', 'O''Brien');").rules[0];
    assert_eq!(rule.area.elements, ["O'Brien"]);
    let Expr::CellRef(reference) = &rule.expression else {
        panic!()
    };
    assert_eq!(reference.cube.as_deref(), Some("Owner's cube"));
    assert_eq!(reference.coordinates, ["O'Brien"]);
}

#[test]
fn escaped_quotes_still_require_a_closing_quote_on_the_same_line() {
    for text in ["['O''Brien] = N: 1;", "['O''\nBrien'] = N: 1;"] {
        assert!(
            big_stack_parse(text)
                .unwrap_err()
                .message()
                .contains("unterminated quoted name")
        );
    }
}

#[test]
fn area_qualifier_and_expression_shapes_parse() {
    let rules = mini_rules();
    let rule = &rules.rules[0];
    assert_eq!(rule.area.qualifier, "N");
    assert_eq!(rule.area.elements, ["Amount"]);
    let Expr::BinaryOp('*', left, _) = &rule.expression else {
        panic!()
    };
    let Expr::CellRef(left) = left.as_ref() else {
        panic!()
    };
    assert_eq!(left.coordinates, ["Units"]);
    let Expr::BinaryOp(_, _, right) = &rules.rules[1].expression else {
        panic!()
    };
    let Expr::CellRef(reference) = right.as_ref() else {
        panic!()
    };
    assert_eq!(reference.cube.as_deref(), Some("Cost"));
    assert_eq!(reference.coordinates, ["!Colour", "Amount"]);
    let feeder = &rules.feeders[0];
    assert_eq!(feeder.area.elements, ["Units"]);
    assert_eq!(feeder.target.elements, ["Amount"]);
    assert!(feeder.target_cube.is_none());
}

#[test]
fn a_cross_cube_feeder_records_its_target_cube() {
    let feeder = &parse("FEEDERS;\n['Amount'] => DB('PnL', !Colour, 'Wages');").feeders[0];
    assert_eq!(feeder.target_cube.as_deref(), Some("PnL"));
    assert_eq!(feeder.target.elements, ["!Colour", "Wages"]);
}

#[test]
fn numbers_keep_their_written_precision_and_precedence_holds() {
    let Expr::Number(n) = &parse("['A'] = N: 1.10;").rules[0].expression else {
        panic!()
    };
    assert_eq!(n.to_sci_string(), "1.10");
    let Expr::BinaryOp('+', _, right) = &parse("['A'] = N: 1 + 2 * 3;").rules[0].expression else {
        panic!()
    };
    assert!(matches!(right.as_ref(), Expr::BinaryOp('*', _, _)));
    let Expr::BinaryOp('*', left, _) = &parse("['A'] = N: (1 + 2) * 3;").rules[0].expression else {
        panic!()
    };
    assert!(matches!(left.as_ref(), Expr::BinaryOp('+', _, _)));
    let Expr::BinaryOp('-', left, right) = &parse("['A'] = N: 10 - 3 - 2;").rules[0].expression
    else {
        panic!()
    };
    assert!(matches!(left.as_ref(), Expr::BinaryOp('-', _, _)));
    assert!(matches!(right.as_ref(), Expr::Number(n) if n.to_sci_string() == "2"));
    let Expr::BinaryOp('+', left, _) = &parse("['A'] = N: -5 + 1;").rules[0].expression else {
        panic!()
    };
    assert!(matches!(left.as_ref(), Expr::Number(n) if n.to_sci_string() == "-5"));
}

#[test]
fn if_areas_and_qualifiers_parse() {
    let Expr::If(condition, _, _) =
        &parse("['A'] = N: IF(['B'] > 0, ['B'], 0);").rules[0].expression
    else {
        panic!()
    };
    assert_eq!(condition.op, ">");
    assert_eq!(
        parse("['Budget', 'Amount'] = N: 1;").rules[0].area.elements,
        ["Budget", "Amount"]
    );
    assert_eq!(parse("['Rate'] = C: 1;").rules[0].area.qualifier, "C");
    assert_eq!(parse("['Rate'] = 1;").rules[0].area.qualifier, "");
    assert_eq!(
        parse("# a comment\n['A'] = N: 1;  # trailing\n")
            .rules
            .len(),
        1
    );
}

#[test]
fn syntax_errors_name_the_file_and_line() {
    let message = big_stack_parse("['A'] = N: 1;\n['B'] = N: ATTRN('Colour', !Colour, 'Rate');")
        .unwrap_err()
        .message();
    assert!(
        message.contains("ATTRN") && message.contains("inline.rules") && message.contains("line 2")
    );
    for text in ["['A'] = N: 1 + 2", "['A] = N: 1;", "Amount = N: 1;"] {
        assert!(
            matches!(big_stack_parse(text), Err(Error::RuleSyntax(_))),
            "{text}"
        );
    }
}

#[test]
fn deeply_nested_rules_are_a_syntax_error_not_a_crash() {
    let nested = format!("['Amount'] = {}1{};\n", "(".repeat(2000), ")".repeat(2000));
    let error = big_stack(move || parse_rules(&nested, "Sales.rules")).unwrap_err();
    assert_eq!(
        error.message(),
        "Sales.rules: rules are nested too deeply to read"
    );
}

// ---------------------------------------------------------------- data loading

const HEADER: &str = "Year,Version,Period,DriverMeasure,Value\n";

fn shipped() -> Model {
    load(&model_root())
}

fn write_csv(dir: &Path, body: &str) -> PyPath {
    let path = dir.join("input.csv");
    std::fs::write(&path, format!("{HEADER}{body}")).unwrap();
    PyPath::new(&s(&path))
}

fn drivers_rows(model: &Model, path: &PyPath) -> Result<Vec<(Vec<String>, Decimal)>, Error> {
    read_csv(path, model.cube("Drivers").unwrap(), model)
}

#[test]
fn a_long_format_csv_loads_into_canonical_coordinates() {
    let model = shipped();
    let tmp = tempfile::tempdir().unwrap();
    let rows = drivers_rows(
        &model,
        &write_csv(tmp.path(), "FY2026-27,Budget,Full Year,SG Rate,0.12\n"),
    )
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, ["FY2026-27", "Budget", "Full Year", "SG Rate"]);
    assert_eq!(rows[0].1.to_sci_string(), "0.12");
    let rows = drivers_rows(
        &model,
        &write_csv(tmp.path(), "fy2026-27,BUDGET,full year,sg rate,0.12\n"),
    )
    .unwrap();
    assert_eq!(rows[0].0, ["FY2026-27", "Budget", "Full Year", "SG Rate"]);
}

#[test]
fn conflicting_rows_name_the_file_and_both_row_numbers() {
    let model = shipped();
    for second in [
        "FY2026-27,Budget,Full Year,SG Rate,0.13",
        " fy2026-27 ,BUDGET,full year,sg rate,0.13",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_csv(
            tmp.path(),
            &format!("FY2026-27,Budget,Full Year,SG Rate,0.12\n\n{second}\n"),
        );
        let mut store = CellStore::new();
        let message = load_into_store(&model, "Drivers", &path, &mut store)
            .unwrap_err()
            .message();
        assert!(message.contains(&path.display()));
        assert!(message.contains("row 4") && message.contains("row 2"));
        assert!(message.contains("conflicting") && message.contains("SG Rate"));
        assert_eq!(
            get(
                &store,
                "Drivers",
                &["FY2026-27", "Budget", "Full Year", "SG Rate"]
            ),
            d("0.12")
        );
    }
}

#[test]
fn repeated_rows_with_equal_decimal_values_remain_valid() {
    let model = shipped();
    let tmp = tempfile::tempdir().unwrap();
    let path = write_csv(
        tmp.path(),
        "FY2026-27,Budget,Full Year,SG Rate,0.12\nfy2026-27,budget,full year,sg rate,0.120\n",
    );
    assert_eq!(drivers_rows(&model, &path).unwrap().len(), 1);
    assert_eq!(
        load_into_store(&model, "Drivers", &path, &mut CellStore::new()).unwrap(),
        1
    );
}

#[test]
fn bad_rows_are_model_errors_with_row_context() {
    let model = shipped();
    let cases: [(&str, &[&str]); 6] = [
        (
            "FY2026-27,Budget,FY,SG Rate,0.12\n",
            &["row 2", "consolidated"],
        ),
        (
            "FY2026-27,Budget,q1,SG Rate,0.12\n",
            &["row 2", "consolidated"],
        ),
        (
            "FY2026-27,Budget,Full Year,SG Rate,0.12\nFY2026-27,Budget,Full Year,Ghost,1\n",
            &["row 3", "Ghost"],
        ),
        (
            "FY2026-27,Budget,Full Year,SG Rate,twelve\n",
            &["row 2", "twelve"],
        ),
        ("FY2026-27,Budget,Full Year,SG Rate,NaN\n", &["finite"]),
        ("FY2026-27,Budget,Full Year,SG Rate\n", &["row 2"]),
    ];
    for (body, expected) in cases {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_csv(tmp.path(), body);
        let error = drivers_rows(&model, &path).unwrap_err();
        assert!(matches!(error, Error::Model(_)));
        let message = error.message();
        assert!(
            message.contains(&path.display()) || !body.contains("FY,"),
            "{message}"
        );
        for part in expected {
            assert!(message.contains(part), "{message}");
        }
    }
}

#[test]
fn a_header_with_the_wrong_column_count_is_a_model_error() {
    let model = shipped();
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("input.csv");
    std::fs::write(&path, "Year,Version,Value\n").unwrap();
    let message = drivers_rows(&model, &PyPath::new(&s(&path)))
        .unwrap_err()
        .message();
    assert!(message.contains("row 1"));
}

#[test]
fn blank_rows_are_skipped_and_values_stay_decimal() {
    let model = shipped();
    let tmp = tempfile::tempdir().unwrap();
    let path = write_csv(tmp.path(), "\nFY2026-27,Budget,Full Year,SG Rate,0.1\n\n");
    let rows = drivers_rows(&model, &path).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1.to_sci_string(), "0.1");
}

#[test]
fn a_byte_order_mark_is_tolerated() {
    let model = shipped();
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("input.csv");
    let mut bytes = b"\xef\xbb\xbf".to_vec();
    bytes.extend(format!("{HEADER}FY2026-27,Budget,Full Year,SG Rate,0.12\n").bytes());
    std::fs::write(&path, bytes).unwrap();
    assert_eq!(
        drivers_rows(&model, &PyPath::new(&s(&path))).unwrap().len(),
        1
    );
}

#[test]
fn every_shipped_example_csv_loads_and_never_writes_a_calculated_cell() {
    let model = shipped();
    let mut store = CellStore::new();
    for (cube, name) in [
        ("Drivers", "drivers.csv"),
        ("Workforce", "workforce.csv"),
        ("Revenue", "revenue.csv"),
        ("Capex", "capex.csv"),
        ("PnL", "pnl-direct.csv"),
    ] {
        let path = PyPath::new(&s(&examples().join(name)));
        assert!(
            load_into_store(&model, cube, &path, &mut store).unwrap() > 0,
            "{cube}"
        );
    }
    let calculated = [
        "Contract Revenue",
        "Plant Hire Revenue",
        "Fuel",
        "Wages and Salaries",
        "Superannuation",
        "Payroll Tax",
        "Depreciation",
    ];
    for (coordinate, _) in store.items_in("PnL") {
        assert!(
            !calculated.contains(&coordinate[5].as_str()),
            "{coordinate:?}"
        );
    }
}

// ---------------------------------------------------------------- evaluation

#[test]
fn a_rule_calculates_a_leaf_cell_and_inputs_survive() {
    let (model, store) = loaded_store(&[
        ("Sales", "Red", "Units", "10"),
        ("Sales", "Red", "Price", "2.50"),
    ]);
    let result = evaluate(&model, &store).unwrap();
    assert_eq!(
        get(&result, "Sales", &["Red", "Amount"]).to_sci_string(),
        "25.00"
    );
    assert_eq!(get(&result, "Sales", &["Red", "Units"]), d("10"));
    assert_eq!(get(&CellStore::new(), "Sales", &["Red", "Units"]), d("0"));
}

#[test]
fn a_rule_reading_another_cube_resolves_db() {
    let (model, store) = loaded_store(&[
        ("Sales", "Red", "Units", "10"),
        ("Sales", "Red", "Price", "2.50"),
        ("Cost", "Red", "Amount", "4"),
        ("Cost", "Blue", "Amount", "6"),
    ]);
    let result = evaluate(&model, &store).unwrap();
    // Margin is Amount less the same colour's cost: 25.00 less 4.
    assert_eq!(
        get(&result, "Sales", &["Red", "Margin"]).to_sci_string(),
        "21.00"
    );
}

#[test]
fn current_element_marker_binds_to_the_cell_being_calculated() {
    let (model, store) = loaded_store(&[
        ("Sales", "Red", "Units", "10"),
        ("Sales", "Red", "Price", "1"),
        ("Sales", "Blue", "Units", "4"),
        ("Sales", "Blue", "Price", "1"),
        ("Cost", "Red", "Amount", "3"),
        ("Cost", "Blue", "Amount", "1"),
    ]);
    let result = evaluate(&model, &store).unwrap();
    assert_eq!(get(&result, "Sales", &["Red", "Margin"]), d("7"));
    assert_eq!(get(&result, "Sales", &["Blue", "Margin"]), d("3"));
}

#[test]
fn the_two_divide_operators() {
    let tmp = tempfile::tempdir().unwrap();
    let model = build_model(tmp.path(), "['Amount'] = N: ['Units'] \\ ['Price'];");
    let mut store = CellStore::new();
    set(&mut store, "Sales", &["Red", "Units"], "10");
    assert_eq!(
        get(
            &evaluate(&model, &store).unwrap(),
            "Sales",
            &["Red", "Amount"]
        ),
        d("0")
    );

    let tmp = tempfile::tempdir().unwrap();
    let model = build_model(tmp.path(), "['Amount'] = N: ['Units'] / ['Price'];");
    let error = eval_err(&model, &store);
    assert!(matches!(&error, Error::Evaluation(m) if m.contains("division by zero")));
    set(&mut store, "Sales", &["Red", "Price"], "4");
    set(&mut store, "Sales", &["Blue", "Price"], "1");
    assert_eq!(
        get(
            &evaluate(&model, &store).unwrap(),
            "Sales",
            &["Red", "Amount"]
        ),
        d("2.5")
    );
}

#[test]
fn rules_depending_on_rules_evaluate_in_order() {
    let (model, store) = loaded_store(&[
        ("Sales", "Red", "Units", "3"),
        ("Sales", "Red", "Price", "3"),
    ]);
    let result = evaluate(&model, &store).unwrap();
    assert_eq!(get(&result, "Sales", &["Red", "Amount"]), d("9"));
    assert_eq!(get(&result, "Sales", &["Red", "Margin"]), d("9"));
}

#[test]
fn a_circular_rule_raises_circular_reference() {
    let tmp = tempfile::tempdir().unwrap();
    let model = build_model(
        tmp.path(),
        "['Amount'] = N: ['Margin'];\n['Margin'] = N: ['Amount'];",
    );
    let Error::Evaluation(message) = eval_err(&model, &CellStore::new()) else {
        panic!()
    };
    assert!(message.starts_with("a rule depends on its own result: "));
    assert!(message.contains("Amount") && message.contains("Margin"));
}

#[test]
fn evaluation_is_decimal_not_float() {
    let tmp = tempfile::tempdir().unwrap();
    let model = build_model(tmp.path(), "['Amount'] = N: ['Units'] + ['Price'];");
    let mut store = CellStore::new();
    set(&mut store, "Sales", &["Red", "Units"], "0.1");
    set(&mut store, "Sales", &["Red", "Price"], "0.2");
    assert_eq!(
        get(
            &evaluate(&model, &store).unwrap(),
            "Sales",
            &["Red", "Amount"]
        )
        .to_sci_string(),
        "0.3"
    );
}

#[test]
fn if_evaluates_every_comparison_operator() {
    let cases = [
        ("=", "2", "2", "10"),
        ("=", "3", "2", "20"),
        ("<>", "3", "2", "10"),
        ("<>", "2", "2", "20"),
        ("<", "2", "3", "10"),
        ("<", "2", "2", "20"),
        ("<=", "2", "2", "10"),
        ("<=", "3", "2", "20"),
        (">", "3", "2", "10"),
        (">", "2", "2", "20"),
        (">=", "2", "2", "10"),
        (">=", "2", "3", "20"),
    ];
    for (operator, units, price, expected) in cases {
        let tmp = tempfile::tempdir().unwrap();
        let model = build_model(
            tmp.path(),
            &format!("['Amount'] = N: IF(['Units'] {operator} ['Price'], 10, 20);"),
        );
        let mut store = CellStore::new();
        set(&mut store, "Sales", &["Red", "Units"], units);
        set(&mut store, "Sales", &["Red", "Price"], price);
        assert_eq!(
            get(
                &evaluate(&model, &store).unwrap(),
                "Sales",
                &["Red", "Amount"]
            ),
            d(expected),
            "{operator}"
        );
    }
}

#[test]
fn consolidation_follows_weights_and_qualifiers() {
    let (model, store) = loaded_store(&[
        ("Sales", "Red", "Units", "10"),
        ("Sales", "Red", "Price", "2"),
    ]);
    let result = evaluate(&model, &store).unwrap();
    // Total is consolidated, so the N rule leaves it to consolidation, which sums 20.
    assert_eq!(
        consolidate(&model, &result, "Sales", &["Total", "Amount"]),
        d("20")
    );

    let (model, store) = loaded_store(&[
        ("Sales", "Red", "Units", "10"),
        ("Sales", "Red", "Price", "1"),
        ("Sales", "Blue", "Units", "5"),
        ("Sales", "Blue", "Price", "1"),
    ]);
    let result = evaluate(&model, &store).unwrap();
    assert_eq!(
        consolidate(&model, &result, "Sales", &["Total", "Amount"]),
        d("15")
    );

    let (model, store) = loaded_store(&[
        ("Sales", "Red", "Units", "10"),
        ("Sales", "Red", "Price", "1"),
        ("Sales", "Blue", "Units", "5"),
        ("Sales", "Blue", "Price", "1"),
        ("Sales", "Contra", "Units", "4"),
        ("Sales", "Contra", "Price", "1"),
    ]);
    let result = evaluate(&model, &store).unwrap();
    assert_eq!(
        consolidate(&model, &result, "Sales", &["Total", "Amount"]),
        d("11")
    );

    let (model, store) = loaded_store(&[
        ("Sales", "Red", "Units", "7"),
        ("Sales", "Red", "Price", "1"),
    ]);
    let result = evaluate(&model, &store).unwrap();
    assert_eq!(
        consolidate(&model, &result, "Sales", &["Red", "Amount"]),
        d("7")
    );
}

#[test]
fn a_rule_naming_an_element_no_dimension_holds_is_a_model_error() {
    let tmp = tempfile::tempdir().unwrap();
    let model = build_model(tmp.path(), "['Ghost'] = N: 1;");
    let error = eval_err(&model, &CellStore::new());
    assert!(matches!(&error, Error::Model(m) if m.contains("Ghost")));
}

#[test]
fn consolidation_rejects_incorrect_coordinate_width() {
    let (model, store) = loaded_store(&[("Sales", "Red", "Units", "3")]);
    for width in [&[][..], &["Red"][..], &["Red", "Units", "unexpected"][..]] {
        let (values, error) = batch(&model, &store, &[("Sales", width)]);
        assert!(values.is_empty());
        assert!(matches!(&error, Some(Error::Model(m)) if m.starts_with("coordinate has")));
        let (values, error) = batch(
            &model,
            &store,
            &[("Sales", &["Red", "Units"]), ("Sales", width)],
        );
        assert_eq!(values, [d("3")]);
        assert!(matches!(&error, Some(Error::Model(m)) if m.contains("dimensions")));
    }
}

#[test]
fn a_c_rule_overrides_consolidation() {
    let tmp = tempfile::tempdir().unwrap();
    let model = build_model(
        tmp.path(),
        "['Amount'] = N: ['Units'] * ['Price'];\n['Price'] = C: 99;",
    );
    let mut store = CellStore::new();
    set(&mut store, "Sales", &["Red", "Price"], "1");
    set(&mut store, "Sales", &["Blue", "Price"], "2");
    let result = evaluate(&model, &store).unwrap();
    assert_eq!(
        consolidate(&model, &result, "Sales", &["Total", "Price"]),
        d("99")
    );
}

#[test]
fn batch_consolidation_preserves_order_weights_and_fresh_inputs() {
    let (model, mut store) = loaded_store(&[
        ("Sales", "Red", "Units", "10"),
        ("Sales", "Red", "Price", "2"),
        ("Sales", "Blue", "Units", "5"),
        ("Sales", "Blue", "Price", "3"),
        ("Sales", "Contra", "Units", "4"),
        ("Sales", "Contra", "Price", "1"),
    ]);
    let cells: [(&str, &[&str]); 2] = [
        ("Sales", &["Total", "Amount"]),
        ("Sales", &["Red", "Amount"]),
    ];
    let others = d("5").mul_(&d("3")).sub_(&d("4").mul_(&d("1")));
    let red = d("10").mul_(&d("2"));
    assert_eq!(batch(&model, &store, &cells).0, [red.add_(&others), red]);
    set(&mut store, "Sales", &["Red", "Price"], "4");
    let red = d("10").mul_(&d("4"));
    assert_eq!(batch(&model, &store, &cells).0, [red.add_(&others), red]);
    assert!(batch(&model, &store, &[]).0.is_empty());
}

// ---------------------------------------------------------------- explanation

fn node<'a>(evidence: &'a Explanation, cube: &str, coordinate: &[&str]) -> &'a Evidence {
    let parts: Vec<String> = coordinate.iter().map(|c| format!("'{c}'")).collect();
    let label = format!("{cube}[{}]", parts.join(", "));
    evidence.cell(&label).unwrap_or_else(|| panic!("{label}"))
}

fn explained(
    model: &Model,
    store: &CellStore,
    cells: &[(&str, &[&str])],
) -> Result<Explanation, Error> {
    let cells: Vec<(String, Vec<String>)> = cells
        .iter()
        .map(|(c, x)| (c.to_string(), coord(x)))
        .collect();
    explain(model, store, &cells)
}

fn list<'a>(evidence: &'a Evidence, field: &str) -> &'a [Item] {
    evidence
        .lists
        .iter()
        .find(|(f, _)| *f == field)
        .map_or(&[][..], |(_, items)| items.as_slice())
}

#[test]
fn rule_explanation_includes_cross_cube_inputs_and_source() {
    let (model, store) = loaded_store(&[
        ("Sales", "Red", "Units", "3"),
        ("Sales", "Red", "Price", "0.10"),
        ("Cost", "Red", "Amount", "0.20"),
    ]);
    let before: Vec<String> = store
        .items()
        .map(|(c, x, v)| format!("{c}{x:?}{}", v.to_sci_string()))
        .collect();
    let evidence = explained(&model, &store, &[("Sales", &["red", "margin"])]).unwrap();
    let margin = node(&evidence, "Sales", &["Red", "Margin"]);
    assert_eq!(margin.value, Some(d("3").mul_(&d("0.10")).sub_(&d("0.20"))));
    assert_eq!(margin.kind, Some("rule"));
    assert_eq!(
        margin.rule,
        Some((Some("cubes/Sales.rules".to_string()), 5))
    );
    let inputs = list(margin, "inputs");
    assert!(
        matches!(&inputs[0], Item::Input { cell, value } if cell == "Sales['Red', 'Amount']" && value.to_sci_string() == "0.30")
    );
    assert!(
        matches!(&inputs[1], Item::Input { cell, value } if cell == "Cost['Red', 'Amount']" && *value == d("0.20"))
    );
    assert!(
        matches!(&list(margin, "steps")[0], Item::Step { operator, result, .. } if operator == "-" && result.to_sci_string() == "0.10")
    );
    assert_eq!(evidence.requested, ["Sales['Red', 'Margin']"]);
    let after: Vec<String> = store
        .items()
        .map(|(c, x, v)| format!("{c}{x:?}{}", v.to_sci_string()))
        .collect();
    assert_eq!(before, after);
}

#[test]
fn consolidation_explanation_preserves_contra_weights_and_repeated_roots() {
    let (model, store) = loaded_store(&[
        ("Sales", "Red", "Units", "10"),
        ("Sales", "Red", "Price", "2"),
        ("Sales", "Blue", "Units", "5"),
        ("Sales", "Blue", "Price", "3"),
        ("Sales", "Contra", "Units", "4"),
        ("Sales", "Contra", "Price", "1"),
    ]);
    let cells: Vec<(&str, &[&str])> = [
        ("Sales", &["Total", "Amount"][..]),
        ("Sales", &["Red", "Amount"][..]),
    ]
    .repeat(2);
    let evidence = explained(&model, &store, &cells).unwrap();
    let total = node(&evidence, "Sales", &["Total", "Amount"]);
    let expected = d("10")
        .mul_(&d("2"))
        .add_(&d("5").mul_(&d("3")))
        .sub_(&d("4"));
    assert_eq!(total.value, Some(expected.clone()));
    assert_eq!(
        expected,
        consolidate(&model, &store, "Sales", &["Total", "Amount"])
    );
    assert_eq!(total.kind, Some("consolidation"));
    let rows: Vec<(String, String, String)> = list(total, "contributions")
        .iter()
        .map(|item| match item {
            Item::Contribution {
                value,
                weight,
                contribution,
                ..
            } => (
                weight.to_sci_string(),
                value.to_sci_string(),
                contribution.to_sci_string(),
            ),
            _ => panic!(),
        })
        .collect();
    let weights: Vec<Decimal> = rows.iter().map(|r| d(&r.0)).collect();
    let values: Vec<Decimal> = rows.iter().map(|r| d(&r.1)).collect();
    let contributions: Vec<Decimal> = rows.iter().map(|r| d(&r.2)).collect();
    assert_eq!(weights, [d("1"), d("1"), d("-1")]);
    assert_eq!(values, [d("20"), d("15"), d("4")]);
    assert_eq!(contributions, [d("20"), d("15"), d("-4")]);
    assert_eq!(evidence.cells.len(), 10);
    assert_eq!(
        evidence.requested,
        ["Sales['Total', 'Amount']", "Sales['Red', 'Amount']"].repeat(2)
    );
}

#[test]
fn explanation_distinguishes_input_zero_from_default_zero() {
    let model = mini_model();
    let mut store = CellStore::new();
    set(&mut store, "Sales", &["Red", "Units"], "0");
    let evidence = explained(
        &model,
        &store,
        &[("Sales", &["Red", "Units"]), ("Sales", &["Blue", "Units"])],
    )
    .unwrap();
    assert_eq!(
        node(&evidence, "Sales", &["Red", "Units"]).kind,
        Some("input")
    );
    assert_eq!(
        node(&evidence, "Sales", &["Blue", "Units"]).kind,
        Some("default_zero")
    );
}

#[test]
fn shared_leaf_keeps_each_weighted_path() {
    let tmp = tempfile::tempdir().unwrap();
    let root = write_model(tmp.path(), &Spec::default());
    let hierarchy = root
        .join("dimensions")
        .join("Colour.hierarchies")
        .join("Colour.json");
    std::fs::write(
        &hierarchy,
        r#"{"Name": "Colour", "Elements": [{"Name": "Total", "Type": "Consolidated"}, {"Name": "Left", "Type": "Consolidated"}, {"Name": "Right", "Type": "Consolidated"}, {"Name": "Red", "Type": "Numeric"}, {"Name": "Blue", "Type": "Numeric"}], "Edges": [{"ParentName": "Total", "ComponentName": "Left", "Weight": "2"}, {"ParentName": "Total", "ComponentName": "Right", "Weight": "-1"}, {"ParentName": "Left", "ComponentName": "Red", "Weight": "0.5"}, {"ParentName": "Left", "ComponentName": "Blue", "Weight": "1"}, {"ParentName": "Right", "ComponentName": "Red", "Weight": "3"}]}"#,
    )
    .unwrap();
    let model = load(&root);
    let mut store = CellStore::new();
    set(&mut store, "Sales", &["Red", "Units"], "0.1");
    set(&mut store, "Sales", &["Blue", "Units"], "0.2");
    let evidence = explained(&model, &store, &[("Sales", &["Total", "Units"])]).unwrap();
    // 2 * (0.5 * 0.1 + 0.2) - 3 * 0.1 = 0.2; Red contributes along both paths.
    let expected = d("2")
        .mul_(&d("0.5").mul_(&d("0.1")).add_(&d("0.2")))
        .sub_(&d("3").mul_(&d("0.1")));
    assert_eq!(
        node(&evidence, "Sales", &["Total", "Units"]).value,
        Some(expected)
    );
    assert_eq!(evidence.cells.len(), 5);
    for (_, calculation) in &evidence.cells {
        if calculation.kind != Some("consolidation") {
            continue;
        }
        let mut sum = Decimal::zero();
        for item in list(calculation, "contributions") {
            let Item::Contribution {
                cell,
                value,
                weight,
                contribution,
            } = item
            else {
                panic!()
            };
            sum = sum.add_(contribution);
            assert_eq!(
                Some(value),
                evidence.cell(cell).and_then(|e| e.value.as_ref())
            );
            assert_eq!(*contribution, value.mul_(weight));
        }
        assert_eq!(Some(sum), calculation.value);
    }
}

#[test]
fn safe_divide_records_its_actual_result() {
    for (divisor, expected) in [("0", "0"), ("2", "5")] {
        let tmp = tempfile::tempdir().unwrap();
        let model = build_model(tmp.path(), "['Amount'] = N: ['Units'] \\ ['Price'];");
        let mut store = CellStore::new();
        set(&mut store, "Sales", &["Red", "Units"], "10");
        set(&mut store, "Sales", &["Red", "Price"], divisor);
        let evidence = explained(&model, &store, &[("Sales", &["Red", "Amount"])]).unwrap();
        let steps = list(node(&evidence, "Sales", &["Red", "Amount"]), "steps");
        assert_eq!(steps.len(), 1);
        assert!(
            matches!(&steps[0], Item::Step { operator, left, right, result }
            if operator == "\\" && *left == d("10") && *right == d(divisor) && *result == d(expected))
        );
    }
}

#[test]
fn explanation_recalculates_rules_and_does_not_reuse_a_previous_run() {
    let (model, mut store) = loaded_store(&[
        ("Sales", "Red", "Units", "3"),
        ("Sales", "Red", "Price", "2"),
    ]);
    set(&mut store, "Sales", &["Red", "Amount"], "999");
    let cells: [(&str, &[&str]); 1] = [("Sales", &["Red", "Amount"])];
    let first = explained(&model, &store, &cells).unwrap();
    set(&mut store, "Sales", &["Red", "Price"], "4");
    let second = explained(&model, &store, &cells).unwrap();
    assert_eq!(
        node(&first, "Sales", &["Red", "Amount"]).value,
        Some(d("6"))
    );
    assert_eq!(
        node(&second, "Sales", &["Red", "Amount"]).value,
        Some(d("12"))
    );
    assert_eq!(get(&store, "Sales", &["Red", "Amount"]), d("999"));
}

#[test]
fn explanation_records_only_the_selected_if_branch() {
    for (units, branch, expected) in [("0", true, "7"), ("2", false, "5")] {
        let tmp = tempfile::tempdir().unwrap();
        let model = build_model(
            tmp.path(),
            "['Amount'] = N: IF(['Units'] = 0, 7, ['Price'] / ['Units']);",
        );
        let mut store = CellStore::new();
        set(&mut store, "Sales", &["Red", "Units"], units);
        set(&mut store, "Sales", &["Red", "Price"], "10");
        let evidence = explained(&model, &store, &[("Sales", &["Red", "Amount"])]).unwrap();
        let result = node(&evidence, "Sales", &["Red", "Amount"]);
        assert_eq!(result.value, Some(d(expected)));
        let steps = list(result, "steps");
        assert!(matches!(&steps[0], Item::Comparison { result, .. } if *result == (units == "0")));
        assert!(matches!(&steps[1], Item::Branch { then } if *then == branch));
        assert_eq!(
            evidence.cell("Sales['Red', 'Price']").is_some(),
            units != "0"
        );
    }
}

#[test]
fn c_rule_overrides_consolidation_in_explanation() {
    let tmp = tempfile::tempdir().unwrap();
    let model = build_model(tmp.path(), "['Price'] = C: 99;");
    let evidence = explained(&model, &CellStore::new(), &[("Sales", &["Total", "Price"])]).unwrap();
    let total = node(&evidence, "Sales", &["Total", "Price"]);
    assert_eq!(total.kind, Some("rule"));
    assert_eq!(total.value, Some(d("99")));
    assert!(list(total, "contributions").is_empty());
    assert_eq!(evidence.cells.len(), 1);
}

#[test]
fn explanation_preserves_calculation_errors() {
    for rule in [
        "['Amount'] = N: ['Units'] / ['Price'];",
        "['Amount'] = N: ['Amount'];",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let model = build_model(tmp.path(), rule);
        let error =
            explained(&model, &CellStore::new(), &[("Sales", &["Red", "Amount"])]).unwrap_err();
        assert!(matches!(error, Error::Evaluation(_)), "{rule}");
    }
}

#[test]
fn explain_cli_outputs_exact_decimal_strings_and_is_repeatable() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(
        data.join("Sales.csv"),
        "Colour,Measure,Value\nRed,Units,3\nRed,Price,0.10\n",
    )
    .unwrap();
    let args = [
        "explain",
        &s(&mini()),
        "--data",
        &s(&data),
        "--cell",
        "sales:red,amount",
    ];
    let (code, out, err) = run(&args);
    assert_eq!(code, 0, "{err}");
    assert_eq!(err, "");
    assert!(out.contains("\"Sales['Red', 'Amount']\": {"));
    assert!(out.contains("\"operator\": \"*\",\n          \"left\": \"3\",\n          \"right\": \"0.1\",\n          \"result\": \"0.3\""));
    assert!(out.contains("\"value\": \"0.3\""));
    assert_eq!(run(&args), (code, out, err));
}

#[test]
fn explain_cli_accepts_a_relative_model_directory() {
    let tmp = tempfile::tempdir().unwrap();
    write_model(
        &tmp.path().join("model"),
        &Spec::rules("['Amount'] = N: 2 * 3;"),
    );
    std::fs::create_dir(tmp.path().join("data")).unwrap();
    std::fs::write(
        tmp.path().join("data").join("Sales.csv"),
        "Colour,Measure,Value\n",
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_pacioliscube"))
        .args([
            "explain",
            "model",
            "--data",
            "data",
            "--cell",
            "Sales:Red,Amount",
        ])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let out = String::from_utf8(output.stdout)
        .unwrap()
        .replace("\r\n", "\n");
    assert!(
        out.contains("\"file\": \"cubes/Sales.rules\",\n        \"line\": 1"),
        "{out}"
    );
    assert!(out.contains("\"value\": \"6\""));
}

#[test]
fn explain_cli_fails_without_partial_json() {
    for (cell, rules, expected) in [
        ("Sales:Red", "", 1),
        ("Sales:Red,Amount", "['Ghost'] = N: 1;", 2),
        (
            "Sales:Red,Amount",
            "['Amount'] = N: ['Units'] / ['Price'];",
            3,
        ),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let model = write_model(&tmp.path().join("model"), &Spec::rules(rules));
        let data = tmp.path().join("data");
        std::fs::create_dir(&data).unwrap();
        std::fs::write(data.join("Sales.csv"), "Colour,Measure,Value\n").unwrap();
        let (code, out, err) = run(&[
            "explain",
            &s(&model),
            "--data",
            &s(&data),
            "--cell",
            "Sales:Red,Units",
            "--cell",
            cell,
        ]);
        assert_eq!(code, expected, "{cell} {rules}");
        assert_eq!(out, "");
        assert!(!err.is_empty() && !err.contains("invalid choice"));
    }
}

// ---------------------------------------------------------------- compare

#[test]
fn comparison_keeps_explanations_and_does_not_sum_overlapping_cells() {
    let tmp = tempfile::tempdir().unwrap();
    let model = s(&write_model(
        &tmp.path().join("model"),
        &Spec::rules("['Amount'] = N: ['Units'] * ['Price'];"),
    ));
    let previous = build_data(
        &tmp.path().join("before"),
        "sales.csv",
        "Red,Units,3\nRed,Price,10\nBlue,Units,2\nBlue,Price,10\n",
    );
    let current = build_data(
        &tmp.path().join("after"),
        "sales.csv",
        "Red,Units,4\nRed,Price,12\nBlue,Units,2\nBlue,Price,10\n",
    );
    let (code, out, err) = run(&[
        "compare",
        &model,
        "--previous-data",
        &previous,
        "--current-data",
        &current,
        "--cell",
        "Sales:Total,Amount",
        "--cell",
        "Sales:Red,Amount",
    ]);
    assert_eq!(code, 0, "{err}");
    let changes = out.split("\"evidence_changes\"").next().unwrap();
    // 3 * 10 + 2 * 10 = 50 and 3 * 10 = 30 before; 4 * 12 + 2 * 10 = 68 and 48 after.
    for (field, values) in [
        ("previous", ["50", "30"]),
        ("current", ["68", "48"]),
        ("difference", ["18", "18"]),
    ] {
        let found: Vec<&str> = changes
            .match_indices(&format!("\"{field}\": \""))
            .map(|(at, m)| changes[at + m.len()..].split('"').next().unwrap())
            .collect();
        assert_eq!(found, values, "{field}");
    }
    assert!(!out.contains("\"total\""));
    let current_part = out.split("\"current_explanation\"").nth(1).unwrap();
    let red = current_part
        .split("\"Sales['Red', 'Amount']\": {")
        .nth(1)
        .unwrap();
    assert!(red.contains("\"kind\": \"rule\""));
}

#[test]
fn removed_explicit_zero_is_an_evidence_change() {
    let tmp = tempfile::tempdir().unwrap();
    let model = s(&write_model(&tmp.path().join("model"), &Spec::default()));
    let previous = build_data(&tmp.path().join("before"), "sales.csv", "Red,Units,0\n");
    let current = build_data(&tmp.path().join("after"), "sales.csv", "Blue,Units,0\n");
    let (code, out, err) = run(&[
        "compare",
        &model,
        "--previous-data",
        &previous,
        "--current-data",
        &current,
        "--cell",
        "Sales:Red,Units",
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("\"difference\": \"0\""));
    let change = out.split("\"evidence_changes\": [").nth(1).unwrap();
    let previous_kind = change.split("\"previous\": {").nth(1).unwrap();
    assert!(
        previous_kind
            .split('}')
            .next()
            .unwrap()
            .contains("\"kind\": \"input\"")
    );
    let current_kind = change.split("\"current\": {").nth(1).unwrap();
    assert!(
        current_kind
            .split('}')
            .next()
            .unwrap()
            .contains("\"kind\": \"default_zero\"")
    );
}

#[test]
fn bad_current_snapshot_leaves_no_partial_comparison() {
    let tmp = tempfile::tempdir().unwrap();
    let model = s(&write_model(&tmp.path().join("model"), &Spec::default()));
    let previous = build_data(&tmp.path().join("before"), "sales.csv", "Red,Units,10\n");
    let current = build_data(&tmp.path().join("after"), "sales.csv", "Ghost,Units,3\n");
    let (code, out, err) = run(&[
        "compare",
        &model,
        "--previous-data",
        &previous,
        "--current-data",
        &current,
        "--cell",
        "Sales:Red,Units",
    ]);
    assert!(code != 0 && out.is_empty() && !err.is_empty());
}

// ---------------------------------------------------------------- rule matching

const MEASURES_PLAIN: &str = MEASURES;

#[test]
fn overlapping_areas_keep_the_first_rule() {
    for areas in [
        ["['Amount']", "['Red', 'Amount']"],
        ["['Red', 'Amount']", "['Amount']"],
        ["['Red']", "['Amount']"],
        ["[]", "['Amount']"],
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let rules: Vec<String> = areas
            .iter()
            .enumerate()
            .map(|(i, a)| format!("{a} = N: {};", i + 1))
            .collect();
        let model = model_with(tmp.path(), &rules.join("\n"), MEASURES_PLAIN);
        let cells: [(&str, &[&str]); 2] = [
            ("Sales", &["Blue", "Amount"]),
            ("Sales", &["Red", "Amount"]),
        ];
        let (values, _) = batch(&model, &CellStore::new(), &cells);
        assert_eq!(values.last().unwrap().to_sci_string(), "1");
        let evidence = explained(&model, &CellStore::new(), &cells).unwrap();
        assert_eq!(
            node(&evidence, "Sales", &["Red", "Amount"])
                .rule
                .as_ref()
                .unwrap()
                .1,
            1
        );
    }
}

#[test]
fn an_earlier_match_hides_a_later_invalid_area() {
    for first in ["['Amount']", "[]"] {
        let tmp = tempfile::tempdir().unwrap();
        let model = model_with(
            tmp.path(),
            &format!("{first} = N: 2;\n['Ghost'] = N: 3;"),
            MEASURES_PLAIN,
        );
        let cells: [(&str, &[&str]); 2] = [
            ("Sales", &["Red", "Amount"]),
            ("Sales", &["Blue", "Amount"]),
        ];
        assert_eq!(batch(&model, &CellStore::new(), &cells).0, [d("2"), d("2")]);
        assert_eq!(
            explained(&model, &CellStore::new(), &cells)
                .unwrap()
                .cells
                .len(),
            2
        );
    }
}

#[test]
fn an_earlier_miss_reaches_the_same_area_error() {
    let with_red = format!(r#"{MEASURES}, {{"Name": "Red", "Type": "Numeric"}}"#);
    for (invalid, measures, message) in [
        (
            "['Ghost']",
            MEASURES.to_string(),
            "no dimension holds an element named 'Ghost'",
        ),
        (
            "['Units', 'Price']",
            MEASURES.to_string(),
            "area names two elements of dimension 'Measure'",
        ),
        (
            "['Red']",
            with_red,
            "element 'Red' is ambiguous, held by Colour, Measure",
        ),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let model = model_with(
            tmp.path(),
            &format!("['Amount'] = N: 2;\n{invalid} = N: 3;"),
            &measures,
        );
        let (values, error) = batch(
            &model,
            &CellStore::new(),
            &[("Sales", &["Red", "Amount"]), ("Sales", &["Blue", "Price"])],
        );
        assert_eq!(values, [d("2")]);
        assert_eq!(
            error,
            Some(Error::model(format!("cube 'Sales': {message}")))
        );
    }
}

#[test]
fn leaf_and_consolidated_lookups_keep_separate_rule_order() {
    for reverse in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let model = model_with(
            tmp.path(),
            "['Amount'] = N: 1;\n['Amount'] = C: 2;\n['Price'] = 3;\n[] = 4;",
            MEASURES,
        );
        let mut cases: Vec<(&[&str], &str)> = vec![
            (&["Red", "Amount"], "1"),
            (&["Total", "Amount"], "2"),
            (&["Red", "Price"], "3"),
            (&["Total", "Price"], "3"),
            (&["Red", "Units"], "4"),
            (&["Total", "Units"], "4"),
        ];
        if reverse {
            cases.reverse();
        }
        let cells: Vec<(&str, &[&str])> = cases.iter().map(|(c, _)| ("Sales", *c)).collect();
        let expected: Vec<Decimal> = cases.iter().map(|(_, v)| d(v)).collect();
        assert_eq!(batch(&model, &CellStore::new(), &cells).0, expected);
    }
}

#[test]
fn a_skipped_qualifier_does_not_resolve_its_invalid_area() {
    let tmp = tempfile::tempdir().unwrap();
    let model = model_with(
        tmp.path(),
        "['Ghost'] = C: 1;\n['Amount'] = N: 2;",
        MEASURES,
    );
    let (values, error) = batch(
        &model,
        &CellStore::new(),
        &[
            ("Sales", &["Red", "Amount"]),
            ("Sales", &["Blue", "Amount"]),
        ],
    );
    assert_eq!(values, [d("2"), d("2")]);
    assert!(error.is_none());
    let (_, error) = batch(
        &model,
        &CellStore::new(),
        &[("Sales", &["Total", "Amount"])],
    );
    assert!(matches!(error, Some(Error::Model(m)) if m.contains("Ghost")));
}

#[test]
fn evaluation_keeps_an_expression_error_before_a_later_area_error() {
    let tmp = tempfile::tempdir().unwrap();
    let model = model_with(
        tmp.path(),
        "['Amount'] = N: 1 / 0;\n['Ghost'] = N: 2;",
        MEASURES,
    );
    assert!(
        matches!(eval_err(&model, &CellStore::new()), Error::Evaluation(m) if m.contains("division by zero"))
    );
}

#[test]
fn rule_keys_use_python_unicode_casefold() {
    for (declared, selector, requested) in [("Straße", "STRASSE", "strasse"), ("Σ", "ς", "σ")] {
        let tmp = tempfile::tempdir().unwrap();
        let measures = format!(r#"{MEASURES}, {{"Name": "{declared}", "Type": "Numeric"}}"#);
        let model = model_with(tmp.path(), &format!("['{selector}'] = N: 1.20;"), &measures);
        let (values, error) = batch(
            &model,
            &CellStore::new(),
            &[
                ("Sales", &["red", requested]),
                ("Sales", &["blue", requested]),
            ],
        );
        assert!(error.is_none(), "{error:?}");
        let shown: Vec<String> = values.iter().map(Decimal::to_sci_string).collect();
        assert_eq!(shown, ["1.20", "1.20"]);
    }
}

#[test]
fn new_engines_observe_changed_rules() {
    let tmp = tempfile::tempdir().unwrap();
    let mut model = model_with(tmp.path(), "['Amount'] = N: 1;", MEASURES);
    assert_eq!(
        batch(&model, &CellStore::new(), &[("Sales", &["Red", "Amount"])]).0,
        [d("1")]
    );
    let source = model.cubes[0].source.display();
    model.cubes[0].rules = Some(parse_rules("['Amount'] = N: 2;", &source).unwrap());
    assert_eq!(
        batch(&model, &CellStore::new(), &[("Sales", &["Red", "Amount"])]).0,
        [d("2")]
    );
}

#[test]
fn a_late_entry_in_one_group_does_not_hide_an_earlier_other_group() {
    let tmp = tempfile::tempdir().unwrap();
    let model = model_with(
        tmp.path(),
        "['Price'] = N: 9;\n['Red', 'Amount'] = N: 10;\n['Amount'] = N: 20;\n['Blue', 'Price'] = N: 30;",
        MEASURES,
    );
    let cells: [(&str, &[&str]); 3] = [
        ("Sales", &["Blue", "Units"]),
        ("Sales", &["Red", "Amount"]),
        ("Sales", &["Blue", "Amount"]),
    ];
    assert_eq!(
        batch(&model, &CellStore::new(), &cells).0,
        [d("0"), d("10"), d("20")]
    );
}

/// A small seeded generator in place of Hypothesis.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[test]
fn generated_overlaps_match_first_principles() {
    let mut rng = Lcg(0xA2EA);
    let areas: [&[&str]; 9] = [
        &[],
        &["Red"],
        &["Blue"],
        &["Total"],
        &["Amount"],
        &["Price"],
        &["Red", "Amount"],
        &["Blue", "Price"],
        &["Total", "Amount"],
    ];
    for _ in 0..100 {
        let count = 1 + rng.below(30);
        let rules: Vec<(&str, &[&str], usize)> = (0..count)
            .map(|_| {
                (
                    ["", "N", "C"][rng.below(3)],
                    areas[rng.below(9)],
                    1 + rng.below(100),
                )
            })
            .collect();
        let text: Vec<String> = rules
            .iter()
            .map(|(q, area, v)| {
                let names: Vec<String> = area.iter().map(|e| format!("'{e}'")).collect();
                if q.is_empty() {
                    format!("[{}] = {v};", names.join(","))
                } else {
                    format!("[{}] = {q}: {v};", names.join(","))
                }
            })
            .collect();
        let tmp = tempfile::tempdir().unwrap();
        let model = model_with(tmp.path(), &text.join("\n"), MEASURES);
        fn expected(rules: &[(&str, &[&str], usize)], colour: &str, measure: &str) -> Decimal {
            let leaf = colour != "Total";
            for (qualifier, area, literal) in rules {
                if (*qualifier == "N" && !leaf) || (*qualifier == "C" && leaf) {
                    continue;
                }
                if area.iter().all(|e| *e == colour || *e == measure) {
                    return Decimal::from_int(*literal as i64);
                }
            }
            if !leaf {
                return expected(rules, "Red", measure).add_(&expected(rules, "Blue", measure));
            }
            Decimal::zero()
        }
        let mut cells: Vec<(&str, Vec<&str>)> = Vec::new();
        let mut wanted = Vec::new();
        for colour in ["Total", "Blue", "Red"] {
            for measure in ["Price", "Amount", "Units"] {
                cells.push(("Sales", vec![colour, measure]));
                wanted.push(expected(&rules, colour, measure));
            }
        }
        let cells: Vec<(&str, &[&str])> = cells.iter().map(|(c, x)| (*c, x.as_slice())).collect();
        assert_eq!(
            batch(&model, &CellStore::new(), &cells).0,
            wanted,
            "{text:?}"
        );
    }
}

// ---------------------------------------------------------------- reference resolution

#[test]
fn reference_metadata_follows_changed_dimension_positions() {
    let tmp = tempfile::tempdir().unwrap();
    let mut model = model_with(tmp.path(), "['Amount'] = N: ['Units'] * 2;", MEASURES);
    let mut store = CellStore::new();
    set(&mut store, "Sales", &["Red", "Units"], "3.00");
    assert_eq!(
        batch(&model, &store, &[("Sales", &["Red", "Amount"])]).0,
        [d("3.00").mul_(&d("2"))]
    );
    model.cubes[0].dimensions.reverse();
    let mut store = CellStore::new();
    set(&mut store, "Sales", &["Units", "Red"], "5.00");
    assert_eq!(
        batch(&model, &store, &[("Sales", &["Amount", "Red"])]).0,
        [d("5.00").mul_(&d("2"))]
    );
}

#[test]
fn same_reference_in_different_cubes_keeps_db_binding() {
    let mut model = mini_model();
    let sales_source = model.cubes[0].source.display();
    let cost_source = model.cubes[1].source.display();
    model.cubes[0].rules = Some(
        parse_rules(
            "['Amount'] = N: ['Units'] + DB('Cost', 'Amount', !Colour);",
            &sales_source,
        )
        .unwrap(),
    );
    model.cubes[1].dimensions = vec!["Measure".into(), "Colour".into()];
    model.cubes[1].rules = Some(parse_rules("['Amount'] = N: ['Units'];", &cost_source).unwrap());
    let mut store = CellStore::new();
    for (colour, units, cost_units) in [("Red", "2", "10"), ("Blue", "4", "20")] {
        set(&mut store, "Sales", &[colour, "Units"], units);
        set(&mut store, "Cost", &["Units", colour], cost_units);
    }
    let cells: [(&str, &[&str]); 2] = [
        ("Sales", &["Red", "Amount"]),
        ("Sales", &["Blue", "Amount"]),
    ];
    assert_eq!(batch(&model, &store, &cells).0, [d("12"), d("24")]);
}

#[test]
fn repeated_references_give_the_same_values_through_every_entry_point() {
    let tmp = tempfile::tempdir().unwrap();
    let model = model_with(
        tmp.path(),
        "['Amount'] = N: IF(['Units'] > 0, ['Units'] * 2 + ['Price'], ['Ghost']);",
        MEASURES,
    );
    let mut store = CellStore::new();
    for (index, colour) in [(1, "Red"), (2, "Blue")] {
        store.set(
            "Sales",
            &coord(&[colour, "Units"]),
            Decimal::from_int(index),
        );
        set(&mut store, "Sales", &[colour, "Price"], "1.25");
    }
    let expected: Vec<String> = [1, 2]
        .iter()
        .map(|i| {
            Decimal::from_int(*i)
                .mul_(&d("2"))
                .add_(&d("1.25"))
                .to_sci_string()
        })
        .collect();
    for _ in 0..2 {
        let result = evaluate(&model, &store).unwrap();
        let values: Vec<String> = ["Red", "Blue"]
            .iter()
            .map(|c| get(&result, "Sales", &[c, "Amount"]).to_sci_string())
            .collect();
        assert_eq!(values, expected);
        let cells: [(&str, &[&str]); 2] = [
            ("Sales", &["Red", "Amount"]),
            ("Sales", &["Blue", "Amount"]),
        ];
        let values: Vec<String> = batch(&model, &store, &cells)
            .0
            .iter()
            .map(Decimal::to_sci_string)
            .collect();
        assert_eq!(values, expected);
        let evidence = explained(&model, &store, &cells).unwrap();
        let values: Vec<String> = ["Red", "Blue"]
            .iter()
            .map(|c| {
                node(&evidence, "Sales", &[c, "Amount"])
                    .value
                    .as_ref()
                    .unwrap()
                    .to_sci_string()
            })
            .collect();
        assert_eq!(values, expected);
    }
}

// ---------------------------------------------------------------- properties

#[test]
fn leaf_amount_is_the_exact_product_of_units_and_price() {
    let model = mini_model();
    let mut rng = Lcg(0xC0BE);
    let amount = |rng: &mut Lcg| {
        let units = rng.below(20_000_000_001) as i64 - 10_000_000_000;
        Decimal::parse(&format!("{units}E-4")).unwrap()
    };
    let mut pairs = vec![
        (d("0"), d("999999.9999")),
        (d("-12.5"), d("4")),
        (d("0.1"), d("0.2")),
    ];
    for _ in 0..80 {
        pairs.push((amount(&mut rng), amount(&mut rng)));
    }
    for (units, price) in pairs {
        let mut store = CellStore::new();
        store.set("Sales", &coord(&["Red", "Units"]), units.clone());
        store.set("Sales", &coord(&["Red", "Price"]), price.clone());
        let calculated = evaluate(&model, &store).unwrap();
        assert_eq!(
            get(&calculated, "Sales", &["Red", "Amount"]),
            units.mul_(&price)
        );
    }
}

#[test]
fn weighted_consolidation_conserves_its_leaf_amounts() {
    let model = mini_model();
    let mut rng = Lcg(0xC01150);
    let amount = |rng: &mut Lcg| {
        let units = rng.below(20_000_000_001) as i64 - 10_000_000_000;
        Decimal::parse(&format!("{units}E-4")).unwrap()
    };
    let mut triples = vec![(d("0"), d("0"), d("0")), (d("10"), d("5"), d("4"))];
    for _ in 0..80 {
        triples.push((amount(&mut rng), amount(&mut rng), amount(&mut rng)));
    }
    for (red, blue, contra) in triples {
        let mut store = CellStore::new();
        for (colour, units) in [("Red", &red), ("Blue", &blue), ("Contra", &contra)] {
            store.set("Sales", &coord(&[colour, "Units"]), units.clone());
            set(&mut store, "Sales", &[colour, "Price"], "1");
        }
        let calculated = evaluate(&model, &store).unwrap();
        assert_eq!(
            consolidate(&model, &calculated, "Sales", &["Total", "Amount"]),
            red.add_(&blue).sub_(&contra)
        );
    }
}
