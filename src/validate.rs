//! Structural checks over a loaded model.
//!
//! Loading a model already refuses anything malformed. Validation is the second
//! pass: it looks for source that parses cleanly but cannot be right, such as a
//! rule naming an element no dimension of its cube holds, a cube with SKIPCHECK and
//! rules but no feeders, or a file on disk that the manifest never lists and which
//! would therefore be missing from a deployment.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::error::{Error, Result, Uncaught};
use crate::model::{Cube, Model};
use crate::py;
use crate::pyjson::Json;
use crate::pypath::{self, PyPath};
use crate::rules::{Area, CellRef, Expr};

pub const ERROR: &str = "error";
pub const WARNING: &str = "warning";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub severity: &'static str,
    pub code: &'static str,
    pub message: String,
    pub location: String,
}

fn finding(severity: &'static str, code: &'static str, message: String, location: &str) -> Finding {
    Finding {
        severity,
        code,
        message,
        location: location.to_string(),
    }
}

/// Every cell reference in an expression, in the preorder `_walk` yielded nodes.
fn cell_refs(expression: &Expr) -> Vec<&CellRef> {
    let mut found = Vec::new();
    let mut stack = vec![expression];
    while let Some(node) = stack.pop() {
        match node {
            Expr::Number(_) => {}
            Expr::CellRef(reference) => found.push(reference),
            Expr::BinaryOp(_, left, right) => {
                stack.push(right);
                stack.push(left);
            }
            Expr::If(condition, then_expr, else_expr) => {
                stack.push(else_expr);
                stack.push(then_expr);
                stack.push(&condition.right);
                stack.push(&condition.left);
            }
        }
    }
    found
}

fn holds(model: &Model, dimension: &str, element: &str) -> Result<bool> {
    Ok(model.has_dimension(dimension) && model.hierarchy(dimension)?.has(element))
}

fn check_elements_resolve(
    model: &Model,
    cube: &Cube,
    names: &[String],
    location: &str,
) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for element in names {
        let mut holders = Vec::new();
        for dimension in &cube.dimensions {
            if holds(model, dimension, element)? {
                holders.push(dimension.as_str());
            }
        }
        if holders.is_empty() {
            findings.push(finding(
                ERROR,
                "ELE001",
                format!(
                    "no dimension of cube {} holds an element named {}",
                    py::repr_str(&cube.name),
                    py::repr_str(element)
                ),
                location,
            ));
        } else if holders.len() > 1 {
            findings.push(finding(
                ERROR,
                "ARE001",
                format!(
                    "element {} is ambiguous in cube {}, held by {}",
                    py::repr_str(element),
                    py::repr_str(&cube.name),
                    holders.join(", ")
                ),
                location,
            ));
        }
    }
    Ok(findings)
}

