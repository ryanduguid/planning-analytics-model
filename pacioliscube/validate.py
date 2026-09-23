"""Structural checks over a loaded model.

Loading a model already refuses anything malformed. Validation is the second
pass: it looks for source that parses cleanly but cannot be right, such as a
rule naming an element no dimension of its cube holds, a cube with SKIPCHECK and
rules but no feeders, or a file on disk that the manifest never lists and which
would therefore be missing from a deployment.
"""

from __future__ import annotations

import re
from typing import Iterator, NamedTuple

from pacioliscube.model import Cube, Model
from pacioliscube.rules import Area, BinaryOp, CellRef, Comparison, Expr, IfExpr

ERROR = "error"
WARNING = "warning"

# TurboIntegrator names a parameter p followed by a capital, as in pYear.
# Requiring the capital keeps ordinary words like "per" out of the match.
_PARAMETER_PATTERN = re.compile(r"\bp[A-Z][A-Za-z0-9_]*\b")


class Finding(NamedTuple):
    severity: str
    code: str
    message: str
    location: str


def _walk(expression: Expr) -> Iterator[Expr]:
    yield expression
    if isinstance(expression, BinaryOp) or isinstance(expression, Comparison):
        yield from _walk(expression.left)
        yield from _walk(expression.right)
    elif isinstance(expression, IfExpr):
        yield from _walk(expression.condition)
        yield from _walk(expression.then_expr)
        yield from _walk(expression.else_expr)


def _area_elements(area: Area) -> tuple[str, ...]:
    return tuple(element for group in area.selectors for element in group)


def _check_elements_resolve(
    model: Model, cube: Cube, names: tuple[str, ...], location: str
) -> list[Finding]:
    findings: list[Finding] = []
    for element in names:
        holders = [
            dimension
            for dimension in cube.dimensions
            if dimension in model.dimensions and model.hierarchy(dimension).has(element)
        ]
        if not holders:
            findings.append(
                Finding(
                    ERROR,
                    "ELE001",
                    f"no dimension of cube {cube.name!r} holds an element named {element!r}",
                    location,
                )
            )
        elif len(holders) > 1:
            findings.append(
                Finding(
                    ERROR,
                    "ARE001",
                    f"element {element!r} is ambiguous in cube {cube.name!r}, held by "
                    + ", ".join(holders),
                    location,
                )
            )
    return findings


def _target_is_consolidated(model: Model, cube: Cube, area: Area) -> bool:
    for element in _area_elements(area):
        for dimension in cube.dimensions:
            if dimension in model.dimensions and model.hierarchy(dimension).has(element):
                if model.hierarchy(dimension).is_consolidated(element):
                    return True
    return False


def _validate_cube_dimensions(model: Model, cube: Cube) -> list[Finding]:
    """Report dimensions named by one cube but absent from the model."""
    findings: list[Finding] = []
    location = str(cube.source)
    for dimension in cube.dimensions:
        if dimension not in model.dimensions:
            findings.append(
                Finding(
                    ERROR,
                    "DIM001",
                    f"cube {cube.name!r} names dimension {dimension!r}, which the model does not hold",
                    location,
                )
            )
    return findings


