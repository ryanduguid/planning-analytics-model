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
from pacioliscube.rules import Area, CellRef, Expr, IfExpr, BinaryOp, Comparison

ERROR = "error"
WARNING = "warning"

# Files that are part of the repository rather than the model, so their absence
# from the manifest is not a finding.
UNLISTED_FILE_ALLOWANCES = (".gitkeep", ".gitattributes", "README.md")

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

    for rule in cube.rules.rules:
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

    for feeder in cube.rules.feeders:
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
        for element in coordinates:
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
            if not any(
                dimension in model.dimensions and model.hierarchy(dimension).has(element)
                for dimension in target.dimensions
            ):
                findings.append(
                    Finding(
                        ERROR,
                        "ELE001",
                        f"no dimension of cube {target.name!r} holds an element named {element!r}",
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


def _validate_feeding(model: Model) -> list[Finding]:
    """FED001 and FED002: is every calculated area fed, from this cube or another."""
    findings: list[Finding] = []
    fed = _fed_elements_by_cube(model)
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
        for rule in cube.rules.rules:
            targets = {element.casefold() for element in _area_elements(rule.area)}
            if targets and not targets & fed_here:
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
        if path.name in UNLISTED_FILE_ALLOWANCES:
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
