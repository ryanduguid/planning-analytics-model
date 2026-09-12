"""Evaluating rules over leaf cells and consolidating along weighted edges."""

from decimal import Decimal
from pathlib import Path

import pytest

from conftest import MEASURES, write_model
from pacioliscube.evaluate import (
    CellStore,
    CircularReference,
    EvaluationError,
    consolidate,
    consolidate_many,
    evaluate,
)
from pacioliscube.model import ModelError, load_model

MINI = Path(__file__).parent / "fixtures" / "mini"

# The built model carries a Margin measure the shared default does not, for the
# rules that calculate one measure from another.
MEASURES_WITH_MARGIN = MEASURES + ', {"Name": "Margin", "Type": "Numeric"}'


def loaded_store(**cells) -> tuple:
    """Load the mini model and seed the input cells named as red_units=10 and so on."""
    model = load_model(MINI)
    store = CellStore()
    for key, value in cells.items():
        cube, colour, measure = key.split("__")
        store.set(cube, (colour, measure), Decimal(value))
    return model, store


def test_a_rule_calculates_a_leaf_cell():
    model, store = loaded_store(Sales__Red__Units="10", Sales__Red__Price="2.50")
    result = evaluate(model, store)
    assert result.get("Sales", ("Red", "Amount")) == Decimal("25.00")


def test_an_input_cell_survives_evaluation():
    model, store = loaded_store(Sales__Red__Units="10", Sales__Red__Price="2.50")
    result = evaluate(model, store)
    assert result.get("Sales", ("Red", "Units")) == Decimal("10")


def test_an_unwritten_cell_reads_as_zero():
    store = CellStore()
    assert store.get("Sales", ("Red", "Units")) == Decimal("0")


def test_a_rule_reading_another_cube_resolves_db():
    model, store = loaded_store(
        Sales__Red__Units="10",
        Sales__Red__Price="2.50",
        Cost__Red__Amount="4",
        Cost__Blue__Amount="6",
    )
    result = evaluate(model, store)
    # Margin is Amount less the same colour's cost: 25.00 less 4.
    assert result.get("Sales", ("Red", "Margin")) == Decimal("21.00")


def test_current_element_marker_binds_to_the_cell_being_calculated():
    model = load_model(MINI)
    store = CellStore()
    for colour, units in (("Red", "10"), ("Blue", "4")):
        store.set("Sales", (colour, "Units"), Decimal(units))
        store.set("Sales", (colour, "Price"), Decimal("1"))
    store.set("Cost", ("Red", "Amount"), Decimal("3"))
    store.set("Cost", ("Blue", "Amount"), Decimal("1"))
    result = evaluate(model, store)
    # Red subtracts the red cost and blue subtracts the blue cost, which is what
    # the current element marker is for. A marker that leaked would give both
    # cells the same answer.
    assert result.get("Sales", ("Red", "Margin")) == Decimal("7")
    assert result.get("Sales", ("Blue", "Margin")) == Decimal("3")


def test_the_safe_divide_operator_yields_zero_for_a_zero_divisor(tmp_path):
    model, store = build_model(tmp_path, "['Amount'] = N: ['Units'] \\ ['Price'];")
    store.set("Sales", ("Red", "Units"), Decimal("10"))
    result = evaluate(model, store)
    assert result.get("Sales", ("Red", "Amount")) == Decimal("0")


def test_the_plain_divide_operator_refuses_a_zero_divisor(tmp_path):
    model, store = build_model(tmp_path, "['Amount'] = N: ['Units'] / ['Price'];")
    store.set("Sales", ("Red", "Units"), Decimal("10"))
    with pytest.raises(EvaluationError) as caught:
        evaluate(model, store)
    assert "division by zero" in str(caught.value)


def test_the_plain_divide_operator_divides_normally(tmp_path):
    model, store = build_model(tmp_path, "['Amount'] = N: ['Units'] / ['Price'];")
    store.set("Sales", ("Red", "Units"), Decimal("10"))
    store.set("Sales", ("Red", "Price"), Decimal("4"))
    store.set("Sales", ("Blue", "Price"), Decimal("1"))
    result = evaluate(model, store)
    assert result.get("Sales", ("Red", "Amount")) == Decimal("2.5")


def test_a_rule_depending_on_a_rule_evaluates_in_order():
    model, store = loaded_store(Sales__Red__Units="3", Sales__Red__Price="3")
    result = evaluate(model, store)
    assert result.get("Sales", ("Red", "Amount")) == Decimal("9")
    assert result.get("Sales", ("Red", "Margin")) == Decimal("9")


def test_a_circular_rule_raises_circular_reference(tmp_path):
    model, store = build_model(
        tmp_path,
        "['Amount'] = N: ['Margin'];\n['Margin'] = N: ['Amount'];",
    )
    with pytest.raises(CircularReference) as caught:
        evaluate(model, store)
    message = str(caught.value)
    assert "Amount" in message and "Margin" in message


def test_evaluation_is_decimal_not_float(tmp_path):
    model, store = build_model(tmp_path, "['Amount'] = N: ['Units'] + ['Price'];")
    store.set("Sales", ("Red", "Units"), Decimal("0.1"))
    store.set("Sales", ("Red", "Price"), Decimal("0.2"))
    result = evaluate(model, store)
    assert result.get("Sales", ("Red", "Amount")) == Decimal("0.3")


