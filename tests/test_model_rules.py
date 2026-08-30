"""The shipped rules: discipline checks a Planning Analytics reviewer runs by eye."""

import re
from decimal import Decimal
from pathlib import Path

from pacioliscube.data import load_csv
from pacioliscube.model import load_model
from pacioliscube.validate import validate_model

MODEL_ROOT = Path(__file__).resolve().parents[1] / "model"
DRIVERS_CSV = Path(__file__).resolve().parents[1] / "examples" / "drivers.csv"

# Zero and one are arithmetic identities. A driver sitting at either carries no
# figure worth protecting, so the guard skipping it costs nothing: the shipped
# Indexation driver is 0.00 for the actual year for exactly that reason.
VALUELESS = frozenset({Decimal(0), Decimal(1)})

# A twelfth for the monthly spread and a hundredth for a percentage are the
# arithmetic the rules are written in. A driver that ever ships at one of these
# would be a real figure the guard could not tell apart from that arithmetic,
# so test_the_guard_names_every_value_it_cannot_cover fails and names it rather
# than the guard falling silent or blaming a rule for its own divisor.
AMBIGUOUS = frozenset({Decimal(12), Decimal(100)})

STRUCTURAL_CONSTANTS = VALUELESS | AMBIGUOUS

# A number as a rule author would write it: thousands separators optional, a
# leading zero optional, a trailing percentage sign optional. The lookbehind
# keeps digits inside a name, such as the 2025 in FY2025-26, from reading as a
# figure. The second branch is the bare decimal form, .0545, which an earlier
# pattern missed, so a rate written that way reached rule text unguarded.
NUMERIC_LITERAL = re.compile(
    r"(?<![\w.])(?:\d{1,3}(?:,\d{3})+|\d+)(?:\.\d+)?%?|(?<![\w.])\.\d+%?"
)


def ruled_cubes() -> tuple[str, ...]:
    """Every cube the shipped model gives rules to, read from the model itself.

    A tuple kept by hand leaves a cube added later unguarded until somebody
    remembers to extend it, which is the same staleness the figure list had.
    """
    model = load_model(MODEL_ROOT)
    return tuple(sorted(name for name, cube in model.cubes.items() if cube.rules is not None))


def guarded_sources() -> dict[str, str]:
    """Every file whose text could carry a statutory figure: rules and scripts."""
    model = load_model(MODEL_ROOT)
    sources = {
        f"{name}.rules": cube.rules_source.read_text(encoding="utf-8")
        for name, cube in model.cubes.items()
        if cube.rules is not None
    }
    sources.update(
        {
            f"{process.name}.ti": process.script
            for process in model.processes.values()
            if process.script
        }
    )
    return sources


def rules_text(name: str) -> str:
    return (MODEL_ROOT / "cubes" / f"{name}.rules").read_text(encoding="utf-8")


def shipped_driver_values() -> dict[tuple[str, ...], Decimal]:
    """Every value in the shipped driver file, keyed by its cube coordinate."""
    model = load_model(MODEL_ROOT)
    return dict(load_csv(DRIVERS_CSV, model.cubes["Drivers"], model))


def shipped_driver_literal(*coordinate: str) -> str:
    """One driver value as a rule author would paste it, named by its keys."""
    return format(shipped_driver_values()[coordinate], "f")


def figures_hard_coded_in(text: str) -> tuple[str, ...]:
    """Numeric literals in rule text that carry a value the driver file ships.

    The figures come from the data rather than from a list kept by hand, so a
    driver added or repriced later is guarded the day it ships. The last hand
    kept list omitted the FY2025-26 maximum contribution base for that reason.
    """
    guarded = {
        value
        for value in shipped_driver_values().values()
        if value not in STRUCTURAL_CONSTANTS
    }
    found = []
    for match in NUMERIC_LITERAL.finditer(text):
        literal = match.group()
        written = Decimal(literal.rstrip("%").replace(",", ""))
        candidates = {written / 100 if literal.endswith("%") else written}
        # A rate can be written as a fraction or as a percentage, so 5.45 has
        # to read as 0.0545. A whole number stays out of that test, because
        # the twelfths these rules divide by would otherwise read as rates.
        if "." in literal:
            candidates.add(written / 100)
        if candidates & guarded:
            found.append(literal)
    return tuple(found)


def test_every_calculated_cube_uses_skipcheck():
    model = load_model(MODEL_ROOT)
    for name in ruled_cubes():
        ruleset = model.cubes[name].rules
        assert ruleset is not None, name
        assert ruleset.skipcheck is True, name


def test_every_source_cube_declares_feeders():
    model = load_model(MODEL_ROOT)
    for name in ("Workforce", "Revenue", "Capex"):
        assert model.cubes[name].rules.feeders, name


def periods_fed_with(cube, measure: str) -> set[str]:
    """Every element a feeder names when it points at one calculated measure."""
    fed: set[str] = set()
    for feeder in cube.rules.feeders:
        named = {element for group in feeder.target.selectors for element in group}
        if measure in named:
            fed |= named
    return fed


