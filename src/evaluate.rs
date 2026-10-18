//! Evaluate rules over leaf cells and consolidate along weighted hierarchy edges.
//!
//! Two ideas carry most of the behaviour here.
//!
//! First, a rule area lists element names, not dimension positions. `['Budget',
//! 'Amount']` means the Budget element of whichever dimension holds it and the
//! Amount element of whichever dimension holds that. Resolution happens against
//! the cube being calculated, and an element that no dimension of the cube holds
//! is a model error rather than a cell that silently never matches.
//!
//! Second, a value is produced on demand. Asking for a cell finds the rule that
//! matches it, and evaluating that rule asks for further cells. A cell with no
//! matching rule is either a stored leaf value or, where any element in the
//! coordinate is consolidated, the weighted sum of the cells beneath it.

use std::collections::HashMap;

use crate::decimal::{Decimal, Signal};
use crate::error::{Error, Result, Uncaught};
use crate::model::{Cube, Model};
use crate::py;
use crate::rules::{Area, CellRef, Comparison, Expr, Rule};

/// A rule area with several unnamed dimensions expands to the product of their
/// leaves. The cap turns a modelling mistake into a clear error rather than a
/// machine that stops responding.
pub const MAX_MATERIALISED_CELLS: usize = 2_000_000;

pub type Coordinate = Vec<String>;

/// Cell values keyed by cube and coordinate, matched case insensitively.
#[derive(Clone, Debug, Default)]
pub struct CellStore {
    values: HashMap<(String, Vec<String>), (Coordinate, Decimal)>,
    order: Vec<(String, Vec<String>)>,
}

fn key_of(cube: &str, coordinate: &[String]) -> (String, Vec<String>) {
    (
        py::casefold(cube),
        coordinate.iter().map(|e| py::casefold(e)).collect(),
    )
}

impl CellStore {
    pub fn new() -> CellStore {
        CellStore::default()
    }

    pub fn set(&mut self, cube: &str, coordinate: &[String], value: Decimal) {
        let key = key_of(cube, coordinate);
        if !self.values.contains_key(&key) {
            self.order.push(key.clone());
        }
        self.values.insert(key, (coordinate.to_vec(), value));
    }

    pub fn get(&self, cube: &str, coordinate: &[String]) -> Decimal {
        self.values
            .get(&key_of(cube, coordinate))
            .map_or_else(Decimal::zero, |(_, v)| v.clone())
    }

    pub fn has(&self, cube: &str, coordinate: &[String]) -> bool {
        self.values.contains_key(&key_of(cube, coordinate))
    }

    /// `(casefolded cube, coordinate as last written, value)`, in first-write order.
    pub fn items(&self) -> impl Iterator<Item = (&str, &Coordinate, &Decimal)> {
        self.order.iter().map(|key| {
            let (coordinate, value) = &self.values[key];
            (key.0.as_str(), coordinate, value)
        })
    }

    /// `items(cube)`: the cells of one cube, matched case insensitively.
    pub fn items_in<'a>(
        &'a self,
        cube: &str,
    ) -> impl Iterator<Item = (&'a Coordinate, &'a Decimal)> + 'a {
        let key = py::casefold(cube);
        self.items()
            .filter(move |(c, _, _)| *c == key)
            .map(|(_, coordinate, value)| (coordinate, value))
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

fn label(cube: &str, coordinate: &[String]) -> String {
    format!("{cube}{}", py::repr_list(coordinate))
}

fn decimal_failure(signal: Signal) -> Uncaught {
    Uncaught::decimal(signal)
}

