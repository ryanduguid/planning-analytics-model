"""The command line's failure type, its exit codes, and one model lookup.

This sits apart from cli.py so that the report and the data loader can refuse a
bad argument the way the command line does without importing the module that
imports them. Nothing here reads a file or prints anything.
"""

from __future__ import annotations

from pacioliscube.model import Model, ModelError

EXIT_OK = 0
EXIT_USAGE = 1
EXIT_INVALID_MODEL = 2
EXIT_CALCULATION = 3


class CliError(Exception):
    """An error the command line reports, carrying the exit code it maps to."""

    def __init__(self, code: int, message: str) -> None:
        super().__init__(message)
        self.code = code


def element_or_error(model: Model, dimension: str, name: str, code: int) -> str:
    """Resolve an element name, taking the exit code that fits who supplied it."""
    try:
        return model.hierarchy(dimension).resolve(name)
    except ModelError as error:
        raise CliError(code, str(error)) from error