def _validate_cube_rules(model: Model, cube: Cube) -> list[Finding]:
    """Report invalid rule targets and cell references for one ruled cube."""
    findings: list[Finding] = []
    rules_location = str(cube.rules_source)
    rules = cube.rules
    assert rules is not None, f"cube {cube.name!r} has no rules to validate"

    for rule in rules.rules:
        where = f"{rules_location} line {rule.source_line}"
        findings.extend(_check_elements_resolve(model, cube, _area_elements(rule.area), where))
        if rule.area.qualifier != "C" and _target_is_consolidated(model, cube, rule.area):
            findings.append(
                Finding(
                    ERROR,
                    "RUL001",
                    "a rule targets a consolidated element without a C qualifier, so it would "
                    "replace the consolidation of its children",
                    where,
                )
            )
        for node in _walk(rule.expression):
            if not isinstance(node, CellRef):
                continue
            if node.cube is None:
                findings.extend(_check_elements_resolve(model, cube, node.coordinates, where))
                continue
            target = model.cubes.get(node.cube)
            if target is None:
                findings.append(
                    Finding(
                        ERROR,
                        "DIM001",
                        f"a rule reads cube {node.cube!r}, which the model does not hold",
                        where,
                    )
                )
                continue
            if len(node.coordinates) != len(target.dimensions):
                findings.append(
                    Finding(
                        ERROR,
                        "ARE001",
                        f"DB('{node.cube}', ...) passes {len(node.coordinates)} coordinates for a "
                        f"cube of {len(target.dimensions)} dimensions",
                        where,
                    )
                )
                continue
            for position, argument in enumerate(node.coordinates):
                dimension = target.dimensions[position]
                if argument.startswith("!"):
                    if argument[1:] not in cube.dimensions:
                        findings.append(
                            Finding(
                                ERROR,
                                "ELE001",
                                f"{argument} names a dimension that cube {cube.name!r} does not have",
                                where,
                            )
                        )
                    continue
                if dimension in model.dimensions and not model.hierarchy(dimension).has(argument):
                    findings.append(
                        Finding(
                            ERROR,
                            "ELE001",
                            f"dimension {dimension!r} of cube {node.cube!r} has no element "
                            f"named {argument!r}",
                            where,
                        )
                    )

    return findings


def _validate_cube_feeders(model: Model, cube: Cube) -> list[Finding]:
    """Report invalid feeder sources and targets for one ruled cube."""
    findings: list[Finding] = []
    rules_location = str(cube.rules_source)
    rules = cube.rules
    assert rules is not None, f"cube {cube.name!r} has no feeders to validate"

    for feeder in rules.feeders:
        where = f"{rules_location} line {feeder.source_line}"
        findings.extend(_check_elements_resolve(model, cube, _area_elements(feeder.area), where))
        if feeder.target_cube is None:
            findings.extend(
                _check_elements_resolve(model, cube, _area_elements(feeder.target), where)
            )
            continue
        target = model.cubes.get(feeder.target_cube)
        if target is None:
            findings.append(
                Finding(
                    ERROR,
                    "DIM001",
                    f"a feeder points at cube {feeder.target_cube!r}, which the model does not hold",
                    where,
                )
            )
            continue
        coordinates = _area_elements(feeder.target)
        if len(coordinates) != len(target.dimensions):
            findings.append(
                Finding(
                    ERROR,
                    "ARE001",
                    f"DB('{feeder.target_cube}', ...) passes {len(coordinates)} coordinates for a "
                    f"cube of {len(target.dimensions)} dimensions",
                    where,
                )
            )
            continue
        # Paired with target.dimensions, because a coordinate names the element of the
        # dimension at its own position. Accepting an element that any target dimension
        # holds let DB('Sales', 'Units', 'Red') pass a cube ordered Colour then Measure,
        # and the swapped feeder reached deployment.
        for dimension, element in zip(target.dimensions, coordinates):
            if element.startswith("!"):
                if element[1:] not in cube.dimensions:
                    findings.append(
                        Finding(
                            ERROR,
                            "ELE001",
                            f"{element} names a dimension that cube {cube.name!r} does not have",
                            where,
                        )
                    )
                continue
            if dimension not in model.dimensions or not model.hierarchy(dimension).has(element):
                findings.append(
                    Finding(
                        ERROR,
                        "ELE001",
                        f"dimension {dimension!r} of cube {target.name!r} holds no element "
                        f"named {element!r}",
                        where,
                    )
                )

    return findings


