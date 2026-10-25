//! `pacioliscube mcp`: the server driven over its standard input and output as
//! a client would, with each figure checked against the command line's own.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use common::*;
use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_pacioliscube");

const DIVIDE_BY_ZERO_RULES: &str =
    "SKIPCHECK;\n['Amount'] = N: ['Units'] / ['Price'];\nFEEDERS;\n['Units'] => ['Amount'];\n";
const UNKNOWN_ELEMENT_RULES: &str =
    "SKIPCHECK;\n['Ghost'] = N: 1;\nFEEDERS;\n['Units'] => ['Amount'];\n";

/// A JSON-RPC client on the server's pipes, one message per line.
struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    info: Value,
}

impl Client {
    fn start(model: &str, data: &str) -> Client {
        let mut child = Command::new(BIN)
            .args(["mcp", model, "--data", data])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("the binary starts");
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
        let mut client = Client {
            child,
            stdin,
            stdout,
            next_id: 0,
            info: Value::Null,
        };
        client.info = client.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0"},
            }),
        );
        client.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        client
    }

    fn shipped() -> Client {
        Client::start(&s(&model_root()), &s(&examples()))
    }

    fn send(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let mut line = String::new();
            assert!(
                self.stdout.read_line(&mut line).unwrap() > 0,
                "the server closed stdout before answering {method}"
            );
            let message: Value = serde_json::from_str(&line).expect("one JSON message per line");
            if message["id"] == json!(id) {
                assert!(message.get("error").is_none(), "{method}: {message}");
                return message["result"].clone();
            }
        }
    }

    /// A tool's text and whether the server marked it an error.
    fn call(&mut self, name: &str, arguments: Value) -> (bool, String) {
        let result = self.request("tools/call", json!({"name": name, "arguments": arguments}));
        let text = result["content"][0]["text"]
            .as_str()
            .expect("text content")
            .to_string();
        (result["isError"] == json!(true), text)
    }

    fn ok(&mut self, name: &str, arguments: Value) -> String {
        let (error, text) = self.call(name, arguments);
        assert!(!error, "{name} failed: {text}");
        text
    }

    fn ok_json(&mut self, name: &str, arguments: Value) -> Value {
        serde_json::from_str(&self.ok(name, arguments)).expect("a JSON document")
    }

    fn refused(&mut self, name: &str, arguments: Value) -> String {
        let (error, text) = self.call(name, arguments);
        assert!(error, "{name} should have failed: {text}");
        text
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // EOF allows the server to flush its profiling data before exit.
        drop(self.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn cli(args: &[&str]) -> String {
    let (code, out, err) = run(args);
    assert_eq!(code, 0, "{args:?}: {err}");
    out
}

fn slice(year: &str, accounts: &[&str], periods: &[&str]) -> Value {
    json!({
        "cube": "PnL",
        "elements": {
            "Year": [year],
            "Version": ["Budget"],
            "Period": periods,
            "Entity": ["CivilCo"],
            "CostCentre": ["All Cost Centres"],
            "Account": accounts,
            "PnLMeasure": ["Amount"],
        },
    })
}

#[test]
fn the_server_names_itself_and_lists_read_only_tools() {
    let mut client = Client::shipped();
    assert_eq!(client.info["serverInfo"]["name"], "pacioliscube");
    let tools = client.request("tools/list", json!({}))["tools"].clone();
    let mut names: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "explain_cells",
            "explain_rules",
            "list_cubes",
            "list_dimensions",
            "read_slice",
            "render_report"
        ]
    );
    for tool in tools.as_array().unwrap() {
        assert_eq!(tool["annotations"]["readOnlyHint"], true, "{tool}");
    }
}

#[test]
fn list_cubes_gives_dimensions_in_coordinate_order() {
    let mut client = Client::shipped();
    let cubes = client.ok_json("list_cubes", json!({}))["cubes"].clone();
    let names: Vec<&str> = cubes
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Drivers", "Workforce", "Revenue", "Capex", "PnL"]);
    let pnl = &cubes[4];
    assert_eq!(
        pnl["dimensions"],
        json!([
            "Year",
            "Version",
            "Period",
            "Entity",
            "CostCentre",
            "Account",
            "PnLMeasure"
        ])
    );
    assert_eq!(pnl["rules_file"], "cubes/PnL.rules");
    assert_eq!(pnl["skipcheck"], true);
}

