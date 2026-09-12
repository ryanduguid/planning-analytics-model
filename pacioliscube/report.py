"""The profit and loss the report subcommand prints.

The shape of the statement lives here: which rows it carries, which slice of the
cube it reads, which lines are taken off the result below them, and how a figure
is written in a printed statement. cli.py loads the model and the data and then
asks this module for the rows to calculate and the lines to print, so the report
can be changed without touching the command line's argument handling.
"""

from __future__ import annotations

from decimal import ROUND_HALF_UP, Decimal
from typing import Sequence

from pacioliscube.errors import EXIT_USAGE, CliError, element_or_error
from pacioliscube.model import Cube, Model, ModelError

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
# by the model. See money for the convention.
DEDUCTION_ROWS = frozenset(
    {"Direct Costs", "Employment Costs", "Overheads", "Depreciation"}
)

# The three dimensions without which the eight rows would all read the same
# cell, so the report would print something that looked right and was not.
REQUIRED_DIMENSIONS = ("Year", "Version", "Account")


def money(value: Decimal) -> str:
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


def fixed_slice() -> str:
    """The slice the report is fixed to, spelled out for an error message."""
    return ", ".join(f"{name} {element!r}" for name, element in REPORT_SLICE.items())


def _selection(model: Model, cube: Cube, year: str, version: str, account: str) -> dict:
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
            resolved[dimension] = element_or_error(model, dimension, wanted[dimension], EXIT_USAGE)
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
                f"{fixed_slice()} and to the rows of a direct profit and loss, so a model "
                "that rolls up differently needs the calculate subcommand instead",
            ) from error
    return resolved


def rows(model: Model, cube: Cube, year: str, version: str) -> list[dict]:
    """One resolved coordinate per statement row, in statement order."""
    absent = [name for name in REQUIRED_DIMENSIONS if name not in cube.dimensions]
    if absent:
        raise CliError(
            EXIT_USAGE,
            f"cube {cube.name!r} has no {', '.join(absent)} dimension, so a profit and loss "
            "cannot be built from it. This report does not fit this model, which is a "
            "different thing from the model being wrong",
        )
    return [_selection(model, cube, year, version, account) for account in REPORT_ROWS]


def lines(cube: Cube, selections: Sequence[dict], values: Sequence[Decimal]) -> list[str]:
    """The statement as printed, one string per line, blank lines included."""
    # The row names drive the sign, not the resolved element, because a model
    # may spell an account in another case and the convention is the report's.
    amounts = [
        money(-value if account in DEDUCTION_ROWS else value)
        for account, value in zip(REPORT_ROWS, values)
    ]
    first = selections[0]
    labels = [selection["Account"] for selection in selections]
    # Name only the restrictions actually applied. A cube without one of these
    # dimensions was never narrowed on it, and an earlier version fell back to
    # the report's own literal, so the header stated a basis the figures below
    # it did not have.
    applied = [first[name] for name in REPORT_SLICE if name in first]
    printed = [f"Profit and loss for {first['Year']}, {first['Version']}"]
    if applied:
        printed.append(f"{cube.name} at {', '.join(applied)}")
    else:
        printed.append(
            f"{cube.name}, whole cube: it carries none of the usual reporting dimensions"
        )
    printed.append("")
    label_width = max(len(label) for label in labels)
    amount_width = max(len(amount) for amount in amounts)
    for label, amount in zip(labels, amounts):
        printed.append(f"{label:<{label_width}}  {amount:>{amount_width}}")
    return printed
