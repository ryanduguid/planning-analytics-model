"""Read long format CSV input into cube coordinates.

The engine reads the same synthetic CSV files that the TurboIntegrator
processes read on a real server. A row carries one leading column per cube
dimension, in the cube's dimension order, then a value column. Every row is
checked before it is used, and every error names the file and the row.
"""

from __future__ import annotations

import csv
from decimal import Decimal, InvalidOperation
from pathlib import Path
from typing import Iterator

from pacioliscube.model import Cube, Model, ModelError

Coordinate = tuple[str, ...]


def load_csv(path: Path, cube: Cube, model: Model) -> Iterator[tuple[Coordinate, Decimal]]:
    """Yield one coordinate and value per data row of a long format CSV."""
    path = Path(path)
    if not path.is_file():
        raise ModelError(f"{path}: file not found")
    width = len(cube.dimensions) + 1
    with path.open("r", encoding="utf-8-sig", newline="") as handle:
        reader = csv.reader(handle)
        try:
            header = next(reader)
        except StopIteration:
            raise ModelError(f"{path}: the file is empty, expected a header row") from None
        if len(header) != width:
            raise ModelError(
                f"{path} row 1: the header has {len(header)} columns but cube {cube.name!r} "
                f"needs {width}, being {', '.join(cube.dimensions)} then a value"
            )
        seen: dict[Coordinate, tuple[Decimal, int]] = {}
        for number, row in enumerate(reader, start=2):
            if not row or all(field.strip() == "" for field in row):
                continue
            if len(row) != width:
                raise ModelError(
                    f"{path} row {number}: {len(row)} columns, expected {width}"
                )
            coordinate = []
            for position, field in enumerate(row[:-1]):
                dimension = cube.dimensions[position]
                element = field.strip()
                try:
                    hierarchy = model.hierarchy(dimension)
                    canonical = hierarchy.resolve(element)
                    if not hierarchy.is_leaf(canonical):
                        raise ModelError(
                            f"{dimension}: element {canonical!r} is consolidated and cannot hold input"
                        )
                    coordinate.append(canonical)
                except ModelError as error:
                    raise ModelError(f"{path} row {number}: {error}") from None
            text = row[-1].strip()
            try:
                value = Decimal(text)
            except InvalidOperation:
                raise ModelError(
                    f"{path} row {number}: value {text!r} is not a number"
                ) from None
            if not value.is_finite():
                raise ModelError(
                    f"{path} row {number}: value {text!r} is not a finite number"
                )
            cell = tuple(coordinate)
            previous = seen.get(cell)
            if previous is not None and previous[0] != value:
                raise ModelError(
                    f"{path} row {number}: conflicting value for {cube.name!r} "
                    f"at {list(cell)}; first supplied on row {previous[1]}"
                )
            if previous is not None:
                continue
            seen[cell] = (value, number)
            yield cell, value


def load_into_store(model: Model, cube_name: str, path: Path, store) -> int:
    """Load one CSV into a cell store and return how many cells it wrote."""
    cube = model.cubes.get(cube_name)
    if cube is None:
        raise ModelError(f"no cube named {cube_name!r} in model {model.name!r}")
    written = 0
    for coordinate, value in load_csv(path, cube, model):
        store.set(cube.name, coordinate, value)
        written += 1
    return written
