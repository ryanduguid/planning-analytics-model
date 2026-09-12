"""The command line, being the way a shell script or a CI job drives this package.

Three subcommands. ``validate`` reports what is structurally wrong with a model
tree, ``calculate`` prints the value at named cells, and ``report`` prints a
small profit and loss. Every run reads: nothing here writes a file, touches a
network, or changes the model it is pointed at.

Argument handling and the three subcommands live here. The statement the report
prints is in report.py and reading a directory of CSV input is in data.py, so
this module is the part a reader consults for what the arguments mean and which
exit code a failure takes.

Exit codes, which a shell script or a CI job can rely on:

0   the command finished and found nothing wrong
1   a usage or input error, being a directory that is not there, a CSV that
    cannot be read, a cell reference that cannot be parsed, or a model the
    report subcommand does not fit
2   the model is not sound: it does not load, validation reports at least one
    error severity finding, or a fault the structural checks do not reach
    surfaces while a figure is being calculated. Every subcommand validates
    before it calculates, because a figure taken from a structurally broken
    model is worse than no figure at all
3   a calculation failed, which is a division by zero, a circular reference, or
    any other EvaluationError

Warnings never change the exit code. A warning is a finding the engine cannot
prove is wrong, so failing a build on one would make the check useless.
"""

from __future__ import annotations

import argparse
import sys
from decimal import Decimal
from pathlib import Path
from typing import Iterator, Optional, Sequence

from pacioliscube import __version__, report
from pacioliscube.data import load_data
from pacioliscube.errors import (
    EXIT_CALCULATION,
    EXIT_INVALID_MODEL,
    EXIT_OK,
    EXIT_USAGE,
    CliError,
    element_or_error,
)
from pacioliscube.evaluate import CellStore, EvaluationError, consolidate_many, evaluate
from pacioliscube.model import Cube, Model, ModelError, load_model
from pacioliscube.validate import ERROR, Finding, validate_model

DEFAULT_MODEL_ROOT = "model"


def _directory(argument: str, what: str) -> Path:
    path = Path(argument)
    if not path.is_dir():
        raise CliError(EXIT_USAGE, f"{path}: there is no {what} directory there")
    return path


def _model_root(args: argparse.Namespace) -> Path:
    """The model directory, given as the positional or left at its default."""
    return _directory(args.model_dir or DEFAULT_MODEL_ROOT, "model")


def _load(root: Path) -> Model:
    try:
        return load_model(root)
    except ModelError as error:
        raise CliError(EXIT_INVALID_MODEL, str(error)) from error
    except UnicodeDecodeError as error:
        # A decode error carries the bytes and not the path, and the loader does
        # not say which of the files it opened was being read, so the message
        # reports the tree and the byte that stopped it.
        raise CliError(
            EXIT_INVALID_MODEL,
            f"{root}: a file in the model tree is not UTF-8 text ({error}). "
            "Every file of a model tree is read as UTF-8",
        ) from error
    except OSError as error:
        raise CliError(
            EXIT_INVALID_MODEL, f"{root}: the model could not be read, {error}"
        ) from error


def _cube(model: Model, name: str) -> Cube:
    for cube in model.cubes.values():
        if cube.name.casefold() == name.casefold():
            return cube
    raise CliError(
        EXIT_USAGE,
        f"no cube named {name!r} in model {model.name!r}. "
        f"The model holds {', '.join(model.cubes) or 'no cubes'}",
    )


def _format_finding(finding: Finding) -> str:
    return f"{finding.severity} {finding.code} {finding.location}: {finding.message}"


def _summary(errors: int, warnings: int) -> str:
    return (
        f"{errors} {'error' if errors == 1 else 'errors'}, "
        f"{warnings} {'warning' if warnings == 1 else 'warnings'}"
    )


