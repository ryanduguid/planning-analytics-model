//! Structural validation findings, one test per check, and the engine's
//! refusal paths the scenario tests do not reach.

mod common;

use std::path::Path;

use common::*;
use pacioliscube::error::Error;
use pacioliscube::evaluate::{CellStore, consolidate_many, evaluate};
use pacioliscube::model::{Model, load_model};
use pacioliscube::pypath::PyPath;
use pacioliscube::rules::{Expr, parse_rules};
use pacioliscube::validate::{Finding, validate_model};

fn build(root: &Path, spec: &Spec) -> Model {
    write_model(root, spec);
    load_model(&PyPath::new(&s(root))).unwrap()
}

fn codes(model: &Model) -> Vec<&'static str> {
    validate_model(model)
        .unwrap()
        .iter()
        .map(|f| f.code)
        .collect()
}

fn rules(text: &str) -> Spec {
    Spec::rules(text)
}

const FEEDER_CUBE: &str = r#"{"Name": "Feeder", "Dimensions@Code.links": ["../dimensions/Colour.json", "../dimensions/Measure.json"], "Rules@Code.link": "Feeder.rules"}"#;

fn with_feeder_cube(rules_text: &str, feeder_rules: &str) -> Spec {
    Spec {
        rules: rules_text.into(),
        extra_files: vec![
            ("cubes/Feeder.json".into(), FEEDER_CUBE.into()),
            ("cubes/Feeder.rules".into(), feeder_rules.into()),
        ],
        manifest_cubes: r#""cubes/Sales.json", "cubes/Feeder.json""#.into(),
        ..Spec::default()
    }
}

fn ghost_dimension(root: &Path) {
    std::fs::create_dir_all(root.join("dimensions").join("Ghost.hierarchies")).unwrap();
    std::fs::write(
        root.join("dimensions").join("Ghost.json"),
        r#"{"Name": "Ghost", "Hierarchies@Code.links": ["Ghost.hierarchies/Ghost.json"]}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("dimensions")
            .join("Ghost.hierarchies")
            .join("Ghost.json"),
        r#"{"Name": "Ghost", "Elements": [{"Name": "One", "Type": "Numeric"}], "Edges": []}"#,
    )
    .unwrap();
}

#[test]
fn a_clean_model_reports_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let model = build(
        tmp.path(),
        &rules(
            "SKIPCHECK;\n['Amount'] = N: ['Units'] * ['Price'];\nFEEDERS;\n['Units'] => ['Amount'];\n",
        ),
    );
    assert!(validate_model(&model).unwrap().is_empty());
}

#[test]
fn cube_validation_phases_preserve_exact_finding_order() {
    let tmp = tempfile::tempdir().unwrap();
    ghost_dimension(tmp.path());
    let model = build(
        tmp.path(),
        &Spec {
            cube_dimensions: r#""../dimensions/Colour.json", "../dimensions/Ghost.json""#.into(),
            rules: "SKIPCHECK;\n['Ghost'] = N: DB('Missing', 'Red');\nFEEDERS;\n['Units'] => DB('Missing', !Colour, 'Amount');\n".into(),
            ..Spec::default()
        },
    );
    let cube = model.cube("Sales").unwrap();
    let cube_location = cube.source.display();
    let rules_location = cube.rules_source.as_ref().unwrap().display();
    let f = |code, message: &str, location: String| Finding {
        severity: "error",
        code,
        message: message.into(),
        location,
    };
    let expected = [
        f(
            "DIM001",
            "cube 'Sales' names dimension 'Ghost', which the model does not hold",
            cube_location,
        ),
        f(
            "ELE001",
            "no dimension of cube 'Sales' holds an element named 'Ghost'",
            format!("{rules_location} line 2"),
        ),
        f(
            "DIM001",
            "a rule reads cube 'Missing', which the model does not hold",
            format!("{rules_location} line 2"),
        ),
        f(
            "ELE001",
            "no dimension of cube 'Sales' holds an element named 'Units'",
            format!("{rules_location} line 4"),
        ),
        f(
            "DIM001",
            "a feeder points at cube 'Missing', which the model does not hold",
            format!("{rules_location} line 4"),
        ),
    ];
    let findings = validate_model(&model).unwrap();
    assert_eq!(findings[..expected.len()], expected[..]);
}

