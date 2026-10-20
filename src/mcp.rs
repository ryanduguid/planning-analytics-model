//! `pacioliscube mcp`: the engine as a read-only Model Context Protocol server
//! on standard input and output.
//!
//! The model and data directories are fixed when the server starts. No tool
//! takes a path, so a client can name cubes, elements and cells but cannot
//! point the engine at another file. Each call reads the model files afresh,
//! as a command line run does, and nothing writes a file.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;

use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::TextStream;
use crate::argparse::{Namespace, Value};
use crate::cli::{self, Streams};
use crate::data::load_data;
use crate::error::{EXIT_OK, EXIT_USAGE, Error, Result as EngineResult};
use crate::evaluate;
use crate::model::{Cube, Model, read_text};
use crate::py;
use crate::pypath::PyPath;
use crate::rules::{Area, Feeder};

/// The longest element, cube or dimension name a tool accepts.
pub const MAX_NAME_CHARS: usize = 256;
/// The most cells one `read_slice` call evaluates.
pub const MAX_SLICE_CELLS: usize = 10_000;
/// The most cells one `explain_cells` call explains.
pub const MAX_EXPLAIN_CELLS: usize = 16;

const USAGE: &str = "usage: pacioliscube mcp [-h] --data DIR [MODEL_DIR]";

const HELP: &str = "

Serve the model read-only to a Model Context Protocol client over standard
input and output. The tools list cubes and dimensions, read a cube slice,
explain rules and their feeders, explain cells and render the profit and loss
report. No tool takes a path or writes a file.

positional arguments:
  MODEL_DIR   the directory holding tm1project.json, model by default

options:
  -h, --help  show this help message and exit
  --data DIR  directory of long format CSV input";