def _refuse_broken_model(model: Model) -> None:
    """Stop calculate and report before they read a model that cannot be right."""
    errors = [finding for finding in validate_model(model) if finding.severity == ERROR]
    if not errors:
        return
    for finding in errors:
        print(_format_finding(finding), file=sys.stderr)
    count = f"{len(errors)} {'error' if len(errors) == 1 else 'errors'}"
    raise CliError(
        EXIT_INVALID_MODEL,
        f"the model has {count}, so nothing was calculated. "
        "Run the validate subcommand for the findings in full",
    )


def _evaluate(model: Model, store: CellStore) -> CellStore:
    try:
        return evaluate(model, store)
    except EvaluationError as error:
        raise CliError(EXIT_CALCULATION, str(error)) from error
    except ModelError as error:
        # Validation has already passed, so a model error at this point is a
        # defect the structural checks do not reach rather than a bad argument.
        raise CliError(EXIT_INVALID_MODEL, str(error)) from error


def _values(
    model: Model, store: CellStore, cells: Sequence[tuple[str, tuple[str, ...]]]
) -> Iterator[Decimal]:
    try:
        yield from consolidate_many(model, store, cells)
    except EvaluationError as error:
        raise CliError(EXIT_CALCULATION, str(error)) from error
    except ModelError as error:
        # The same reasoning as _evaluate, and the same exit code for the same
        # exception. The coordinate reached here has already been resolved
        # element by element against the model, so what is left is a fault in
        # the model rather than a bad argument.
        raise CliError(EXIT_INVALID_MODEL, str(error)) from error


def _parse_cell(model: Model, text: str) -> tuple[str, tuple[str, ...]]:
    """Turn CUBE:element,element into a cube name and a canonical coordinate."""
    cube_name, separator, coordinate_text = text.partition(":")
    if not separator or not cube_name.strip() or not coordinate_text.strip():
        raise CliError(
            EXIT_USAGE,
            f"cell reference {text!r} is not in the form CUBE:element,element,...",
        )
    cube = _cube(model, cube_name.strip())
    names = [part.strip() for part in coordinate_text.split(",")]
    if not all(names):
        raise CliError(EXIT_USAGE, f"cell reference {text!r} has an empty element name")
    if len(names) != len(cube.dimensions):
        raise CliError(
            EXIT_USAGE,
            f"cell reference {text!r} gives {len(names)} elements for cube {cube.name!r}, "
            f"which takes {len(cube.dimensions)}, being {', '.join(cube.dimensions)}",
        )
    coordinate = tuple(
        element_or_error(model, dimension, name, EXIT_USAGE)
        for dimension, name in zip(cube.dimensions, names)
    )
    return cube.name, coordinate


def _plain(value: Decimal) -> str:
    """A Decimal in full, never in exponent form and never through a float."""
    text = format(value, "f")
    if "." in text:
        text = text.rstrip("0").rstrip(".")
    return text


def _validate_command(args: argparse.Namespace) -> int:
    model = _load(_model_root(args))
    findings = validate_model(model)
    errors = [finding for finding in findings if finding.severity == ERROR]
    warnings = [finding for finding in findings if finding.severity != ERROR]
    for finding in errors + warnings:
        print(_format_finding(finding))
    print(_summary(len(errors), len(warnings)))
    return EXIT_INVALID_MODEL if errors else EXIT_OK


def _calculate_command(args: argparse.Namespace) -> int:
    model = _load(_model_root(args))
    _refuse_broken_model(model)
    data = _directory(args.data, "data")
    # Cell references are parsed before the CSVs are read so that a typo costs
    # a second rather than a full load and evaluation.
    cells = [_parse_cell(model, text) for text in args.cell]
    store = _evaluate(model, load_data(model, data))
    values = _values(model, store, cells)
    for (cube, coordinate), value in zip(cells, values):
        print(f"{cube}:{','.join(coordinate)} = {_plain(value)}")
    return EXIT_OK


