"""The command line, being the way a shell script or a CI job drives this package.

Three subcommands. ``validate`` reports what is structurally wrong with a model
tree, ``calculate`` prints the value at named cells, and ``report`` prints a
small profit and loss. Every run reads: nothing here writes a file, touches a
network, or changes the model it is pointed at.

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
from decimal import ROUND_HALF_UP, Decimal
from pathlib import Path
from typing import Optional, Sequence

from pacioliscube.data import load_into_store
from pacioliscube.evaluate import CellStore, EvaluationError, consolidate, evaluate
from pacioliscube.model import Cube, Model, ModelError, load_model
from pacioliscube.validate import ERROR, Finding, validate_model
from pacioliscube.version import __version__

EXIT_OK = 0
EXIT_USAGE = 1
EXIT_INVALID_MODEL = 2
EXIT_CALCULATION = 3

DEFAULT_MODEL_ROOT = "model"

# The stem of a data file names the cube it feeds. The shipped example files are
# listed rather than derived, because pnl-direct is not spelled like the cube it
# loads. A stem that matches a cube name is taken as well, which is how a model
# built for a test feeds cubes this map has never heard of, and _load_data
# refuses the case that makes the fallback dangerous: two files, one cube.
CUBE_BY_STEM = {
    "drivers": "Drivers",
    "workforce": "Workforce",
    "revenue": "Revenue",
    "capex": "Capex",
    "pnl-direct": "PnL",
}

REPORT_CUBE = "PnL"

# The profit and loss the report prints, in statement order. Each name is an
# element of the Account dimension, consolidated or not.
REPORT_ROWS = (
    "Revenue",
    "Direct Costs",
    "Gross Margin",
    "Employment Costs",
    "Overheads",
    "EBITDA",
    "Depreciation",
    "EBIT",
)

# The slice the report reads: the whole year, the whole group, every cost
# centre. Year and Version come from the command line instead.
REPORT_SLICE = {
    "Period": "FY",
    "Entity": "Group",
    "CostCentre": "All Cost Centres",
}
REPORT_MEASURE = {"PnLMeasure": "Amount"}

# Which report dimensions the caller supplies, and so which ones name the
# argument at fault rather than the shape this report is fixed to.
FROM_COMMAND_LINE = ("Year", "Version")

# The rows that reduce the result above them. A cost is stored as a positive
# number, so the reader is told which lines are taken off by the report and not
# by the model. See _money for the convention.
DEDUCTION_ROWS = frozenset(
    {"Direct Costs", "Employment Costs", "Overheads", "Depreciation"}
)


class CliError(Exception):
    """An error the command line reports, carrying the exit code it maps to."""

    def __init__(self, code: int, message: str) -> None:
        super().__init__(message)
        self.code = code


def _directory(argument: str, what: str) -> Path:
    path = Path(argument)
    if not path.is_dir():
        raise CliError(EXIT_USAGE, f"{path}: there is no {what} directory there")
    return path


def _model_root(args: argparse.Namespace) -> Path:
    """The model directory, given either as the positional or as --model."""
    if args.model_dir is not None and args.model_option is not None:
        raise CliError(
            EXIT_USAGE,
            "the model directory is given twice, once as MODEL_DIR and once as --model. "
            "Give it once",
        )
    return _directory(args.model_dir or args.model_option or DEFAULT_MODEL_ROOT, "model")


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


def _element(model: Model, dimension: str, name: str, code: int) -> str:
    """Resolve an element name, taking the exit code that fits who supplied it."""
    try:
        return model.hierarchy(dimension).resolve(name)
    except ModelError as error:
        raise CliError(code, str(error)) from error


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


def _cube_for_file(model: Model, path: Path) -> Cube:
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


def _load_data(model: Model, directory: Path) -> CellStore:
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
        feeding.setdefault(_cube_for_file(model, path).name, []).append(path)
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


def _evaluate(model: Model, store: CellStore) -> CellStore:
    try:
        return evaluate(model, store)
    except EvaluationError as error:
        raise CliError(EXIT_CALCULATION, str(error)) from error
    except ModelError as error:
        # Validation has already passed, so a model error at this point is a
        # defect the structural checks do not reach rather than a bad argument.
        raise CliError(EXIT_INVALID_MODEL, str(error)) from error


def _value(model: Model, store: CellStore, cube: str, coordinate: tuple[str, ...]) -> Decimal:
    try:
        return consolidate(model, store, cube, coordinate)
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
        _element(model, dimension, name, EXIT_USAGE)
        for dimension, name in zip(cube.dimensions, names)
    )
    return cube.name, coordinate


def _plain(value: Decimal) -> str:
    """A Decimal in full, never in exponent form and never through a float."""
    text = format(value, "f")
    if "." in text:
        text = text.rstrip("0").rstrip(".")
    return text


def _money(value: Decimal) -> str:
    """Whole dollars with thousands separators, rounded the way a ledger rounds.

    A negative prints in brackets, which is the convention a reader of a printed
    statement expects. The report hands this function the signed effect a line
    has on the result below it, so a deduction arrives here negative and prints
    bracketed, and a cost that happens to be a credit prints plain.

    Each line rounds on its own, so a printed subtotal can sit a dollar away
    from the printed lines above it. Forcing the difference into a line would
    misstate that line, so the report leaves it where the arithmetic puts it.
    """
    dollars = value.quantize(Decimal("1"), rounding=ROUND_HALF_UP)
    if dollars == 0:
        # Rounding a small negative gives minus zero, which reads as a figure
        # the statement does not hold.
        return "0"
    if dollars < 0:
        return f"({-dollars:,})"
    return f"{dollars:,}"


def _fixed_slice() -> str:
    """The slice the report is fixed to, spelled out for an error message."""
    return ", ".join(f"{name} {element!r}" for name, element in REPORT_SLICE.items())


def _report_selection(model: Model, cube: Cube, year: str, version: str, account: str) -> dict:
    """Resolve one report row, keyed by dimension so the cube's order can drive it."""
    wanted = dict(REPORT_SLICE)
    wanted.update(REPORT_MEASURE)
    wanted["Year"] = year
    wanted["Version"] = version
    wanted["Account"] = account
    resolved = {}
    for dimension in cube.dimensions:
        if dimension not in wanted:
            raise CliError(
                EXIT_USAGE,
                f"cube {cube.name!r} has a dimension {dimension!r} that the report has no "
                "element for, so this report does not fit this model. Use the calculate "
                "subcommand, which takes the whole coordinate from you",
            )
        if dimension in FROM_COMMAND_LINE:
            resolved[dimension] = _element(model, dimension, wanted[dimension], EXIT_USAGE)
            continue
        # Everything else is spelled by the report rather than by the caller, so
        # a model that does not hold it is a model this report does not fit. It
        # is not a broken model, and it does not take the invalid model code.
        try:
            resolved[dimension] = model.hierarchy(dimension).resolve(wanted[dimension])
        except ModelError as error:
            raise CliError(
                EXIT_USAGE,
                f"cube {cube.name!r}: the report reads {dimension} {wanted[dimension]!r}, "
                f"which this model does not hold ({error}). The report is fixed to "
                f"{_fixed_slice()} and to the rows of a direct profit and loss, so a model "
                "that rolls up differently needs the calculate subcommand instead",
            ) from error
    return resolved


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
    store = _evaluate(model, _load_data(model, data))
    for cube, coordinate in cells:
        print(f"{cube}:{','.join(coordinate)} = {_plain(_value(model, store, cube, coordinate))}")
    return EXIT_OK