#[test]
fn dim001_a_cube_naming_an_absent_dimension() {
    let tmp = tempfile::tempdir().unwrap();
    ghost_dimension(tmp.path());
    let model = build(
        tmp.path(),
        &Spec {
            cube_dimensions: r#""../dimensions/Colour.json", "../dimensions/Ghost.json""#.into(),
            ..Spec::default()
        },
    );
    assert!(codes(&model).contains(&"DIM001"));
}

#[test]
fn element_and_area_checks() {
    for (text, code) in [
        (
            "SKIPCHECK;\n['Ghost'] = N: 1;\nFEEDERS;\n['Units'] => ['Ghost'];\n",
            "ELE001",
        ),
        (
            "SKIPCHECK;\n['Amount'] = N: DB('Sales', 'Red', 'Ghost');\nFEEDERS;\n['Units'] => ['Amount'];\n",
            "ELE001",
        ),
        (
            "SKIPCHECK;\n['Amount'] = N: DB('Sales', 'Red');\nFEEDERS;\n['Units'] => ['Amount'];\n",
            "ARE001",
        ),
        (
            "SKIPCHECK;\n['Amount'] = N: 1;\nFEEDERS;\n['Units'] => DB('Sales', 'Red');\n",
            "ARE001",
        ),
        (
            "SKIPCHECK;\n['Total'] = N: 1;\nFEEDERS;\n['Units'] => ['Total'];\n",
            "RUL001",
        ),
        ("SKIPCHECK;\n['Amount'] = N: 1;\n", "FED001"),
        (
            "SKIPCHECK;\nFEEDERS;\n['Units'] => DB('Ghost', !Colour, 'Amount');\n",
            "DIM001",
        ),
        (
            "SKIPCHECK;\nFEEDERS;\n['Units'] => DB('Sales', !Colour, 'Ghost');\n",
            "ELE001",
        ),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        assert!(
            codes(&build(tmp.path(), &rules(text))).contains(&code),
            "{text}"
        );
    }
    let tmp = tempfile::tempdir().unwrap();
    let model = build(
        tmp.path(),
        &rules("SKIPCHECK;\n['Total'] = C: 1;\nFEEDERS;\n['Units'] => ['Total'];\n"),
    );
    assert!(!codes(&model).contains(&"RUL001"));
}

#[test]
fn fed003_feeders_in_a_rules_file_without_skipcheck() {
    for text in [
        "['Amount'] = N: ['Units'] * ['Price'];\nFEEDERS;\n['Units'] => ['Amount'];\n",
        "FEEDERS;\n['Units'] => ['Amount'];\n",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let findings = validate_model(&build(tmp.path(), &rules(text))).unwrap();
        let finding = findings.iter().find(|f| f.code == "FED003").unwrap();
        assert_eq!(finding.severity, "error");
    }
}

#[test]
fn fed002_a_calculated_area_no_feeder_points_at() {
    let tmp = tempfile::tempdir().unwrap();
    let findings = validate_model(&build(tmp.path(), &rules("SKIPCHECK;\n['Amount'] = N: 1;\n['Price'] = N: 2;\nFEEDERS;\n['Units'] => ['Amount'];\n"))).unwrap();
    let finding = findings.iter().find(|f| f.code == "FED002").unwrap();
    assert_eq!(finding.severity, "warning");
}