def _validate_cube(model: Model, cube: Cube) -> list[Finding]:
    """Run the dimension, rule and feeder phases for one cube in order."""
    findings = _validate_cube_dimensions(model, cube)
    if cube.rules is None:
        return findings
    findings.extend(_validate_cube_rules(model, cube))
    findings.extend(_validate_cube_feeders(model, cube))
    return findings


def _fed_elements_by_cube(model: Model) -> dict[str, set[str]]:
    """Which elements each cube's cells are fed at, from every cube's feeders."""
    fed: dict[str, set[str]] = {}
    for cube in model.cubes.values():
        if cube.rules is None:
            continue
        for feeder in cube.rules.feeders:
            target_name = feeder.target_cube or cube.name
            bucket = fed.setdefault(target_name.casefold(), set())
            for element in _area_elements(feeder.target):
                if not element.startswith("!"):
                    bucket.add(element.casefold())
    return fed


def _constraints(model: Model, cube: Cube, area: Area) -> dict[str, set[str]]:
    """An area as the elements it names per dimension of `cube`.

    An element the model cannot place in one of the cube's dimensions constrains
    nothing and is left out. An element that exists in more than one dimension takes
    the first in the cube's own order, as the rest of this module does.
    """
    out: dict[str, set[str]] = {}
    for element in _area_elements(area):
        for dimension in cube.dimensions:
            if dimension in model.dimensions and model.hierarchy(dimension).has(element):
                out.setdefault(dimension, set()).add(element.casefold())
                break
    return out


def _positional_constraints(
    model: Model,
    cube: Cube,
    coordinates: tuple[str, ...],
    source: dict[str, set[str]],
) -> dict[str, set[str]]:
    """A DB() target, whose coordinate at each position names that dimension's element.

    `_constraints` would place an element found in two dimensions in the first, and
    the dimension the coordinate really names would then constrain nothing. A
    `!Dim` coordinate carries the current element of the source's `Dim`, so it
    takes whatever selection the feeder's source area makes on `Dim`, if any.
    """
    out: dict[str, set[str]] = {}
    for dimension, element in zip(cube.dimensions, coordinates):
        if element.startswith("!"):
            name = element[1:]
            if name in source:
                # A consolidation on the feeder's left feeds from every leaf below it,
                # so its leaves are part of the selection the target position takes.
                # `source` comes from _constraints, which holds model dimensions only.
                selected = set(source[name])
                for chosen in source[name]:
                    selected.update(
                        leaf.casefold() for leaf in model.hierarchy(name).leaves_under(chosen)
                    )
                out[dimension] = selected
        elif dimension in model.dimensions and model.hierarchy(dimension).has(element):
            out[dimension] = {element.casefold()}
    return out


def _may_intersect(rule: dict[str, set[str]], feeder: dict[str, set[str]]) -> bool:
    """False when some dimension both areas constrain has no element in common."""
    for dimension, elements in rule.items():
        other = feeder.get(dimension)
        if other is not None and not (elements & other):
            return False
    return True


def _fed_areas_by_cube(model: Model) -> dict[str, list[dict[str, set[str]]]]:
    """Every feeder target area, kept whole and per dimension of the cube it feeds."""
    by_name = {cube.name.casefold(): cube for cube in model.cubes.values()}
    fed: dict[str, list[dict[str, set[str]]]] = {}
    for cube in model.cubes.values():
        if cube.rules is None:
            continue
        for feeder in cube.rules.feeders:
            target_name = feeder.target_cube or cube.name
            target = by_name.get(target_name.casefold())
            if target is None:
                continue
            coordinates = _area_elements(feeder.target)
            positional = bool(feeder.target_cube) and len(coordinates) == len(target.dimensions)
            fed.setdefault(target_name.casefold(), []).append(
                _positional_constraints(
                    model, target, coordinates, _constraints(model, cube, feeder.area)
                )
                if positional
                else _constraints(model, target, feeder.target)
            )
    return fed


