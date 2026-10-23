//! Load an IBM Planning Analytics model from its git native source tree.
//!
//! The layout this module reads is the one the TM1 database itself publishes when
//! it pushes a model to git: a `tm1project.json` manifest at the root, then
//! `dimensions/`, `cubes/` and `processes/` folders where each object is a
//! JSON file, and where rule and TurboIntegrator text sits beside it in a plain
//! text file referenced by a `@Code.link` property. A cube names its dimensions
//! as IBM's TM1 source specification shows, `{"@id": "Dimensions('Year')"}`.
//! The earlier form of this repository's model, a `Dimensions@Code.links` list
//! of dimension files, is still read.
//!
//! Nothing here touches a network. The whole tree is data on disk.

use std::collections::{HashMap, HashSet};

use crate::decimal::Decimal;
use crate::error::{Error, Result, Uncaught};
use crate::py::{self, OsStyle, SigDecoder};
use crate::pyjson::{self, Json};
use crate::pypath::PyPath;
use crate::rules::{RuleSet, parse_rules};

pub const CONSOLIDATED: &str = "Consolidated";
pub const NUMERIC: &str = "Numeric";
pub const STRING: &str = "String";

#[derive(Clone, Debug)]
pub struct Element {
    pub name: String,
    pub element_type: String,
}

#[derive(Clone, Debug)]
pub struct Edge {
    pub parent: String,
    pub component: String,
    pub weight: Decimal,
}

/// `Path.read_text(encoding="utf-8-sig")`: universal newlines, one byte
/// order mark dropped, failures as Python raised them.
pub fn read_text(path: &PyPath) -> Result<String> {
    let bytes = std::fs::read(path.fs())
        .map_err(|e| Error::Os(py::oserror_str(&e, &path.display(), OsStyle::Errno)))?;
    let text = SigDecoder::default()
        .decode(&bytes, true)
        .map_err(|e| Error::Decode(e.to_string()))?;
    Ok(py::universal_newlines(&text))
}

/// `decimal_or_raise(text, where, ModelError)` for a JSON value.
pub fn decimal_of_json(value: &Json, place: &str) -> Result<Decimal> {
    Decimal::parse(&value.py_str())
        .map_err(|_| Error::model(format!("{place}: {} is not a number", value.repr())))
}

fn read_json(path: &PyPath) -> Result<Vec<(String, Json)>> {
    if !path.is_file() {
        return Err(Error::model(format!("{path}: file not found")));
    }
    let text = read_text(path)?;
    match pyjson::loads(&text) {
        Ok(Json::Object(pairs)) => Ok(pairs),
        Ok(_) => Err(Error::model(format!(
            "{path}: expected a JSON object at the top level"
        ))),
        Err(pyjson::DecodeError::Value(message)) => {
            Err(Error::model(format!("{path}: invalid JSON, {message}")))
        }
        Err(pyjson::DecodeError::TooDeep) => Err(Error::model(format!(
            "{path}: invalid JSON, nested too deeply to read"
        ))),
    }
}

fn get<'a>(payload: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    payload.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn truthy(value: Option<&Json>) -> bool {
    value.is_some_and(Json::truthy)
}

/// One hierarchy of a dimension: its elements and its weighted edges.
#[derive(Debug)]
pub struct Hierarchy {
    pub name: String,
    pub dimension: String,
    pub source: PyPath,
    pub elements: Vec<Element>,
    by_key: HashMap<String, usize>,
    pub edges: Vec<Edge>,
    children: HashMap<usize, Vec<usize>>,
}