#[test]
fn cross_cube_feeders_cover_their_target_areas() {
    let tmp = tempfile::tempdir().unwrap();
    let model = build(
        tmp.path(),
        &with_feeder_cube(
            "SKIPCHECK;\n['Amount'] = N: 1;\nFEEDERS;\n['Units'] => ['Price'];\n",
            "SKIPCHECK;\nFEEDERS;\n['Units'] => DB('Sales', !Colour, 'Amount');\n",
        ),
    );
    assert!(!codes(&model).contains(&"FED002"));
    for (source, fed) in [
        ("['Blue','Units']", false),
        ("['Red','Units']", true),
        ("['Total','Units']", true),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let model = build(
            tmp.path(),
            &with_feeder_cube(
                "SKIPCHECK;\n['Red','Amount'] = N: 1;\nFEEDERS;\n['Units'] => ['Price'];\n",
                &format!("SKIPCHECK;\nFEEDERS;\n{source} => DB('Sales', !Colour, 'Amount');\n"),
            ),
        );
        assert_eq!(codes(&model).contains(&"FED002"), !fed, "{source}");
    }
}

#[test]
fn feeder_areas_are_compared_whole() {
    let tmp = tempfile::tempdir().unwrap();
    let model = build(
        tmp.path(),
        &rules("SKIPCHECK;\n['Red','Amount'] = N: 1;\nFEEDERS;\n['Units'] => ['Blue','Amount'];\n"),
    );
    assert!(codes(&model).contains(&"FED002"));
    let tmp = tempfile::tempdir().unwrap();
    let model = build(
        tmp.path(),
        &rules("SKIPCHECK;\n['Red','Amount'] = N: 1;\nFEEDERS;\n['Units'] => ['Red','Amount'];\n"),
    );
    assert!(!codes(&model).contains(&"FED002"));
}

