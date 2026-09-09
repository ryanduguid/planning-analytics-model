"""Evaluate rules over leaf cells and consolidate along weighted hierarchy edges.

Two ideas carry most of the behaviour here.

First, a rule area lists element names, not dimension positions. ``['Budget',
'Amount']`` means the Budget element of whichever dimension holds it and the
Amount element of whichever dimension holds that. Resolution happens against
the cube being calculated, and an element that no dimension of the cube holds
is a model error rather than a cell that silently never matches.

Second, a value is produced on demand. Asking for a cell finds the rule that
matches it, and evaluating that rule asks for further cells. A cell with no
matching rule is either a stored leaf value or, where any element in the
coordinate is consolidated, the weighted sum of the cells beneath it.
"""

from __future__ import annotations

from decimal import Decimal
from typing import Iterable, Iterator, Optional

from pacioliscube.model import Cube, Model, ModelError
from pacioliscube.rules import (
    Area,
    BinaryOp,
    CellRef,
    Comparison,
    Expr,
    IfExpr,
    Number,
    Rule,
)

ZERO = Decimal("0")

# A rule area with several unnamed dimensions expands to the product of their
# leaves. The cap turns a modelling mistake into a clear error rather than a
# machine that stops responding.
MAX_MATERIALISED_CELLS = 2_000_000

Coordinate = tuple[str, ...]


class EvaluationError(ModelError):
    """Raised when a rule cannot be evaluated for a cell."""


class CircularReference(EvaluationError):
    """Raised when a cell's value depends, directly or not, on itself."""


class CellStore:
    """Cell values keyed by cube and coordinate, matched case insensitively."""

    def __init__(self) -> None:
        self._values: dict[tuple[str, Coordinate], Decimal] = {}
        self._spelling: dict[tuple[str, Coordinate], Coordinate] = {}

    @staticmethod
    def _key(cube: str, coordinate: Coordinate) -> tuple[str, Coordinate]:
        return cube.casefold(), tuple(element.casefold() for element in coordinate)

    def set(self, cube: str, coordinate: Coordinate, value: Decimal) -> None:
        if not isinstance(value, Decimal):
            raise TypeError("cell values must be Decimal, never float")
        key = self._key(cube, coordinate)
        self._values[key] = value
        self._spelling[key] = tuple(coordinate)

    def get(self, cube: str, coordinate: Coordinate) -> Decimal:
        return self._values.get(self._key(cube, coordinate), ZERO)

    def has(self, cube: str, coordinate: Coordinate) -> bool:
        return self._key(cube, coordinate) in self._values

    def items(self, cube: Optional[str] = None) -> Iterator[tuple[str, Coordinate, Decimal]]:
        for key, value in self._values.items():
            cube_key = key[0]
            if cube is not None and cube_key != cube.casefold():
                continue
            yield cube_key, self._spelling[key], value

    def copy(self) -> "CellStore":
        clone = CellStore()
        clone._values = dict(self._values)
        clone._spelling = dict(self._spelling)
        return clone


def _dimension_of(model: Model, cube: Cube, element: str) -> int:
    """Find which of a cube's dimensions holds an element, as a rule area does."""
    matches = [
        index
        for index, dimension in enumerate(cube.dimensions)
        if model.hierarchy(dimension).has(element)
    ]
    if not matches:
        raise ModelError(
            f"cube {cube.name!r}: no dimension holds an element named {element!r}"
        )
    if len(matches) > 1:
        holders = ", ".join(cube.dimensions[index] for index in matches)
        raise ModelError(
            f"cube {cube.name!r}: element {element!r} is ambiguous, held by {holders}"
        )
    return matches[0]


def _area_positions(model: Model, cube: Cube, area: Area) -> dict[int, str]:
    """Map an area's element names to positions in the cube's dimension order."""
    positions: dict[int, str] = {}
    for group in area.selectors:
        for element in group:
            index = _dimension_of(model, cube, element)
            canonical = model.hierarchy(cube.dimensions[index]).resolve(element)
            if index in positions and positions[index] != canonical:
                raise ModelError(
                    f"cube {cube.name!r}: area names two elements of dimension "
                    f"{cube.dimensions[index]!r}"
                )
            positions[index] = canonical
    return positions


def _is_leaf_cell(model: Model, cube: Cube, coordinate: Coordinate) -> bool:
    return all(
        model.hierarchy(dimension).is_leaf(element)
        for dimension, element in zip(cube.dimensions, coordinate)
    )


