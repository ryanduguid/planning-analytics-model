"""Seeded exact-money properties for the public calculation engine."""

from decimal import Decimal
from pathlib import Path

from hypothesis import example, given, seed, settings, strategies as st

from pacioliscube.evaluate import CellStore, consolidate, evaluate
from pacioliscube.model import load_model


MINI = Path(__file__).parent / "fixtures" / "mini"
MODEL = load_model(MINI)
PROPERTY_SETTINGS = settings(max_examples=80, database=None, deadline=None)
AMOUNTS = st.decimals(
    min_value=Decimal("-1000000"),
    max_value=Decimal("1000000"),
    places=4,
    allow_nan=False,
    allow_infinity=False,
)


@seed(0xC0BE)
@PROPERTY_SETTINGS
@example(Decimal("0"), Decimal("999999.9999"))
@example(Decimal("-12.5"), Decimal("4"))
@example(Decimal("0.1"), Decimal("0.2"))
@given(units=AMOUNTS, price=AMOUNTS)
def test_leaf_amount_is_the_exact_product_of_units_and_price(
    units: Decimal, price: Decimal
) -> None:
    """Changing the rule evaluator's multiplication branch must break this."""
    store = CellStore()
    store.set("Sales", ("Red", "Units"), units)
    store.set("Sales", ("Red", "Price"), price)

    calculated = evaluate(MODEL, store)

    assert calculated.get("Sales", ("Red", "Amount")) == units * price


@seed(0xC01150)
@PROPERTY_SETTINGS
@example(Decimal("0"), Decimal("0"), Decimal("0"))
@example(Decimal("10"), Decimal("5"), Decimal("4"))
@given(red=AMOUNTS, blue=AMOUNTS, contra=AMOUNTS)
def test_weighted_consolidation_conserves_its_leaf_amounts(
    red: Decimal, blue: Decimal, contra: Decimal
) -> None:
    """Ignoring an edge, its negative weight or a leaf value must break this."""
    store = CellStore()
    for colour, units in (("Red", red), ("Blue", blue), ("Contra", contra)):
        store.set("Sales", (colour, "Units"), units)
        store.set("Sales", (colour, "Price"), Decimal("1"))

    calculated = evaluate(MODEL, store)

    assert consolidate(MODEL, calculated, "Sales", ("Total", "Amount")) == red + blue - contra
