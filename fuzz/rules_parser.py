"""Fuzz the TM1 rules parser with fabricated rule text."""

from __future__ import annotations

import sys
from pathlib import Path

from pacioliscube.rules import RuleSyntaxError, parse_rules

PATH = Path("Fuzz.rules")


def test_one_input(data: bytes) -> None:
    text = data.decode("utf-8", errors="replace")
    try:
        parsed = parse_rules(text, PATH)
    except RuleSyntaxError:
        return
    # Parsing has no hidden state: the same text gives the same rule set.
    if parse_rules(text, PATH) != parsed:
        raise AssertionError("parsing the same rules twice gave different results")
    for rule in parsed.rules:
        if rule.area.qualifier not in ("", "N", "C"):
            raise AssertionError(f"unexpected rule qualifier {rule.area.qualifier!r}")


if __name__ == "__main__":
    import atheris

    atheris.instrument_all()
    atheris.Setup(sys.argv, test_one_input)
    atheris.Fuzz()