impl Hierarchy {
    pub fn new(
        name: String,
        dimension: String,
        elements: Vec<Element>,
        edges: Vec<Edge>,
        source: PyPath,
    ) -> Result<Hierarchy> {
        let mut by_key = HashMap::new();
        let mut kept = Vec::new();
        for element in elements {
            let key = py::casefold(&element.name);
            if by_key.contains_key(&key) {
                return Err(Error::model(format!(
                    "{source}: element {} is declared twice",
                    py::repr_str(&element.name)
                )));
            }
            by_key.insert(key, kept.len());
            kept.push(element);
        }
        let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
        for (index, edge) in edges.iter().enumerate() {
            for end in [&edge.parent, &edge.component] {
                if !by_key.contains_key(&py::casefold(end)) {
                    return Err(Error::model(format!(
                        "{source}: edge names element {}, which the hierarchy does not declare",
                        py::repr_str(end)
                    )));
                }
            }
            let parent = by_key[&py::casefold(&edge.parent)];
            children.entry(parent).or_default().push(index);
        }
        let hierarchy = Hierarchy {
            name,
            dimension,
            source,
            elements: kept,
            by_key,
            edges,
            children,
        };
        hierarchy.check_for_cycles()?;
        Ok(hierarchy)
    }

    fn component_index(&self, edge: usize) -> usize {
        self.by_key[&py::casefold(&self.edges[edge].component)]
    }

    fn check_for_cycles(&self) -> Result<()> {
        let mut done: HashSet<usize> = HashSet::new();
        for start in 0..self.elements.len() {
            if done.contains(&start) {
                continue;
            }
            // The walk Python made recursively, kept on an explicit stack.
            let mut visiting: Vec<usize> = vec![start];
            let mut cursor: Vec<usize> = vec![0];
            while let Some(&current) = visiting.last() {
                let position = *cursor.last().expect("a cursor per visit");
                let edges = self.children.get(&current).map_or(&[][..], Vec::as_slice);
                if position == edges.len() {
                    visiting.pop();
                    cursor.pop();
                    done.insert(current);
                    continue;
                }
                *cursor.last_mut().expect("a cursor per visit") += 1;
                let next = self.component_index(edges[position]);
                if done.contains(&next) {
                    continue;
                }
                if let Some(at) = visiting.iter().position(|&v| v == next) {
                    let mut names: Vec<&str> = visiting[at..]
                        .iter()
                        .map(|&v| self.elements[v].name.as_str())
                        .collect();
                    names.push(&self.elements[next].name);
                    return Err(Error::model(format!(
                        "{}: hierarchy contains a cycle, {}",
                        self.source,
                        names.join(" -> ")
                    )));
                }
                visiting.push(next);
                cursor.push(0);
            }
        }
        Ok(())
    }

    fn index(&self, name: &str) -> Result<usize> {
        self.by_key
            .get(&py::casefold(name))
            .copied()
            .ok_or_else(|| {
                Error::model(format!(
                    "{}: no element named {} in hierarchy {}",
                    self.dimension,
                    py::repr_str(name),
                    py::repr_str(&self.name)
                ))
            })
    }

    /// The canonical spelling of an element, matching case insensitively.
    pub fn resolve(&self, name: &str) -> Result<&str> {
        Ok(&self.elements[self.index(name)?].name)
    }

    pub fn has(&self, name: &str) -> bool {
        self.by_key.contains_key(&py::casefold(name))
    }

    /// The edges under an element, in source order.
    pub fn children(&self, parent: &str) -> Result<Vec<&Edge>> {
        let index = self.index(parent)?;
        Ok(self.children.get(&index).map_or(Vec::new(), |edges| {
            edges.iter().map(|&e| &self.edges[e]).collect()
        }))
    }

    pub fn is_leaf(&self, name: &str) -> Result<bool> {
        let index = self.index(name)?;
        Ok(self.children.get(&index).is_none_or(Vec::is_empty))
    }

    pub fn is_consolidated(&self, name: &str) -> Result<bool> {
        Ok(!self.is_leaf(name)?)
    }

    /// Whether the element at `index` has children.
    pub fn index_is_leaf(&self, index: usize) -> bool {
        self.children.get(&index).is_none_or(Vec::is_empty)
    }

    /// Every leaf beneath an element, in edge order, each reported once.
    pub fn leaves_under(&self, name: &str) -> Result<Vec<&str>> {
        let start = self.index(name)?;
        let mut found = Vec::new();
        let mut seen = HashSet::new();
        let mut stack = vec![start];
        while let Some(current) = stack.pop() {
            if !seen.insert(current) {
                continue;
            }
            if self.index_is_leaf(current) {
                found.push(self.elements[current].name.as_str());
                continue;
            }
            let edges = &self.children[&current];
            for &edge in edges.iter().rev() {
                stack.push(self.component_index(edge));
            }
        }
        Ok(found)
    }
}

