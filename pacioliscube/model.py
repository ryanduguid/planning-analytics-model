"""Load an IBM Planning Analytics model from its git native source tree.

The layout this module reads is the one the TM1 database itself publishes when
it pushes a model to git: a ``tm1project.json`` manifest at the root, then
``dimensions/``, ``cubes/`` and ``processes/`` folders where each object is a
JSON file, and where rule and TurboIntegrator text sits beside it in a plain
text file referenced by a ``@Code.link`` property.

Nothing here touches a network. The whole tree is data on disk.
"""

from __future__ import annotations

import json
from decimal import Decimal, InvalidOperation
from pathlib import Path
from typing import Iterable, NamedTuple, Optional

from pacioliscube.rules import RuleSet, parse_rules

CONSOLIDATED = "Consolidated"
NUMERIC = "Numeric"
STRING = "String"

_ELEMENT_TYPES = {CONSOLIDATED, NUMERIC, STRING}


class ModelError(ValueError):
    """Raised when model source is missing, malformed or self contradictory."""


class Element(NamedTuple):
    name: str
    element_type: str


class Edge(NamedTuple):
    parent: str
    component: str
    weight: Decimal


def _decimal(value: object, where: str) -> Decimal:
    """Parse a weight or value without ever routing it through a float."""
    try:
        return Decimal(str(value))
    except (InvalidOperation, ValueError) as error:
        raise ModelError(f"{where}: {value!r} is not a number") from error


def _read_json(path: Path) -> dict:
    if not path.is_file():
        raise ModelError(f"{path}: file not found")
    try:
        payload = json.loads(path.read_text(encoding="utf-8-sig"))
    except json.JSONDecodeError as error:
        raise ModelError(f"{path}: invalid JSON, {error}") from error
    if not isinstance(payload, dict):
        raise ModelError(f"{path}: expected a JSON object at the top level")
    return payload


class Hierarchy:
    """One hierarchy of a dimension: its elements and its weighted edges."""

    def __init__(
        self,
        name: str,
        dimension: str,
        elements: Iterable[Element],
        edges: Iterable[Edge],
        source: Path,
    ) -> None:
        self.name = name
        self.dimension = dimension
        self.source = source
        self.elements: dict[str, Element] = {}
        self._by_key: dict[str, str] = {}
        for element in elements:
            key = element.name.casefold()
            if key in self._by_key:
                raise ModelError(f"{source}: element {element.name!r} is declared twice")
            self.elements[element.name] = element
            self._by_key[key] = element.name
        self.edges: tuple[Edge, ...] = tuple(edges)
        self._children: dict[str, list[Edge]] = {}
        for edge in self.edges:
            for end in (edge.parent, edge.component):
                if end.casefold() not in self._by_key:
                    raise ModelError(
                        f"{source}: edge names element {end!r}, which the hierarchy does not declare"
                    )
            parent = self._by_key[edge.parent.casefold()]
            self._children.setdefault(parent, []).append(edge)
        self._check_for_cycles()

    def _check_for_cycles(self) -> None:
        visiting: list[str] = []
        done: set[str] = set()

        def walk(name: str) -> None:
            if name in done:
                return
            if name in visiting:
                cycle = " -> ".join(visiting[visiting.index(name):] + [name])
                raise ModelError(f"{self.source}: hierarchy contains a cycle, {cycle}")
            visiting.append(name)
            for edge in self._children.get(name, ()):
                walk(self._by_key[edge.component.casefold()])
            visiting.pop()
            done.add(name)

        for element in self.elements:
            walk(element)

    def resolve(self, name: str) -> str:
        """Return the canonical spelling of an element, matching case insensitively."""
        try:
            return self._by_key[name.casefold()]
        except KeyError:
            raise ModelError(
                f"{self.dimension}: no element named {name!r} in hierarchy {self.name!r}"
            ) from None

    def has(self, name: str) -> bool:
        return name.casefold() in self._by_key

    def children(self, parent: str) -> tuple[Edge, ...]:
        return tuple(self._children.get(self.resolve(parent), ()))

    def is_leaf(self, name: str) -> bool:
        return not self._children.get(self.resolve(name))

    def is_consolidated(self, name: str) -> bool:
        return not self.is_leaf(name)

    def leaves_under(self, name: str) -> tuple[str, ...]:
        """Every leaf beneath an element, in edge order, each reported once."""
        found: list[str] = []
        seen: set[str] = set()

        def walk(current: str) -> None:
            if self.is_leaf(current):
                if current not in seen:
                    seen.add(current)
                    found.append(current)
                return
            for edge in self.children(current):
                walk(self.resolve(edge.component))

        walk(self.resolve(name))
        return tuple(found)