const INSTRUCTIONS: &str = "Read-only access to an IBM Planning Analytics model kept as \
source files and evaluated offline, with no TM1 server. Start with list_cubes and \
list_dimensions for names, then read_slice for values, explain_rules for rule text and the \
feeders that reach it, explain_cells for calculation evidence and render_report for the \
profit and loss. Amounts are exact decimal strings. Results describe the offline model, not \
agreement with a native TM1 server.";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DimensionRequest {
    /// One dimension to list with its elements; every dimension when omitted.
    #[serde(default)]
    pub dimension: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SliceRequest {
    /// The cube to read, as list_cubes names it.
    pub cube: String,
    /// Elements to read, keyed by each of the cube's dimension names exactly as
    /// list_cubes gives them, with at least one element per dimension. The slice
    /// is every combination, so its size is the product of the list lengths.
    pub elements: BTreeMap<String, Vec<String>>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RulesRequest {
    /// The cube whose rules to explain.
    pub cube: String,
    /// Only the rule statement starting at this line of the cube's rules file.
    #[serde(default)]
    pub line: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CellsRequest {
    /// Cell references in the form CUBE:element,element,... with one element per
    /// dimension in the cube's order.
    pub cells: Vec<String>,
}

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// The printed statement, each line rounded to whole dollars.
    Text,
    /// Schema profit-and-loss.v1 with exact decimal strings.
    Json,
    /// One row per statement line with exact decimal strings.
    Csv,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReportRequest {
    /// An element of the Year dimension, such as FY2026-27.
    pub year: String,
    /// An element of the Version dimension, such as Budget.
    pub version: String,
    /// The output format; text when omitted.
    #[serde(default)]
    pub format: Option<Format>,
}

/// Refuse oversized or control-character text before it reaches the engine.
fn check_text(field: &str, value: &str, limit: usize) -> Result<(), String> {
    if value.chars().count() > limit {
        return Err(format!("{field} is longer than {limit} characters"));
    }
    if value.chars().any(char::is_control) {
        return Err(format!("{field} holds a control character"));
    }
    Ok(())
}

fn usage(message: String) -> Error {
    Error::cli(EXIT_USAGE, message)
}

/// A command's result as tool text, with what it reported on stderr kept in a failure.
fn finish(result: EngineResult<String>, err: &[u8]) -> Result<String, String> {
    result.map_err(|error| {
        let mut text = String::from_utf8_lossy(err).into_owned();
        text.push_str(&cli::diagnostic(error).0);
        text
    })
}

fn pretty(value: &Json) -> String {
    serde_json::to_string_pretty(value).expect("a JSON value serialises")
}

fn model_relative(model: &Model, path: &PyPath) -> EngineResult<String> {
    Ok(path
        .relative_to_posix(&model.root.resolve()?)
        .unwrap_or_else(|| path.display()))
}

/// The statement starting at `line`: its source lines up to the one whose code ends it.
// shortcut: a `;` or `#` inside a quoted element name ends the excerpt early, because the
// parser keeps no end position. Record each statement's last line in rules.rs if a model
// ever needs those characters.
fn statement(text: &str, line: usize) -> String {
    let mut lines = Vec::new();
    for source in text.lines().skip(line.saturating_sub(1)) {
        lines.push(source.trim_end());
        if source.split('#').next().unwrap_or_default().contains(';') {
            break;
        }
    }
    lines.join("\n")
}

/// Leaf elements an area reaches, per cube dimension position; absent positions are open.
type Reach = HashMap<usize, HashSet<String>>;

fn leaves(model: &Model, dimension: &str, element: &str) -> EngineResult<HashSet<String>> {
    Ok(model
        .hierarchy(dimension)?
        .leaves_under(element)?
        .into_iter()
        .map(str::to_string)
        .collect())
}

fn area_reach(model: &Model, cube: &Cube, area: &Area) -> EngineResult<Reach> {
    let mut reach = Reach::new();
    for element in &area.elements {
        let index = evaluate::dimension_of(model, cube, element)?;
        reach.insert(index, leaves(model, &cube.dimensions[index], element)?);
    }
    Ok(reach)
}

/// The cells of `target` a feeder in `source` points at.
fn feeder_reach(
    model: &Model,
    source: &Cube,
    target: &Cube,
    feeder: &Feeder,
) -> EngineResult<Reach> {
    if feeder.target_cube.is_none() {
        return area_reach(model, source, &feeder.target);
    }
    let mut reach = Reach::new();
    for (index, element) in feeder.target.elements.iter().enumerate() {
        // `!Dimension` follows the fed cell, so it leaves the position open.
        if element.starts_with('!') || index >= target.dimensions.len() {
            continue;
        }
        reach.insert(index, leaves(model, &target.dimensions[index], element)?);
    }
    Ok(reach)
}

/// Whether two areas can share a cell: every position both constrain shares a leaf.
fn overlaps(a: &Reach, b: &Reach) -> bool {
    a.iter()
        .all(|(index, leaves)| b.get(index).is_none_or(|other| !leaves.is_disjoint(other)))
}

fn rules_text(cube: &Cube) -> EngineResult<String> {
    match &cube.rules_source {
        Some(source) => read_text(source),
        None => Ok(String::new()),
    }
}

#[derive(Clone)]
pub struct Server {
    model_dir: String,
    data_dir: String,
    tool_router: ToolRouter<Self>,
}

impl Server {
    fn model(&self) -> EngineResult<Model> {
        cli::load(&cli::directory(&self.model_dir, "model")?)
    }

    /// Run `work` on the loaded model, refusing a model with validation errors when `sound`.
    fn with_model(
        &self,
        sound: bool,
        work: impl FnOnce(&Model) -> EngineResult<Json>,
    ) -> Result<String, String> {
        let mut err = Vec::new();
        let result = self.model().and_then(|model| {
            if sound {
                cli::refuse_broken_model(
                    &model,
                    &mut Streams {
                        out: &mut std::io::sink(),
                        err: &mut err,
                    },
                )?;
            }
            work(&model)
        });
        finish(result.map(|value| pretty(&value)), &err)
    }

    /// Run a command line subcommand on the server's directories and return its output.
    fn command(
        &self,
        run: fn(&Namespace, &mut Streams) -> EngineResult<i32>,
        mut args: Namespace,
    ) -> Result<String, String> {
        args.values
            .insert("model_dir", Value::Str(self.model_dir.clone()));
        args.values
            .insert("data", Value::Str(self.data_dir.clone()));
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let result = run(
            &args,
            &mut Streams {
                out: &mut out,
                err: &mut err,
            },
        );
        finish(
            result.map(|_| String::from_utf8_lossy(&out).into_owned()),
            &err,
        )
    }
}

#[tool_router]
impl Server {
    pub fn new(model_dir: String, data_dir: String) -> Server {
        Server {
            model_dir,
            data_dir,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "List the model's cubes: each cube's dimensions in coordinate order, its \
                       rules file, whether it declares SKIPCHECK and its rule and feeder counts.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    fn list_cubes(&self) -> Result<String, String> {
        self.with_model(false, |model| {
            let mut cubes = Vec::new();
            for cube in &model.cubes {
                let file = match &cube.rules_source {
                    Some(source) => Json::String(model_relative(model, source)?),
                    None => Json::Null,
                };
                let rules = cube.rules.as_ref();
                cubes.push(json!({
                    "name": cube.name,
                    "dimensions": cube.dimensions,
                    "rules_file": file,
                    "skipcheck": rules.is_some_and(|r| r.skipcheck),
                    "rules": rules.map_or(0, |r| r.rules.len()),
                    "feeders": rules.map_or(0, |r| r.feeders.len()),
                }));
            }
            Ok(json!({ "cubes": cubes }))
        })
    }

    #[tool(
        description = "List dimensions with their elements: each element's type and its \
                       children with consolidation weights. Name one dimension to list only it.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    fn list_dimensions(
        &self,
        Parameters(request): Parameters<DimensionRequest>,
    ) -> Result<String, String> {
        if let Some(name) = &request.dimension {
            check_text("dimension", name, MAX_NAME_CHARS)?;
        }
        self.with_model(false, |model| {
            let chosen: Vec<_> = match &request.dimension {
                None => model.dimensions.iter().collect(),
                Some(name) => {
                    let dimension = model.dimension(name).ok_or_else(|| {
                        let names: Vec<&str> =
                            model.dimensions.iter().map(|d| d.name.as_str()).collect();
                        usage(format!(
                            "no dimension named {}. The model holds {}",
                            py::repr_str(name),
                            names.join(", ")
                        ))
                    })?;
                    vec![dimension]
                }
            };
            let mut dimensions = Vec::new();
            for dimension in chosen {
                let hierarchy = dimension.default_hierarchy()?;
                let mut elements = Vec::new();
                for element in &hierarchy.elements {
                    let mut children = Vec::new();
                    for edge in hierarchy.children(&element.name)? {
                        children.push(json!({
                            "element": edge.component,
                            "weight": cli::plain(&edge.weight)?,
                        }));
                    }
                    elements.push(json!({
                        "name": element.name,
                        "type": element.element_type,
                        "children": children,
                    }));
                }
                dimensions.push(json!({
                    "name": dimension.name,
                    "hierarchy": hierarchy.name,
                    "elements": elements,
                }));
            }
            Ok(json!({ "dimensions": dimensions }))
        })
    }

    #[tool(
        description = "Read the values of a cube slice: every combination of the elements given \
                       per dimension, leaf or consolidated, as exact decimal strings. Validates \
                       the model and evaluates its rules over the server's data first.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    fn read_slice(&self, Parameters(request): Parameters<SliceRequest>) -> Result<String, String> {
        check_text("cube", &request.cube, MAX_NAME_CHARS)?;
        let mut count: usize = 1;
        for (dimension, names) in &request.elements {
            check_text("dimension", dimension, MAX_NAME_CHARS)?;
            for name in names {
                check_text("element", name, MAX_NAME_CHARS)?;
            }
            count = count.saturating_mul(names.len());
        }
        if count > MAX_SLICE_CELLS {
            return Err(format!(
                "the slice has {count} cells; read at most {MAX_SLICE_CELLS} per call"
            ));
        }
        self.with_model(true, |model| {
            let cube = cli::cube(model, &request.cube)?;
            if let Some(extra) = request
                .elements
                .keys()
                .find(|key| !cube.dimensions.contains(key))
            {
                return Err(usage(format!(
                    "cube {} has no dimension named {}; its dimensions are {}",
                    cube.name,
                    py::repr_str(extra),
                    cube.dimensions.join(", ")
                )));
            }
            let mut axes = Vec::new();
            for dimension in &cube.dimensions {
                let names = request
                    .elements
                    .get(dimension)
                    .filter(|names| !names.is_empty())
                    .ok_or_else(|| {
                        usage(format!(
                            "give at least one element of dimension {dimension}"
                        ))
                    })?;
                let mut resolved = Vec::new();
                for name in names {
                    resolved.push(cli::element_or_error(model, dimension, name, EXIT_USAGE)?);
                }
                axes.push(resolved);
            }
            let data = cli::directory(&self.data_dir, "data")?;
            let store = cli::evaluated(model, &load_data(model, &data)?)?;
            let mut cells = Vec::with_capacity(count);
            let mut position = vec![0; axes.len()];
            'cells: loop {
                let coordinate = axes
                    .iter()
                    .zip(&position)
                    .map(|(axis, &i)| axis[i].clone())
                    .collect();
                cells.push((cube.name.clone(), coordinate));
                for axis in (0..axes.len()).rev() {
                    position[axis] += 1;
                    if position[axis] < axes[axis].len() {
                        continue 'cells;
                    }
                    position[axis] = 0;
                }
                break;
            }
            let mut values = Vec::with_capacity(cells.len());
            evaluate::consolidate_many(model, &store, &cells, |_, value| values.push(value))
                .map_err(cli::calculation)?;
            let mut rows = Vec::new();
            for ((_, coordinate), value) in cells.iter().zip(&values) {
                rows.push(json!({ "coordinate": coordinate, "value": cli::plain(value)? }));
            }
            Ok(json!({
                "cube": cube.name,
                "dimensions": cube.dimensions,
                "cells": rows,
            }))
        })
    }

    #[tool(
        description = "Explain a cube's rules: each statement's text, file and line, the area it \
                       calculates and the feeders, in any cube, whose target can reach that area. \
                       Feeder reach is read statically from the rules files, expanding \
                       consolidated elements to their leaves. Give a line to explain one rule.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    fn explain_rules(
        &self,
        Parameters(request): Parameters<RulesRequest>,
    ) -> Result<String, String> {
        check_text("cube", &request.cube, MAX_NAME_CHARS)?;
        self.with_model(false, |model| {
            let cube = cli::cube(model, &request.cube)?;
            let Some(rule_set) = &cube.rules else {
                return Ok(json!({
                    "cube": cube.name,
                    "rules_file": null,
                    "skipcheck": false,
                    "rules": [],
                }));
            };
            let file = match &cube.rules_source {
                Some(source) => Json::String(model_relative(model, source)?),
                None => Json::Null,
            };
            let chosen: Vec<_> = rule_set
                .rules
                .iter()
                .filter(|rule| request.line.is_none_or(|line| rule.source_line == line))
                .collect();
            if let (Some(line), true) = (request.line, chosen.is_empty()) {
                let starts: Vec<String> = rule_set
                    .rules
                    .iter()
                    .map(|rule| rule.source_line.to_string())
                    .collect();
                return Err(usage(format!(
                    "no rule of cube {} starts at line {line}; rules start at lines {}",
                    cube.name,
                    starts.join(", ")
                )));
            }
            // Every feeder in the model that points into this cube, with where it points.
            let mut feeders = Vec::new();
            for source in &model.cubes {
                let Some(source_rules) = &source.rules else {
                    continue;
                };
                let mut text = None;
                for feeder in &source_rules.feeders {
                    let target = feeder.target_cube.as_deref().unwrap_or(&source.name);
                    if target != cube.name {
                        continue;
                    }
                    let text = match &mut text {
                        Some(text) => text,
                        None => text.insert(rules_text(source)?),
                    };
                    let file = match &source.rules_source {
                        Some(path) => Json::String(model_relative(model, path)?),
                        None => Json::Null,
                    };
                    let reach = feeder_reach(model, source, cube, feeder)?;
                    let described = json!({
                        "cube": source.name,
                        "file": file,
                        "line": feeder.source_line,
                        "statement": statement(text, feeder.source_line),
                    });
                    feeders.push((reach, described));
                }
            }
            let text = rules_text(cube)?;
            let mut rules = Vec::new();
            for rule in chosen {
                // Consolidated cells need no feeders, so a C: rule has none listed.
                let fed_by: Vec<&Json> = if rule.area.qualifier == "C" {
                    Vec::new()
                } else {
                    let reach = area_reach(model, cube, &rule.area)?;
                    feeders
                        .iter()
                        .filter(|(target, _)| overlaps(&reach, target))
                        .map(|(_, described)| described)
                        .collect()
                };
                rules.push(json!({
                    "line": rule.source_line,
                    "qualifier": rule.area.qualifier,
                    "area": rule.area.elements,
                    "statement": statement(&text, rule.source_line),
                    "fed_by": fed_by,
                }));
            }
            Ok(json!({
                "cube": cube.name,
                "rules_file": file,
                "skipcheck": rule_set.skipcheck,
                "rules": rules,
            }))
        })
    }

    #[tool(
        description = "Explain how cells are calculated: JSON evidence for each requested cell and \
                       its dependencies, recording inputs, rule file and line, the arithmetic and \
                       comparisons evaluated and weighted consolidation contributions. The same \
                       document as `pacioliscube explain`.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    fn explain_cells(
        &self,
        Parameters(request): Parameters<CellsRequest>,
    ) -> Result<String, String> {
        if request.cells.is_empty() || request.cells.len() > MAX_EXPLAIN_CELLS {
            return Err(format!(
                "give between 1 and {MAX_EXPLAIN_CELLS} cells, not {}",
                request.cells.len()
            ));
        }
        for cell in &request.cells {
            check_text("cell", cell, MAX_NAME_CHARS * 16)?;
        }
        let mut args = Namespace::default();
        args.values.insert("cell", Value::List(request.cells));
        self.command(cli::explain_command, args)
    }

    #[tool(
        description = "Render the group profit and loss for one year and version at the FY \
                       period and every cost centre, as `pacioliscube report` prints it: rounded \
                       text by default, or JSON (schema profit-and-loss.v1) or CSV with exact \
                       decimal strings.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    fn render_report(
        &self,
        Parameters(request): Parameters<ReportRequest>,
    ) -> Result<String, String> {
        check_text("year", &request.year, MAX_NAME_CHARS)?;
        check_text("version", &request.version, MAX_NAME_CHARS)?;
        let format = match request.format.unwrap_or(Format::Text) {
            Format::Text => "text",
            Format::Json => "json",
            Format::Csv => "csv",
        };
        let mut args = Namespace::default();
        args.values.insert("year", Value::Str(request.year));
        args.values.insert("version", Value::Str(request.version));
        args.values.insert("format", Value::Str(format.into()));
        self.command(cli::report_command, args)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("pacioliscube", cli::VERSION))
            .with_instructions(INSTRUCTIONS)
    }
}

