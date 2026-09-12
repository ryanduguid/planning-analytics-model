"""Read long format CSV input into cube coordinates, a file or a directory at a time.

The engine reads the same synthetic CSV files that the TurboIntegrator
processes read on a real server. A row carries one leading column per cube
dimension, in the cube's dimension order, then a value column. Every row is
checked before it is used, and every error names the file and the row.

``load_csv`` and ``load_into_store`` read one file into one cube. ``load_data``
reads a whole directory, which is what the command line points at, and decides
which cube each file feeds.
"""

from __future__ import annotations

import csv
from decimal import Decimal
from pathlib import Path
from typing import Iterator

from pacioliscube.errors import EXIT_USAGE, CliError
from pacioliscube.evaluate import CellStore
from pacioliscube.model import Cube, Model, ModelError
from pacioliscube.rules import decimal_or_raise

Coordinate = tuple[str, ...]

# The stem of a data file names the cube it feeds. The shipped example files are
# listed rather than derived, because pnl-direct is not spelled like the cube it
# loads. A stem that matches a cube name is taken as well, which is how a model
# built for a test feeds cubes this map has never heard of, and load_data
# refuses the case that makes the fallback dangerous: two files, one cube.
CUBE_BY_STEM = {
    "drivers": "Drivers",
    "workforce": "Workforce",
    "revenue": "Revenue",
    "capex": "Capex",
    "pnl-direct": "PnL",
}


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
            value = decimal_or_raise(text, f"{path} row {number}", ModelError, "value ")
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


def cube_for_file(model: Model, path: Path) -> Cube:
    """Which cube a data file loads into, by its stem."""
    stem = path.stem.casefold()
    named = CUBE_BY_STEM.get(stem)
    if named is not None and named in model.cubes:
        return model.cubes[named]
    for cube in model.cubes.values():
        if cube.name.casefold() == stem:
            return cube
    raise CliError(
        EXIT_USAGE,
        f"{path}: no cube in model {model.name!r} matches the file name {path.stem!r}",
    )


def load_data(model: Model, directory: Path) -> CellStore:
    """Load every CSV in a directory into one store, one file per cube."""
    store = CellStore()
    try:
        boundary = directory.resolve()
        files = sorted(
            path
            for path in directory.iterdir()
            if path.is_file() and path.suffix.casefold() == ".csv"
        )
    except OSError as error:
        raise CliError(
            EXIT_USAGE, f"{directory}: the data directory could not be read, {error}"
        ) from error
    if not files:
        raise CliError(EXIT_USAGE, f"{directory}: there are no CSV files there")

    # Every file is matched to its cube before any of them is read. A cell store
    # takes the last write, so two files feeding one cube would leave the cells
    # they share holding whichever file sorted second, and the run would print a
    # wrong figure and exit 0. Guessing which of the two was meant is worse than
    # saying that both are there.
    feeding: dict[str, list[Path]] = {}
    for path in files:
        if not path.resolve().is_relative_to(boundary):
            raise CliError(EXIT_USAGE, f"{path}: this file resolves outside the data directory")
        feeding.setdefault(cube_for_file(model, path).name, []).append(path)
    for cube_name, paths in feeding.items():
        if len(paths) > 1:
            raise CliError(
                EXIT_USAGE,
                f"cube {cube_name!r} is fed by more than one file: "
                f"{', '.join(str(path) for path in paths)}. "
                "Leave the one that belongs to this run in the data directory and move "
                "the rest out",
            )

    for cube_name, paths in feeding.items():
        path = paths[0]
        try:
            load_into_store(model, cube_name, path, store)
        except ModelError as error:
            raise CliError(EXIT_USAGE, str(error)) from error
        except UnicodeDecodeError as error:
            raise CliError(
                EXIT_USAGE,
                f"{path}: this file is not UTF-8 text ({error}). "
                "A spreadsheet export saves as UTF-8 from its own save dialogue",
            ) from error
        except OSError as error:
            raise CliError(EXIT_USAGE, f"{path}: this file could not be read, {error}") from error
    return store
