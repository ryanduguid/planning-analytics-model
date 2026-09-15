"""Calculation evidence must follow the same arithmetic as the evaluator."""

import json
from decimal import Decimal

import pytest

from conftest import write_model
from pacioliscube import evaluate as evaluation
from pacioliscube.evaluate import CellStore, CircularReference, EvaluationError, consolidate
from pacioliscube.model import load_model
from test_cli import run_once
from test_evaluate import MINI, build_model, loaded_store


def node(evidence, cube, *coordinate):
    return evidence["cells"][f"{cube}{list(coordinate)}"]


def test_rule_explanation_includes_cross_cube_inputs_and_source():
    model, store = loaded_store(
        Sales__Red__Units="3", Sales__Red__Price="0.10", Cost__Red__Amount="0.20",
    )
    original = list(store.items())
    evidence = evaluation.explain(model, store, [("Sales", ("red", "margin"))])
    margin = node(evidence, "Sales", "Red", "Margin")
    assert margin["value"] == Decimal("3") * Decimal("0.10") - Decimal("0.20")
    assert margin["kind"] == "rule"
    assert margin["rule"] == {"file": "cubes/Sales.rules", "line": 5}
    assert margin["inputs"] == [
        {"cell": "Sales['Red', 'Amount']", "value": Decimal("0.30")},
        {"cell": "Cost['Red', 'Amount']", "value": Decimal("0.20")},
    ]
    assert margin["steps"] == [
        {"operator": "-", "left": Decimal("0.30"), "right": Decimal("0.20"),
         "result": Decimal("0.10")},
    ]
    assert evidence["requested"] == ["Sales['Red', 'Margin']"]
    assert list(store.items()) == original


def test_consolidation_explanation_preserves_contra_weights_and_repeated_roots():
    model, store = loaded_store(
        Sales__Red__Units="10", Sales__Red__Price="2",
        Sales__Blue__Units="5", Sales__Blue__Price="3",
        Sales__Contra__Units="4", Sales__Contra__Price="1",
    )
    cells = [("Sales", ("Total", "Amount")), ("Sales", ("Red", "Amount"))] * 2
    evidence = evaluation.explain(model, store, cells)
    total = node(evidence, "Sales", "Total", "Amount")
    expected = Decimal("10") * 2 + Decimal("5") * 3 - Decimal("4")
    assert total["value"] == expected == consolidate(model, store, *cells[0])
    assert total["kind"] == "consolidation"
    assert [row["weight"] for row in total["contributions"]] == [1, 1, -1]
    assert [row["value"] for row in total["contributions"]] == [20, 15, 4]
    assert [row["contribution"] for row in total["contributions"]] == [20, 15, -4]
    assert sum(row["contribution"] for row in total["contributions"]) == expected
    assert len(evidence["cells"]) == 10  # Total plus 3 colours with 3 cells each.
    assert evidence["requested"] == ["Sales['Total', 'Amount']", "Sales['Red', 'Amount']"] * 2


def test_explanation_distinguishes_input_zero_from_default_zero():
    model = load_model(MINI)
    store = CellStore()
    store.set("Sales", ("Red", "Units"), Decimal("0"))
    evidence = evaluation.explain(model, store, [
        ("Sales", ("Red", "Units")), ("Sales", ("Blue", "Units")),
    ])
    assert node(evidence, "Sales", "Red", "Units")["kind"] == "input"
    assert node(evidence, "Sales", "Blue", "Units")["kind"] == "default_zero"


def test_shared_leaf_keeps_each_weighted_path(tmp_path):
    root = write_model(tmp_path)
    hierarchy = root / "dimensions" / "Colour.hierarchies" / "Colour.json"
    hierarchy.write_text(json.dumps({
        "Name": "Colour",
        "Elements": [
            {"Name": name, "Type": "Consolidated" if name in ("Total", "Left", "Right") else "Numeric"}
            for name in ("Total", "Left", "Right", "Red", "Blue")
        ],
        "Edges": [
            {"ParentName": parent, "ComponentName": child, "Weight": weight}
            for parent, child, weight in [
                ("Total", "Left", "2"), ("Total", "Right", "-1"),
                ("Left", "Red", "0.5"), ("Left", "Blue", "1"), ("Right", "Red", "3"),
            ]
        ],
    }), encoding="utf-8")
    model, store = load_model(root), CellStore()
    store.set("Sales", ("Red", "Units"), Decimal("0.1"))
    store.set("Sales", ("Blue", "Units"), Decimal("0.2"))
    evidence = evaluation.explain(model, store, [("Sales", ("Total", "Units"))])
    # 2 * (0.5 * 0.1 + 0.2) - 3 * 0.1 = 0.2; Red contributes along both paths.
    expected = 2 * (Decimal("0.5") * Decimal("0.1") + Decimal("0.2")) - 3 * Decimal("0.1")
    assert node(evidence, "Sales", "Total", "Units")["value"] == expected
    assert len(evidence["cells"]) == 5
    for calculation in evidence["cells"].values():
        if calculation["kind"] == "consolidation":
            assert sum(row["contribution"] for row in calculation["contributions"]) == calculation["value"]
            for row in calculation["contributions"]:
                assert row["value"] == evidence["cells"][row["cell"]]["value"]
                assert row["contribution"] == row["value"] * row["weight"]


