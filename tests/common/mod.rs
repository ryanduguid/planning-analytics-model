//! The repository paths, the model writer and the command runner the test
//! files share, as conftest.py was for the Python suite.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use pacioliscube::cli::{self, Streams};
use pacioliscube::decimal::{Decimal, Rounding};
use pacioliscube::terminal::Terminal;

pub fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn model_root() -> PathBuf {
    repo().join("model")
}

pub fn examples() -> PathBuf {
    repo().join("examples")
}

pub fn mini() -> PathBuf {
    repo().join("tests").join("fixtures").join("mini")
}

pub fn s(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Red and Blue roll up to Total, which is what a test needs to tell a leaf
/// cell from a consolidated one.
pub const COLOUR_ELEMENTS: &str = r#"{"Name": "Total", "Type": "Consolidated"}, {"Name": "Red", "Type": "Numeric"}, {"Name": "Blue", "Type": "Numeric"}"#;
pub const COLOUR_EDGES: &str = r#"{"ParentName": "Total", "ComponentName": "Red", "Weight": 1}, {"ParentName": "Total", "ComponentName": "Blue", "Weight": 1}"#;
pub const MEASURES: &str = r#"{"Name": "Units", "Type": "Numeric"}, {"Name": "Price", "Type": "Numeric"}, {"Name": "Amount", "Type": "Numeric"}"#;
pub const CUBE_DIMENSIONS: &str = r#""../dimensions/Colour.json", "../dimensions/Measure.json""#;

/// What `write_model` writes; every field defaults as conftest.py's did.
pub struct Spec {
    pub rules: String,
    pub measures: String,
    pub cube_dimensions: String,
    pub extra_files: Vec<(String, String)>,
    pub manifest_cubes: String,
    pub processes: String,
    pub parameters: String,
}

impl Default for Spec {
    fn default() -> Spec {
        Spec {
            rules: String::new(),
            measures: MEASURES.into(),
            cube_dimensions: CUBE_DIMENSIONS.into(),
            extra_files: Vec::new(),
            manifest_cubes: r#""cubes/Sales.json""#.into(),
            processes: String::new(),
            parameters: "[]".into(),
        }
    }
}

impl Spec {
    pub fn rules(rules: &str) -> Spec {
        Spec {
            rules: rules.into(),
            ..Spec::default()
        }
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Write the smallest model that can carry the defect under test.
pub fn write_model(root: &Path, spec: &Spec) -> PathBuf {
    for (name, elements, edges) in [
        ("Colour", COLOUR_ELEMENTS, COLOUR_EDGES),
        ("Measure", spec.measures.as_str(), ""),
    ] {
        write(
            &root.join("dimensions").join(format!("{name}.json")),
            &format!(
                r#"{{"Name": "{name}", "Hierarchies@Code.links": ["{name}.hierarchies/{name}.json"]}}"#
            ),
        );
        write(
            &root
                .join("dimensions")
                .join(format!("{name}.hierarchies"))
                .join(format!("{name}.json")),
            &format!(r#"{{"Name": "{name}", "Elements": [{elements}], "Edges": [{edges}]}}"#),
        );
    }
    let rules_link = if spec.rules.is_empty() {
        ""
    } else {
        r#", "Rules@Code.link": "Sales.rules""#
    };
    write(
        &root.join("cubes").join("Sales.json"),
        &format!(
            r#"{{"Name": "Sales", "Dimensions@Code.links": [{}]{rules_link}}}"#,
            spec.cube_dimensions
        ),
    );
    if !spec.rules.is_empty() {
        write(&root.join("cubes").join("Sales.rules"), &spec.rules);
    }
    let mut process_entry = String::new();
    if !spec.processes.is_empty() {
        write(
            &root.join("processes").join("Load.json"),
            &format!(
                r#"{{"Name": "Load", "Parameters": {}, "DataSource": {{}}, "Code@Code.link": "Load.ti"}}"#,
                spec.parameters
            ),
        );
        write(&root.join("processes").join("Load.ti"), &spec.processes);
        process_entry = r#", "Processes": ["processes/Load.json"]"#.into();
    }
    write(
        &root.join("tm1project.json"),
        &format!(
            r#"{{"Version": 1.0, "Name": "built", "Objects": {{"Dimensions": ["dimensions/Colour.json", "dimensions/Measure.json"], "Cubes": [{}]{process_entry}}}}}"#,
            spec.manifest_cubes
        ),
    );
    for (name, content) in &spec.extra_files {
        write(&root.join(name), content);
    }
    root.to_path_buf()
}

/// Write a data directory beside a model, never inside it.
pub fn build_data(root: &Path, name: &str, body: &str) -> String {
    write(&root.join(name), &format!("Colour,Measure,Value\n{body}"));
    s(root)
}

/// Run on a thread with the binary's stack, as `main` does.
pub fn big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

/// Run the command line, keeping its exit code and what it printed.
pub fn run(args: &[&str]) -> (i32, String, String) {
    let argv: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    big_stack(move || {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = cli::main(
            &argv,
            &Terminal::plain(),
            &mut Streams {
                out: &mut out,
                err: &mut err,
            },
        );
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    })
}

pub fn d(text: &str) -> Decimal {
    Decimal::parse(text).unwrap_or_else(|_| panic!("{text} is a number"))
}

/// Decimal arithmetic for expected values written in a test body.
pub trait Arith {
    fn add_(&self, other: &Decimal) -> Decimal;
    fn sub_(&self, other: &Decimal) -> Decimal;
    fn mul_(&self, other: &Decimal) -> Decimal;
    fn div_(&self, other: &Decimal) -> Decimal;
    fn cents(&self) -> Decimal;
    fn round2(&self) -> Decimal;
}

impl Arith for Decimal {
    fn add_(&self, other: &Decimal) -> Decimal {
        self.add(other).unwrap()
    }
    fn sub_(&self, other: &Decimal) -> Decimal {
        self.sub(other).unwrap()
    }
    fn mul_(&self, other: &Decimal) -> Decimal {
        self.mul(other).unwrap()
    }
    fn div_(&self, other: &Decimal) -> Decimal {
        self.div(other).unwrap()
    }
    /// `quantize(Decimal("0.01"), rounding=ROUND_HALF_UP)`.
    fn cents(&self) -> Decimal {
        self.quantize(-2, Rounding::HalfUp).unwrap()
    }
    /// `round(value, 2)`, which rounds half to even.
    fn round2(&self) -> Decimal {
        self.quantize(-2, Rounding::HalfEven).unwrap()
    }
}

pub fn coord(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

/// `abs(a - b) <= tolerance`.
pub fn close(a: &Decimal, b: &Decimal, tolerance: &str) -> bool {
    let difference = a.sub_(b);
    let limit = d(tolerance);
    difference <= limit && difference >= limit.neg().unwrap()
}