class _Engine:
    def __init__(self, model: Model, store: CellStore) -> None:
        self.model = model
        self.store = store
        self.memo: dict[tuple[str, Coordinate], Decimal] = {}
        self.visiting: list[str] = []
        self._positions: dict[tuple[str, int], dict[int, str]] = {}

    def rule_positions(self, cube: Cube, index: int, rule: Rule) -> dict[int, str]:
        key = (cube.name, index)
        if key not in self._positions:
            self._positions[key] = _area_positions(self.model, cube, rule.area)
        return self._positions[key]

    def matching_rule(self, cube: Cube, coordinate: Coordinate) -> Optional[Rule]:
        if cube.rules is None:
            return None
        leaf = _is_leaf_cell(self.model, cube, coordinate)
        for index, rule in enumerate(cube.rules.rules):
            if rule.area.qualifier == "N" and not leaf:
                continue
            if rule.area.qualifier == "C" and leaf:
                continue
            positions = self.rule_positions(cube, index, rule)
            if all(
                coordinate[position].casefold() == element.casefold()
                for position, element in positions.items()
            ):
                return rule
        return None

    def value(self, cube_name: str, coordinate: Coordinate) -> Decimal:
        cube = self.model.cubes.get(cube_name)
        if cube is None:
            raise ModelError(f"no cube named {cube_name!r} in model {self.model.name!r}")
        if len(coordinate) != len(cube.dimensions):
            raise ModelError(
                f"coordinate has {len(coordinate)} elements; "
                f"cube {cube.name!r} has {len(cube.dimensions)} dimensions"
            )
        canonical = tuple(
            self.model.hierarchy(dimension).resolve(element)
            for dimension, element in zip(cube.dimensions, coordinate)
        )
        key = (cube.name, tuple(element.casefold() for element in canonical))
        if key in self.memo:
            return self.memo[key]
        label = f"{cube.name}{list(canonical)}"
        if label in self.visiting:
            chain = " -> ".join(self.visiting[self.visiting.index(label):] + [label])
            raise CircularReference(f"a rule depends on its own result: {chain}")
        self.visiting.append(label)
        try:
            rule = self.matching_rule(cube, canonical)
            if rule is not None:
                result = self.evaluate_expression(rule.expression, cube, canonical)
            elif _is_leaf_cell(self.model, cube, canonical):
                result = self.store.get(cube.name, canonical)
            else:
                result = self.consolidated(cube, canonical)
        finally:
            self.visiting.pop()
        self.memo[key] = result
        return result

    def consolidated(self, cube: Cube, coordinate: Coordinate) -> Decimal:
        """Sum the cells beneath the first consolidated element in the coordinate."""
        for position, (dimension, element) in enumerate(zip(cube.dimensions, coordinate)):
            hierarchy = self.model.hierarchy(dimension)
            if hierarchy.is_leaf(element):
                continue
            total = ZERO
            for edge in hierarchy.children(element):
                child = list(coordinate)
                child[position] = hierarchy.resolve(edge.component)
                total += edge.weight * self.value(cube.name, tuple(child))
            return total
        return self.store.get(cube.name, coordinate)

    def evaluate_expression(self, expression: Expr, cube: Cube, coordinate: Coordinate) -> Decimal:
        if isinstance(expression, Number):
            return expression.value
        if isinstance(expression, CellRef):
            target_cube, target_coordinate = self.resolve_reference(expression, cube, coordinate)
            return self.value(target_cube, target_coordinate)
        if isinstance(expression, BinaryOp):
            left = self.evaluate_expression(expression.left, cube, coordinate)
            right = self.evaluate_expression(expression.right, cube, coordinate)
            if expression.op == "+":
                return left + right
            if expression.op == "-":
                return left - right
            if expression.op == "*":
                return left * right
            if expression.op == "\\":
                # TM1's backslash is the safe divide: a zero divisor gives zero.
                return ZERO if right == ZERO else left / right
            if expression.op == "/":
                if right == ZERO:
                    raise EvaluationError(
                        f"cube {cube.name!r} at {list(coordinate)}: division by zero. "
                        "Use the backslash operator where a zero divisor is expected."
                    )
                return left / right
            raise EvaluationError(f"unsupported operator {expression.op!r}")
        if isinstance(expression, IfExpr):
            if self.evaluate_condition(expression.condition, cube, coordinate):
                return self.evaluate_expression(expression.then_expr, cube, coordinate)
            return self.evaluate_expression(expression.else_expr, cube, coordinate)
        if isinstance(expression, Comparison):
            return Decimal("1") if self.evaluate_condition(expression, cube, coordinate) else ZERO
        raise EvaluationError(f"unsupported expression node {type(expression).__name__}")

    def evaluate_condition(self, condition: Comparison, cube: Cube, coordinate: Coordinate) -> bool:
        left = self.evaluate_expression(condition.left, cube, coordinate)
        right = self.evaluate_expression(condition.right, cube, coordinate)
        if condition.op == "=":
            return left == right
        if condition.op == "<>":
            return left != right
        if condition.op == "<":
            return left < right
        if condition.op == ">":
            return left > right
        if condition.op == "<=":
            return left <= right
        if condition.op == ">=":
            return left >= right
        raise EvaluationError(f"unsupported comparison {condition.op!r}")

    def resolve_reference(
        self, reference: CellRef, cube: Cube, coordinate: Coordinate
    ) -> tuple[str, Coordinate]:
        if reference.cube is None:
            target = list(coordinate)
            for element in reference.coordinates:
                position = _dimension_of(self.model, cube, element)
                target[position] = self.model.hierarchy(cube.dimensions[position]).resolve(element)
            return cube.name, tuple(target)

        target_cube = self.model.cubes.get(reference.cube)
        if target_cube is None:
            raise ModelError(
                f"cube {cube.name!r}: a rule reads cube {reference.cube!r}, which the model does not hold"
            )
        if len(reference.coordinates) != len(target_cube.dimensions):
            raise ModelError(
                f"cube {cube.name!r}: DB('{reference.cube}', ...) passes "
                f"{len(reference.coordinates)} coordinates for a cube of "
                f"{len(target_cube.dimensions)} dimensions"
            )
        resolved: list[str] = []
        for position, argument in enumerate(reference.coordinates):
            dimension = target_cube.dimensions[position]
            if argument.startswith("!"):
                source_dimension = argument[1:]
                if source_dimension not in cube.dimensions:
                    raise ModelError(
                        f"cube {cube.name!r}: !{source_dimension} names a dimension the cube does not have"
                    )
                element = coordinate[cube.dimensions.index(source_dimension)]
            else:
                element = argument
            resolved.append(self.model.hierarchy(dimension).resolve(element))
        return target_cube.name, tuple(resolved)