def _report_command(args: argparse.Namespace) -> int:
    model = _load(_model_root(args))
    _refuse_broken_model(model)
    data = _directory(args.data, "data")
    cube = _cube(model, REPORT_CUBE)
    # Without these three the eight rows would all read the same cell, so the
    # report would print something that looked right and was not.
    absent = [name for name in ("Year", "Version", "Account") if name not in cube.dimensions]
    if absent:
        raise CliError(
            EXIT_USAGE,
            f"cube {cube.name!r} has no {', '.join(absent)} dimension, so a profit and loss "
            "cannot be built from it. This report does not fit this model, which is a "
            "different thing from the model being wrong",
        )
    rows = [
        _report_selection(model, cube, args.year, args.version, account)
        for account in REPORT_ROWS
    ]
    store = _evaluate(model, _load_data(model, data))
    values = [
        _value(model, store, cube.name, tuple(row[dimension] for dimension in cube.dimensions))
        for row in rows
    ]
    # The row names drive the sign, not the resolved element, because a model
    # may spell an account in another case and the convention is the report's.
    amounts = [
        _money(-value if account in DEDUCTION_ROWS else value)
        for account, value in zip(REPORT_ROWS, values)
    ]

    first = rows[0]
    labels = [row["Account"] for row in rows]
    # Name only the restrictions actually applied. A cube without one of these
    # dimensions was never narrowed on it, and an earlier version fell back to
    # the report's own literal, so the header stated a basis the figures below
    # it did not have.
    applied = [first[name] for name in REPORT_SLICE if name in first]
    print(f"Profit and loss for {first['Year']}, {first['Version']}")
    if applied:
        print(f"{cube.name} at {', '.join(applied)}")
    else:
        print(f"{cube.name}, whole cube: it carries none of the usual reporting dimensions")
    print()
    label_width = max(len(label) for label in labels)
    amount_width = max(len(amount) for amount in amounts)
    for label, amount in zip(labels, amounts):
        print(f"{label:<{label_width}}  {amount:>{amount_width}}")
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
        # The same directory, spelled as an option. Scripts that pass every
        # path as a named argument read better for it, and the packaging job
        # calls the command that way.
        subparser.add_argument(
            "--model",
            dest="model_option",
            default=None,
            metavar="DIR",
            help="the same directory, named as an option instead of the positional",
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

    report = with_model_dir(
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
    report.add_argument(
        "--data", required=True, metavar="DIR", help="directory of long format CSV input"
    )
    report.add_argument(
        "--year", required=True, metavar="Y", help="an element of the Year dimension"
    )
    report.add_argument(
        "--version", required=True, metavar="V", help="an element of the Version dimension"
    )
    report.set_defaults(handler=_report_command)
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