fn target_is_consolidated(model: &Model, cube: &Cube, area: &Area) -> Result<bool> {
    for element in &area.elements {
        for dimension in &cube.dimensions {
            if holds(model, dimension, element)?
                && model.hierarchy(dimension)?.is_consolidated(element)?
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn validate_cube_dimensions(model: &Model, cube: &Cube) -> Vec<Finding> {
    let location = cube.source.display();
    cube.dimensions
        .iter()
        .filter(|d| !model.has_dimension(d))
        .map(|dimension| {
            finding(
                ERROR,
                "DIM001",
                format!(
                    "cube {} names dimension {}, which the model does not hold",
                    py::repr_str(&cube.name),
                    py::repr_str(dimension)
                ),
                &location,
            )
        })
        .collect()
}

fn rules_location(cube: &Cube) -> String {
    cube.rules_source
        .as_ref()
        .map_or_else(|| "None".to_string(), PyPath::display)
}

fn validate_cube_rules(model: &Model, cube: &Cube) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    let location = rules_location(cube);
    let rules = cube.rules.as_ref().expect("a ruled cube");
    for rule in &rules.rules {
        let place = format!("{location} line {}", rule.source_line);
        findings.extend(check_elements_resolve(
            model,
            cube,
            &rule.area.elements,
            &place,
        )?);
        if rule.area.qualifier != "C" && target_is_consolidated(model, cube, &rule.area)? {
            findings.push(finding(
                ERROR,
                "RUL001",
                "a rule targets a consolidated element without a C qualifier, so it would \
                 replace the consolidation of its children"
                    .into(),
                &place,
            ));
        }
        for node in cell_refs(&rule.expression) {
            let Some(target_name) = &node.cube else {
                findings.extend(check_elements_resolve(
                    model,
                    cube,
                    &node.coordinates,
                    &place,
                )?);
                continue;
            };
            let Some(target) = model.cube(target_name) else {
                findings.push(finding(
                    ERROR,
                    "DIM001",
                    format!(
                        "a rule reads cube {}, which the model does not hold",
                        py::repr_str(target_name)
                    ),
                    &place,
                ));
                continue;
            };
            if node.coordinates.len() != target.dimensions.len() {
                findings.push(finding(
                    ERROR,
                    "ARE001",
                    format!(
                        "DB('{target_name}', ...) passes {} coordinates for a cube of {} dimensions",
                        node.coordinates.len(),
                        target.dimensions.len()
                    ),
                    &place,
                ));
                continue;
            }
            for (dimension, argument) in target.dimensions.iter().zip(&node.coordinates) {
                if let Some(source) = argument.strip_prefix('!') {
                    if !cube.dimensions.iter().any(|d| d == source) {
                        findings.push(finding(
                            ERROR,
                            "ELE001",
                            format!(
                                "{argument} names a dimension that cube {} does not have",
                                py::repr_str(&cube.name)
                            ),
                            &place,
                        ));
                    }
                    continue;
                }
                if model.has_dimension(dimension) && !model.hierarchy(dimension)?.has(argument) {
                    findings.push(finding(
                        ERROR,
                        "ELE001",
                        format!(
                            "dimension {} of cube {} has no element named {}",
                            py::repr_str(dimension),
                            py::repr_str(target_name),
                            py::repr_str(argument)
                        ),
                        &place,
                    ));
                }
            }
        }
    }
    Ok(findings)
}

fn validate_cube_feeders(model: &Model, cube: &Cube) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    let location = rules_location(cube);
    let rules = cube.rules.as_ref().expect("a ruled cube");
    for feeder in &rules.feeders {
        let place = format!("{location} line {}", feeder.source_line);
        findings.extend(check_elements_resolve(
            model,
            cube,
            &feeder.area.elements,
            &place,
        )?);
        let Some(target_name) = &feeder.target_cube else {
            findings.extend(check_elements_resolve(
                model,
                cube,
                &feeder.target.elements,
                &place,
            )?);
            continue;
        };
        let Some(target) = model.cube(target_name) else {
            findings.push(finding(
                ERROR,
                "DIM001",
                format!(
                    "a feeder points at cube {}, which the model does not hold",
                    py::repr_str(target_name)
                ),
                &place,
            ));
            continue;
        };
        let coordinates = &feeder.target.elements;
        if coordinates.len() != target.dimensions.len() {
            findings.push(finding(
                ERROR,
                "ARE001",
                format!(
                    "DB('{target_name}', ...) passes {} coordinates for a cube of {} dimensions",
                    coordinates.len(),
                    target.dimensions.len()
                ),
                &place,
            ));
            continue;
        }
        // Paired with target.dimensions, because a coordinate names the element of the
        // dimension at its own position.
        for (dimension, element) in target.dimensions.iter().zip(coordinates) {
            if let Some(source) = element.strip_prefix('!') {
                if !cube.dimensions.iter().any(|d| d == source) {
                    findings.push(finding(
                        ERROR,
                        "ELE001",
                        format!(
                            "{element} names a dimension that cube {} does not have",
                            py::repr_str(&cube.name)
                        ),
                        &place,
                    ));
                }
                continue;
            }
            if !holds(model, dimension, element)? {
                findings.push(finding(
                    ERROR,
                    "ELE001",
                    format!(
                        "dimension {} of cube {} holds no element named {}",
                        py::repr_str(dimension),
                        py::repr_str(&target.name),
                        py::repr_str(element)
                    ),
                    &place,
                ));
            }
        }
    }
    Ok(findings)
}

fn validate_cube(model: &Model, cube: &Cube) -> Result<Vec<Finding>> {
    let mut findings = validate_cube_dimensions(model, cube);
    if cube.rules.is_none() {
        return Ok(findings);
    }
    findings.extend(validate_cube_rules(model, cube)?);
    findings.extend(validate_cube_feeders(model, cube)?);
    Ok(findings)
}

/// Which elements each cube's cells are fed at, from every cube's feeders.
fn fed_elements_by_cube(model: &Model) -> HashMap<String, HashSet<String>> {
    let mut fed: HashMap<String, HashSet<String>> = HashMap::new();
    for cube in &model.cubes {
        let Some(rules) = &cube.rules else { continue };
        for feeder in &rules.feeders {
            let target_name = match &feeder.target_cube {
                Some(name) if !name.is_empty() => name,
                _ => &cube.name,
            };
            let bucket = fed.entry(py::casefold(target_name)).or_default();
            for element in &feeder.target.elements {
                if !element.starts_with('!') {
                    bucket.insert(py::casefold(element));
                }
            }
        }
    }
    fed
}

/// Element sets keyed by dimension, in the order a Python dict filled them.
type Constraints = Vec<(String, HashSet<String>)>;

fn entry<'a>(out: &'a mut Constraints, dimension: &str) -> &'a mut HashSet<String> {
    let at = match out.iter().position(|(d, _)| d == dimension) {
        Some(at) => at,
        None => {
            out.push((dimension.to_string(), HashSet::new()));
            out.len() - 1
        }
    };
    &mut out[at].1
}