/// What the `mcp` arguments asked for: help, or a server on two directories.
enum Parsed {
    Help,
    Serve { model_dir: String, data_dir: String },
}

fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut model_dir = None;
    let mut data_dir = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Parsed::Help),
            "--data" => match rest.next() {
                Some(value) => data_dir = Some(value.clone()),
                None => return Err("argument --data: expected one argument".into()),
            },
            _ if arg.starts_with("--data=") => data_dir = Some(arg["--data=".len()..].into()),
            _ if arg.starts_with('-') && arg.len() > 1 => {
                return Err(format!("unrecognised argument: {arg}"));
            }
            _ if model_dir.is_none() => model_dir = Some(arg.clone()),
            _ => return Err(format!("unrecognised argument: {arg}")),
        }
    }
    let data_dir = data_dir.ok_or("the following argument is required: --data")?;
    let model_dir = model_dir
        .filter(|dir| !dir.is_empty())
        .unwrap_or_else(|| cli::DEFAULT_MODEL_ROOT.into());
    Ok(Parsed::Serve {
        model_dir,
        data_dir,
    })
}

/// Run `pacioliscube mcp` with the arguments after `mcp`, returning the exit code.
pub fn main(args: &[String]) -> i32 {
    let mut err = TextStream::new(std::io::stderr());
    let (model_dir, data_dir) = match parse(args) {
        Ok(Parsed::Help) => {
            let mut out = TextStream::new(std::io::stdout());
            let _ = writeln!(out, "{USAGE}{HELP}");
            return EXIT_OK;
        }
        Ok(Parsed::Serve {
            model_dir,
            data_dir,
        }) => (model_dir, data_dir),
        Err(message) => {
            let _ = writeln!(err, "{USAGE}\npacioliscube mcp: error: {message}");
            return EXIT_USAGE;
        }
    };
    // Refuse missing directories and a model that does not load before serving.
    let ready = cli::directory(&model_dir, "model")
        .and_then(|root| cli::load(&root))
        .and_then(|_| cli::directory(&data_dir, "data"));
    if let Err(error) = ready {
        let (message, code) = cli::diagnostic(error);
        let _ = writeln!(err, "{message}");
        return code;
    }
    let server = Server::new(model_dir, data_dir);
    // One thread, the caller's: the engine keeps the deep stack main gives it.
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = writeln!(err, "pacioliscube mcp: {error}");
            return EXIT_USAGE;
        }
    };
    let served = runtime.block_on(async {
        let service = server.serve(rmcp::transport::stdio()).await?;
        service.waiting().await?;
        Ok::<(), Box<dyn std::error::Error>>(())
    });
    match served {
        Ok(()) => EXIT_OK,
        Err(error) => {
            let _ = writeln!(err, "pacioliscube mcp: {error}");
            EXIT_USAGE
        }
    }
}
