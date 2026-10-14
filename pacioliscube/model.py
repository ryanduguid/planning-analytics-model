"""Load an IBM Planning Analytics model from its git native source tree.

The layout this module reads is the one the TM1 database itself publishes when
it pushes a model to git: a ``tm1project.json`` manifest at the root, then
``dimensions/``, ``cubes/`` and ``processes/`` folders where each object is a
JSON file, and where rule and TurboIntegrator text sits beside it in a plain
text file referenced by a ``@Code.link`` property. A cube names its dimensions
as IBM's TM1 source specification shows, ``{"@id": "Dimensions('Year')"}``.
The earlier form of this repository's model, a ``Dimensions@Code.links`` list
of dimension files, is still read.

Nothing here touches a network. The whole tree is data on disk.
"""

from __future__ import annotations

import json
import re
from decimal import Decimal
from pathlib import Path
from typing import Iterable, NamedTuple, Optional

from pacioliscube.rules import RuleSet, decimal_or_raise, parse_rules

CONSOLIDATED = "Consolidated"
NUMERIC = "Numeric"
STRING = "String"

_ELEMENT_TYPES = {CONSOLIDATED, NUMERIC, STRING}
# How the server references a dimension from a cube: Dimensions('Name'), with a
# single quote inside the name doubled as OData requires.
_DIMENSION_ID = re.compile(r"Dimensions\('((?:[^']|'')+)'\)")


class ModelError(ValueError):
    """Raised when model source is missing, malformed or self contradictory."""


class Element(NamedTuple):
    name: str
    element_type: str


class Edge(NamedTuple):
    parent: str
    component: str
    weight: Decimal


def _read_json(path: Path) -> dict:
    if not path.is_file():
        raise ModelError(f"{path}: file not found")
    # Read outside the try: the command line reports a file that is not UTF-8.
    text = path.read_text(encoding="utf-8-sig")
    try:
        payload = json.loads(text)
    except json.JSONDecodeError as error:
        raise ModelError(f"{path}: invalid JSON, {error}") from error
    except RecursionError as error:
        raise ModelError(f"{path}: invalid JSON, nested too deeply to read") from error
    except ValueError as error:
        # The interpreter's integer-digit limit, among others.
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
            if current in seen:
                return
            seen.add(current)
            if self.is_leaf(current):
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
    if not isinstance(link, str):
        raise ModelError(f"{base}: link must be a string")
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
    if not isinstance(name, str):
        raise ModelError(f"{path}: hierarchy Name must be a string")
    for collection in ("Elements", "Edges"):
        if collection in payload and not isinstance(payload[collection], list):
            raise ModelError(f"{path}: hierarchy {collection} must be a list")
    elements = []
    for entry in payload.get("Elements", ()):
        if not isinstance(entry, dict):
            raise ModelError(f"{path}: an element is {type(entry).__name__}, expected an object")
        element_name = entry.get("Name")
        if not element_name:
            raise ModelError(f"{path}: an element has no Name")
        if not isinstance(element_name, str):
            raise ModelError(f"{path}: element Name must be a string")
        element_type = entry.get("Type", NUMERIC)
        if not isinstance(element_type, str) or element_type not in _ELEMENT_TYPES:
            raise ModelError(
                f"{path}: element {element_name!r} has type {element_type!r}, "
                f"expected one of {sorted(_ELEMENT_TYPES)}"
            )
        elements.append(Element(element_name, element_type))
    edges = []
    for entry in payload.get("Edges", ()):
        if not isinstance(entry, dict):
            raise ModelError(f"{path}: an edge is {type(entry).__name__}, expected an object")
        parent = entry.get("ParentName")
        component = entry.get("ComponentName")
        if not parent or not component:
            raise ModelError(f"{path}: an edge is missing ParentName or ComponentName")
        if not isinstance(parent, str) or not isinstance(component, str):
            raise ModelError(f"{path}: edge ParentName and ComponentName must be strings")
        where = f"{path}: edge {parent} to {component}"
        text = entry.get("Weight", 1)
        weight = decimal_or_raise(text, where, ModelError)
        if not weight.is_finite():
            raise ModelError(f"{where}: {text!r} is not a finite number")
        edges.append(Edge(parent, component, weight))
    return Hierarchy(name, dimension_name, elements, edges, path)


def load_dimension(path: Path, root: Optional[Path] = None) -> Dimension:
    """Load one dimension and every hierarchy it links to."""
    payload = _read_json(path)
    name = payload.get("Name")
    if not name:
        raise ModelError(f"{path}: dimension has no Name")
    if not isinstance(name, str):
        raise ModelError(f"{path}: dimension Name must be a string")
    links = payload.get("Hierarchies@Code.links")
    if links is not None and not isinstance(links, list):
        raise ModelError(f"{path}: dimension {name!r} Hierarchies@Code.links must be a list")
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