class Dimension(NamedTuple):
    name: str
    hierarchies: dict[str, Hierarchy]
    source: Path

    @property
    def default_hierarchy(self) -> Hierarchy:
        """The hierarchy sharing the dimension's name, else the only one present."""
        if self.name in self.hierarchies:
            return self.hierarchies[self.name]
        if len(self.hierarchies) == 1:
            return next(iter(self.hierarchies.values()))
        raise ModelError(f"{self.source}: no hierarchy named {self.name!r} to use as the default")


class Cube(NamedTuple):
    name: str
    dimensions: tuple[str, ...]
    rules: Optional[RuleSet]
    source: Path
    rules_source: Optional[Path]


class Process(NamedTuple):
    name: str
    parameters: tuple[dict, ...]
    datasource: dict
    script: str
    source: Path
    script_source: Optional[Path]


def _resolve_link(base: Path, link: str, root: Optional[Path] = None) -> Path:
    """Resolve a link from a model file, refusing anything outside the model root.

    The manifest and the object files are data, so a link that climbs out of the
    model tree is refused before the file is opened rather than after.
    """
    candidate = (base.parent / link).resolve()
    boundary = (root or base.parent).resolve()
    if not candidate.is_relative_to(boundary):
        raise ModelError(f"{base}: link {link!r} resolves outside the model root")
    return candidate


def _load_hierarchy(path: Path, dimension_name: str) -> Hierarchy:
    payload = _read_json(path)
    name = payload.get("Name")
    if not name:
        raise ModelError(f"{path}: hierarchy has no Name")
    elements = []
    for entry in payload.get("Elements", ()):
        element_name = entry.get("Name")
        if not element_name:
            raise ModelError(f"{path}: an element has no Name")
        element_type = entry.get("Type", NUMERIC)
        if element_type not in _ELEMENT_TYPES:
            raise ModelError(
                f"{path}: element {element_name!r} has type {element_type!r}, "
                f"expected one of {sorted(_ELEMENT_TYPES)}"
            )
        elements.append(Element(element_name, element_type))
    edges = []
    for entry in payload.get("Edges", ()):
        parent = entry.get("ParentName")
        component = entry.get("ComponentName")
        if not parent or not component:
            raise ModelError(f"{path}: an edge is missing ParentName or ComponentName")
        weight = _decimal(entry.get("Weight", 1), f"{path}: edge {parent} to {component}")
        edges.append(Edge(parent, component, weight))
    return Hierarchy(name, dimension_name, elements, edges, path)


def load_dimension(path: Path, root: Optional[Path] = None) -> Dimension:
    """Load one dimension and every hierarchy it links to."""
    payload = _read_json(path)
    name = payload.get("Name")
    if not name:
        raise ModelError(f"{path}: dimension has no Name")
    links = payload.get("Hierarchies@Code.links")
    if not links:
        raise ModelError(f"{path}: dimension {name!r} links no hierarchy file")
    hierarchies: dict[str, Hierarchy] = {}
    for link in links:
        hierarchy_path = _resolve_link(path, link, root)
        hierarchy = _load_hierarchy(hierarchy_path, name)
        if hierarchy.name in hierarchies:
            raise ModelError(f"{path}: hierarchy {hierarchy.name!r} is linked twice")
        hierarchies[hierarchy.name] = hierarchy
    return Dimension(name, hierarchies, path)


