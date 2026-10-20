//! `pacioliscube mcp`: the server driven over its standard input and output as
//! a client would, with each figure checked against the command line's own.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

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