#[test]
fn list_dimensions_gives_elements_and_weighted_children() {
    let mut client = Client::shipped();
    let listed = client.ok_json("list_dimensions", json!({"dimension": "Entity"}));
    assert_eq!(
        listed["dimensions"][0]["elements"],
        json!([
            {"name": "CivilCo", "type": "Numeric", "children": []},
            {"name": "HaulCo", "type": "Numeric", "children": []},
            {"name": "Group", "type": "Consolidated", "children": [
                {"element": "CivilCo", "weight": "1"},
                {"element": "HaulCo", "weight": "1"},
            ]},
        ])
    );
    let all = client.ok_json("list_dimensions", json!({}));
    assert_eq!(all["dimensions"].as_array().unwrap().len(), 13);
    let refused = client.refused("list_dimensions", json!({"dimension": "Nowhere"}));
    assert!(
        refused.contains("no dimension named 'Nowhere'"),
        "{refused}"
    );
}

#[test]
fn read_slice_matches_calculate_cell_by_cell() {
    let mut client = Client::shipped();
    let accounts = ["Gross Margin", "EBIT"];
    let periods = ["Jul", "FY"];
    let read = client.ok_json("read_slice", slice("FY2026-27", &accounts, &periods));
    let cells = read["cells"].as_array().unwrap();
    assert_eq!(cells.len(), 4);
    let (model, data) = (s(&model_root()), s(&examples()));
    let mut index = 0;
    // The last dimension varies fastest, and Account comes after Period.
    for period in periods {
        for account in accounts {
            let cell =
                format!("PnL:FY2026-27,Budget,{period},CivilCo,All Cost Centres,{account},Amount");
            let printed = cli(&["calculate", &model, "--data", &data, "--cell", &cell]);
            let value = cells[index]["value"].as_str().unwrap();
            assert_eq!(printed.trim_end(), format!("{cell} = {value}"));
            assert_eq!(cells[index]["coordinate"][2], period);
            assert_eq!(cells[index]["coordinate"][5], account);
            index += 1;
        }
    }
}

#[test]
fn explain_rules_lists_the_feeders_that_reach_a_rule() {
    let mut client = Client::shipped();
    let explained = client.ok_json("explain_rules", json!({"cube": "PnL", "line": 23}));
    assert_eq!(explained["rules_file"], "cubes/PnL.rules");
    let rule = &explained["rules"][0];
    assert_eq!(rule["qualifier"], "N");
    assert_eq!(
        rule["area"],
        json!(["CivilCo", "Corporate", "Payroll Tax", "Amount"])
    );
    let statement = rule["statement"].as_str().unwrap();
    assert!(statement.starts_with("['CivilCo', 'Corporate', 'Payroll Tax', 'Amount'] = N:"));
    assert!(statement.ends_with(") );"), "{statement}");
    let fed_by: Vec<(String, u64)> = rule["fed_by"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["file"].as_str().unwrap().to_string(),
                f["line"].as_u64().unwrap(),
            )
        })
        .collect();
    // PnL.rules says the threshold feeds these cells from Drivers.rules, and
    // the general payroll tax feeder comes from Workforce.
    assert_eq!(
        fed_by,
        [
            ("cubes/Drivers.rules".to_string(), 12),
            ("cubes/Workforce.rules".to_string(), 35),
        ]
    );
    let all = client.ok_json("explain_rules", json!({"cube": "PnL"}));
    assert_eq!(all["rules"].as_array().unwrap().len(), 9);
    let refused = client.refused("explain_rules", json!({"cube": "PnL", "line": 21}));
    assert!(
        refused.contains("no rule of cube PnL starts at line 21"),
        "{refused}"
    );
}