@pytest.mark.parametrize("divisor, expected", [("0", "0"), ("2", "5")])
def test_safe_divide_records_its_actual_result(tmp_path, divisor, expected):
    model, store = build_model(tmp_path, "['Amount'] = N: ['Units'] \\ ['Price'];")
    store.set("Sales", ("Red", "Units"), Decimal("10"))
    store.set("Sales", ("Red", "Price"), Decimal(divisor))
    evidence = evaluation.explain(model, store, [("Sales", ("Red", "Amount"))])
    assert node(evidence, "Sales", "Red", "Amount")["steps"] == [{
        "operator": "\\", "left": Decimal("10"), "right": Decimal(divisor),
        "result": Decimal(expected),
    }]


def test_explanation_recalculates_rules_and_does_not_reuse_a_previous_run():
    model, store = loaded_store(Sales__Red__Units="3", Sales__Red__Price="2")
    store.set("Sales", ("Red", "Amount"), Decimal("999"))
    cells = [("Sales", ("Red", "Amount"))]
    first = evaluation.explain(model, store, cells)
    store.set("Sales", ("Red", "Price"), Decimal("4"))
    second = evaluation.explain(model, store, cells)
    assert node(first, "Sales", "Red", "Amount")["value"] == Decimal("3") * 2
    assert node(second, "Sales", "Red", "Amount")["value"] == Decimal("3") * 4
    assert store.get("Sales", ("Red", "Amount")) == Decimal("999")


@pytest.mark.parametrize("units, branch, expected", [("0", "then", "7"), ("2", "else", "5")])
def test_explanation_records_only_the_selected_if_branch(tmp_path, units, branch, expected):
    model, store = build_model(
        tmp_path, "['Amount'] = N: IF(['Units'] = 0, 7, ['Price'] / ['Units']);",
    )
    store.set("Sales", ("Red", "Units"), Decimal(units))
    store.set("Sales", ("Red", "Price"), Decimal("10"))
    evidence = evaluation.explain(model, store, [("Sales", ("Red", "Amount"))])
    result = node(evidence, "Sales", "Red", "Amount")
    # With zero units the unused division must not run; otherwise 10 / 2 = 5.
    assert result["value"] == Decimal(expected)
    assert result["steps"][0]["result"] is (units == "0")
    assert result["steps"][1] == {"operator": "IF", "branch": branch}
    assert ("Sales['Red', 'Price']" in evidence["cells"]) is (units != "0")


def test_c_rule_overrides_consolidation_in_explanation(tmp_path):
    model, store = build_model(tmp_path, "['Price'] = C: 99;")
    evidence = evaluation.explain(model, store, [("Sales", ("Total", "Price"))])
    total = node(evidence, "Sales", "Total", "Price")
    assert total["kind"] == "rule"
    assert total["value"] == Decimal("99")
    assert "contributions" not in total
    assert len(evidence["cells"]) == 1


@pytest.mark.parametrize("rule, error", [
    ("['Amount'] = N: ['Units'] / ['Price'];", EvaluationError),
    ("['Amount'] = N: ['Amount'];", CircularReference),
])
def test_explanation_preserves_calculation_errors(tmp_path, rule, error):
    model, store = build_model(tmp_path, rule)
    with pytest.raises(error):
        evaluation.explain(model, store, [("Sales", ("Red", "Amount"))])


def test_explain_cli_outputs_exact_decimal_strings_and_is_repeatable(tmp_path):
    data = tmp_path / "data"
    data.mkdir()
    (data / "Sales.csv").write_text(
        "Colour,Measure,Value\nRed,Units,3\nRed,Price,0.10\n", encoding="utf-8",
    )
    args = ("explain", str(MINI), "--data", str(data), "--cell", "sales:red,amount")
    code, out, err = run_once(*args)
    assert code == 0, err
    assert err == ""
    result = node(json.loads(out), "Sales", "Red", "Amount")
    assert result["value"] == "0.3"  # 3 * 0.10 exactly, without a float conversion.
    assert result["steps"] == [{"operator": "*", "left": "3", "right": "0.1", "result": "0.3"}]
    assert run_once(*args) == (code, out, err)


def test_explain_cli_accepts_a_relative_model_directory(tmp_path, monkeypatch):
    write_model(tmp_path / "model", rules="['Amount'] = N: 2 * 3;")
    data = tmp_path / "data"
    data.mkdir()
    (data / "Sales.csv").write_text("Colour,Measure,Value\n", encoding="utf-8")
    monkeypatch.chdir(tmp_path)
    code, out, err = run_once(
        "explain", "model", "--data", "data", "--cell", "Sales:Red,Amount",
    )
    assert code == 0, err
    result = node(json.loads(out), "Sales", "Red", "Amount")
    assert result["value"] == "6"  # The selected rule is 2 * 3.
    assert result["rule"] == {"file": "cubes/Sales.rules", "line": 1}


@pytest.mark.parametrize("cell, rules, expected_code", [
    ("Sales:Red", "", 1),
    ("Sales:Red,Amount", "['Ghost'] = N: 1;", 2),
    ("Sales:Red,Amount", "['Amount'] = N: ['Units'] / ['Price'];", 3),
])
def test_explain_cli_fails_without_partial_json(tmp_path, cell, rules, expected_code):
    model = write_model(tmp_path / "model", rules=rules)
    data = tmp_path / "data"
    data.mkdir()
    (data / "Sales.csv").write_text("Colour,Measure,Value\n", encoding="utf-8")
    code, out, err = run_once(
        "explain", str(model), "--data", str(data), "--cell", "Sales:Red,Units", "--cell", cell,
    )
    assert code == expected_code
    assert out == ""
    assert err
    assert "invalid choice" not in err