def _report_command(args: argparse.Namespace) -> int:
    model = _load(_model_root(args))
    _refuse_broken_model(model)
    data = _directory(args.data, "data")
    cube = _cube(model, report.REPORT_CUBE)
    selections = report.rows(model, cube, args.year, args.version)
    store = _evaluate(model, load_data(model, data))
    cells = [
        (cube.name, tuple(selection[dimension] for dimension in cube.dimensions))
        for selection in selections
    ]
    values = list(_values(model, store, cells))
    for line in report.lines(cube, selections, values):
        print(line)
    return EXIT_OK


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="pacioliscube",
        description="Validate, calculate and report on a Planning Analytics model tree. "
        "Reads the model and its data, never writes.",
    )
    parser.add_argument("--version", action="version", version=f"pacioliscube {__version__}")
    subcommands = parser.add_subparsers(dest="command", metavar="SUBCOMMAND", required=True)

    def with_model_dir(subparser: argparse.ArgumentParser) -> argparse.ArgumentParser:
        subparser.add_argument(
            "model_dir",
            nargs="?",
            default=None,
            metavar="MODEL_DIR",
            help=f"the directory holding tm1project.json, {DEFAULT_MODEL_ROOT} by default",
        )
        return subparser

    validate = with_model_dir(
        subcommands.add_parser("validate", help="report structural findings in a model")
    )
    validate.set_defaults(handler=_validate_command)

    calculate = with_model_dir(
        subcommands.add_parser("calculate", help="print the value at one or more cells")
    )
    calculate.add_argument(
        "--data", required=True, metavar="DIR", help="directory of long format CSV input"
    )
    calculate.add_argument(
        "--cell",
        required=True,
        action="append",
        metavar="CUBE:ELEMENT,...",
        help="a cell to print, given once per cell, as CUBE:element,element,...",
    )
    calculate.set_defaults(handler=_calculate_command)

    report_parser = with_model_dir(
        subcommands.add_parser(
            "report",
            help="print a profit and loss for a year and version",
            description="Print the group profit and loss for one year and version, at the FY "
            "period and every cost centre. A line that is taken off the result below it, being "
            "direct costs, employment costs, overheads and depreciation, prints in brackets, as "
            "does any figure that comes out negative, so the column reads in the direction it "
            "adds. Each line is rounded to whole dollars on its own, so a subtotal can sit a "
            "dollar away from the lines above it: in the shipped budget, EBITDA less "
            "depreciation prints as 18,684,639 while EBIT prints as 18,684,640.",
        )
    )
    report_parser.add_argument(
        "--data", required=True, metavar="DIR", help="directory of long format CSV input"
    )
    report_parser.add_argument(
        "--year", required=True, metavar="Y", help="an element of the Year dimension"
    )
    report_parser.add_argument(
        "--version", required=True, metavar="V", help="an element of the Version dimension"
    )
    report_parser.set_defaults(handler=_report_command)
    return parser


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = build_parser()
    try:
        args = parser.parse_args(list(sys.argv[1:] if argv is None else argv))
    except SystemExit as stop:
        # argparse has already written its own message. It exits 0 for --help
        # and --version and 2 for a bad argument, so only the codes change here.
        return EXIT_OK if stop.code in (0, None) else EXIT_USAGE
    try:
        return args.handler(args)
    except CliError as error:
        print(f"pacioliscube: {error}", file=sys.stderr)
        return error.code
    except EvaluationError as error:  # a backstop: a command line should not traceback
        print(f"pacioliscube: {error}", file=sys.stderr)
        return EXIT_CALCULATION
    except ModelError as error:
        print(f"pacioliscube: {error}", file=sys.stderr)
        return EXIT_INVALID_MODEL
    except (OSError, UnicodeDecodeError) as error:
        # Another backstop. The places that read a file map their own failures,
        # because only they know whether the file was model source or input, so
        # anything arriving here is a path this module has not thought about and
        # takes the input error code.
        print(f"pacioliscube: {error}", file=sys.stderr)
        return EXIT_USAGE


if __name__ == "__main__":
    raise SystemExit(main())