#[derive(Debug)]
pub struct Dimension {
    pub name: String,
    pub hierarchies: Vec<Hierarchy>,
    pub source: PyPath,
}

impl Dimension {
    /// The hierarchy sharing the dimension's name, else the only one present.
    pub fn default_hierarchy(&self) -> Result<&Hierarchy> {
        if let Some(h) = self.hierarchies.iter().find(|h| h.name == self.name) {
            return Ok(h);
        }
        if self.hierarchies.len() == 1 {
            return Ok(&self.hierarchies[0]);
        }
        Err(Error::model(format!(
            "{}: no hierarchy named {} to use as the default",
            self.source,
            py::repr_str(&self.name)
        )))
    }
}

#[derive(Debug)]
pub struct Cube {
    pub name: String,
    pub dimensions: Vec<String>,
    pub rules: Option<RuleSet>,
    pub source: PyPath,
    pub rules_source: Option<PyPath>,
}

/// A process parameter as Python held it: `tuple(payload.get("Parameters", ()))`.
#[derive(Debug)]
pub struct Process {
    pub name: String,
    pub parameters: Vec<Json>,
    pub script: String,
    pub source: PyPath,
    pub script_source: Option<PyPath>,
}

/// A whole model tree: its manifest, dimensions, cubes and processes.
#[derive(Debug)]
pub struct Model {
    /// `manifest.get("Name") or root.name`, which need not be a string.
    pub name: Json,
    pub root: PyPath,
    pub dimensions: Vec<Dimension>,
    dimension_index: HashMap<String, usize>,
    pub cubes: Vec<Cube>,
    cube_index: HashMap<String, usize>,
    pub processes: Vec<Process>,
    pub files: Vec<PyPath>,
}

impl Model {
    pub fn name_repr(&self) -> String {
        self.name.repr()
    }

    pub fn dimension(&self, name: &str) -> Option<&Dimension> {
        self.dimension_index.get(name).map(|&i| &self.dimensions[i])
    }

    pub fn has_dimension(&self, name: &str) -> bool {
        self.dimension_index.contains_key(name)
    }

    pub fn hierarchy(&self, dimension: &str) -> Result<&Hierarchy> {
        match self.dimension(dimension) {
            Some(d) => d.default_hierarchy(),
            None => Err(Error::model(format!(
                "no dimension named {} in model {}",
                py::repr_str(dimension),
                self.name_repr()
            ))),
        }
    }

    /// `model.cubes.get(name)`: an exact match on the cube's name.
    pub fn cube(&self, name: &str) -> Option<&Cube> {
        self.cube_index.get(name).map(|&i| &self.cubes[i])
    }

    /// The cubes' names, in manifest order.
    pub fn cube_names(&self) -> Vec<&str> {
        self.cubes.iter().map(|c| c.name.as_str()).collect()
    }
}

/// `_resolve_link(base, link, root)`: refuse anything outside the model root.
fn resolve_link(base: &PyPath, link: &Json, root: &PyPath) -> Result<PyPath> {
    let Json::Str(link) = link else {
        return Err(Error::model(format!("{base}: link must be a string")));
    };
    let candidate = base.parent().join(link).resolve()?;
    let boundary = root.resolve()?;
    if !candidate.is_relative_to(&boundary) {
        return Err(Error::model(format!(
            "{base}: link {} resolves outside the model root",
            py::repr_str(link)
        )));
    }
    Ok(candidate)
}

fn check_name(payload: &[(String, Json)], path: &PyPath, what: &str) -> Result<String> {
    let name = get(payload, "Name");
    if !truthy(name) {
        return Err(Error::model(format!("{path}: {what} has no Name")));
    }
    match name {
        Some(Json::Str(s)) => Ok(s.clone()),
        _ => Err(Error::model(format!(
            "{path}: {what} Name must be a string"
        ))),
    }
}