# Every comparison TM1 rules allow, each one at a pair of operands that makes it
# hold and a pair that makes it fail, so neither arm of the IF nor either answer
# of the operator is taken on trust. The rule returns 10 when the comparison
# holds and 20 when it does not, and each expected figure is derived by hand:
# 2 = 2 holds and 3 = 2 does not; 3 <> 2 holds and 2 <> 2 does not; 2 < 3 holds
# and 2 < 2 does not; 2 <= 2 holds and 3 <= 2 does not; 3 > 2 holds and 2 > 2
# does not; 2 >= 2 holds and 2 >= 3 does not. The three two character operators
# also prove the tokeniser reads them as one token rather than as two.
@pytest.mark.parametrize(
    "operator, units, price, expected",
    [
        ("=", "2", "2", "10"),
        ("=", "3", "2", "20"),
        ("<>", "3", "2", "10"),
        ("<>", "2", "2", "20"),
        ("<", "2", "3", "10"),
        ("<", "2", "2", "20"),
        ("<=", "2", "2", "10"),
        ("<=", "3", "2", "20"),
        (">", "3", "2", "10"),
        (">", "2", "2", "20"),
        (">=", "2", "2", "10"),
        (">=", "2", "3", "20"),
    ],
)
def test_if_evaluates_every_comparison_operator(tmp_path, operator, units, price, expected):
    model, store = build_model(
        tmp_path, f"['Amount'] = N: IF(['Units'] {operator} ['Price'], 10, 20);"
    )
    store.set("Sales", ("Red", "Units"), Decimal(units))
    store.set("Sales", ("Red", "Price"), Decimal(price))
    result = evaluate(model, store)
    assert result.get("Sales", ("Red", "Amount")) == Decimal(expected)


def test_an_n_rule_does_not_calculate_a_consolidated_cell():
    model, store = loaded_store(Sales__Red__Units="10", Sales__Red__Price="2")
    result = evaluate(model, store)
    # Total is consolidated, so the N rule leaves it to consolidation, which sums 20.
    assert consolidate(model, result, "Sales", ("Total", "Amount")) == Decimal("20")


def test_a_rule_naming_an_element_no_dimension_holds_is_a_model_error(tmp_path):
    model, store = build_model(tmp_path, "['Ghost'] = N: 1;")
    with pytest.raises(ModelError) as caught:
        evaluate(model, store)
    assert "Ghost" in str(caught.value)


# Consolidation


def test_a_consolidated_element_sums_its_children():
    model, store = loaded_store(
        Sales__Red__Units="10", Sales__Red__Price="1", Sales__Blue__Units="5", Sales__Blue__Price="1"
    )
    result = evaluate(model, store)
    assert consolidate(model, result, "Sales", ("Total", "Amount")) == Decimal("15")


def test_a_negative_edge_weight_subtracts():
    model, store = loaded_store(
        Sales__Red__Units="10",
        Sales__Red__Price="1",
        Sales__Blue__Units="5",
        Sales__Blue__Price="1",
        Sales__Contra__Units="4",
        Sales__Contra__Price="1",
    )
    result = evaluate(model, store)
    assert consolidate(model, result, "Sales", ("Total", "Amount")) == Decimal("11")


def test_consolidation_leaves_a_leaf_coordinate_alone():
    model, store = loaded_store(Sales__Red__Units="7", Sales__Red__Price="1")
    result = evaluate(model, store)
    assert consolidate(model, result, "Sales", ("Red", "Amount")) == Decimal("7")


@pytest.mark.parametrize("coordinate", [(), ("Red",), ("Red", "Units", "unexpected")])
@pytest.mark.parametrize("batch", [False, True])
def test_consolidation_rejects_incorrect_coordinate_width(coordinate, batch):
    model, store = loaded_store(Sales__Red__Units="3")
    if batch:
        values = consolidate_many(
            model, store, [("Sales", ("Red", "Units")), ("Sales", coordinate)],
        )
        assert next(values) == Decimal("3")
        with pytest.raises(ModelError, match="coordinate.*dimensions"):
            next(values)
    else:
        with pytest.raises(ModelError, match="coordinate.*dimensions"):
            consolidate(model, store, "Sales", coordinate)


def test_a_c_rule_overrides_consolidation(tmp_path):
    model, store = build_model(
        tmp_path,
        "['Amount'] = N: ['Units'] * ['Price'];\n['Price'] = C: 99;",
    )
    store.set("Sales", ("Red", "Price"), Decimal("1"))
    store.set("Sales", ("Blue", "Price"), Decimal("2"))
    result = evaluate(model, store)
    # Without the C rule the total price would be the meaningless sum of 1 and 2.
    assert consolidate(model, result, "Sales", ("Total", "Price")) == Decimal("99")


def test_batch_consolidation_preserves_order_weights_and_fresh_inputs():
    from pacioliscube.evaluate import consolidate_many

    model, store = loaded_store(
        Sales__Red__Units="10", Sales__Red__Price="2",
        Sales__Blue__Units="5", Sales__Blue__Price="3",
        Sales__Contra__Units="4", Sales__Contra__Price="1",
    )
    cells = [("Sales", ("Total", "Amount")), ("Sales", ("Red", "Amount"))]
    other_colours = Decimal("5") * Decimal("3") - Decimal("4") * Decimal("1")
    red = Decimal("10") * Decimal("2")
    assert list(consolidate_many(model, store, cells)) == [red + other_colours, red]
    store.set("Sales", ("Red", "Price"), Decimal("4"))
    red = Decimal("10") * Decimal("4")
    assert list(consolidate_many(model, store, cells)) == [red + other_colours, red]
    assert list(consolidate_many(model, store, [])) == []


def build_model(root: Path, rules_text: str):
    """Write a two dimension Sales cube with the given rules and return it loaded."""
    write_model(
        root,
        rules="SKIPCHECK;\n" + rules_text + "\nFEEDERS;\n['Units'] => ['Amount'];\n",
        measures=MEASURES_WITH_MARGIN,
    )
    return load_model(root), CellStore()