def _rule_targets(model: Model, cube: Cube, rule: Rule, index: int) -> Iterator[Coordinate]:
    """Every leaf cell a rule with an N or empty qualifier calculates."""
    positions = _area_positions(model, cube, rule.area)
    choices: list[tuple[str, ...]] = []
    total = 1
    for position, dimension in enumerate(cube.dimensions):
        if position in positions:
            choices.append((positions[position],))
        else:
            hierarchy = model.hierarchy(dimension)
            leaves = tuple(name for name in hierarchy.elements if hierarchy.is_leaf(name))
            choices.append(leaves)
        total *= len(choices[-1])
        if total > MAX_MATERIALISED_CELLS:
            raise EvaluationError(
                f"cube {cube.name!r} rule at line {rule.source_line} expands to more than "
                f"{MAX_MATERIALISED_CELLS} cells. Name more elements in its area."
            )

    def walk(position: int, built: list[str]) -> Iterator[Coordinate]:
        if position == len(choices):
            yield tuple(built)
            return
        for element in choices[position]:
            yield from walk(position + 1, built + [element])

    yield from walk(0, [])


def evaluate(model: Model, store: CellStore) -> CellStore:
    """Return a store holding the input values plus every rule calculated leaf cell."""
    result = store.copy()
    engine = _Engine(model, store)
    for cube in model.cubes.values():
        if cube.rules is None:
            continue
        for index, rule in enumerate(cube.rules.rules):
            if rule.area.qualifier == "C":
                continue
            for coordinate in _rule_targets(model, cube, rule, index):
                result.set(cube.name, coordinate, engine.value(cube.name, coordinate))
    return result


def consolidate(model: Model, store: CellStore, cube: str, coordinate: Coordinate) -> Decimal:
    """Resolve a coordinate that may name consolidated elements, applying C rules."""
    return _Engine(model, store).value(cube, coordinate)


def consolidate_many(
    model: Model, store: CellStore, cells: Iterable[tuple[str, Coordinate]]
) -> Iterator[Decimal]:
    """Yield a batch in order, sharing cached values only within this iteration.

    Keep the model and store unchanged until the batch has been consumed.
    """
    engine = _Engine(model, store)
    for cube, coordinate in cells:
        yield engine.value(cube, coordinate)