/// `payload.get(key, ())` iterated: the members of a list Python accepted.
fn members(value: Option<&Json>) -> &[Json] {
    match value {
        Some(Json::Array(items)) => items,
        _ => &[],
    }
}

fn load_hierarchy(path: &PyPath, dimension_name: &str) -> Result<Hierarchy> {
    let payload = read_json(path)?;
    let name = check_name(&payload, path, "hierarchy")?;
    for collection in ["Elements", "Edges"] {
        if let Some(value) = get(&payload, collection)
            && !matches!(value, Json::Array(_))
        {
            return Err(Error::model(format!(
                "{path}: hierarchy {collection} must be a list"
            )));
        }
    }
    let mut elements = Vec::new();
    for entry in members(get(&payload, "Elements")) {
        let Json::Object(fields) = entry else {
            return Err(Error::model(format!(
                "{path}: an element is {}, expected an object",
                entry.type_name()
            )));
        };
        let element_name = get(fields, "Name");
        if !truthy(element_name) {
            return Err(Error::model(format!("{path}: an element has no Name")));
        }
        let Some(Json::Str(element_name)) = element_name else {
            return Err(Error::model(format!(
                "{path}: element Name must be a string"
            )));
        };
        let numeric = Json::Str(NUMERIC.into());
        let element_type = get(fields, "Type").unwrap_or(&numeric);
        match element_type {
            Json::Str(t) if [CONSOLIDATED, NUMERIC, STRING].contains(&t.as_str()) => {
                elements.push(Element {
                    name: element_name.clone(),
                    element_type: t.clone(),
                });
            }
            other => {
                return Err(Error::model(format!(
                    "{path}: element {} has type {}, expected one of ['Consolidated', 'Numeric', 'String']",
                    py::repr_str(element_name),
                    other.repr()
                )));
            }
        }
    }
    let mut edges = Vec::new();
    for entry in members(get(&payload, "Edges")) {
        let Json::Object(fields) = entry else {
            return Err(Error::model(format!(
                "{path}: an edge is {}, expected an object",
                entry.type_name()
            )));
        };
        let parent = get(fields, "ParentName");
        let component = get(fields, "ComponentName");
        if !truthy(parent) || !truthy(component) {
            return Err(Error::model(format!(
                "{path}: an edge is missing ParentName or ComponentName"
            )));
        }
        let (Some(Json::Str(parent)), Some(Json::Str(component))) = (parent, component) else {
            return Err(Error::model(format!(
                "{path}: edge ParentName and ComponentName must be strings"
            )));
        };
        let place = format!("{path}: edge {parent} to {component}");
        let one = Json::Int(1.into());
        let text = get(fields, "Weight").unwrap_or(&one);
        let weight = decimal_of_json(text, &place)?;
        if !weight.is_finite() {
            return Err(Error::model(format!(
                "{place}: {} is not a finite number",
                text.repr()
            )));
        }
        edges.push(Edge {
            parent: parent.clone(),
            component: component.clone(),
            weight,
        });
    }
    Hierarchy::new(
        name,
        dimension_name.to_string(),
        elements,
        edges,
        path.clone(),
    )
}

fn dimension_in(path: &PyPath, root: &PyPath) -> Result<Dimension> {
    let payload = read_json(path)?;
    let name = check_name(&payload, path, "dimension")?;
    let links = get(&payload, "Hierarchies@Code.links");
    if let Some(links) = links
        && !matches!(links, Json::Null | Json::Array(_))
    {
        return Err(Error::model(format!(
            "{path}: dimension {} Hierarchies@Code.links must be a list",
            py::repr_str(&name)
        )));
    }
    if !truthy(links) {
        return Err(Error::model(format!(
            "{path}: dimension {} links no hierarchy file",
            py::repr_str(&name)
        )));
    }
    let mut hierarchies: Vec<Hierarchy> = Vec::new();
    for link in members(links) {
        let hierarchy_path = resolve_link(path, link, root)?;
        let hierarchy = load_hierarchy(&hierarchy_path, &name)?;
        if hierarchies.iter().any(|h| h.name == hierarchy.name) {
            return Err(Error::model(format!(
                "{path}: hierarchy {} is linked twice",
                py::repr_str(&hierarchy.name)
            )));
        }
        hierarchies.push(hierarchy);
    }
    Ok(Dimension {
        name,
        hierarchies,
        source: path.clone(),
    })
}

