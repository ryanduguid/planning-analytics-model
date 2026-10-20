//! The command line: what each subcommand prints and which exit code it returns.

mod common;

use std::path::Path;

use common::*;
use pacioliscube::decimal::Decimal;
use pacioliscube::report::money;

const CELL: &str = "PnL:FY2026-27,Budget,Jul,CivilCo,Earthworks,Contract Revenue,Amount";
const GROUP_EBITDA: &str = "PnL:FY2026-27,Budget,FY,Group,All Cost Centres,EBITDA,Amount";

const STATEMENT_ROWS: [&str; 8] = [
    "Revenue",
    "Direct Costs",
    "Gross Margin",
    "Employment Costs",
    "Overheads",
    "EBITDA",
    "Depreciation",
    "EBIT",
];

const UNFED_RULES: &str = "SKIPCHECK;\n['Amount'] = N: ['Units'] * ['Price'];\n['Price'] = N: 2;\nFEEDERS;\n['Units'] => ['Amount'];\n";
const UNKNOWN_ELEMENT_RULES: &str =
    "SKIPCHECK;\n['Ghost'] = N: 1;\nFEEDERS;\n['Units'] => ['Amount'];\n";
const DIVIDE_BY_ZERO_RULES: &str =
    "SKIPCHECK;\n['Amount'] = N: ['Units'] / ['Price'];\nFEEDERS;\n['Units'] => ['Amount'];\n";
// Red and Blue are both elements of Colour, so this area names one dimension
// twice. Validation does not look for that, and the C qualifier keeps the area
// out of the evaluation pass, so the fault waits until a consolidated cell is
// asked for.
const AMBIGUOUS_AREA_RULES: &str =
    "SKIPCHECK;\n['Red','Blue'] = C: 1;\nFEEDERS;\n['Units'] => ['Amount'];\n";

fn model() -> String {
    s(&model_root())
}

fn ex() -> String {
    s(&examples())
}