#[test]
fn db_feeder_coordinates_pair_with_their_own_dimension() {
    let tmp = tempfile::tempdir().unwrap();
    let model = build(
        tmp.path(),
        &rules("SKIPCHECK;\nFEEDERS;\n['Units'] => DB('Sales', 'Units', 'Red');\n"),
    );
    let messages: Vec<String> = validate_model(&model)
        .unwrap()
        .into_iter()
        .filter(|f| f.code == "ELE001")
        .map(|f| f.message)
        .collect();
    assert_eq!(
        messages,
        [
            "dimension 'Colour' of cube 'Sales' holds no element named 'Units'",
            "dimension 'Measure' of cube 'Sales' holds no element named 'Red'",
        ]
    );
    let tmp = tempfile::tempdir().unwrap();
    let model = build(
        tmp.path(),
        &rules("SKIPCHECK;\nFEEDERS;\n['Units'] => DB('Sales', 'Red', 'Amount');\n"),
    );
    assert!(!codes(&model).contains(&"ELE001"));
    let tmp = tempfile::tempdir().unwrap();
    let model = build(
        tmp.path(),
        &Spec {
            measures: format!(r#"{MEASURES}, {{"Name": "Red", "Type": "Numeric"}}"#),
            rules:
                "SKIPCHECK;\n['Price'] = N: 1;\nFEEDERS;\n['Units'] => DB('Sales', 'Red', 'Red');\n"
                    .into(),
            ..Spec::default()
        },
    );
    assert!(codes(&model).contains(&"FED002"));
}

#[test]
fn man001_a_file_the_manifest_does_not_list() {
    let tmp = tempfile::tempdir().unwrap();
    let model = build(
        tmp.path(),
        &Spec {
            rules: "SKIPCHECK;\n['Amount'] = N: 1;\nFEEDERS;\n['Units'] => ['Amount'];\n".into(),
            extra_files: vec![(
                "cubes/Forgotten.json".into(),
                r#"{"Name": "Forgotten"}"#.into(),
            )],
            ..Spec::default()
        },
    );
    let findings: Vec<Finding> = validate_model(&model)
        .unwrap()
        .into_iter()
        .filter(|f| f.code == "MAN001")
        .collect();
    assert!(findings[0].location.contains("Forgotten.json"));
}

#[test]
fn process_parameters_are_checked_against_live_code() {
    let check = |script: &str, parameters: &str| {
        let tmp = tempfile::tempdir().unwrap();
        codes(&build(
            tmp.path(),
            &Spec {
                processes: script.into(),
                parameters: parameters.into(),
                ..Spec::default()
            },
        ))
        .contains(&"PRC001")
    };
    assert!(check("sVersion = pVersion;\n", "[]"));
    assert!(!check(
        "sVersion = pVersion;\n",
        r#"[{"Name": "pVersion", "Type": "String", "Value": "Budget"}]"#
    ));
    assert!(!check(
        "#Region Prolog\n# One column per cube dimension, put in place.\n#EndRegion\n",
        "[]"
    ));
    assert!(check(
        "#Region Prolog\nsTag = 'run #1'; CellPutN(1, 'Sales', pGhost);\n#EndRegion\n",
        "[]"
    ));
    assert!(check(
        "#Region Prolog\nnValue = CellGetN('Sales', pYear, 'Amount');\n#EndRegion\n",
        "[]"
    ));
}

#[test]
fn the_shipped_model_validates_clean() {
    let model = load_model(&PyPath::new(&s(&model_root()))).unwrap();
    assert!(
        validate_model(&model)
            .unwrap()
            .iter()
            .all(|f| f.severity != "error")
    );
}

#[test]
fn ambiguity_and_missing_dimensions_in_db_arguments_are_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let model = build(
        tmp.path(),
        &Spec {
            measures: format!(r#"{MEASURES}, {{"Name": "Red", "Type": "Numeric"}}"#),
            rules: "['Amount'] = ['Red'];\n".into(),
            ..Spec::default()
        },
    );
    let findings = validate_model(&model).unwrap();
    assert!(
        findings
            .iter()
            .any(|f| f.code == "ARE001" && f.message.contains("'Red' is ambiguous"))
    );
    for text in [
        "['Amount'] = DB('Sales', !Region, 'Units');\n",
        "SKIPCHECK;\nFEEDERS;\n['Units'] => DB('Sales', !Region, 'Amount');\n",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let findings = validate_model(&build(tmp.path(), &rules(text))).unwrap();
        assert!(findings.iter().any(|f| f.code == "ELE001"
            && f.message == "!Region names a dimension that cube 'Sales' does not have"));
    }
}

// ---------------------------------------------------------------- engine refusals

fn engine_model(root: &Path, rules_text: &str) -> Model {
    build(root, &rules(rules_text))
}

#[test]
fn an_unknown_cube_is_a_model_error() {
    let tmp = tempfile::tempdir().unwrap();
    let model = engine_model(tmp.path(), "");
    let cells = [("Budget".to_string(), coord(&["Red", "Units"]))];
    let error = consolidate_many(&model, &CellStore::new(), &cells, |_, _| {}).unwrap_err();
    assert_eq!(
        error,
        Error::model("no cube named 'Budget' in model 'built'")
    );
}

#[test]
fn consolidation_of_an_all_leaf_coordinate_reads_the_store() {
    let tmp = tempfile::tempdir().unwrap();
    let model = engine_model(tmp.path(), "");
    let mut store = CellStore::new();
    store.set("Sales", &coord(&["Red", "Units"]), d("4"));
    let mut values = Vec::new();
    consolidate_many(
        &model,
        &store,
        &[("Sales".into(), coord(&["Red", "Units"]))],
        |_, v| values.push(v),
    )
    .unwrap();
    assert_eq!(values, [d("4")]);
}

#[test]
fn arithmetic_overflow_is_a_calculation_error() {
    let tmp = tempfile::tempdir().unwrap();
    let model = engine_model(tmp.path(), "['Amount'] = ['Units'] * ['Price'];\n");
    let mut store = CellStore::new();
    store.set("Sales", &coord(&["Red", "Units"]), d("9E+999999"));
    store.set("Sales", &coord(&["Red", "Price"]), d("9E+999999"));
    let error = consolidate_many(
        &model,
        &store,
        &[("Sales".into(), coord(&["Red", "Amount"]))],
        |_, _| {},
    )
    .unwrap_err();
    assert_eq!(
        error,
        Error::evaluation("cube 'Sales' at ['Red', 'Amount']: arithmetic failed (Overflow)")
    );

    let tmp = tempfile::tempdir().unwrap();
    let model = engine_model(tmp.path(), "");
    let mut store = CellStore::new();
    store.set("Sales", &coord(&["Red", "Units"]), d("9E+999999"));
    store.set("Sales", &coord(&["Blue", "Units"]), d("9E+999999"));
    let error = consolidate_many(
        &model,
        &store,
        &[("Sales".into(), coord(&["Total", "Units"]))],
        |_, _| {},
    )
    .unwrap_err();
    assert!(matches!(error, Error::Evaluation(m) if m.contains("arithmetic failed")));
}

#[test]
fn a_rule_area_too_large_to_materialise_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let names = |prefix: &str| -> String {
        (0..1500)
            .map(|i| format!(r#"{{"Name": "{prefix}{i}", "Type": "Numeric"}}"#))
            .collect::<Vec<_>>()
            .join(", ")
    };
    write_model(
        tmp.path(),
        &Spec {
            measures: format!("{}, {MEASURES}", names("M")),
            rules: "['Amount'] = ['Units'] * ['Price'];\n[] = N: 1;\n".into(),
            ..Spec::default()
        },
    );
    // Colour holds three leaves here; widen it so the unnamed product is
    // 1,500 by 1,503 cells, past the two million the engine will expand.
    let colour = tmp
        .path()
        .join("dimensions")
        .join("Colour.hierarchies")
        .join("Colour.json");
    std::fs::write(
        &colour,
        format!(
            r#"{{"Name": "Colour", "Elements": [{}], "Edges": []}}"#,
            names("Q")
        ),
    )
    .unwrap();
    let model = load_model(&PyPath::new(&s(tmp.path()))).unwrap();
    let error = evaluate(&model, &CellStore::new()).unwrap_err();
    assert_eq!(
        error,
        Error::evaluation(
            "cube 'Sales' rule at line 2 expands to more than 2000000 cells. Name more elements in its area."
        )
    );
}

#[test]
fn rule_syntax_errors() {
    for (text, message) in [
        ("['Amount'] = ;\n", "unexpected ';' in an expression"),
        (
            "['Amount'] = IF(['Units'], 1, 0);\n",
            "expected a comparison operator inside IF",
        ),
        (
            "1;\n",
            "a statement must start with an area in square brackets",
        ),
        (
            "FEEDERS;\n['Amount'] = 1;\n",
            "a statement after FEEDERS must be a feeder using =>",
        ),
        (
            "['Amount'] = N + 1;\n",
            "N is outside the supported rule grammar",
        ),
    ] {
        let error = parse_rules(text, "Sales.rules").unwrap_err();
        assert!(
            matches!(&error, Error::RuleSyntax(m) if m.contains(message)),
            "{error:?}"
        );
    }
    let rule = &parse_rules("['Amount'] = -['Units'];\n", "Sales.rules")
        .unwrap()
        .rules[0];
    let Expr::BinaryOp('-', zero, reference) = &rule.expression else {
        panic!()
    };
    assert!(matches!(zero.as_ref(), Expr::Number(n) if n.is_zero()));
    assert!(
        matches!(reference.as_ref(), Expr::CellRef(r) if r.cube.is_none() && r.coordinates == ["Units"])
    );
    let parsed = parse_rules("UNDEFVALS;\n['Amount'] = 1;\n", "Sales.rules").unwrap();
    assert!(!parsed.skipcheck);
    assert_eq!(parsed.rules.len(), 1);
}

#[test]
fn a_cell_reference_with_an_empty_element_is_a_usage_error() {
    let tmp = tempfile::tempdir().unwrap();
    write_model(&tmp.path().join("model"), &Spec::default());
    let data = build_data(&tmp.path().join("data"), "sales.csv", "");
    let (code, _, err) = run(&[
        "calculate",
        &s(&tmp.path().join("model")),
        "--data",
        &data,
        "--cell",
        "Sales:Red,",
    ]);
    assert_eq!(code, 1);
    assert!(err.contains("has an empty element name"));
}