def _validate_feeding(model: Model) -> list[Finding]:
    """FED001 and FED002: is every calculated area fed, from this cube or another."""
    findings: list[Finding] = []
    fed = _fed_elements_by_cube(model)
    fed_areas_by_cube = _fed_areas_by_cube(model)
    for cube in model.cubes.values():
        if cube.rules is None:
            continue
        fed_here = fed.get(cube.name.casefold(), set())
        if cube.rules.skipcheck and cube.rules.rules and not fed_here:
            findings.append(
                Finding(
                    ERROR,
                    "FED001",
                    f"cube {cube.name!r} uses SKIPCHECK and has rules, but no feeder in any cube "
                    "points into it, so calculated cells would not appear in a real database",
                    str(cube.rules_source),
                )
            )
            continue
        fed_areas = fed_areas_by_cube.get(cube.name.casefold(), [])
        for rule in cube.rules.rules:
            # Whole areas, not a flat set of names. Sharing any one element name read
            # as coverage, so a rule for Budget Amount was treated as fed by a feeder
            # targeting Actual Amount even though their Version selections make the
            # areas disjoint and the calculated Budget cells stay unfed.
            rule_area = _constraints(model, cube, rule.area)
            names = {e.casefold() for e in _area_elements(rule.area) if not e.startswith("!")}
            # Where nothing in the area can be placed in a dimension, fall back to the
            # name test: ELE001 already reports the unknown element, and dropping the
            # warning as well would leave that rule with nothing said about its feeding.
            covered = (
                any(_may_intersect(rule_area, area) for area in fed_areas)
                if rule_area else bool(names & fed_here)
            )
            if names and not covered:
                findings.append(
                    Finding(
                        WARNING,
                        "FED002",
                        "no feeder, in this cube or any other, points at the area this rule "
                        "calculates. A warning rather than an error, because a model may feed "
                        "an area indirectly",
                        f"{cube.rules_source} line {rule.source_line}",
                    )
                )
    return findings


def _validate_manifest(model: Model) -> list[Finding]:
    findings: list[Finding] = []
    listed = {path.resolve() for path in model.files}
    for path in sorted(model.root.rglob("*")):
        if not path.is_file():
            continue
        if path.resolve() not in listed:
            findings.append(
                Finding(
                    ERROR,
                    "MAN001",
                    "this file sits inside the model tree but the manifest does not list it, so a "
                    "deployment would leave it behind",
                    str(path),
                )
            )
    return findings


def _strip_comments(script: str) -> str:
    """Drop TurboIntegrator comments so prose cannot look like code.

    A hash starts a comment unless it sits inside a quoted string, which is
    where MDX braces and file paths legitimately carry one.
    """
    lines = []
    for line in script.splitlines():
        quoted = False
        cut = len(line)
        for index, character in enumerate(line):
            if character == "'":
                quoted = not quoted
            elif character == "#" and not quoted:
                cut = index
                break
        lines.append(line[:cut])
    return "\n".join(lines)


def _validate_processes(model: Model) -> list[Finding]:
    findings: list[Finding] = []
    for process in model.processes.values():
        declared = {
            str(parameter.get("Name", "")).casefold()
            for parameter in process.parameters
            if parameter.get("Name")
        }
        used = {name.casefold() for name in _PARAMETER_PATTERN.findall(_strip_comments(process.script))}
        for name in sorted(used - declared):
            findings.append(
                Finding(
                    ERROR,
                    "PRC001",
                    f"the script uses {name} as a parameter but the process does not declare it",
                    str(process.script_source or process.source),
                )
            )
    return findings


def validate_model(model: Model) -> tuple[Finding, ...]:
    """Return every structural finding in a loaded model, errors and warnings alike."""
    findings: list[Finding] = []
    for cube in model.cubes.values():
        findings.extend(_validate_cube(model, cube))
    findings.extend(_validate_feeding(model))
    findings.extend(_validate_processes(model))
    findings.extend(_validate_manifest(model))
    return tuple(findings)