fn budget_report() -> Vec<String> {
    [
        "report",
        &model(),
        "--data",
        &ex(),
        "--year",
        "FY2026-27",
        "--version",
        "Budget",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn run_owned(args: &[String]) -> (i32, String, String) {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run(&refs)
}

fn build_model(root: &Path, rules: &str, extra: &[(&str, &str)]) -> String {
    let spec = Spec {
        rules: rules.into(),
        extra_files: extra
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect(),
        ..Spec::default()
    };
    s(&write_model(root, &spec))
}

/// A data directory holding one header only CSV for a shaped report model.
fn write_report_data(folder: &Path, header: &str) -> String {
    std::fs::create_dir_all(folder).unwrap();
    std::fs::write(folder.join("PnL.csv"), format!("{header}\n")).unwrap();
    s(folder)
}

/// A model holding one cube named PnL over the dimensions given, each holding
/// the leaf elements named for it.
fn build_report_model(root: &Path, elements: &[(&str, &[&str])]) -> String {
    std::fs::create_dir_all(root.join("cubes")).unwrap();
    for (dimension, names) in elements {
        let folder = root
            .join("dimensions")
            .join(format!("{dimension}.hierarchies"));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            root.join("dimensions").join(format!("{dimension}.json")),
            format!(
                r#"{{"Name": "{dimension}", "Hierarchies@Code.links": ["{dimension}.hierarchies/{dimension}.json"]}}"#
            ),
        )
        .unwrap();
        let listed: Vec<String> = names
            .iter()
            .map(|n| format!(r#"{{"Name": "{n}", "Type": "Numeric"}}"#))
            .collect();
        std::fs::write(
            folder.join(format!("{dimension}.json")),
            format!(
                r#"{{"Name": "{dimension}", "Elements": [{}], "Edges": []}}"#,
                listed.join(", ")
            ),
        )
        .unwrap();
    }
    let links: Vec<String> = elements
        .iter()
        .map(|(n, _)| format!(r#""../dimensions/{n}.json""#))
        .collect();
    std::fs::write(
        root.join("cubes").join("PnL.json"),
        format!(
            r#"{{"Name": "PnL", "Dimensions@Code.links": [{}]}}"#,
            links.join(", ")
        ),
    )
    .unwrap();
    let dims: Vec<String> = elements
        .iter()
        .map(|(n, _)| format!(r#""dimensions/{n}.json""#))
        .collect();
    std::fs::write(
        root.join("tm1project.json"),
        format!(
            r#"{{"Version": 1.0, "Name": "shaped", "Objects": {{"Dimensions": [{}], "Cubes": ["cubes/PnL.json"]}}}}"#,
            dims.join(", ")
        ),
    )
    .unwrap();
    s(root)
}

fn dollars(printed: &str) -> i64 {
    if let Some(inner) = printed.strip_prefix('(').and_then(|p| p.strip_suffix(')')) {
        return -inner.replace(',', "").parse::<i64>().unwrap();
    }
    printed.replace(',', "").parse().unwrap()
}

fn report_rows(args: &[String]) -> Vec<(String, String)> {
    let (code, out, _err) = run_owned(args);
    assert_eq!(code, 0, "{out}");
    let body = out.split_once("\n\n").unwrap().1;
    body.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let (label, amount) = line.rsplit_once(' ').unwrap();
            (label.trim().to_string(), amount.to_string())
        })
        .collect()
}

#[test]
fn the_shipped_model_validates_with_a_zero_exit() {
    let (code, out, _) = run(&["validate", &model()]);
    assert_eq!(code, 0);
    assert!(out.contains("0 errors, 0 warnings"));
}

#[test]
fn the_model_directory_defaults_to_the_model_folder() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_pacioliscube"))
        .arg("validate")
        .current_dir(repo())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("0 errors, 0 warnings"));
}

#[test]
fn an_empty_model_directory_argument_means_the_default_folder() {
    // `args.model_dir or DEFAULT_MODEL_ROOT` treats "" as absent.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_pacioliscube"))
        .args(["validate", ""])
        .current_dir(repo())
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("0 errors, 0 warnings"));
}

/// A destination that refuses every write, as a full disk or closed pipe does.
struct Refusing;

impl std::io::Write for Refusing {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("the destination refused the write"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_failed_write_ends_the_command_as_an_os_error() {
    let model = s(&model_root());
    let data = s(&examples());
    let cell = CELL.to_string();
    let (code, err) = big_stack(move || {
        let mut err = Vec::new();
        let argv: Vec<String> = [
            "calculate",
            &model,
            "--data",
            &data,
            "--cell",
            &cell,
            "--cell",
            &cell,
        ]
        .iter()
        .map(|a| a.to_string())
        .collect();
        let code = pacioliscube::cli::main(
            &argv,
            &pacioliscube::terminal::Terminal::plain(),
            &mut pacioliscube::cli::Streams {
                out: &mut Refusing,
                err: &mut err,
            },
        );
        (code, String::from_utf8(err).unwrap())
    });
    // Python's OSError backstop: one message and exit 1, and no second cell.
    assert_eq!(code, 1);
    assert_eq!(err, "pacioliscube: the destination refused the write\n");
}

/// A model or data file whose name is not Unicode text is refused, never read
/// under a replacement-character name that could be a different file.
#[cfg(target_os = "linux")]
#[test]
fn a_file_name_that_is_not_unicode_text_is_refused_not_replaced() {
    use std::os::unix::ffi::OsStrExt;
    let odd = |name: &[u8]| std::ffi::OsStr::from_bytes(name).to_os_string();
    let tmp = tempdir();
    let root = build_model(&tmp.path().join("model"), "", &[]);
    let data = build_data(&tmp.path().join("data"), "sales.csv", "Red,Units,10\n");
    let cell = ["--cell", "Sales:Total,Units"];
    let (code, out, err) = run(&[&["calculate", &root, "--data", &data][..], &cell].concat());
    assert_eq!(
        (code, out.as_str()),
        (0, "Sales:Total,Units = 10\n"),
        "{err}"
    );

    std::fs::write(
        Path::new(&data).join(odd(b"\xff.csv")),
        "Colour,Measure,Value\n",
    )
    .unwrap();
    let (code, out, err) = run(&[&["calculate", &root, "--data", &data][..], &cell].concat());
    assert_eq!(code, 1);
    assert!(out.is_empty());
    assert!(
        err.contains("UnicodeError: a file name is not UTF-8 text"),
        "{err}"
    );

    std::fs::write(Path::new(&root).join(odd(b"\xff.json")), "{}").unwrap();
    let (code, _, err) = run(&["validate", &root]);
    assert_eq!(code, 1);
    assert!(
        err.contains("UnicodeError: a file name is not UTF-8 text"),
        "{err}"
    );
}

#[test]
fn a_failed_final_flush_exits_120_as_python_does() {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_pacioliscube"))
        .arg("--help")
        .env_remove("PYTHONUNBUFFERED")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // Close the reading end before the help text is flushed.
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(120));
    let err = String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n");
    assert!(
        err.starts_with("Exception ignored while flushing sys.stdout:\n"),
        "{err}"
    );
}

#[test]
fn pythonunbuffered_ends_the_command_at_the_failed_print() {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_pacioliscube"))
        .arg("demo")
        .env("PYTHONUNBUFFERED", "1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let err = String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n");
    assert!(err.starts_with("pacioliscube: [Errno "), "{err}");
    assert_eq!(err.lines().count(), 1, "{err}");
}

#[test]
fn pythonintmaxstrdigits_sets_the_integer_digit_limit_as_python_does() {
    let tmp = tempdir();
    let root = tmp.path().join("model");
    copy_dir(&model_root(), &root);
    let manifest = root.join("tm1project.json");
    let text = std::fs::read_to_string(&manifest).unwrap();
    let at = text.find('{').unwrap() + 1;
    let field = format!("\"Unused\": {}, ", "7".repeat(4301));
    std::fs::write(&manifest, format!("{}{field}{}", &text[..at], &text[at..])).unwrap();
    let validate = |limit: Option<&str>| {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_pacioliscube"));
        command.arg("validate").arg(&root);
        match limit {
            Some(value) => command.env("PYTHONINTMAXSTRDIGITS", value),
            None => command.env_remove("PYTHONINTMAXSTRDIGITS"),
        };
        let output = command.output().unwrap();
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n"),
        )
    };
    let (code, err) = validate(None);
    assert_eq!(code, Some(2));
    assert!(err.contains("Exceeds the limit (4300 digits)"), "{err}");
    assert_eq!(validate(Some("0")).0, Some(0));
    let (code, err) = validate(Some("640"));
    assert_eq!(code, Some(2));
    assert!(err.contains("Exceeds the limit (640 digits)"), "{err}");
    let (code, err) = validate(Some("5"));
    assert_eq!(code, Some(1));
    assert!(
        err.starts_with("Fatal Python error: config_init_int_max_str_digits: "),
        "{err}"
    );
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[test]
fn a_failed_print_wins_over_a_later_cells_model_fault() {
    // The second cell fails only while consolidating, after the first prints.
    let tmp = tempdir();
    let root = build_model(&tmp.path().join("model"), AMBIGUOUS_AREA_RULES, &[]);
    let data = build_data(&tmp.path().join("data"), "sales.csv", "Red,Units,10\n");
    let (code, err) = big_stack(move || {
        let argv: Vec<String> = [
            "calculate",
            &root,
            "--data",
            &data,
            "--cell",
            "Sales:Red,Units",
            "--cell",
            "Sales:Total,Amount",
        ]
        .iter()
        .map(|a| a.to_string())
        .collect();
        let mut err = Vec::new();
        let code = pacioliscube::cli::main(
            &argv,
            &pacioliscube::terminal::Terminal::plain(),
            &mut pacioliscube::cli::Streams {
                out: &mut Refusing,
                err: &mut err,
            },
        );
        (code, String::from_utf8(err).unwrap())
    });
    assert_eq!(code, 1);
    assert_eq!(err, "pacioliscube: the destination refused the write\n");
}

/// A refused stream on the binary's own standard handles: `Out` closes the
/// reading end of stdout, `Err` of stderr, before the process writes.
enum Closed {
    Out,
    Err,
}

fn with_closed(closed: Closed, args: &[&str], unbuffered: bool) -> (Option<i32>, String) {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_pacioliscube"));
    command
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if unbuffered {
        command.env("PYTHONUNBUFFERED", "1");
    } else {
        command.env_remove("PYTHONUNBUFFERED");
    }
    let mut child = command.spawn().unwrap();
    match closed {
        Closed::Out => drop(child.stdout.take()),
        Closed::Err => drop(child.stderr.take()),
    }
    let output = child.wait_with_output().unwrap();
    let kept = match closed {
        Closed::Out => output.stderr,
        Closed::Err => output.stdout,
    };
    (
        output.status.code(),
        String::from_utf8_lossy(&kept).replace("\r\n", "\n"),
    )
}

#[test]
fn output_left_pending_by_a_failed_write_fails_again_at_shutdown() {
    // The first value stays buffered; the second, larger than the buffer,
    // fails while flushing it, and the pending bytes fail again at shutdown.
    let tmp = tempdir();
    let root = tmp.path().join("model");
    write(
        &root.join("tm1project.json"),
        r#"{"Name":"two","Objects":{"Dimensions":["dimensions/D.json"],"Cubes":["cubes/Sales.json"]}}"#,
    );
    write(
        &root.join("dimensions/D.json"),
        r#"{"Name":"D","Hierarchies@Code.links":["h.json"]}"#,
    );
    write(
        &root.join("dimensions/h.json"),
        r#"{"Name":"D","Elements":[{"Name":"First","Type":"Numeric"},{"Name":"Second","Type":"Numeric"}],"Edges":[]}"#,
    );
    write(
        &root.join("cubes/Sales.json"),
        r#"{"Name":"Sales","Dimensions":[{"@id":"Dimensions('D')"}]}"#,
    );
    let data = tmp.path().join("data");
    write(
        &data.join("Sales.csv"),
        "D,Value\nFirst,1E10000\nSecond,1E200000\n",
    );
    let (root, data) = (s(&root), s(&data));
    let args = [
        "calculate",
        &root,
        "--data",
        &data,
        "--cell",
        "Sales:First",
        "--cell",
        "Sales:Second",
    ];
    let (code, err) = with_closed(Closed::Out, &args, false);
    assert_eq!(code, Some(120), "{err}");
    let lines: Vec<&str> = err.lines().collect();
    assert_eq!(lines.len(), 3, "{err}");
    assert!(lines[0].starts_with("pacioliscube: [Errno "), "{err}");
    assert_eq!(lines[1], "Exception ignored while flushing sys.stdout:");
    // Unbuffered, the first write fails at once and leaves nothing pending.
    let (code, err) = with_closed(Closed::Out, &args, true);
    assert_eq!(code, Some(1), "{err}");
    assert_eq!(err.lines().count(), 1, "{err}");
}

#[test]
fn a_diagnostic_that_cannot_be_written_loses_the_original_exit_code() {
    let tmp = tempdir();
    let bad = tmp.path().join("model");
    write(&bad.join("tm1project.json"), r#"{"Name": "#);
    let bad = s(&bad);
    assert_eq!(run(&["validate", &bad]).0, 2);
    // Buffered stderr fails at shutdown (120); unbuffered, the print raises
    // out of Python's handler (1).
    assert_eq!(
        with_closed(Closed::Err, &["validate", &bad], false).0,
        Some(120)
    );
    assert_eq!(
        with_closed(Closed::Err, &["validate", &bad], true).0,
        Some(1)
    );
    // An argument error: argparse swallows the failed write, then buffered
    // stderr fails at shutdown.
    assert_eq!(with_closed(Closed::Err, &["--nope"], false).0, Some(120));
    assert_eq!(with_closed(Closed::Err, &["--nope"], true).0, Some(1));
}

#[test]
fn a_finding_prints_as_severity_code_location_then_message() {
    let tmp = tempdir();
    let root = build_model(&tmp.path().join("model"), UNKNOWN_ELEMENT_RULES, &[]);
    let (_, out, _) = run(&["validate", &root]);
    let line = out.lines().next().unwrap();
    assert!(line.starts_with("error ELE001 "));
    assert!(line.ends_with("no dimension of cube 'Sales' holds an element named 'Ghost'"));
    assert!(line.contains("Sales.rules line 2:"));
}

#[test]
fn errors_print_before_warnings_whatever_order_they_are_found_in() {
    let tmp = tempdir();
    let root = build_model(
        &tmp.path().join("model"),
        UNFED_RULES,
        &[("notes.txt", "left behind\n")],
    );
    let (_, out, _) = run(&["validate", &root]);
    let lines: Vec<&str> = out.lines().collect();
    let severities: Vec<&str> = lines[..lines.len() - 1]
        .iter()
        .map(|l| l.split(' ').next().unwrap())
        .collect();
    assert_eq!(severities, ["error", "warning"]);
}

#[test]
fn validate_returns_two_when_the_model_has_an_error() {
    let tmp = tempdir();
    let root = build_model(&tmp.path().join("model"), UNKNOWN_ELEMENT_RULES, &[]);
    let (code, out, _) = run(&["validate", &root]);
    assert_eq!(code, 2);
    assert!(out.contains("1 error, 1 warning"));
}

#[test]
fn validate_returns_zero_when_the_model_has_only_warnings() {
    let tmp = tempdir();
    let root = build_model(&tmp.path().join("model"), UNFED_RULES, &[]);
    let (code, out, _) = run(&["validate", &root]);
    assert_eq!(code, 0);
    assert!(out.contains("0 errors, 1 warning"));
}

#[test]
fn a_model_directory_that_is_not_there_is_a_usage_error() {
    let tmp = tempdir();
    let (code, _, err) = run(&["validate", &s(&tmp.path().join("absent"))]);
    assert_eq!(code, 1);
    assert!(err.contains("no model directory"));
}

#[test]
fn a_directory_holding_no_manifest_is_an_invalid_model() {
    let tmp = tempdir();
    let (code, _, err) = run(&["validate", &s(tmp.path())]);
    assert_eq!(code, 2);
    assert!(err.contains("tm1project.json"));
}

#[test]
fn malformed_hierarchy_shapes_are_invalid_models() {
    for (field, invalid) in [
        ("Elements", "null"),
        ("Elements", "12"),
        ("Edges", "null"),
        ("Edges", "12"),
        ("Type", "[]"),
    ] {
        let tmp = tempdir();
        let root = build_model(&tmp.path().join("model"), "", &[]);
        let source = Path::new(&root)
            .join("dimensions")
            .join("Colour.hierarchies")
            .join("Colour.json");
        let text = if field == "Type" {
            format!(
                r#"{{"Name": "Colour", "Elements": [{{"Name": "Total", "Type": {invalid}}}], "Edges": []}}"#
            )
        } else if field == "Elements" {
            format!(r#"{{"Name": "Colour", "Elements": {invalid}, "Edges": [{COLOUR_EDGES}]}}"#)
        } else {
            format!(r#"{{"Name": "Colour", "Elements": [{COLOUR_ELEMENTS}], "Edges": {invalid}}}"#)
        };
        std::fs::write(&source, text).unwrap();
        let (code, _, err) = run(&["validate", &root]);
        assert_eq!(code, 2, "{field} {invalid}");
        assert!(err.contains(&s(&source)), "{err}");
        assert!(!err.contains("Traceback"));
    }
}

#[test]
fn a_nan_weight_is_an_invalid_model_before_calculation_or_report() {
    for command in ["calculate", "report"] {
        let tmp = tempdir();
        let root = build_model(&tmp.path().join("model"), "", &[]);
        let source = Path::new(&root)
            .join("dimensions")
            .join("Colour.hierarchies")
            .join("Colour.json");
        std::fs::write(
            &source,
            format!(
                r#"{{"Name": "Colour", "Elements": [{COLOUR_ELEMENTS}], "Edges": [{{"ParentName": "Total", "ComponentName": "Red", "Weight": "NaN"}}]}}"#
            ),
        )
        .unwrap();
        let data = build_data(&tmp.path().join("data"), "sales.csv", "Red,Units,10\n");
        let (code, out, err) = if command == "calculate" {
            run(&[
                "calculate",
                &root,
                "--data",
                &data,
                "--cell",
                "Sales:Total,Units",
            ])
        } else {
            run(&[
                "report",
                &root,
                "--data",
                &data,
                "--year",
                "FY2026-27",
                "--version",
                "Budget",
            ])
        };
        assert_eq!(code, 2);
        assert_eq!(out, "");
        assert!(err.contains("is not a finite number"));
    }
}

#[test]
fn rules_outside_the_grammar_are_an_invalid_model_not_a_traceback() {
    let tmp = tempdir();
    let model = build_model(tmp.path(), "['Red', 'Units'] = N: @;\n", &[]);
    let (code, _, err) = run(&["validate", &model]);
    assert_eq!(code, 2);
    assert!(err.contains("unexpected character"));
}

#[test]
fn calculate_prints_the_value_at_a_leaf_cell() {
    let (code, out, _) = run(&["calculate", &model(), "--data", &ex(), "--cell", CELL]);
    assert_eq!(code, 0);
    assert_eq!(out, format!("{CELL} = 1534000\n"));
}

#[test]
fn a_consolidated_coordinate_is_summed_through_the_hierarchy() {
    let (code, out, _) = run(&[
        "calculate",
        &model(),
        "--data",
        &ex(),
        "--cell",
        GROUP_EBITDA,
    ]);
    assert_eq!(code, 0);
    assert_eq!(out, format!("{GROUP_EBITDA} = 20066782.4118\n"));
}

#[test]
fn cell_may_be_given_more_than_once() {
    let (code, out, _) = run(&[
        "calculate",
        &model(),
        "--data",
        &ex(),
        "--cell",
        CELL,
        "--cell",
        GROUP_EBITDA,
    ]);
    assert_eq!(code, 0);
    let names: Vec<&str> = out
        .lines()
        .map(|l| l.split(" = ").next().unwrap())
        .collect();
    assert_eq!(names, [CELL, GROUP_EBITDA]);
}

#[test]
fn a_cell_reference_matches_elements_without_regard_to_case() {
    let (code, out, _) = run(&[
        "calculate",
        &model(),
        "--data",
        &ex(),
        "--cell",
        "pnl:fy2026-27,budget,jul,civilco,earthworks,contract revenue,amount",
    ]);
    assert_eq!(code, 0);
    assert_eq!(out, format!("{CELL} = 1534000\n"));
}

#[test]
fn malformed_cell_references_are_usage_errors() {
    let cases = [
        ("FY2026-27,Budget", "CUBE:element"),
        ("Ghost:Jul", "no cube named 'Ghost'"),
        ("PnL:FY2026-27,Budget", "gives 2 elements"),
        (
            "PnL:FY2026-27,Budget,Jul,CivilCo,Earthworks,Ghost,Amount",
            "'Ghost'",
        ),
    ];
    for (cell, expected) in cases {
        let (code, _, err) = run(&["calculate", &model(), "--data", &ex(), "--cell", cell]);
        assert_eq!(code, 1, "{cell}");
        assert!(err.contains(expected), "{err}");
    }
    let (_, _, err) = run(&[
        "calculate",
        &model(),
        "--data",
        &ex(),
        "--cell",
        "PnL:FY2026-27,Budget",
    ]);
    assert!(err.contains("takes 7"));
}

#[test]
fn a_data_directory_that_is_not_there_or_holds_no_csv_is_a_usage_error() {
    let tmp = tempdir();
    let (code, _, err) = run(&[
        "calculate",
        &model(),
        "--data",
        &s(&tmp.path().join("absent")),
        "--cell",
        CELL,
    ]);
    assert_eq!(code, 1);
    assert!(err.contains("no data directory"));
    let empty = tmp.path().join("data");
    std::fs::create_dir(&empty).unwrap();
    let (code, _, err) = run(&["calculate", &model(), "--data", &s(&empty), "--cell", CELL]);
    assert_eq!(code, 1);
    assert!(err.contains("no CSV files"));
}

#[test]
fn a_csv_row_naming_an_element_the_model_does_not_hold_is_a_usage_error() {
    let tmp = tempdir();
    let data = tmp.path().join("data");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(
        data.join("drivers.csv"),
        "Year,Version,Period,DriverMeasure,Value\nFY2026-27,Budget,Full Year,Ghost,1\n",
    )
    .unwrap();
    let (code, _, err) = run(&["calculate", &model(), "--data", &s(&data), "--cell", CELL]);
    assert_eq!(code, 1);
    assert!(err.contains("row 2") && err.contains("Ghost"));
}

#[test]
fn a_csv_that_is_not_utf_8_is_a_usage_error() {
    let tmp = tempdir();
    let data = tmp.path().join("data");
    std::fs::create_dir(&data).unwrap();
    let mut bytes = b"Year,Version,Period,DriverMeasure,Value\n".to_vec();
    bytes.extend(b"FY2026-27,Budget,Full Year,Crew \xa3 rate,1\n");
    std::fs::write(data.join("drivers.csv"), bytes).unwrap();
    let (code, _, err) = run(&["calculate", &model(), "--data", &s(&data), "--cell", CELL]);
    assert_eq!(code, 1);
    assert!(err.contains("drivers.csv") && err.contains("UTF-8"));
}

#[test]
fn a_model_file_that_is_not_utf_8_is_an_invalid_model() {
    let tmp = tempdir();
    let root = build_model(&tmp.path().join("model"), "", &[]);
    let manifest = Path::new(&root).join("tm1project.json");
    let text = std::fs::read(&manifest).unwrap();
    let text = String::from_utf8(text)
        .unwrap()
        .replace("\"built\"", "\"buil\u{fffd}\"");
    let mut bytes = text.into_bytes();
    let at = bytes
        .windows(3)
        .position(|w| w == "\u{fffd}".as_bytes())
        .unwrap();
    bytes.splice(at..at + 3, [0xff]);
    std::fs::write(&manifest, bytes).unwrap();
    let (code, _, err) = run(&["validate", &root]);
    assert_eq!(code, 2);
    assert!(err.contains("UTF-8"));
}

/// A file another handle holds open without sharing, which Windows refuses
/// to read as Python's PermissionError.
#[cfg(windows)]
fn locked(path: &Path) -> std::fs::File {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .unwrap()
}

#[cfg(windows)]
#[test]
fn an_operating_system_error_reading_a_csv_is_a_usage_error() {
    let tmp = tempdir();
    let data = build_data(&tmp.path().join("data"), "drivers.csv", "");
    let csv = Path::new(&data).join("drivers.csv");
    let _guard = locked(&csv);
    let (code, _, err) = run(&["calculate", &model(), "--data", &data, "--cell", CELL]);
    assert_eq!(code, 1);
    assert!(err.contains("Permission denied"), "{err}");
    assert!(err.contains("drivers.csv"));
}

#[cfg(windows)]
#[test]
fn an_operating_system_error_reading_the_model_is_an_invalid_model() {
    let tmp = tempdir();
    let root = build_model(&tmp.path().join("model"), "", &[]);
    let _guard = locked(&Path::new(&root).join("tm1project.json"));
    let (code, _, err) = run(&["validate", &root]);
    assert_eq!(code, 2);
    assert!(err.contains("Permission denied"), "{err}");
}

#[cfg(unix)]
#[test]
fn an_operating_system_error_reading_a_csv_is_a_usage_error() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempdir();
    let data = build_data(&tmp.path().join("data"), "drivers.csv", "");
    let csv = Path::new(&data).join("drivers.csv");
    std::fs::set_permissions(&csv, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&csv).is_ok() {
        return; // running as root: the permission cannot be refused here
    }
    let (code, _, err) = run(&["calculate", &model(), "--data", &data, "--cell", CELL]);
    assert_eq!(code, 1);
    assert!(err.contains("Permission denied"), "{err}");
    assert!(err.contains("drivers.csv"));
}

#[cfg(unix)]
#[test]
fn an_operating_system_error_reading_the_model_is_an_invalid_model() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempdir();
    let root = build_model(&tmp.path().join("model"), "", &[]);
    let manifest = Path::new(&root).join("tm1project.json");
    std::fs::set_permissions(&manifest, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&manifest).is_ok() {
        return;
    }
    let (code, _, err) = run(&["validate", &root]);
    assert_eq!(code, 2);
    assert!(err.contains("Permission denied"), "{err}");
}

#[test]
fn a_csv_that_resolves_outside_the_data_directory_is_refused() {
    let tmp = tempdir();
    let outside = tmp.path().join("outside.csv");
    std::fs::write(&outside, "Year,Version,Period,DriverMeasure,Value\n").unwrap();
    let data = tmp.path().join("data");
    std::fs::create_dir(&data).unwrap();
    let link = data.join("drivers.csv");
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(&outside, &link).is_ok();
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(&outside, &link).is_ok();
    if !made {
        return; // this machine will not create a symlink
    }
    let (code, _, err) = run(&["calculate", &model(), "--data", &s(&data), "--cell", CELL]);
    assert_eq!(code, 1);
    assert!(err.contains("resolves outside the data directory"));
}

#[test]
fn two_data_files_feeding_one_cube_are_a_usage_error() {
    let tmp = tempdir();
    let data = tmp.path().join("data");
    std::fs::create_dir(&data).unwrap();
    for entry in std::fs::read_dir(examples()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "csv") {
            std::fs::copy(&path, data.join(path.file_name().unwrap())).unwrap();
        }
    }
    std::fs::write(
        data.join("PnL.csv"),
        "Year,Version,Period,Entity,CostCentre,Account,PnLMeasure,Value\nFY2026-27,Budget,Jul,CivilCo,Earthworks,Subcontractor Costs,Amount,1000000000\n",
    )
    .unwrap();
    let (code, out, err) = run(&[
        "report",
        &model(),
        "--data",
        &s(&data),
        "--year",
        "FY2026-27",
        "--version",
        "Budget",
    ]);
    assert_eq!(code, 1);
    assert_eq!(out, "");
    assert!(err.contains("PnL.csv") && err.contains("pnl-direct.csv"));
}

#[test]
fn conflicting_rows_are_a_usage_error_without_calculated_output() {
    let tmp = tempdir();
    let root = build_model(&tmp.path().join("model"), "", &[]);
    let data = build_data(
        &tmp.path().join("data"),
        "sales.csv",
        "Red,Units,10\nred,units,20\n",
    );
    let (code, out, err) = run(&[
        "calculate",
        &root,
        "--data",
        &data,
        "--cell",
        "Sales:Red,Units",
    ]);
    assert_eq!(code, 1);
    assert_eq!(out, "");
    assert!(err.contains("sales.csv") && err.contains("row 2") && err.contains("row 3"));
}

#[test]
fn a_csv_matching_no_cube_is_a_usage_error() {
    let tmp = tempdir();
    let data = tmp.path().join("data");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(data.join("ledger.csv"), "Year,Value\n").unwrap();
    let (code, _, err) = run(&["calculate", &model(), "--data", &s(&data), "--cell", CELL]);
    assert_eq!(code, 1);
    assert!(err.contains("'ledger'"));
}

#[test]
fn a_missing_required_option_is_a_usage_error() {
    let (code, _, err) = run(&["calculate", &model(), "--cell", CELL]);
    assert_eq!(code, 1);
    assert!(err.contains("the following arguments are required: --data"));
}

#[test]
fn help_and_version_exit_zero() {
    let (code, out, _) = run(&["--help"]);
    assert_eq!(code, 0);
    assert!(out.starts_with("usage: pacioliscube [-h] [--version] SUBCOMMAND ..."));
    let (code, out, _) = run(&["--version"]);
    assert_eq!(code, 0);
    assert_eq!(out, format!("pacioliscube {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn calculate_refuses_a_model_that_does_not_validate() {
    let tmp = tempdir();
    let root = build_model(&tmp.path().join("model"), UNKNOWN_ELEMENT_RULES, &[]);
    let data = build_data(&tmp.path().join("data"), "sales.csv", "Red,Units,10\n");
    let (code, out, err) = run(&[
        "calculate",
        &root,
        "--data",
        &data,
        "--cell",
        "Sales:Red,Amount",
    ]);
    assert_eq!(code, 2);
    assert_eq!(out, "");
    assert!(err.contains("ELE001"));
}

#[test]
fn a_division_by_zero_while_evaluating_returns_three() {
    let tmp = tempdir();
    let root = build_model(&tmp.path().join("model"), DIVIDE_BY_ZERO_RULES, &[]);
    let data = build_data(&tmp.path().join("data"), "sales.csv", "Red,Units,10\n");
    let (code, _, err) = run(&[
        "calculate",
        &root,
        "--data",
        &data,
        "--cell",
        "Sales:Red,Amount",
    ]);
    assert_eq!(code, 3);
    assert!(err.contains("division by zero"));
}

#[test]
fn a_model_fault_found_while_consolidating_returns_two() {
    let tmp = tempdir();
    let root = build_model(&tmp.path().join("model"), AMBIGUOUS_AREA_RULES, &[]);
    let data = build_data(&tmp.path().join("data"), "sales.csv", "Red,Units,10\n");
    assert_eq!(run(&["validate", &root]).0, 0);
    let (code, out, err) = run(&[
        "calculate",
        &root,
        "--data",
        &data,
        "--cell",
        "Sales:Red,Units",
        "--cell",
        "Sales:Total,Amount",
    ]);
    assert_eq!(code, 2);
    assert_eq!(out, "Sales:Red,Units = 10\n");
    assert!(err.contains("area names two elements of dimension 'Colour'"));
}

#[test]
fn a_data_file_loads_into_the_cube_its_name_matches() {
    let tmp = tempdir();
    let root = build_model(
        &tmp.path().join("model"),
        "SKIPCHECK;\n['Amount'] = N: ['Units'] * 2;\nFEEDERS;\n['Units'] => ['Amount'];\n",
        &[],
    );
    let data = build_data(&tmp.path().join("data"), "sales.csv", "Red,Units,10\n");
    let (code, out, _) = run(&[
        "calculate",
        &root,
        "--data",
        &data,
        "--cell",
        "Sales:Red,Amount",
    ]);
    assert_eq!(code, 0);
    assert_eq!(out, "Sales:Red,Amount = 20\n");
}

#[test]
fn the_report_prints_the_profit_and_loss_lines_in_statement_order() {
    let rows = report_rows(&budget_report());
    let labels: Vec<&str> = rows.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(labels, STATEMENT_ROWS);
}

#[test]
fn the_report_totals_the_shipped_budget() {
    let rows = report_rows(&budget_report());
    let expected = [
        ("Revenue", "52,764,000"),
        ("Direct Costs", "(14,884,800)"),
        ("Gross Margin", "37,879,200"),
        ("Employment Costs", "(11,428,418)"),
        ("Overheads", "(6,384,000)"),
        ("EBITDA", "20,066,782"),
        ("Depreciation", "(1,382,143)"),
        ("EBIT", "18,684,640"),
    ];
    let rows: Vec<(&str, &str)> = rows.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    assert_eq!(rows, expected);
}

#[test]
fn a_deduction_prints_in_brackets_and_a_result_prints_plain() {
    let rows = report_rows(&budget_report());
    let mut bracketed: Vec<&str> = rows
        .iter()
        .filter(|(_, a)| a.starts_with('('))
        .map(|(l, _)| l.as_str())
        .collect();
    bracketed.sort();
    assert_eq!(
        bracketed,
        [
            "Depreciation",
            "Direct Costs",
            "Employment Costs",
            "Overheads"
        ]
    );
}

#[test]
fn the_report_help_names_the_sign_convention() {
    let (_, out, _) = run(&["report", "--help"]);
    assert!(out.contains("brackets"));
}

#[test]
fn money_brackets_a_negative_and_prints_a_rounded_zero_plain() {
    let m = |t: &str| money(&Decimal::parse(t).unwrap()).unwrap();
    assert_eq!(m("1234.5"), "1,235");
    assert_eq!(m("-1234.5"), "(1,235)");
    assert_eq!(m("-0.4"), "0");
}

#[test]
fn the_printed_lines_articulate_to_within_the_rounding() {
    let rows = report_rows(&budget_report());
    let value = |name: &str| dollars(&rows.iter().find(|(l, _)| l == name).unwrap().1);
    assert_eq!(
        value("Gross Margin"),
        value("Revenue") + value("Direct Costs")
    );
    assert!((value("EBIT") - (value("EBITDA") + value("Depreciation"))).abs() <= 1);
}

#[test]
fn the_report_names_the_year_the_version_and_the_slice_it_read() {
    let (_, out, _) = run(&[
        "report",
        &model(),
        "--data",
        &ex(),
        "--year",
        "fy2026-27",
        "--version",
        "budget",
    ]);
    let header: Vec<&str> = out.lines().take(2).collect();
    assert_eq!(
        header,
        [
            "Profit and loss for FY2026-27, Budget",
            "PnL at FY, Group, All Cost Centres"
        ]
    );
}

#[test]
fn the_report_returns_one_for_a_year_or_version_the_model_does_not_hold() {
    let (code, _, err) = run(&[
        "report",
        &model(),
        "--data",
        &ex(),
        "--year",
        "FY2099-00",
        "--version",
        "Budget",
    ]);
    assert_eq!(code, 1);
    assert!(err.contains("FY2099-00"));
    let (code, _, err) = run(&[
        "report",
        &model(),
        "--data",
        &ex(),
        "--year",
        "FY2026-27",
        "--version",
        "Forecast",
    ]);
    assert_eq!(code, 1);
    assert!(err.contains("Forecast"));
}

#[test]
fn the_report_refuses_a_cube_without_a_year_version_and_account() {
    let tmp = tempdir();
    let root = build_report_model(
        &tmp.path().join("model"),
        &[("Period", &["FY"]), ("Entity", &["Group"])],
    );
    let data = tmp.path().join("data");
    std::fs::create_dir(&data).unwrap();
    let (code, out, err) = run(&[
        "report",
        &root,
        "--data",
        &s(&data),
        "--year",
        "FY2026-27",
        "--version",
        "Budget",
    ]);
    assert_eq!(code, 1);
    assert_eq!(out, "");
    assert!(err.contains("no Year, Version, Account dimension"));
}

#[test]
fn the_report_refuses_a_model_that_rolls_up_to_something_else() {
    let tmp = tempdir();
    let root = build_report_model(
        &tmp.path().join("model"),
        &[
            ("Year", &["FY2026-27"]),
            ("Version", &["Budget"]),
            ("Period", &["Full Year"]),
            ("Account", &STATEMENT_ROWS),
        ],
    );
    let data = tmp.path().join("data");
    std::fs::create_dir(&data).unwrap();
    assert_eq!(run(&["validate", &root]).0, 0);
    let (code, out, err) = run(&[
        "report",
        &root,
        "--data",
        &s(&data),
        "--year",
        "FY2026-27",
        "--version",
        "Budget",
    ]);
    assert_eq!(code, 1);
    assert_eq!(out, "");
    assert!(err.contains("Period 'FY'") && err.contains("calculate"));
}

#[test]
fn the_report_refuses_a_cube_holding_a_dimension_it_has_no_element_for() {
    let tmp = tempdir();
    let root = build_report_model(
        &tmp.path().join("model"),
        &[
            ("Year", &["FY2026-27"]),
            ("Version", &["Budget"]),
            ("Account", &STATEMENT_ROWS),
            ("Colour", &["Red"]),
        ],
    );
    let data = tmp.path().join("data");
    std::fs::create_dir(&data).unwrap();
    let (code, out, err) = run(&[
        "report",
        &root,
        "--data",
        &s(&data),
        "--year",
        "FY2026-27",
        "--version",
        "Budget",
    ]);
    assert_eq!(code, 1);
    assert_eq!(out, "");
    assert!(err.contains("'Colour'"));
}

#[test]
fn a_run_writes_nothing_into_the_model_or_the_data_directory() {
    fn snapshot() -> Vec<(String, u64, std::time::SystemTime)> {
        let mut out = Vec::new();
        let mut pending = vec![model_root(), examples()];
        while let Some(dir) = pending.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                let meta = entry.metadata().unwrap();
                if meta.is_dir() {
                    pending.push(entry.path());
                }
                out.push((s(&entry.path()), meta.len(), meta.modified().unwrap()));
            }
        }
        out.sort();
        out
    }
    let before = snapshot();
    assert_eq!(run_owned(&budget_report()).0, 0);
    assert_eq!(snapshot(), before);
}

#[test]
fn the_report_header_names_only_the_restrictions_it_applied() {
    let tmp = tempdir();
    let root = build_report_model(
        &tmp.path().join("model"),
        &[
            ("Year", &["FY2026-27"]),
            ("Version", &["Budget"]),
            ("Account", &STATEMENT_ROWS),
        ],
    );
    let data = write_report_data(&tmp.path().join("data"), "Year,Version,Account,Value");
    let (code, out, _) = run(&[
        "report",
        &root,
        "--data",
        &data,
        "--year",
        "FY2026-27",
        "--version",
        "Budget",
    ]);
    assert_eq!(code, 0);
    assert!(!out.contains("FY, Group, All Cost Centres"));
    assert!(out.contains("whole cube"));
}

#[test]
fn the_report_header_names_a_partial_slice_without_inventing_the_rest() {
    let tmp = tempdir();
    let root = build_report_model(
        &tmp.path().join("model"),
        &[
            ("Year", &["FY2026-27"]),
            ("Version", &["Budget"]),
            ("Period", &["FY"]),
            ("Account", &STATEMENT_ROWS),
        ],
    );
    let data = write_report_data(
        &tmp.path().join("data"),
        "Year,Version,Period,Account,Value",
    );
    let (code, out, _) = run(&[
        "report",
        &root,
        "--data",
        &data,
        "--year",
        "FY2026-27",
        "--version",
        "Budget",
    ]);
    assert_eq!(code, 0);
    assert!(out.contains("PnL at FY"));
    assert!(!out.contains("Group") && !out.contains("All Cost Centres"));
}

#[test]
fn the_shipped_report_still_names_its_full_slice() {
    let (code, out, _) = run_owned(&budget_report());
    assert_eq!(code, 0);
    assert!(out.contains("PnL at FY, Group, All Cost Centres"));
}

#[test]
fn output_cells_share_one_calculation_within_one_command() {
    // Python counted CellStore.get calls; the binary's batch shares one engine,
    // so asking for the same consolidated cell twice gives the same figure
    // without recalculating, and the report's eight rows come from one pass.
    let (code, out, err) = run(&[
        "calculate",
        &model(),
        "--data",
        &ex(),
        "--cell",
        GROUP_EBITDA,
        "--cell",
        GROUP_EBITDA,
    ]);
    assert_eq!(code, 0);
    assert!(err.is_empty());
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], lines[1]);
}

const DEMO_LINE: &str =
    "PnL:FY2026-27,Budget,Jul,CivilCo,All Cost Centres,Gross Margin,Amount = 1652100\n";

#[test]
fn demo_reads_its_bundled_inputs_from_an_empty_directory_without_writes() {
    let empty = tempdir();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_pacioliscube"))
        .arg("demo")
        .current_dir(empty.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    // The independent gross-margin arithmetic is pinned in tests/calculations.rs.
    // Standard output is text mode, as in Python, so Windows ends lines with CRLF.
    let printed = String::from_utf8(output.stdout)
        .unwrap()
        .replace("\r\n", "\n");
    assert_eq!(printed, DEMO_LINE);
    assert_eq!(std::fs::read_dir(empty.path()).unwrap().count(), 0);
}

#[test]
fn demo_removes_its_unpacked_inputs_afterwards() {
    let prefix = format!("pacioliscube-demo-{}-", std::process::id());
    let leftover = || {
        std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter(|e| {
                let name = e.as_ref().unwrap().file_name();
                name.to_string_lossy().starts_with(&prefix)
            })
            .count()
    };
    let (code, out, err) = run(&["demo"]);
    assert_eq!((code, out.as_str(), err.as_str()), (0, DEMO_LINE, ""));
    assert_eq!(leftover(), 0);
}

#[test]
fn the_json_report_keeps_exact_values_coordinates_and_deduction_signs() {
    let mut args = budget_report();
    args.extend(["--format".into(), "json".into()]);
    let (code, out, err) = run_owned(&args);
    assert_eq!(code, 0, "{err}");
    let document = pacioliscube::pyjson::loads(&out).unwrap();
    assert_eq!(
        document.get("schema_version").unwrap().as_str(),
        Some("profit-and-loss.v1")
    );
    assert_eq!(document.get("cube").unwrap().as_str(), Some("PnL"));
    let Some(pacioliscube::pyjson::Json::Array(rows)) = document.get("rows") else {
        panic!("rows is a list")
    };
    let accounts: Vec<&str> = rows
        .iter()
        .map(|r| r.get("account").unwrap().as_str().unwrap())
        .collect();
    assert_eq!(accounts, STATEMENT_ROWS);
    for row in rows {
        let Some(pacioliscube::pyjson::Json::Object(coordinate)) = row.get("coordinate") else {
            panic!("coordinate is an object")
        };
        let element = |d: &str| {
            coordinate
                .iter()
                .find(|(k, _)| k == d)
                .and_then(|(_, v)| v.as_str())
                .unwrap()
        };
        assert_eq!(
            (element("Year"), element("Version")),
            ("FY2026-27", "Budget")
        );
        let elements: Vec<&str> = coordinate
            .iter()
            .map(|(_, v)| v.as_str().unwrap())
            .collect();
        let cell = format!("PnL:{}", elements.join(","));
        let (calculated, value, errors) =
            run(&["calculate", &model(), "--data", &ex(), "--cell", &cell]);
        assert_eq!(calculated, 0, "{errors}");
        let stored = row.get("value").unwrap().as_str().unwrap();
        assert_eq!(stored, value.split(" = ").nth(1).unwrap().trim());
        let account = row.get("account").unwrap().as_str().unwrap();
        let deduction = [
            "Direct Costs",
            "Employment Costs",
            "Overheads",
            "Depreciation",
        ];
        let value = Decimal::parse(stored).unwrap();
        let effect = if deduction.contains(&account) {
            value.copy_negate()
        } else {
            value
        };
        let printed = Decimal::parse(row.get("effect").unwrap().as_str().unwrap()).unwrap();
        assert_eq!(printed.to_fixed_string(), effect.to_fixed_string());
    }
}

/// Every record of a CSV document, header first.
fn csv_records(text: &str) -> Vec<Vec<String>> {
    let mut reader = pacioliscube::csvread::Reader::new(text.as_bytes());
    let mut records = Vec::new();
    while let Some(record) = reader.next_record().unwrap() {
        records.push(record);
    }
    records
}

#[test]
fn the_csv_report_matches_the_json_values_and_coordinates() {
    let report = |format: &str| {
        let mut args = budget_report();
        args.extend(["--format".into(), format.into()]);
        let (code, out, err) = run_owned(&args);
        assert_eq!(code, 0, "{err}");
        out
    };
    let records = csv_records(&report("csv"));
    let document = pacioliscube::pyjson::loads(&report("json")).unwrap();
    let Some(pacioliscube::pyjson::Json::Array(rows)) = document.get("rows") else {
        panic!("rows is a list")
    };
    assert_eq!(records.len(), rows.len() + 1);
    let mut header = vec!["cube".to_string()];
    let Some(pacioliscube::pyjson::Json::Object(first)) = rows[0].get("coordinate") else {
        panic!("coordinate is an object")
    };
    header.extend(first.iter().map(|(k, _)| k.clone()));
    header.extend(["value".to_string(), "effect".to_string()]);
    assert_eq!(records[0], header);
    for (record, row) in records[1..].iter().zip(rows) {
        let Some(pacioliscube::pyjson::Json::Object(coordinate)) = row.get("coordinate") else {
            panic!("coordinate is an object")
        };
        let mut expected = vec!["PnL".to_string()];
        expected.extend(
            coordinate
                .iter()
                .map(|(_, v)| v.as_str().unwrap().to_string()),
        );
        for key in ["value", "effect"] {
            expected.push(row.get(key).unwrap().as_str().unwrap().to_string());
        }
        assert_eq!(record, &expected);
    }
}

#[test]
fn a_failing_machine_report_prints_nothing_to_stdout() {
    for format in ["json", "csv"] {
        let mut args = budget_report();
        args.extend([
            "--year".into(),
            "absent".into(),
            "--format".into(),
            format.into(),
        ]);
        let (code, out, err) = run_owned(&args);
        assert_eq!(code, 1);
        assert_eq!(out, "");
        assert!(err.contains("absent"));
    }
}

#[test]
fn the_text_format_remains_the_default() {
    let mut args = budget_report();
    let default = run_owned(&args);
    args.extend(["--format".into(), "text".into()]);
    assert_eq!(run_owned(&args), default);
}

fn report_tree(tmp: &Path, year: &str, version: &str) -> (String, String) {
    let root = build_report_model(
        &tmp.join("model"),
        &[
            ("Year", &[year]),
            ("Version", &[version]),
            ("Account", &STATEMENT_ROWS),
        ],
    );
    let data = write_report_data(&tmp.join("data"), "Year,Version,Account,Value");
    (root, data)
}

#[test]
fn the_machine_report_keeps_precision_and_only_applied_dimensions() {
    let tmp = tempdir();
    let (root, data) = report_tree(tmp.path(), "FY2026-27", "Budget");
    let amount = "123456789012345678901234567890.123456789";
    let csv = Path::new(&data).join("PnL.csv");
    let mut text = std::fs::read_to_string(&csv).unwrap();
    text.push_str(&format!("FY2026-27,Budget,Direct Costs,{amount}\n"));
    std::fs::write(&csv, text).unwrap();
    let args = |format: &'static str| {
        let common = ["report", &root, "--data", &data, "--year", "FY2026-27"];
        let mut all: Vec<String> = common.iter().map(|s| s.to_string()).collect();
        all.extend(["--version", "Budget", "--format", format].map(String::from));
        all
    };
    let (code, out, err) = run_owned(&args("json"));
    assert_eq!(code, 0, "{err}");
    let document = pacioliscube::pyjson::loads(&out).unwrap();
    let Some(pacioliscube::pyjson::Json::Array(rows)) = document.get("rows") else {
        panic!("rows is a list")
    };
    assert_eq!(rows[1].get("value").unwrap().as_str(), Some(amount));
    let negative = format!("-{amount}");
    assert_eq!(
        rows[1].get("effect").unwrap().as_str(),
        Some(negative.as_str())
    );
    let Some(pacioliscube::pyjson::Json::Object(coordinate)) = rows[1].get("coordinate") else {
        panic!("coordinate is an object")
    };
    let keys: Vec<&str> = coordinate.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["Year", "Version", "Account"]);
    let (code, out, err) = run_owned(&args("csv"));
    assert_eq!(code, 0, "{err}");
    let records = csv_records(&out);
    assert_eq!(
        records[0],
        ["cube", "Year", "Version", "Account", "value", "effect"]
    );
    assert_eq!(records[2][4], amount);
    assert_eq!(records[2][5], negative);
}