#[test]
fn explain_rules_preserves_source_constraints_in_cross_cube_feeders() {
    for (source, target, expected) in [
        ("Red", "!Colour", true),
        ("Blue", "!Colour", false),
        ("Total", "!Colour", true),
        ("Blue", "'Total'", true),
    ] {
        let tmp = tempdir();
        let spec = Spec {
            rules: "SKIPCHECK;\n['Red','Amount'] = N: 1;\n".into(),
            manifest_cubes: "\"cubes/Sales.json\", \"cubes/Feeder.json\"".into(),
            extra_files: vec![
                (
                    "cubes/Feeder.json".into(),
                    format!(
                        r#"{{"Name":"Feeder","Dimensions@Code.links":[{CUBE_DIMENSIONS}],"Rules@Code.link":"Feeder.rules"}}"#
                    ),
                ),
                (
                    "cubes/Feeder.rules".into(),
                    format!(
                        "SKIPCHECK;\nFEEDERS;\n['{source}','Units'] => DB('Sales', {target}, 'Amount');\n"
                    ),
                ),
            ],
            ..Spec::default()
        };
        let root = s(&write_model(&tmp.path().join("model"), &spec));
        let data = build_data(&tmp.path().join("data"), "sales.csv", "Red,Units,10\n");
        let mut client = Client::start(&root, &data);
        let explained = client.ok_json("explain_rules", json!({"cube": "Sales", "line": 2}));
        let feeders = explained["rules"][0]["fed_by"].as_array().unwrap();
        assert_eq!(
            !feeders.is_empty(),
            expected,
            "source={source}, target={target}: {explained}"
        );
    }
}

#[test]
fn explain_rules_preserves_inherited_local_feeder_coordinates() {
    for (area, target, expected) in [
        ("'Red','Units'", "'Amount'", true),
        ("'Blue','Units'", "'Amount'", false),
        ("'Total','Units'", "'Amount'", true),
        ("'Blue','Units'", "'Red','Amount'", true),
        ("'Blue','Units'", "'Total','Amount'", true),
        ("'Units'", "'Amount'", true),
    ] {
        let tmp = tempdir();
        let spec = Spec::rules(&format!(
            "SKIPCHECK;\n['Red','Amount'] = N: 1;\nFEEDERS;\n[{area}] => [{target}];\n"
        ));
        let root = s(&write_model(&tmp.path().join("model"), &spec));
        let data = build_data(&tmp.path().join("data"), "sales.csv", "Red,Units,10\n");
        let mut client = Client::start(&root, &data);
        let explained = client.ok_json("explain_rules", json!({"cube": "Sales", "line": 2}));
        let feeders = explained["rules"][0]["fed_by"].as_array().unwrap();
        assert_eq!(
            !feeders.is_empty(),
            expected,
            "area={area}, target={target}: {explained}"
        );
    }
}