/// `_DIMENSION_ID.fullmatch(identifier)`: `Dimensions\('((?:[^']|'')+)'\)`.
pub fn dimension_id(identifier: &str) -> Option<String> {
    let inner = identifier
        .strip_prefix("Dimensions('")?
        .strip_suffix("')")?;
    if inner.is_empty() {
        return None;
    }
    let mut run = 0;
    for c in inner.chars() {
        if c == '\'' {
            run += 1;
        } else {
            if run % 2 == 1 {
                return None;
            }
            run = 0;
        }
    }
    if run % 2 == 1 {
        return None;
    }
    Some(inner.replace("''", "'"))
}

fn dimension_from_reference(path: &PyPath, cube: &str, reference: &Json) -> Result<String> {
    let identifier = match reference {
        Json::Object(_) => reference.get("@id"),
        _ => None,
    };
    if let Some(Json::Str(identifier)) = identifier
        && let Some(name) = dimension_id(identifier)
    {
        return Ok(name);
    }
    Err(Error::model(format!(
        "{path}: cube {} has dimension reference {}, expected {{\"@id\": \"Dimensions('Name')\"}}",
        py::repr_str(cube),
        reference.repr()
    )))
}

fn cube_in(path: &PyPath, root: &PyPath) -> Result<Cube> {
    let payload = read_json(path)?;
    let name = check_name(&payload, path, "cube")?;
    let has_references = get(&payload, "Dimensions").is_some();
    if has_references && get(&payload, "Dimensions@Code.links").is_some() {
        return Err(Error::model(format!(
            "{path}: cube {} gives its dimensions both as Dimensions and as \
             Dimensions@Code.links; keep one",
            py::repr_str(&name)
        )));
    }
    let form = if has_references {
        "Dimensions"
    } else {
        "Dimensions@Code.links"
    };
    let entries = get(&payload, form);
    if let Some(entries) = entries
        && !matches!(entries, Json::Null | Json::Array(_))
    {
        return Err(Error::model(format!(
            "{path}: cube {} {form} must be a list",
            py::repr_str(&name)
        )));
    }
    if !truthy(entries) {
        return Err(Error::model(format!(
            "{path}: cube {} names no dimensions",
            py::repr_str(&name)
        )));
    }
    let mut dimensions = Vec::new();
    for entry in members(entries) {
        if has_references {
            dimensions.push(dimension_from_reference(path, &name, entry)?);
            continue;
        }
        let dimension_path = resolve_link(path, entry, root)?;
        if !dimension_path.is_file() {
            return Err(Error::model(format!(
                "{path}: linked dimension file {} not found",
                entry.repr()
            )));
        }
        let linked = read_json(&dimension_path)?;
        let dimension_name = match get(&linked, "Name") {
            Some(value) if value.truthy() => value.clone(),
            _ => Json::Str(dimension_path.stem().to_string()),
        };
        let Json::Str(dimension_name) = dimension_name else {
            return Err(Error::model(format!(
                "{dimension_path}: dimension Name must be a string"
            )));
        };
        dimensions.push(dimension_name);
    }
    let mut rules = None;
    let mut rules_source = None;
    match get(&payload, "Rules@Code.link") {
        None | Some(Json::Null) => {}
        Some(Json::Str(link)) if !link.is_empty() => {
            let source = resolve_link(path, &Json::Str(link.clone()), root)?;
            if !source.is_file() {
                return Err(Error::model(format!(
                    "{path}: cube {} links rules file {}, which is missing",
                    py::repr_str(&name),
                    py::repr_str(link)
                )));
            }
            rules = Some(parse_rules(&read_text(&source)?, &source.display())?);
            rules_source = Some(source);
        }
        Some(Json::Str(_)) => {}
        Some(_) => return Err(Error::model(format!("{path}: link must be a string"))),
    }
    Ok(Cube {
        name,
        dimensions,
        rules,
        source: path.clone(),
        rules_source,
    })
}