fn lookup<'a>(constraints: &'a Constraints, dimension: &str) -> Option<&'a HashSet<String>> {
    constraints
        .iter()
        .find(|(d, _)| d == dimension)
        .map(|(_, s)| s)
}

/// An area as the elements it names per dimension of `cube`.
fn constraints(model: &Model, cube: &Cube, area: &Area) -> Result<Constraints> {
    let mut out = Constraints::new();
    for element in &area.elements {
        for dimension in &cube.dimensions {
            if holds(model, dimension, element)? {
                entry(&mut out, dimension).insert(py::casefold(element));
                break;
            }
        }
    }
    Ok(out)
}

/// A DB() target, whose coordinate at each position names that dimension's element.
fn positional_constraints(
    model: &Model,
    cube: &Cube,
    coordinates: &[String],
    source: &Constraints,
) -> Result<Constraints> {
    let mut out = Constraints::new();
    for (dimension, element) in cube.dimensions.iter().zip(coordinates) {
        if let Some(name) = element.strip_prefix('!') {
            if let Some(chosen_set) = lookup(source, name) {
                let mut selected = chosen_set.clone();
                for chosen in chosen_set {
                    for leaf in model.hierarchy(name)?.leaves_under(chosen)? {
                        selected.insert(py::casefold(leaf));
                    }
                }
                *entry(&mut out, dimension) = selected;
            }
        } else if holds(model, dimension, element)? {
            *entry(&mut out, dimension) = HashSet::from([py::casefold(element)]);
        }
    }
    Ok(out)
}

/// False when some dimension both areas constrain has no element in common.
fn may_intersect(rule: &Constraints, feeder: &Constraints) -> bool {
    rule.iter().all(|(dimension, elements)| {
        lookup(feeder, dimension).is_none_or(|other| !elements.is_disjoint(other))
    })
}

fn fed_areas_by_cube(model: &Model) -> Result<HashMap<String, Vec<Constraints>>> {
    let by_name: HashMap<String, &Cube> = model
        .cubes
        .iter()
        .map(|c| (py::casefold(&c.name), c))
        .collect();
    let mut fed: HashMap<String, Vec<Constraints>> = HashMap::new();
    for cube in &model.cubes {
        let Some(rules) = &cube.rules else { continue };
        for feeder in &rules.feeders {
            let named = feeder.target_cube.as_deref().filter(|n| !n.is_empty());
            let target_name = named.unwrap_or(&cube.name);
            let Some(target) = by_name.get(&py::casefold(target_name)) else {
                continue;
            };
            let coordinates = &feeder.target.elements;
            let positional = named.is_some() && coordinates.len() == target.dimensions.len();
            let area = if positional {
                let source = constraints(model, cube, &feeder.area)?;
                positional_constraints(model, target, coordinates, &source)?
            } else {
                constraints(model, target, &feeder.target)?
            };
            fed.entry(py::casefold(target_name)).or_default().push(area);
        }
    }
    Ok(fed)
}

/// FED001 to FED003: is every calculated area fed, and will TM1 read the feeders.
fn validate_feeding(model: &Model) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    let fed = fed_elements_by_cube(model);
    let fed_areas_by_cube = fed_areas_by_cube(model)?;
    let none = HashSet::new();
    for cube in &model.cubes {
        let Some(rules) = &cube.rules else { continue };
        let location = rules_location(cube);
        if !rules.feeders.is_empty() && !rules.skipcheck {
            findings.push(finding(
                ERROR,
                "FED003",
                format!(
                    "cube {} has feeders but no SKIPCHECK, and TM1 ignores every feeder in a \
                     rules file without it",
                    py::repr_str(&cube.name)
                ),
                &location,
            ));
        }
        let key = py::casefold(&cube.name);
        let fed_here = fed.get(&key).unwrap_or(&none);
        if rules.skipcheck && !rules.rules.is_empty() && fed_here.is_empty() {
            findings.push(finding(
                ERROR,
                "FED001",
                format!(
                    "cube {} uses SKIPCHECK and has rules, but no feeder in any cube points \
                     into it, so calculated cells would not appear in a real database",
                    py::repr_str(&cube.name)
                ),
                &location,
            ));
            continue;
        }
        let fed_areas = fed_areas_by_cube.get(&key).map_or(&[][..], Vec::as_slice);
        for rule in &rules.rules {
            let rule_area = constraints(model, cube, &rule.area)?;
            let names: HashSet<String> = rule
                .area
                .elements
                .iter()
                .filter(|e| !e.starts_with('!'))
                .map(|e| py::casefold(e))
                .collect();
            let covered = if rule_area.is_empty() {
                !names.is_disjoint(fed_here)
            } else {
                fed_areas.iter().any(|area| may_intersect(&rule_area, area))
            };
            if !names.is_empty() && !covered {
                findings.push(finding(
                    WARNING,
                    "FED002",
                    "no feeder, in this cube or any other, points at the area this rule \
                     calculates. A warning rather than an error, because a model may feed an \
                     area indirectly"
                        .into(),
                    &format!("{location} line {}", rule.source_line),
                ));
            }
        }
    }
    Ok(findings)
}