def test_every_month_of_depreciation_is_fed_by_name():
    """A feeder keeps the source cell's own period wherever the target is silent.

    Additions are held at the Full Year period, so a cell to cell feeder reaches
    Full Year and no month. Depreciation is calculated in all twelve months and
    the P and L pulls it month by month, so each month has to be named. Left
    unfed the charge is invisible in a real database while the rule still reads
    as correct, which is the one failure a feeder exists to prevent.
    """
    model = load_model(MODEL_ROOT)
    months = model.hierarchy("Period").leaves_under("FY")
    fed = periods_fed_with(model.cubes["Capex"], "Depreciation Charge")
    unfed = [month for month in months if month not in fed]
    assert not unfed, (
        "no feeder in Capex.rules names " + ", ".join(unfed) + ", so depreciation "
        "would not appear in those months on a real server"
    )


def test_the_reporting_cube_is_fed_from_its_source_cubes():
    model = load_model(MODEL_ROOT)
    cross = [
        feeder
        for name in ("Workforce", "Revenue", "Capex")
        for feeder in model.cubes[name].rules.feeders
        if feeder.target_cube == "PnL"
    ]
    assert len(cross) >= 6


def test_no_statutory_rate_is_hard_coded_in_rule_text():
    for name, text in guarded_sources().items():
        found = figures_hard_coded_in(text)
        assert not found, f"{name} hard codes {', '.join(found)}"


def test_the_guard_catches_a_driver_value_pasted_over_its_db_call():
    """Swap one driver call for the number it reads and the guard has to bite."""
    call = "DB('Drivers', !Year, !Version, 'Full Year', 'Maximum Contribution Base')"
    text = rules_text("Workforce")
    assert call in text, "the cap no longer reads the driver, so rewrite this proof"
    literal = shipped_driver_literal(
        "FY2025-26", "Actual", "Full Year", "Maximum Contribution Base"
    )
    tampered = text.replace(call, literal, 1)
    assert literal in figures_hard_coded_in(tampered), (
        f"a rule holding the literal {literal} passes the guard"
    )


def test_the_guard_covers_every_shipped_driver_value():
    """A hand maintained list of figures goes stale; the shipped data does not."""
    unguarded = []
    for coordinate, value in shipped_driver_values().items():
        if value in STRUCTURAL_CONSTANTS:
            continue
        literal = format(value, "f")
        # The probe carries no divisor. An earlier version wrote the statement
        # as "* {literal} / 12", so a driver shipping at 12 was found in the
        # scaffold's own twelfth and the test passed while the guard could not
        # actually tell the two apart.
        statement = f"['Base Pay'] = N: ['Headcount'] * {literal};"
        if literal not in figures_hard_coded_in(statement):
            unguarded.append(f"{' '.join(coordinate)} = {literal}")
    assert not unguarded, "the guard misses " + "; ".join(unguarded)


def test_the_guard_reads_a_rate_written_without_its_leading_zero():
    """A rate pasted as .0545 rather than 0.0545 still has to be caught."""
    rate = shipped_driver_literal("FY2026-27", "Budget", "Full Year", "Payroll Tax Rate")
    assert rate.startswith("0."), "this proof assumes the shipped rate is a fraction"
    statement = f"['Payroll Tax Cost'] = N: ['Base Pay'] * {rate[1:]};"
    assert figures_hard_coded_in(statement), f"a rule holding {rate[1:]} passes the guard"


def test_the_guard_names_every_value_it_cannot_cover():
    """Keep the blind spot visible instead of letting a driver slip into it.

    A driver that ships at a twelfth or a hundred cannot be told apart from the
    arithmetic the rules are written in. No shipped driver does today. If one
    ever does, this fails and names it, rather than the guard either falling
    silent or accusing a rule of hard coding its own divisor.
    """
    collisions = {
        " ".join(coordinate): format(value, "f")
        for coordinate, value in shipped_driver_values().items()
        if value in AMBIGUOUS
    }
    assert not collisions, (
        "these drivers ship at a value the rules use as arithmetic, so the guard "
        f"cannot see them: {collisions}"
    )


def test_the_guard_reaches_the_turbointegrator_scripts_as_well():
    """A figure pasted into a process is as hard coded as one in a rule."""
    assert any(name.endswith(".ti") for name in guarded_sources())
    rate = shipped_driver_literal("FY2026-27", "Budget", "Full Year", "SG Rate")
    assert figures_hard_coded_in(f"nRate = {rate};")


def test_superannuation_reads_its_rate_from_the_drivers_cube():
    text = rules_text("Workforce")
    assert "DB('Drivers'" in text
    assert "'SG Rate'" in text
    assert "'Maximum Contribution Base'" in text


def test_payroll_tax_is_levied_on_wages_grossed_up_by_superannuation():
    text = rules_text("Workforce")
    assert "( ['Base Pay'] + ['Superannuation Cost'] )" in text


def test_the_threshold_credit_sits_with_the_designated_group_employer():
    text = rules_text("PnL")
    assert "'Payroll Tax Threshold'" in text
    assert text.index("'CivilCo', 'Corporate', 'Payroll Tax'") < text.index(
        "['Payroll Tax', 'Amount']"
    ), "the specific statement must come before the general one, first match wins"


def test_the_drivers_cube_is_pure_input():
    assert load_model(MODEL_ROOT).cubes["Drivers"].rules is None


def test_the_model_validates_with_no_errors_and_no_warnings():
    findings = validate_model(load_model(MODEL_ROOT))
    assert findings == (), [f"{f.code}: {f.message} ({f.location})" for f in findings]