/// `tuple(value)`, as `tuple(payload.get("Parameters", ()))` iterated it.
fn as_tuple(value: Option<&Json>) -> Result<Vec<Json>> {
    match value {
        None => Ok(Vec::new()),
        Some(Json::Array(items)) => Ok(items.clone()),
        Some(Json::Object(pairs)) => Ok(pairs.iter().map(|(k, _)| Json::Str(k.clone())).collect()),
        Some(Json::Str(s)) => Ok(s.chars().map(|c| Json::Str(c.to_string())).collect()),
        Some(other) => Err(Error::Uncaught(Uncaught::new(
            "TypeError",
            format!("'{}' object is not iterable", other.type_name()),
        ))),
    }
}

fn process_in(path: &PyPath, root: &PyPath) -> Result<Process> {
    let payload = read_json(path)?;
    let name = check_name(&payload, path, "process")?;
    let mut script = String::new();
    let mut script_source = None;
    match get(&payload, "Code@Code.link") {
        None | Some(Json::Null) => {}
        Some(Json::Str(link)) if !link.is_empty() => {
            let source = resolve_link(path, &Json::Str(link.clone()), root)?;
            if !source.is_file() {
                return Err(Error::model(format!(
                    "{path}: process {} links script {}, which is missing",
                    py::repr_str(&name),
                    py::repr_str(link)
                )));
            }
            script = read_text(&source)?;
            script_source = Some(source);
        }
        Some(Json::Str(_)) => {}
        Some(_) => return Err(Error::model(format!("{path}: link must be a string"))),
    }
    let parameters = as_tuple(get(&payload, "Parameters"))?;
    Ok(Process {
        name,
        parameters,
        script,
        source: path.clone(),
        script_source,
    })
}

/// Load one dimension and every hierarchy it links to. Without a model root,
/// links are fenced to the dimension file's own folder.
pub fn load_dimension(path: &PyPath, root: Option<&PyPath>) -> Result<Dimension> {
    dimension_in(path, root.unwrap_or(&path.parent()))
}

/// Load one cube, resolving its dimensions and its rules file. A cube read on
/// its own is fenced to the tree holding `cubes/`, so the earlier
/// `../dimensions/X.json` link form still loads.
pub fn load_cube(path: &PyPath, root: Option<&PyPath>) -> Result<Cube> {
    cube_in(path, root.unwrap_or(&path.parent().parent()))
}

/// Load one TurboIntegrator process and the script text linked beside it.
pub fn load_process(path: &PyPath, root: Option<&PyPath>) -> Result<Process> {
    process_in(path, root.unwrap_or(&path.parent()))
}