/// `root.rglob("*")`: every entry below the root, not following directory links.
fn rglob(root: &PyPath) -> Vec<PyPath> {
    let mut found = Vec::new();
    let mut pending = vec![root.clone()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(directory.fs()) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = directory.join(&name);
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            if is_dir {
                pending.push(path.clone());
            }
            found.push(path);
        }
    }
    found
}

fn validate_manifest(model: &Model) -> Result<Vec<Finding>> {
    let mut listed = HashSet::new();
    for path in &model.files {
        listed.insert(pypath::eq_key(&path.resolve()?));
    }
    let mut paths: Vec<(Vec<String>, PyPath)> = rglob(&model.root)
        .into_iter()
        .map(|p| (pypath::sort_key(&p), p))
        .collect();
    paths.sort_by(|a, b| a.0.cmp(&b.0));
    let mut findings = Vec::new();
    for (_, path) in paths {
        if !path.is_file() {
            continue;
        }
        if !listed.contains(&pypath::eq_key(&path.resolve()?)) {
            findings.push(finding(
                ERROR,
                "MAN001",
                "this file sits inside the model tree but the manifest does not list it, so a \
                 deployment would leave it behind"
                    .into(),
                &path.display(),
            ));
        }
    }
    Ok(findings)
}

/// Drop TurboIntegrator comments so prose cannot look like code.
fn strip_comments(script: &str) -> String {
    let mut lines = Vec::new();
    for line in py::splitlines(script) {
        let mut quoted = false;
        let mut cut = line.len();
        for (index, character) in line.char_indices() {
            if character == '\'' {
                quoted = !quoted;
            } else if character == '#' && !quoted {
                cut = index;
                break;
            }
        }
        lines.push(&line[..cut]);
    }
    lines.join("\n")
}

/// `re.findall(r"\bp[A-Z][A-Za-z0-9_]*\b", text)`.
fn parameter_names(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let starts = chars[i] == 'p'
            && (i == 0 || !py::is_word(chars[i - 1]))
            && chars.get(i + 1).is_some_and(char::is_ascii_uppercase);
        if starts {
            let mut j = i + 2;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            if j == chars.len() || !py::is_word(chars[j]) {
                found.push(chars[i..j].iter().collect());
                i = j;
                continue;
            }
        }
        i += 1;
    }
    found
}

fn validate_processes(model: &Model) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for process in &model.processes {
        let mut declared = HashSet::new();
        for parameter in &process.parameters {
            let Json::Object(_) = parameter else {
                return Err(Error::Uncaught(Uncaught::new(
                    "AttributeError",
                    format!("'{}' object has no attribute 'get'", parameter.type_name()),
                )));
            };
            if let Some(name) = parameter.get("Name")
                && name.truthy()
            {
                declared.insert(py::casefold(&name.py_str()));
            }
        }
        let used: BTreeSet<String> = parameter_names(&strip_comments(&process.script))
            .iter()
            .map(|n| py::casefold(n))
            .collect();
        let location = process
            .script_source
            .as_ref()
            .unwrap_or(&process.source)
            .display();
        for name in used.difference(&declared.into_iter().collect()) {
            findings.push(finding(
                ERROR,
                "PRC001",
                format!(
                    "the script uses {name} as a parameter but the process does not declare it"
                ),
                &location,
            ));
        }
    }
    Ok(findings)
}

/// Every structural finding in a loaded model, errors and warnings alike.
pub fn validate_model(model: &Model) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for cube in &model.cubes {
        findings.extend(validate_cube(model, cube)?);
    }
    findings.extend(validate_feeding(model)?);
    findings.extend(validate_processes(model)?);
    findings.extend(validate_manifest(model)?);
    Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameters_match_the_python_pattern() {
        assert_eq!(
            parameter_names("pYear, xpNo, pA1_b, pAé, per pZ"),
            vec!["pYear", "pA1_b", "pZ"]
        );
        assert_eq!(parameter_names("pA\u{b2}"), Vec::<String>::new());
    }

    #[test]
    fn comments_end_at_an_unquoted_hash() {
        assert_eq!(
            strip_comments("a # pX\r\n'#' pY # z\x0bq"),
            "a \n'#' pY \nq"
        );
    }
}