def load_cube(path: Path, root: Optional[Path] = None) -> Cube:
    """Load one cube, resolving its dimension links and its rules file."""
    payload = _read_json(path)
    name = payload.get("Name")
    if not name:
        raise ModelError(f"{path}: cube has no Name")
    dimension_links = payload.get("Dimensions@Code.links")
    if not dimension_links:
        raise ModelError(f"{path}: cube {name!r} links no dimensions")
    dimensions = []
    for link in dimension_links:
        dimension_path = _resolve_link(path, link, root)
        if not dimension_path.is_file():
            raise ModelError(f"{path}: linked dimension file {link!r} not found")
        dimensions.append(_read_json(dimension_path).get("Name") or dimension_path.stem)
    rules = None
    rules_source = None
    rules_link = payload.get("Rules@Code.link")
    if rules_link:
        rules_source = _resolve_link(path, rules_link, root)
        if not rules_source.is_file():
            raise ModelError(f"{path}: cube {name!r} links rules file {rules_link!r}, which is missing")
        rules = parse_rules(rules_source.read_text(encoding="utf-8-sig"), rules_source)
    return Cube(name, tuple(dimensions), rules, path, rules_source)


def load_process(path: Path, root: Optional[Path] = None) -> Process:
    """Load one TurboIntegrator process and the script text linked beside it."""
    payload = _read_json(path)
    name = payload.get("Name")
    if not name:
        raise ModelError(f"{path}: process has no Name")
    script = ""
    script_source = None
    script_link = payload.get("Code@Code.link")
    if script_link:
        script_source = _resolve_link(path, script_link, root)
        if not script_source.is_file():
            raise ModelError(f"{path}: process {name!r} links script {script_link!r}, which is missing")
        script = script_source.read_text(encoding="utf-8-sig")
    parameters = tuple(payload.get("Parameters", ()))
    datasource = payload.get("DataSource", {})
    return Process(name, parameters, datasource, script, path, script_source)


class Model:
    """A whole model tree: its manifest, dimensions, cubes and processes."""

    def __init__(
        self,
        name: str,
        root: Path,
        dimensions: dict[str, Dimension],
        cubes: dict[str, Cube],
        processes: dict[str, Process],
        files: Iterable[Path],
    ) -> None:
        self.name = name
        self.root = root
        self.dimensions = dimensions
        self.cubes = cubes
        self.processes = processes
        self.files: tuple[Path, ...] = tuple(files)

    def hierarchy(self, dimension: str) -> Hierarchy:
        try:
            return self.dimensions[dimension].default_hierarchy
        except KeyError:
            raise ModelError(f"no dimension named {dimension!r} in model {self.name!r}") from None


def load_model(root: Path) -> Model:
    """Load a model from the directory holding its tm1project.json manifest."""
    root = Path(root)
    manifest_path = root / "tm1project.json"
    manifest = _read_json(manifest_path)
    name = manifest.get("Name") or root.name
    objects = manifest.get("Objects", {})
    if not isinstance(objects, dict):
        raise ModelError(f"{manifest_path}: Objects must be an object")

    files: list[Path] = [manifest_path]
    dimensions: dict[str, Dimension] = {}
    for link in objects.get("Dimensions", ()):
        path = _resolve_link(manifest_path, link, root)
        if not path.is_file():
            raise ModelError(f"{manifest_path}: lists {link!r}, which is not on disk")
        dimension = load_dimension(path, root)
        dimensions[dimension.name] = dimension
        files.append(path)
        files.extend(hierarchy.source for hierarchy in dimension.hierarchies.values())

    cubes: dict[str, Cube] = {}
    for link in objects.get("Cubes", ()):
        path = _resolve_link(manifest_path, link, root)
        if not path.is_file():
            raise ModelError(f"{manifest_path}: lists {link!r}, which is not on disk")
        cube = load_cube(path, root)
        cubes[cube.name] = cube
        files.append(path)
        if cube.rules_source is not None:
            files.append(cube.rules_source)

    processes: dict[str, Process] = {}
    for link in objects.get("Processes", ()):
        path = _resolve_link(manifest_path, link, root)
        if not path.is_file():
            raise ModelError(f"{manifest_path}: lists {link!r}, which is not on disk")
        process = load_process(path, root)
        processes[process.name] = process
        files.append(path)
        if process.script_source is not None:
            files.append(process.script_source)

    return Model(name, root, dimensions, cubes, processes, files)