#[cfg(test)]
thread_local! {
    /// Every element `dimension_of` was asked about, for the caching tests.
    static DIMENSION_OF_CALLS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Find which of a cube's dimensions holds an element, as a rule area does.
fn dimension_of(model: &Model, cube: &Cube, element: &str) -> Result<usize> {
    #[cfg(test)]
    DIMENSION_OF_CALLS.with(|calls| calls.borrow_mut().push(element.to_string()));
    let mut matches = Vec::new();
    for (index, dimension) in cube.dimensions.iter().enumerate() {
        if model.hierarchy(dimension)?.has(element) {
            matches.push(index);
        }
    }
    match matches.len() {
        0 => Err(Error::model(format!(
            "cube {}: no dimension holds an element named {}",
            py::repr_str(&cube.name),
            py::repr_str(element)
        ))),
        1 => Ok(matches[0]),
        _ => {
            let holders: Vec<&str> = matches
                .iter()
                .map(|&i| cube.dimensions[i].as_str())
                .collect();
            Err(Error::model(format!(
                "cube {}: element {} is ambiguous, held by {}",
                py::repr_str(&cube.name),
                py::repr_str(element),
                holders.join(", ")
            )))
        }
    }
}

/// Positions in the cube's dimension order, in the order an area first names them.
type Positions = Vec<(usize, String)>;

/// Map an area's element names to positions in the cube's dimension order.
fn area_positions(model: &Model, cube: &Cube, area: &Area) -> Result<Positions> {
    let mut positions: Positions = Vec::new();
    for element in &area.elements {
        let index = dimension_of(model, cube, element)?;
        let canonical = model
            .hierarchy(&cube.dimensions[index])?
            .resolve(element)?
            .to_string();
        match positions.iter_mut().find(|(i, _)| *i == index) {
            Some((_, existing)) if *existing != canonical => {
                return Err(Error::model(format!(
                    "cube {}: area names two elements of dimension {}",
                    py::repr_str(&cube.name),
                    py::repr_str(&cube.dimensions[index])
                )));
            }
            Some(_) => {}
            None => positions.push((index, canonical)),
        }
    }
    Ok(positions)
}

fn is_leaf_cell(model: &Model, cube: &Cube, coordinate: &[String]) -> Result<bool> {
    for (dimension, element) in cube.dimensions.iter().zip(coordinate) {
        if !model.hierarchy(dimension)?.is_leaf(element)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Areas that constrain the same dimensions: those positions, the earliest
/// rule among them, and each element tuple's earliest rule.
type Group = (Vec<usize>, usize, HashMap<Vec<String>, usize>);

/// Index a safely resolved prefix by the dimensions each area constrains.
#[derive(Default)]
struct RuleIndex {
    next_rule: usize,
    groups: Vec<Group>,
}

impl RuleIndex {
    fn add(&mut self, index: usize, positions: &Positions) {
        let mut sorted: Vec<&(usize, String)> = positions.iter().collect();
        sorted.sort_by_key(|(p, _)| *p);
        let dimensions: Vec<usize> = sorted.iter().map(|(p, _)| *p).collect();
        let elements: Vec<String> = sorted.iter().map(|(_, e)| py::casefold(e)).collect();
        // Identical areas keep their earliest rule; different shapes compete
        // by source index during lookup.
        let group = match self.groups.iter().position(|(d, _, _)| *d == dimensions) {
            Some(g) => g,
            None => {
                self.groups.push((dimensions, index, HashMap::new()));
                self.groups.len() - 1
            }
        };
        self.groups[group].2.entry(elements).or_insert(index);
    }

    fn matches(&self, coordinate_key: &[String]) -> Option<usize> {
        let mut first: Option<usize> = None;
        for (dimensions, earliest, areas) in &self.groups {
            // Groups arrive in source order. Once their first rule cannot
            // beat a match, neither can any of the remaining groups.
            if first.is_some_and(|f| *earliest >= f) {
                break;
            }
            let elements: Vec<String> = dimensions
                .iter()
                .map(|&p| coordinate_key[p].clone())
                .collect();
            if let Some(&index) = areas.get(&elements)
                && first.is_none_or(|f| index < f)
            {
                first = Some(index);
            }
        }
        first
    }
}

/// One item of calculation evidence, in the order Python's keyword arguments gave its fields.
#[derive(Clone, Debug)]
pub enum Item {
    Input {
        cell: String,
        value: Decimal,
    },
    Step {
        operator: String,
        left: Decimal,
        right: Decimal,
        result: Decimal,
    },
    Comparison {
        operator: String,
        left: Decimal,
        right: Decimal,
        result: bool,
    },
    Branch {
        then: bool,
    },
    Contribution {
        cell: String,
        value: Decimal,
        weight: Decimal,
        contribution: Decimal,
    },
}

/// The evidence recorded for one cell.
#[derive(Clone, Debug)]
pub struct Evidence {
    pub cube: String,
    pub coordinate: Coordinate,
    pub kind: Option<&'static str>,
    /// The rule's model-relative file, if any, and its line.
    pub rule: Option<(Option<String>, usize)>,
    /// `inputs`, `steps` and `contributions`, in the order each was first recorded.
    pub lists: Vec<(&'static str, Vec<Item>)>,
    pub value: Option<Decimal>,
}

/// The calculation evidence `explain` returns.
#[derive(Clone, Debug, Default)]
pub struct Explanation {
    pub requested: Vec<String>,
    pub cells: Vec<(String, Evidence)>,
}

impl Explanation {
    pub fn cell(&self, label: &str) -> Option<&Evidence> {
        self.cells.iter().find(|(l, _)| l == label).map(|(_, e)| e)
    }
}

struct Engine<'a> {
    model: &'a Model,
    store: &'a CellStore,
    trace: Option<Vec<(String, Evidence)>>,
    trace_index: HashMap<String, usize>,
    memo: HashMap<(String, Vec<String>), Decimal>,
    visiting: Vec<String>,
    positions: HashMap<(String, usize), Positions>,
    rule_indexes: HashMap<(String, bool), RuleIndex>,
    same_cube_references: HashMap<(String, CellRef), Vec<(usize, String)>>,
}

impl<'a> Engine<'a> {
    fn new(model: &'a Model, store: &'a CellStore, trace: bool) -> Engine<'a> {
        Engine {
            model,
            store,
            trace: trace.then(Vec::new),
            trace_index: HashMap::new(),
            memo: HashMap::new(),
            visiting: Vec::new(),
            positions: HashMap::new(),
            rule_indexes: HashMap::new(),
            same_cube_references: HashMap::new(),
        }
    }

    fn evidence(&mut self) -> Option<&mut Evidence> {
        let label = self.visiting.last()?;
        let index = *self.trace_index.get(label)?;
        self.trace.as_mut().map(|t| &mut t[index].1)
    }

    /// Attach evidence to the cell currently being evaluated.
    fn record(&mut self, field: &'static str, item: Item) {
        if let Some(evidence) = self.evidence() {
            match evidence.lists.iter_mut().find(|(f, _)| *f == field) {
                Some((_, items)) => items.push(item),
                None => evidence.lists.push((field, vec![item])),
            }
        }
    }

    fn rule_positions(&mut self, cube: &Cube, index: usize, rule: &Rule) -> Result<Positions> {
        let key = (cube.name.clone(), index);
        if let Some(found) = self.positions.get(&key) {
            return Ok(found.clone());
        }
        let found = area_positions(self.model, cube, &rule.area)?;
        self.positions.insert(key, found.clone());
        Ok(found)
    }

    fn matching_rule(
        &mut self,
        cube: &'a Cube,
        coordinate: &[String],
        coordinate_key: &[String],
    ) -> Result<Option<&'a Rule>> {
        let Some(rules) = &cube.rules else {
            return Ok(None);
        };
        let leaf = is_leaf_cell(self.model, cube, coordinate)?;
        let key = (cube.name.clone(), leaf);
        let mut lookup = self.rule_indexes.remove(&key).unwrap_or_default();
        let outcome = (|| {
            if let Some(matched) = lookup.matches(coordinate_key) {
                return Ok(Some(&rules.rules[matched]));
            }
            // Extend only after the prepared prefix misses. Resolving later areas
            // eagerly would expose errors hidden by an earlier matching rule.
            while lookup.next_rule < rules.rules.len() {
                let index = lookup.next_rule;
                let rule = &rules.rules[index];
                if (rule.area.qualifier == "N" && !leaf) || (rule.area.qualifier == "C" && leaf) {
                    lookup.next_rule += 1;
                    continue;
                }
                let positions = self.rule_positions(cube, index, rule)?;
                lookup.add(index, &positions);
                lookup.next_rule += 1;
                if lookup.matches(coordinate_key).is_some() {
                    return Ok(Some(rule));
                }
            }
            Ok(None)
        })();
        self.rule_indexes.insert(key, lookup);
        outcome
    }

    fn value(&mut self, cube_name: &str, coordinate: &[String]) -> Result<Decimal> {
        let model = self.model;
        let Some(cube) = model.cube(cube_name) else {
            return Err(Error::model(format!(
                "no cube named {} in model {}",
                py::repr_str(cube_name),
                model.name_repr()
            )));
        };
        if coordinate.len() != cube.dimensions.len() {
            return Err(Error::model(format!(
                "coordinate has {} elements; cube {} has {} dimensions",
                coordinate.len(),
                py::repr_str(&cube.name),
                cube.dimensions.len()
            )));
        }
        let mut canonical = Vec::with_capacity(coordinate.len());
        for (dimension, element) in cube.dimensions.iter().zip(coordinate) {
            canonical.push(model.hierarchy(dimension)?.resolve(element)?.to_string());
        }
        let key = (
            cube.name.clone(),
            canonical
                .iter()
                .map(|e| py::casefold(e))
                .collect::<Vec<_>>(),
        );
        if let Some(found) = self.memo.get(&key) {
            return Ok(found.clone());
        }
        let cell_label = label(&cube.name, &canonical);
        if let Some(at) = self.visiting.iter().position(|v| *v == cell_label) {
            let mut chain: Vec<&str> = self.visiting[at..].iter().map(String::as_str).collect();
            chain.push(&cell_label);
            return Err(Error::evaluation(format!(
                "a rule depends on its own result: {}",
                chain.join(" -> ")
            )));
        }
        if let Some(trace) = &mut self.trace {
            let evidence = Evidence {
                cube: cube.name.clone(),
                coordinate: canonical.clone(),
                kind: None,
                rule: None,
                lists: Vec::new(),
                value: None,
            };
            match self.trace_index.get(&cell_label) {
                Some(&i) => trace[i].1 = evidence,
                None => {
                    self.trace_index.insert(cell_label.clone(), trace.len());
                    trace.push((cell_label.clone(), evidence));
                }
            }
        }
        self.visiting.push(cell_label);
        let result = self.compute(cube, &canonical, &key.1);
        self.visiting.pop();
        let result = result?;
        self.memo.insert(key, result.clone());
        if self.trace.is_some() {
            let index = self.trace_index[&label(&cube.name, &canonical)];
            if let Some(trace) = &mut self.trace {
                trace[index].1.value = Some(result.clone());
            }
        }
        Ok(result)
    }

    fn arithmetic_failed(cube: &Cube, canonical: &[String], error: Error) -> Error {
        match error {
            Error::Uncaught(u) if u.name.starts_with("decimal.") => Error::evaluation(format!(
                "cube {} at {}: arithmetic failed ({})",
                py::repr_str(&cube.name),
                py::repr_list(canonical),
                &u.name["decimal.".len()..]
            )),
            other => other,
        }
    }

    fn compute(&mut self, cube: &'a Cube, canonical: &[String], key: &[String]) -> Result<Decimal> {
        let model = self.model;
        if let Some(rule) = self.matching_rule(cube, canonical, key)? {
            if self.trace.is_some() {
                let file = match &cube.rules_source {
                    Some(source) => {
                        let root = model.root.resolve()?;
                        match source.relative_to_posix(&root) {
                            Some(file) => Some(file),
                            None => {
                                return Err(Error::Uncaught(Uncaught::new(
                                    "ValueError",
                                    format!(
                                        "{} is not in the subpath of {}",
                                        py::repr_str(&source.display()),
                                        py::repr_str(&root.display())
                                    ),
                                )));
                            }
                        }
                    }
                    None => None,
                };
                if let Some(evidence) = self.evidence() {
                    evidence.kind = Some("rule");
                    evidence.rule = Some((file, rule.source_line));
                }
            }
            return self
                .evaluate_expression(&rule.expression, cube, canonical)
                .map_err(|e| Self::arithmetic_failed(cube, canonical, e));
        }
        if is_leaf_cell(model, cube, canonical)? {
            if self.trace.is_some() {
                let kind = if self.store.has(&cube.name, canonical) {
                    "input"
                } else {
                    "default_zero"
                };
                if let Some(evidence) = self.evidence() {
                    evidence.kind = Some(kind);
                }
            }
            return Ok(self.store.get(&cube.name, canonical));
        }
        if let Some(evidence) = self.evidence() {
            evidence.kind = Some("consolidation");
        }
        self.consolidated(cube, canonical)
            .map_err(|e| Self::arithmetic_failed(cube, canonical, e))
    }

    /// Sum the cells beneath the first consolidated element in the coordinate.
    fn consolidated(&mut self, cube: &Cube, coordinate: &[String]) -> Result<Decimal> {
        let model = self.model;
        for (position, (dimension, element)) in cube.dimensions.iter().zip(coordinate).enumerate() {
            let hierarchy = model.hierarchy(dimension)?;
            if hierarchy.is_leaf(element)? {
                continue;
            }
            let mut total = Decimal::zero();
            for edge in hierarchy.children(element)? {
                let mut child = coordinate.to_vec();
                child[position] = hierarchy.resolve(&edge.component)?.to_string();
                let value = self.value(&cube.name, &child)?;
                let contribution = edge.weight.mul(&value).map_err(decimal_failure)?;
                total = total.add(&contribution).map_err(decimal_failure)?;
                if self.trace.is_some() {
                    self.record(
                        "contributions",
                        Item::Contribution {
                            cell: label(&cube.name, &child),
                            value,
                            weight: edge.weight.clone(),
                            contribution,
                        },
                    );
                }
            }
            return Ok(total);
        }
        Ok(self.store.get(&cube.name, coordinate))
    }

    fn evaluate_expression(
        &mut self,
        expression: &Expr,
        cube: &'a Cube,
        coordinate: &[String],
    ) -> Result<Decimal> {
        match expression {
            Expr::Number(value) => Ok(value.clone()),
            Expr::CellRef(reference) => {
                let (target_cube, target) = self.resolve_reference(reference, cube, coordinate)?;
                let result = self.value(&target_cube, &target)?;
                if self.trace.is_some() {
                    self.record(
                        "inputs",
                        Item::Input {
                            cell: label(&target_cube, &target),
                            value: result.clone(),
                        },
                    );
                }
                Ok(result)
            }
            Expr::BinaryOp(op, left, right) => {
                let left = self.evaluate_expression(left, cube, coordinate)?;
                let right = self.evaluate_expression(right, cube, coordinate)?;
                let result = match op {
                    '+' => left.add(&right),
                    '-' => left.sub(&right),
                    '*' => left.mul(&right),
                    '\\' if right.is_zero() => Ok(Decimal::zero()),
                    '\\' => left.div(&right),
                    _ => {
                        if right.is_zero() {
                            return Err(Error::evaluation(format!(
                                "cube {} at {}: division by zero. Use the backslash operator \
                                 where a zero divisor is expected.",
                                py::repr_str(&cube.name),
                                py::repr_list(coordinate)
                            )));
                        }
                        left.div(&right)
                    }
                }
                .map_err(decimal_failure)?;
                if self.trace.is_some() {
                    self.record(
                        "steps",
                        Item::Step {
                            operator: op.to_string(),
                            left,
                            right,
                            result: result.clone(),
                        },
                    );
                }
                Ok(result)
            }
            Expr::If(condition, then_expr, else_expr) => {
                let condition = self.evaluate_condition(condition, cube, coordinate)?;
                if self.trace.is_some() {
                    self.record("steps", Item::Branch { then: condition });
                }
                if condition {
                    self.evaluate_expression(then_expr, cube, coordinate)
                } else {
                    self.evaluate_expression(else_expr, cube, coordinate)
                }
            }
        }
    }

    fn evaluate_condition(
        &mut self,
        condition: &Comparison,
        cube: &'a Cube,
        coordinate: &[String],
    ) -> Result<bool> {
        let left = self.evaluate_expression(&condition.left, cube, coordinate)?;
        let right = self.evaluate_expression(&condition.right, cube, coordinate)?;
        let ordering = left.cmp_value(&right);
        let result = match condition.op {
            "=" => ordering.is_eq(),
            "<>" => ordering.is_ne(),
            "<" => ordering.is_lt(),
            ">" => ordering.is_gt(),
            "<=" => ordering.is_le(),
            _ => ordering.is_ge(),
        };
        if self.trace.is_some() {
            self.record(
                "steps",
                Item::Comparison {
                    operator: condition.op.to_string(),
                    left,
                    right,
                    result,
                },
            );
        }
        Ok(result)
    }

    fn resolve_reference(
        &mut self,
        reference: &CellRef,
        cube: &Cube,
        coordinate: &[String],
    ) -> Result<(String, Coordinate)> {
        let model = self.model;
        let Some(target_cube_name) = &reference.cube else {
            let mut target = coordinate.to_vec();
            let key = (cube.name.clone(), reference.clone());
            if let Some(bindings) = self.same_cube_references.get(&key) {
                for (position, canonical) in bindings {
                    target[*position] = canonical.clone();
                }
            } else {
                let mut pairs = Vec::new();
                for element in &reference.coordinates {
                    let position = dimension_of(model, cube, element)?;
                    let canonical = model
                        .hierarchy(&cube.dimensions[position])?
                        .resolve(element)?
                        .to_string();
                    target[position] = canonical.clone();
                    pairs.push((position, canonical));
                }
                self.same_cube_references.insert(key, pairs);
            }
            return Ok((cube.name.clone(), target));
        };
        // Validation refuses an unknown cube (DIM001), the wrong number of DB
        // coordinates (ARE001) and a !Dimension the cube does not have (ELE001),
        // and every caller validates before it calculates.
        let assertion = || Error::Uncaught(Uncaught::new("AssertionError", ""));
        let target_cube = model.cube(target_cube_name).ok_or_else(assertion)?;
        if reference.coordinates.len() != target_cube.dimensions.len() {
            return Err(assertion());
        }
        let mut resolved = Vec::new();
        for (position, argument) in reference.coordinates.iter().enumerate() {
            let dimension = &target_cube.dimensions[position];
            let element = match argument.strip_prefix('!') {
                Some(source_dimension) => {
                    let index = cube
                        .dimensions
                        .iter()
                        .position(|d| d == source_dimension)
                        .ok_or_else(assertion)?;
                    &coordinate[index]
                }
                None => argument,
            };
            resolved.push(model.hierarchy(dimension)?.resolve(element)?.to_string());
        }
        Ok((target_cube.name.clone(), resolved))
    }
}

/// Every leaf cell a rule with an N or empty qualifier calculates.
fn rule_targets(model: &Model, cube: &Cube, rule: &Rule) -> Result<Vec<Vec<String>>> {
    let positions = area_positions(model, cube, &rule.area)?;
    let mut choices: Vec<Vec<String>> = Vec::new();
    let mut total: usize = 1;
    for (position, dimension) in cube.dimensions.iter().enumerate() {
        match positions.iter().find(|(p, _)| *p == position) {
            Some((_, element)) => choices.push(vec![element.clone()]),
            None => {
                let hierarchy = model.hierarchy(dimension)?;
                let leaves = (0..hierarchy.elements.len())
                    .filter(|&i| hierarchy.index_is_leaf(i))
                    .map(|i| hierarchy.elements[i].name.clone())
                    .collect();
                choices.push(leaves);
            }
        }
        total = total.saturating_mul(choices.last().expect("a choice").len());
        if total > MAX_MATERIALISED_CELLS {
            return Err(Error::evaluation(format!(
                "cube {} rule at line {} expands to more than {MAX_MATERIALISED_CELLS} cells. \
                 Name more elements in its area.",
                py::repr_str(&cube.name),
                rule.source_line
            )));
        }
    }
    Ok(choices)
}

/// The cells of a product of choices, in the order `itertools.product` gives them.
fn for_each_cell(
    choices: &[Vec<String>],
    mut visit: impl FnMut(&[String]) -> Result<()>,
) -> Result<()> {
    if choices.iter().any(Vec::is_empty) {
        return Ok(());
    }
    let mut indices = vec![0; choices.len()];
    let mut cell: Vec<String> = choices.iter().map(|c| c[0].clone()).collect();
    loop {
        visit(&cell)?;
        let mut position = choices.len();
        loop {
            if position == 0 {
                return Ok(());
            }
            position -= 1;
            indices[position] += 1;
            if indices[position] < choices[position].len() {
                cell[position] = choices[position][indices[position]].clone();
                break;
            }
            indices[position] = 0;
            cell[position] = choices[position][0].clone();
        }
    }
}

/// A store holding the input values plus every rule calculated leaf cell.
pub fn evaluate(model: &Model, store: &CellStore) -> Result<CellStore> {
    let mut result = store.clone();
    let mut engine = Engine::new(model, store, false);
    for cube in &model.cubes {
        let Some(rules) = &cube.rules else {
            continue;
        };
        for rule in &rules.rules {
            if rule.area.qualifier == "C" {
                continue;
            }
            let choices = rule_targets(model, cube, rule)?;
            for_each_cell(&choices, |coordinate| {
                let value = engine.value(&cube.name, coordinate)?;
                result.set(&cube.name, coordinate, value);
                Ok(())
            })?;
        }
    }
    Ok(result)
}

/// Values for a batch of cells in order, sharing cached values within the batch.
/// `emit` receives each value as it is calculated, so a failure part way
/// leaves the earlier values delivered, as Python's generator did.
pub fn consolidate_many(
    model: &Model,
    store: &CellStore,
    cells: &[(String, Coordinate)],
    mut emit: impl FnMut(usize, Decimal),
) -> Result<()> {
    let mut engine = Engine::new(model, store, false);
    for (i, (cube, coordinate)) in cells.iter().enumerate() {
        emit(i, engine.value(cube, coordinate)?);
    }
    Ok(())
}

/// Explain requested cells and their dependencies without changing the inputs.
pub fn explain(
    model: &Model,
    store: &CellStore,
    cells: &[(String, Coordinate)],
) -> Result<Explanation> {
    let mut engine = Engine::new(model, store, true);
    let mut requested = Vec::new();
    for (cube, coordinate) in cells {
        engine.value(cube, coordinate)?;
        let dimensions = &model.cube(cube).expect("a requested cube").dimensions;
        let mut canonical = Vec::new();
        for (dimension, element) in dimensions.iter().zip(coordinate) {
            canonical.push(model.hierarchy(dimension)?.resolve(element)?.to_string());
        }
        requested.push(label(cube, &canonical));
    }
    Ok(Explanation {
        requested,
        cells: engine.trace.unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::load_model;
    use crate::pypath::PyPath;

    fn calls() -> Vec<String> {
        DIMENSION_OF_CALLS.with(|c| c.borrow().clone())
    }

    fn clear() {
        DIMENSION_OF_CALLS.with(|c| c.borrow_mut().clear());
    }

    fn model(dir: &std::path::Path, measures: &str, rules: &str) -> Model {
        let write = |path: &str, text: &str| {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        for (name, elements, edges) in [
            (
                "Colour",
                r#"{"Name": "Total", "Type": "Consolidated"}, {"Name": "Red"}, {"Name": "Blue"}"#,
                r#"{"ParentName": "Total", "ComponentName": "Red"}, {"ParentName": "Total", "ComponentName": "Blue"}"#,
            ),
            ("Measure", measures, ""),
        ] {
            write(
                &format!("dimensions/{name}.json"),
                &format!(
                    r#"{{"Name": "{name}", "Hierarchies@Code.links": ["{name}.h/{name}.json"]}}"#
                ),
            );
            write(
                &format!("dimensions/{name}.h/{name}.json"),
                &format!(r#"{{"Name": "{name}", "Elements": [{elements}], "Edges": [{edges}]}}"#),
            );
        }
        let link = if rules.is_empty() {
            ""
        } else {
            r#", "Rules@Code.link": "Sales.rules""#
        };
        write(
            "cubes/Sales.json",
            &format!(
                r#"{{"Name": "Sales", "Dimensions": [{{"@id": "Dimensions('Colour')"}}, {{"@id": "Dimensions('Measure')"}}]{link}}}"#
            ),
        );
        if !rules.is_empty() {
            write("cubes/Sales.rules", rules);
        }
        write(
            "tm1project.json",
            r#"{"Name": "built", "Objects": {"Dimensions": ["dimensions/Colour.json", "dimensions/Measure.json"], "Cubes": ["cubes/Sales.json"]}}"#,
        );
        load_model(&PyPath::new(&dir.to_string_lossy())).unwrap()
    }

    const MEASURES: &str = r#"{"Name": "Units"}, {"Name": "Price"}, {"Name": "Amount"}"#;

    #[test]
    fn public_calls_resolve_repeated_references_once() {
        let dir = tempfile::tempdir().unwrap();
        let model = model(
            dir.path(),
            MEASURES,
            "['Amount'] = N: IF(['Units'] > 0, ['Units'] * 2 + ['Price'], ['Ghost']);",
        );
        let mut store = CellStore::new();
        for (index, colour) in [(1, "Red"), (2, "Blue")] {
            store.set(
                "Sales",
                &[colour.into(), "Units".into()],
                Decimal::from_int(index),
            );
            store.set(
                "Sales",
                &[colour.into(), "Price".into()],
                Decimal::parse("1.25").unwrap(),
            );
        }
        let cells: Vec<(String, Coordinate)> = ["Red", "Blue"]
            .iter()
            .map(|c| ("Sales".to_string(), vec![c.to_string(), "Amount".into()]))
            .collect();
        for operation in ["evaluate", "batch", "explain"] {
            for _ in 0..2 {
                clear();
                match operation {
                    "evaluate" => drop(evaluate(&model, &store).unwrap()),
                    "batch" => consolidate_many(&model, &store, &cells, |_, _| {}).unwrap(),
                    _ => drop(explain(&model, &store, &cells).unwrap()),
                }
                let seen = calls();
                assert_eq!(
                    seen.iter().filter(|e| *e == "Units").count(),
                    1,
                    "{operation}"
                );
                assert_eq!(
                    seen.iter().filter(|e| *e == "Price").count(),
                    1,
                    "{operation}"
                );
                assert!(!seen.contains(&"Ghost".to_string()));
            }
        }
    }

    #[test]
    fn ordered_selectors_and_failed_resolutions() {
        let dir = tempfile::tempdir().unwrap();
        let measures = format!(r#"{MEASURES}, {{"Name": "Red"}}, {{"Name": "Straße"}}"#);
        let model = model(dir.path(), &measures, "");
        let cube = model.cube("Sales").unwrap();
        let store = CellStore::new();
        let mut engine = Engine::new(&model, &store, false);
        let reference = |names: &[&str]| CellRef {
            cube: None,
            coordinates: names.iter().map(|n| n.to_string()).collect(),
        };
        let at = |colour: &str| vec![colour.to_string(), "Amount".to_string()];
        clear();
        for colour in ["Blue", "Total"] {
            let found = engine
                .resolve_reference(&reference(&["Units", "Price"]), cube, &at(colour))
                .unwrap();
            assert_eq!(
                found,
                (
                    "Sales".to_string(),
                    vec![colour.to_string(), "Price".into()]
                )
            );
        }
        assert_eq!(calls(), ["Units", "Price"]);
        for (selectors, message, expected) in [
            (
                &["Units", "Ghost"][..],
                "no dimension holds an element named 'Ghost'",
                &["Units", "Ghost"][..],
            ),
            (
                &["Red", "Ghost"][..],
                "element 'Red' is ambiguous, held by Colour, Measure",
                &["Red"][..],
            ),
            (
                &["Ghost", "Red"][..],
                "no dimension holds an element named 'Ghost'",
                &["Ghost"][..],
            ),
        ] {
            for _ in 0..2 {
                clear();
                let error = engine
                    .resolve_reference(&reference(selectors), cube, &at("Blue"))
                    .unwrap_err();
                assert_eq!(error, Error::model(format!("cube 'Sales': {message}")));
                assert_eq!(calls(), expected);
            }
        }
        clear();
        for colour in ["Blue", "Total"] {
            let found = engine
                .resolve_reference(&reference(&["STRASSE"]), cube, &at(colour))
                .unwrap();
            assert_eq!(
                found,
                (
                    "Sales".to_string(),
                    vec![colour.to_string(), "Straße".into()]
                )
            );
        }
        assert_eq!(calls(), ["STRASSE"]);
    }
}