#[test]
fn explain_rules_resolves_carried_elements_in_the_target_hierarchy() {
    for (elements, edges, target) in [
        (
            r#"{"Name":"RED","Type":"Numeric"},{"Name":"BLUE","Type":"Numeric"}"#,
            "",
            "RED",
        ),
        (
            r#"{"Name":"Red","Type":"Consolidated"},{"Name":"Red1","Type":"Numeric"},{"Name":"Red2","Type":"Numeric"},{"Name":"Blue","Type":"Numeric"}"#,
            r#"{"ParentName":"Red","ComponentName":"Red1","Weight":1},{"ParentName":"Red","ComponentName":"Red2","Weight":1}"#,
            "Red1",
        ),
    ] {
        let tmp = tempdir();
        let spec = Spec {
            rules: format!("SKIPCHECK;\n['{target}','Amount'] = N: 1;\n"),
            manifest_cubes: "\"cubes/Sales.json\", \"cubes/Feeder.json\"".into(),
            extra_files: vec![
                (
                    "dimensions/Colour.hierarchies/Colour.json".into(),
                    format!(r#"{{"Name":"Colour","Elements":[{elements}],"Edges":[{edges}]}}"#),
                ),
                (
                    "dimensions/Shade.json".into(),
                    r#"{"Name":"Shade","Hierarchies@Code.links":["Shade.hierarchies/Shade.json"]}"#.into(),
                ),
                (
                    "dimensions/Shade.hierarchies/Shade.json".into(),
                    r#"{"Name":"Shade","Elements":[{"Name":"Red","Type":"Numeric"},{"Name":"Blue","Type":"Numeric"}],"Edges":[]}"#.into(),
                ),
                (
                    "cubes/Feeder.json".into(),
                    r#"{"Name":"Feeder","Dimensions@Code.links":["../dimensions/Shade.json","../dimensions/Measure.json"],"Rules@Code.link":"Feeder.rules"}"#.into(),
                ),
                (
                    "cubes/Feeder.rules".into(),
                    "SKIPCHECK;\nFEEDERS;\n['Red','Units'] => DB('Sales', !Shade, 'Amount');\n".into(),
                ),
                (
                    "tm1project.json".into(),
                    r#"{"Version":1.0,"Name":"built","Objects":{"Dimensions":["dimensions/Colour.json","dimensions/Measure.json","dimensions/Shade.json"],"Cubes":["cubes/Sales.json","cubes/Feeder.json"]}}"#.into(),
                ),
            ],
            ..Spec::default()
        };
        let root = s(&write_model(&tmp.path().join("model"), &spec));
        let data = build_data(
            &tmp.path().join("data"),
            "sales.csv",
            &format!("{target},Units,10\n"),
        );
        let mut client = Client::start(&root, &data);
        let explained = client.ok_json("explain_rules", json!({"cube": "Sales", "line": 2}));
        assert_eq!(
            explained["rules"][0]["fed_by"].as_array().unwrap().len(),
            1,
            "target={target}: {explained}"
        );
    }
}

#[test]
fn explain_rules_keeps_repeated_source_coordinates_together() {
    for (source, target, left, right, expected) in [
        ("'Total',", "!Shade, !Shade", "Left1", "Right2", false),
        ("'Total',", "!Shade, !Shade", "Left1", "Right1", true),
        ("'Total',", "!Shade, !Shade", "Left2", "Right2", true),
        ("", "!Shade, !Shade", "Left1", "Right2", false),
        ("", "!Shade, 'Right1'", "Unmapped", "Right1", false),
        ("", "!Shade, 'Right1'", "Left2", "Right1", true),
        ("'Missing',", "!Shade, !Shade", "Missing", "Right1", false),
        ("", "!Shade, 'Right1'", "Missing", "Right1", true),
    ] {
        let tmp = tempdir();
        let spec = Spec {
            rules: format!("SKIPCHECK;\n['{left}','{right}','Amount'] = N: 1;\n"),
            cube_dimensions: r#""../dimensions/Colour.json","../dimensions/Right.json","../dimensions/Measure.json""#.into(),
            extra_files: vec![
                (
                    "dimensions/Colour.hierarchies/Colour.json".into(),
                    r#"{"Name":"Colour","Elements":[{"Name":"A","Type":"Consolidated"},{"Name":"B","Type":"Consolidated"},{"Name":"Left1","Type":"Numeric"},{"Name":"Left2","Type":"Numeric"},{"Name":"Missing","Type":"Numeric"},{"Name":"Unmapped","Type":"Numeric"}],"Edges":[{"ParentName":"A","ComponentName":"Left1","Weight":1},{"ParentName":"B","ComponentName":"Left2","Weight":1}]}"#.into(),
                ),
                (
                    "dimensions/Right.json".into(),
                    r#"{"Name":"Right","Hierarchies@Code.links":["Right.hierarchies/Right.json"]}"#.into(),
                ),
                (
                    "dimensions/Right.hierarchies/Right.json".into(),
                    r#"{"Name":"Right","Elements":[{"Name":"A","Type":"Consolidated"},{"Name":"B","Type":"Consolidated"},{"Name":"Right1","Type":"Numeric"},{"Name":"Right2","Type":"Numeric"}],"Edges":[{"ParentName":"A","ComponentName":"Right1","Weight":1},{"ParentName":"B","ComponentName":"Right2","Weight":1}]}"#.into(),
                ),
                (
                    "dimensions/Shade.json".into(),
                    r#"{"Name":"Shade","Hierarchies@Code.links":["Shade.hierarchies/Shade.json"]}"#.into(),
                ),
                (
                    "dimensions/Shade.hierarchies/Shade.json".into(),
                    r#"{"Name":"Shade","Elements":[{"Name":"Total","Type":"Consolidated"},{"Name":"A","Type":"Numeric"},{"Name":"B","Type":"Numeric"},{"Name":"Missing","Type":"Numeric"}],"Edges":[{"ParentName":"Total","ComponentName":"A","Weight":1},{"ParentName":"Total","ComponentName":"B","Weight":1}]}"#.into(),
                ),
                (
                    "cubes/Feeder.json".into(),
                    r#"{"Name":"Feeder","Dimensions@Code.links":["../dimensions/Shade.json","../dimensions/Measure.json"],"Rules@Code.link":"Feeder.rules"}"#.into(),
                ),
                (
                    "cubes/Feeder.rules".into(),
                    format!("SKIPCHECK;\nFEEDERS;\n[{source}'Units'] => DB('Sales', {target}, 'Amount');\n"),
                ),
                (
                    "tm1project.json".into(),
                    r#"{"Version":1.0,"Name":"built","Objects":{"Dimensions":["dimensions/Colour.json","dimensions/Measure.json","dimensions/Right.json","dimensions/Shade.json"],"Cubes":["cubes/Sales.json","cubes/Feeder.json"]}}"#.into(),
                ),
            ],
            ..Spec::default()
        };
        let root = s(&write_model(&tmp.path().join("model"), &spec));
        // Only the directory exists; explaining source rules does not load CSV data.
        let data = tmp.path().join("data");
        std::fs::create_dir(&data).unwrap();
        let mut client = Client::start(&root, &s(&data));
        let explained = client.ok_json("explain_rules", json!({"cube": "Sales", "line": 2}));
        assert_eq!(
            !explained["rules"][0]["fed_by"]
                .as_array()
                .unwrap()
                .is_empty(),
            expected,
            "source={source}, target={target}, rule={left}/{right}: {explained}"
        );
    }
}

#[test]
fn explain_rules_keeps_quoted_delimiters_and_stops_at_the_real_statement_end() {
    for name in ["A;B", "A#B", "A';#B"] {
        let tmp = tempdir();
        let quoted = name.replace('\'', "''");
        let first = format!("['{quoted}'] = N: # comment with a semicolon ;\n  ['Units'] * 2;");
        let spec = Spec {
            rules: format!("SKIPCHECK;\n{first}\n['Price'] = N: 3;\n"),
            measures: format!(r#"{MEASURES}, {{"Name":"{name}","Type":"Numeric"}}"#),
            ..Spec::default()
        };
        let root = s(&write_model(&tmp.path().join("model"), &spec));
        let data = build_data(&tmp.path().join("data"), "sales.csv", "Red,Units,10\n");
        let mut client = Client::start(&root, &data);
        let explained = client.ok_json("explain_rules", json!({"cube": "Sales"}));
        assert_eq!(
            explained["rules"][0]["statement"], first,
            "{name}: {explained}"
        );
        assert_eq!(explained["rules"][1]["statement"], "['Price'] = N: 3;");
    }
}

#[test]
fn explain_rules_isolates_rules_and_feeders_sharing_source_lines() {
    for name in ["A;B", "A#B", "A';#B"] {
        let tmp = tempdir();
        let quoted = name.replace('\'', "''");
        let red_rule = format!("['Red','{quoted}'] = N: 1;");
        let blue_rule = format!("['Blue','{quoted}'] = N: 2;");
        let red_feeder = format!("['Red','Units'] => ['{quoted}'];");
        let blue_feeder = format!("['Blue','Units'] => ['{quoted}'];");
        let spec = Spec {
            rules: format!(
                "# leading comment ;\nSKIPCHECK; {red_rule} {blue_rule} # trailing comment ;\nFEEDERS; {red_feeder} {blue_feeder}\n"
            ),
            measures: format!(r#"{MEASURES}, {{"Name":"{name}","Type":"Numeric"}}"#),
            ..Spec::default()
        };
        let root = s(&write_model(&tmp.path().join("model"), &spec));
        let data = tmp.path().join("data");
        std::fs::create_dir(&data).unwrap();
        let mut client = Client::start(&root, &s(&data));
        let explained = client.ok_json("explain_rules", json!({"cube": "Sales", "line": 2}));
        let rules = explained["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 2);
        for (index, (rule, feeder)) in [(red_rule, red_feeder), (blue_rule, blue_feeder)]
            .into_iter()
            .enumerate()
        {
            assert_eq!(rules[index]["statement"], rule, "{name}: {explained}");
            assert_eq!(rules[index]["fed_by"].as_array().unwrap().len(), 1);
            assert_eq!(rules[index]["fed_by"][0]["statement"], feeder);
        }
    }
}

#[test]
fn explain_cells_limits_each_name_before_reading_the_model() {
    let tmp = tempdir();
    let root = write_model(
        &tmp.path().join("model"),
        &Spec::rules("['Amount'] = N: ['Units'] * 2;\n"),
    );
    let data = build_data(&tmp.path().join("data"), "sales.csv", "Red,Units,10\n");
    let mut client = Client::start(&s(&root), &data);
    client.ok("explain_cells", json!({"cells": ["Sales:Red,Amount"]}));
    let manifest = root.join("tm1project.json");
    let original = std::fs::read(&manifest).unwrap();
    std::fs::write(&manifest, "not JSON").unwrap();
    let long = "x".repeat(257);
    for cell in [format!("{long}:Red,Amount"), format!("Sales:{long},Amount")] {
        let refused = client.refused("explain_cells", json!({"cells": [cell]}));
        assert!(refused.contains("longer than 256"), "{refused}");
    }
    std::fs::write(&manifest, original).unwrap();
    client.ok("list_cubes", json!({}));
}

#[test]
fn explain_cells_returns_the_explain_document() {
    let mut client = Client::shipped();
    let cell = "PnL:FY2026-27,Budget,Jul,CivilCo,Earthworks,Contract Revenue,Amount";
    let served = client.ok("explain_cells", json!({"cells": [cell]}));
    let printed = cli(&[
        "explain",
        &s(&model_root()),
        "--data",
        &s(&examples()),
        "--cell",
        cell,
    ]);
    assert_eq!(served, printed);
}

#[test]
fn render_report_matches_the_report_command_in_every_format() {
    let mut client = Client::shipped();
    let (model, data) = (s(&model_root()), s(&examples()));
    for format in ["text", "json", "csv"] {
        let served = client.ok(
            "render_report",
            json!({"year": "FY2026-27", "version": "Budget", "format": format}),
        );
        let printed = cli(&[
            "report",
            &model,
            "--data",
            &data,
            "--year",
            "FY2026-27",
            "--version",
            "Budget",
            "--format",
            format,
        ]);
        assert_eq!(served, printed, "{format}");
    }
    let default = client.ok(
        "render_report",
        json!({"year": "FY2026-27", "version": "Budget"}),
    );
    assert!(default.starts_with("Profit and loss for FY2026-27, Budget\n"));
}

#[test]
fn tool_inputs_are_checked_at_the_boundary() {
    let mut client = Client::shipped();
    let refused = client.refused("list_cubes", json!({"unexpected": "value"}));
    assert!(refused.contains("unknown field `unexpected`"), "{refused}");
    // No tool takes a path: an extra field is refused, not read.
    let refused = client.refused(
        "render_report",
        json!({"year": "FY2026-27", "version": "Budget", "data": "/"}),
    );
    assert!(refused.contains("unknown field `data`"), "{refused}");
    // A name shaped like a path is only ever a name.
    let refused = client.refused("explain_rules", json!({"cube": "../../Cargo.toml"}));
    assert!(
        refused.contains("no cube named '../../Cargo.toml'"),
        "{refused}"
    );
    let refused = client.refused("read_slice", slice("FY2026-27\n", &["EBIT"], &["Jul"]));
    assert!(refused.contains("control character"), "{refused}");
    let long = "x".repeat(257);
    let refused = client.refused("explain_rules", json!({"cube": long}));
    assert!(refused.contains("longer than 256"), "{refused}");
    // 101 periods by 100 accounts is refused before anything is read.
    let periods: Vec<String> = (0..101).map(|i| format!("P{i}")).collect();
    let accounts: Vec<String> = (0..100).map(|i| format!("A{i}")).collect();
    let periods: Vec<&str> = periods.iter().map(String::as_str).collect();
    let accounts: Vec<&str> = accounts.iter().map(String::as_str).collect();
    let refused = client.refused("read_slice", slice("FY2026-27", &accounts, &periods));
    assert!(refused.contains("the slice has 10100 cells"), "{refused}");
    let mut missing = slice("FY2026-27", &["EBIT"], &["Jul"]);
    missing["elements"]
        .as_object_mut()
        .unwrap()
        .remove("Version");
    let refused = client.refused("read_slice", missing);
    assert!(
        refused.contains("give at least one element of dimension Version"),
        "{refused}"
    );
    let mut extra = slice("FY2026-27", &["EBIT"], &["Jul"]);
    extra["elements"]["Colour"] = json!(["Red"]);
    let refused = client.refused("read_slice", extra);
    assert!(refused.contains("no dimension named 'Colour'"), "{refused}");
    let refused = client.refused("read_slice", slice("FY2026-27", &["Ghost"], &["Jul"]));
    assert!(refused.contains("'Ghost'"), "{refused}");
    let cells: Vec<String> = (0..17).map(|_| "PnL:x".to_string()).collect();
    let refused = client.refused("explain_cells", json!({"cells": cells}));
    assert!(
        refused.contains("between 1 and 16 cells, not 17"),
        "{refused}"
    );
    // The server goes on answering after refusals.
    client.ok("list_cubes", json!({}));
}

#[test]
fn a_calculation_failure_is_a_tool_error_and_the_server_carries_on() {
    let tmp = tempdir();
    let root = s(&write_model(
        &tmp.path().join("model"),
        &Spec::rules(DIVIDE_BY_ZERO_RULES),
    ));
    let data = build_data(&tmp.path().join("data"), "sales.csv", "Red,Units,10\n");
    let mut client = Client::start(&root, &data);
    let refused = client.refused(
        "read_slice",
        json!({"cube": "Sales", "elements": {"Colour": ["Red"], "Measure": ["Amount"]}}),
    );
    assert!(refused.contains("division by zero"), "{refused}");
    let cubes = client.ok_json("list_cubes", json!({}));
    assert_eq!(cubes["cubes"][0]["name"], "Sales");
}

#[test]
fn a_model_with_validation_errors_is_not_calculated() {
    let tmp = tempdir();
    let root = s(&write_model(
        &tmp.path().join("model"),
        &Spec::rules(UNKNOWN_ELEMENT_RULES),
    ));
    let data = build_data(&tmp.path().join("data"), "sales.csv", "Red,Units,10\n");
    let mut client = Client::start(&root, &data);
    let refused = client.refused(
        "read_slice",
        json!({"cube": "Sales", "elements": {"Colour": ["Red"], "Measure": ["Units"]}}),
    );
    assert!(refused.contains("'Ghost'"), "{refused}");
    assert!(refused.contains("so nothing was calculated"), "{refused}");
}

#[test]
fn closing_stdin_stops_the_server_cleanly() {
    let mut client = Client::shipped();
    drop(client.stdin.take());
    let status = client.child.wait().unwrap();
    assert!(status.success(), "{status}");
}

fn exit(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(BIN)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

#[test]
fn startup_refuses_bad_arguments_and_directories() {
    let model = s(&model_root());
    let (code, out, _) = exit(&["mcp", "--help"]);
    assert_eq!(code, 0);
    assert!(out.starts_with("usage: pacioliscube mcp [-h] --data DIR [MODEL_DIR]"));
    let (code, _, err) = exit(&["mcp", &model]);
    assert_eq!(code, 1);
    assert!(
        err.contains("the following argument is required: --data"),
        "{err}"
    );
    let (code, _, err) = exit(&["mcp", &model, "--data", &model, "--write"]);
    assert_eq!(code, 1);
    assert!(err.contains("unrecognised argument: --write"), "{err}");
    let tmp = tempdir();
    let missing = s(&tmp.path().join("missing"));
    let (code, _, err) = exit(&["mcp", &model, "--data", &missing]);
    assert_eq!(code, 1);
    assert!(err.contains("there is no data directory there"), "{err}");
    let empty = s(tmp.path());
    let (code, _, _) = exit(&["mcp", &empty, "--data", &empty]);
    assert_eq!(code, 2);
}