/// Load a model from the directory holding its tm1project.json manifest.
pub fn load_model(root: &PyPath) -> Result<Model> {
    let manifest_path = root.join("tm1project.json");
    let manifest = read_json(&manifest_path)?;
    let name = match get(&manifest, "Name") {
        Some(value) if value.truthy() => value.clone(),
        _ => Json::Str(root.name().to_string()),
    };
    let empty = Json::Object(Vec::new());
    let objects = get(&manifest, "Objects").unwrap_or(&empty);
    if !matches!(objects, Json::Object(_)) {
        return Err(Error::model(format!(
            "{manifest_path}: Objects must be an object"
        )));
    }
    for kind in ["Dimensions", "Cubes", "Processes"] {
        if let Some(value) = objects.get(kind)
            && !matches!(value, Json::Array(_))
        {
            return Err(Error::model(format!(
                "{manifest_path}: Objects.{kind} must be a list"
            )));
        }
    }
    let listed = |link: &Json| -> Result<PyPath> {
        let path = resolve_link(&manifest_path, link, root)?;
        if !path.is_file() {
            return Err(Error::model(format!(
                "{manifest_path}: lists {}, which is not on disk",
                link.repr()
            )));
        }
        Ok(path)
    };

    let mut files = vec![manifest_path.clone()];
    let mut dimensions: Vec<Dimension> = Vec::new();
    let mut dimension_index = HashMap::new();
    for link in members(objects.get("Dimensions")) {
        let path = listed(link)?;
        let dimension = dimension_in(&path, root)?;
        if let Some(&previous) = dimension_index.get(&dimension.name) {
            let previous: &Dimension = &dimensions[previous];
            return Err(Error::model(format!(
                "{path}: dimension {} is also declared in {}",
                py::repr_str(&dimension.name),
                previous.source
            )));
        }
        files.push(path);
        files.extend(dimension.hierarchies.iter().map(|h| h.source.clone()));
        dimension_index.insert(dimension.name.clone(), dimensions.len());
        dimensions.push(dimension);
    }

    let mut cubes: Vec<Cube> = Vec::new();
    let mut cube_index = HashMap::new();
    let mut cube_keys: HashMap<String, usize> = HashMap::new();
    for link in members(objects.get("Cubes")) {
        let path = listed(link)?;
        let cube = cube_in(&path, root)?;
        let key = py::casefold(&cube.name);
        if let Some(&previous) = cube_keys.get(&key) {
            let previous: &Cube = &cubes[previous];
            return Err(Error::model(format!(
                "{path}: cube {} conflicts with {} declared in {}",
                py::repr_str(&cube.name),
                py::repr_str(&previous.name),
                previous.source
            )));
        }
        files.push(path);
        if let Some(source) = &cube.rules_source {
            files.push(source.clone());
        }
        cube_keys.insert(key, cubes.len());
        cube_index.insert(cube.name.clone(), cubes.len());
        cubes.push(cube);
    }

    let mut processes: Vec<Process> = Vec::new();
    for link in members(objects.get("Processes")) {
        let path = listed(link)?;
        let process = process_in(&path, root)?;
        if let Some(previous) = processes.iter().find(|p| p.name == process.name) {
            return Err(Error::model(format!(
                "{path}: process {} is also declared in {}",
                py::repr_str(&process.name),
                previous.source
            )));
        }
        files.push(path);
        if let Some(source) = &process.script_source {
            files.push(source.clone());
        }
        processes.push(process);
    }

    Ok(Model {
        name,
        root: root.clone(),
        dimensions,
        dimension_index,
        cubes,
        cube_index,
        processes,
        files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimension_ids_follow_the_odata_pattern() {
        assert_eq!(dimension_id("Dimensions('Year')").as_deref(), Some("Year"));
        assert_eq!(
            dimension_id("Dimensions('O''Brien')").as_deref(),
            Some("O'Brien")
        );
        assert_eq!(dimension_id("Dimensions('a''')").as_deref(), Some("a'"));
        assert_eq!(dimension_id("Dimensions('')"), None);
        assert_eq!(dimension_id("Dimensions('a'b')"), None);
        assert_eq!(dimension_id("Dimensions('a')x"), None);
        // A quote left unpaired at the end: the pattern needs `''` or a non-quote.
        assert_eq!(dimension_id("Dimensions('a'')"), None);
    }

    #[test]
    fn parameters_iterate_as_tuple_did() {
        assert_eq!(as_tuple(None).unwrap(), Vec::<Json>::new());
        let object = Json::Object(vec![("pYear".into(), Json::Int(1.into()))]);
        assert_eq!(
            as_tuple(Some(&object)).unwrap(),
            [Json::Str("pYear".into())]
        );
        assert_eq!(
            as_tuple(Some(&Json::Str("ab".into()))).unwrap(),
            [Json::Str("a".into()), Json::Str("b".into())]
        );
        assert_eq!(
            as_tuple(Some(&Json::Bool(true))).unwrap_err(),
            Error::Uncaught(Uncaught::new("TypeError", "'bool' object is not iterable"))
        );
    }

    #[test]
    fn a_manifest_without_a_name_takes_the_folder_name() {
        // `manifest.get("Name") or root.name`: an empty name is falsy.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("planning");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(
            root.join("tm1project.json"),
            r#"{"Name": "", "Objects": {}}"#,
        )
        .unwrap();
        let model = load_model(&PyPath::new(&root.to_string_lossy())).unwrap();
        assert_eq!(model.name, Json::Str("planning".into()));
    }
}