#[test]
fn a_zero_deduction_has_no_negative_zero_effect() {
    let tmp = tempdir();
    let (root, data) = report_tree(tmp.path(), "FY2026-27", "Budget");
    for format in ["json", "csv"] {
        let (code, out, err) = run(&[
            "report",
            &root,
            "--data",
            &data,
            "--year",
            "FY2026-27",
            "--version",
            "Budget",
            "--format",
            format,
        ]);
        assert_eq!(code, 0, "{err}");
        let effects: Vec<String> = if format == "json" {
            let document = pacioliscube::pyjson::loads(&out).unwrap();
            let Some(pacioliscube::pyjson::Json::Array(rows)) = document.get("rows") else {
                panic!("rows is a list")
            };
            rows.iter()
                .map(|r| r.get("effect").unwrap().as_str().unwrap().to_string())
                .collect()
        } else {
            csv_records(&out)[1..]
                .iter()
                .map(|r| r[5].clone())
                .collect()
        };
        assert_eq!(effects, vec!["0"; STATEMENT_ROWS.len()], "{format}");
    }
}

#[test]
fn csv_quotes_coordinates_and_keeps_formula_labels_literal_without_changing_json() {
    let tmp = tempdir();
    // The model file is JSON, so the quotes inside this element name are escaped there.
    let (root, data) = report_tree(tmp.path(), r#"FY,\"2027\""#, "=1+1");
    let year = r#"FY,"2027""#;
    let report = |format: &str| {
        run(&[
            "report",
            &root,
            "--data",
            &data,
            "--year",
            year,
            "--version",
            "=1+1",
            "--format",
            format,
        ])
    };
    let (code, out, err) = report("csv");
    assert_eq!(code, 0, "{err}");
    assert!(
        out.lines()
            .nth(1)
            .unwrap()
            .starts_with(r#"PnL,"FY,""2027""",'=1+1,"#)
    );
    let record = &csv_records(&out)[1];
    assert_eq!((record[1].as_str(), record[2].as_str()), (year, "'=1+1"));
    let (code, out, err) = report("json");
    assert_eq!(code, 0, "{err}");
    let document = pacioliscube::pyjson::loads(&out).unwrap();
    let Some(pacioliscube::pyjson::Json::Array(rows)) = document.get("rows") else {
        panic!("rows is a list")
    };
    let version = rows[0].get("coordinate").unwrap().get("Version").unwrap();
    assert_eq!(version.as_str(), Some("=1+1"));
}

#[test]
fn csv_formula_prefixes_get_a_leading_apostrophe() {
    for label in [
        "=1+1", " +2", "-3", "@SUM(A1)", "\tlabel", "\rlabel", "\nlabel",
    ] {
        let tmp = tempdir();
        let escaped = label
            .replace('\t', "\\t")
            .replace('\r', "\\r")
            .replace('\n', "\\n");
        let (root, data) = report_tree(tmp.path(), "FY2026-27", &escaped);
        let (code, out, err) = run(&[
            "report",
            &root,
            "--data",
            &data,
            "--year",
            "FY2026-27",
            "--version",
            label,
            "--format",
            "csv",
        ]);
        assert_eq!(code, 0, "{label:?} {err}");
        assert_eq!(csv_records(&out)[1][2], format!("'{label}"), "{label:?}");
    }
}