def _dimension_from_reference(path: Path, cube: str, reference: object) -> str:
    """Return the dimension name in a ``{"@id": "Dimensions('X')"}`` reference.

    OData doubles a single quote inside a key, so ``Dimensions('O''Brien')``
    names the dimension ``O'Brien``.
    """
    identifier = reference.get("@id") if isinstance(reference, dict) else None
    match = _DIMENSION_ID.fullmatch(identifier) if isinstance(identifier, str) else None
    if match is None:
        raise ModelError(
            f"{path}: cube {cube!r} has dimension reference {reference!r}, "
            "expected {\"@id\": \"Dimensions('Name')\"}"
        )
    return match.group(1).replace("''", "'")


def load_cube(path: Path, root: Optional[Path] = None) -> Cube:
    """Load one cube, resolving its dimensions and its rules file.

    The TM1 source specification gives a cube's dimensions as ``@id`` references
    by name. This repository's earlier form linked them as
    ``../dimensions/X.json`` instead, so a cube read on its own is fenced to the
    tree holding ``cubes/`` rather than to ``cubes/`` itself, which no such cube
    could satisfy. Loading a whole model still passes the manifest's own root.
    """
    if root is None:
        root = path.parent.parent
    payload = _read_json(path)
    name = payload.get("Name")
    if not name:
        raise ModelError(f"{path}: cube has no Name")
    if not isinstance(name, str):
        raise ModelError(f"{path}: cube Name must be a string")
    # Presence, not truthiness: an empty or null Dimensions beside legacy links is
    # still a contradiction, and must not fall through to the links unnoticed.
    if "Dimensions" in payload and "Dimensions@Code.links" in payload:
        raise ModelError(
            f"{path}: cube {name!r} gives its dimensions both as Dimensions and as "
            "Dimensions@Code.links; keep one"
        )
    form = "Dimensions" if "Dimensions" in payload else "Dimensions@Code.links"
    entries = payload.get(form)
    if entries is not None and not isinstance(entries, list):
        raise ModelError(f"{path}: cube {name!r} {form} must be a list")
    if not entries:
        raise ModelError(f"{path}: cube {name!r} names no dimensions")
    dimensions = []
    if form == "Dimensions":
        dimensions = [_dimension_from_reference(path, name, entry) for entry in entries]
    else:
        for link in entries:
            dimension_path = _resolve_link(path, link, root)
            if not dimension_path.is_file():
                raise ModelError(f"{path}: linked dimension file {link!r} not found")
            dimension_name = _read_json(dimension_path).get("Name") or dimension_path.stem
            if not isinstance(dimension_name, str):
                raise ModelError(f"{dimension_path}: dimension Name must be a string")
            dimensions.append(dimension_name)
    rules = None
    rules_source = None
    rules_link = payload.get("Rules@Code.link")
    if rules_link is not None and not isinstance(rules_link, str):
        raise ModelError(f"{path}: link must be a string")
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
    if not isinstance(name, str):
        raise ModelError(f"{path}: process Name must be a string")
    script = ""
    script_source = None
    script_link = payload.get("Code@Code.link")
    if script_link is not None and not isinstance(script_link, str):
        raise ModelError(f"{path}: link must be a string")
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
    for kind in ("Dimensions", "Cubes", "Processes"):
        if kind in objects and not isinstance(objects[kind], list):
            raise ModelError(f"{manifest_path}: Objects.{kind} must be a list")

    files: list[Path] = [manifest_path]
    dimensions: dict[str, Dimension] = {}
    for link in objects.get("Dimensions", ()):
        path = _resolve_link(manifest_path, link, root)
        if not path.is_file():
            raise ModelError(f"{manifest_path}: lists {link!r}, which is not on disk")
        dimension = load_dimension(path, root)
        if dimension.name in dimensions:
            raise ModelError(
                f"{path}: dimension {dimension.name!r} is also declared in "
                f"{dimensions[dimension.name].source}"
            )
        dimensions[dimension.name] = dimension
        files.append(path)
        files.extend(hierarchy.source for hierarchy in dimension.hierarchies.values())

    cubes: dict[str, Cube] = {}
    cube_names: dict[str, Cube] = {}
    for link in objects.get("Cubes", ()):
        path = _resolve_link(manifest_path, link, root)
        if not path.is_file():
            raise ModelError(f"{manifest_path}: lists {link!r}, which is not on disk")
        cube = load_cube(path, root)
        key = cube.name.casefold()
        if key in cube_names:
            previous = cube_names[key]
            raise ModelError(
                f"{path}: cube {cube.name!r} conflicts with {previous.name!r} "
                f"declared in {previous.source}"
            )
        cube_names[key] = cube
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
        if process.name in processes:
            raise ModelError(
                f"{path}: process {process.name!r} is also declared in "
                f"{processes[process.name].source}"
            )
        processes[process.name] = process
        files.append(path)
        if process.script_source is not None:
            files.append(process.script_source)

    return Model(name, root, dimensions, cubes, processes, files)
