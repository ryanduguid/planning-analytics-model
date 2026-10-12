"""The refusal and error paths the scenario tests do not reach.

Each test names the defect a path guards against, so a later change that drops
the guard fails here rather than turning a clean refusal into a traceback.
"""

import argparse
import importlib
import json
from decimal import Decimal
from pathlib import Path

import pytest

import pacioliscube
from conftest import MEASURES, write_model
from pacioliscube import cli
from pacioliscube import evaluate as engine
from pacioliscube.data import load_csv, load_data, load_into_store
from pacioliscube.errors import EXIT_CALCULATION, EXIT_INVALID_MODEL, EXIT_USAGE, CliError
from pacioliscube.evaluate import CellStore, EvaluationError
from pacioliscube.model import ModelError, load_dimension, load_model
from pacioliscube.rules import BinaryOp, CellRef, Comparison, Number, RuleSyntaxError, parse_rules
from pacioliscube.validate import validate_model


def _edit_json(path: Path, change) -> None:
    payload = json.loads(path.read_text(encoding="utf-8"))
    change(payload)
    path.write_text(json.dumps(payload), encoding="utf-8")


def _colour_hierarchy(root: Path) -> Path:
    return root / "dimensions" / "Colour.hierarchies" / "Colour.json"


# The model loader: each structural refusal reports the file it read.


@pytest.mark.parametrize(
    "change,message",
    [
        (lambda root: (root / "tm1project.json").write_text("[]", encoding="utf-8"),
         "expected a JSON object at the top level"),
        (lambda root: _edit_json(_colour_hierarchy(root), lambda h: h["Elements"].append({"Name": "red"})),
         "element 'red' is declared twice"),
        (lambda root: _edit_json(_colour_hierarchy(root), lambda h: h.pop("Name")),
         "hierarchy has no Name"),
        (lambda root: _edit_json(_colour_hierarchy(root), lambda h: h["Elements"].append({"Type": "Numeric"})),
         "an element has no Name"),
        (lambda root: _edit_json(_colour_hierarchy(root), lambda h: h["Elements"].append({"Name": "Pink", "Type": "Text"})),
         "element 'Pink' has type 'Text'"),
        (lambda root: _edit_json(_colour_hierarchy(root), lambda h: h["Edges"].append({"ParentName": "Total"})),
         "an edge is missing ParentName or ComponentName"),
        (lambda root: _edit_json(root / "dimensions" / "Colour.json", lambda d: d.pop("Name")),
         "dimension has no Name"),
        (lambda root: _edit_json(root / "dimensions" / "Colour.json",
                                 lambda d: d["Hierarchies@Code.links"].append("Colour.hierarchies/Colour.json")),
         "hierarchy 'Colour' is linked twice"),
        (lambda root: _edit_json(root / "cubes" / "Sales.json", lambda c: c.pop("Name")),
         "cube has no Name"),
        (lambda root: _edit_json(root / "cubes" / "Sales.json",
                                 lambda c: c["Dimensions@Code.links"].append("../dimensions/Missing.json")),
         "linked dimension file '../dimensions/Missing.json' not found"),
        (lambda root: _edit_json(root / "tm1project.json", lambda m: m.update(Objects=[])),
         "Objects must be an object"),
        (lambda root: _edit_json(root / "tm1project.json",
                                 lambda m: m["Objects"]["Dimensions"].append("dimensions/Missing.json")),
         "lists 'dimensions/Missing.json', which is not on disk"),
        (lambda root: _edit_json(root / "tm1project.json",
                                 lambda m: m["Objects"].update(Processes=["processes/Missing.json"])),
         "lists 'processes/Missing.json', which is not on disk"),
    ],
)
def test_a_malformed_model_tree_is_refused_with_its_reason(tmp_path, change, message):
    root = write_model(tmp_path)
    change(root)
    with pytest.raises(ModelError, match=message):
        load_model(root)


@pytest.mark.parametrize(
    "change,message",
    [
        (lambda process: process.pop("Name"), "process has no Name"),
        (lambda process: process.update({"Code@Code.link": "Missing.ti"}),
         "process 'Load' links script 'Missing.ti', which is missing"),
    ],
)
def test_a_malformed_process_is_refused(tmp_path, change, message):
    root = write_model(tmp_path, processes="#****Begin: Prolog script****\n")
    _edit_json(root / "processes" / "Load.json", change)
    with pytest.raises(ModelError, match=message):
        load_model(root)


def test_the_default_hierarchy_falls_back_to_the_only_one(tmp_path):
    root = write_model(tmp_path)
    _edit_json(_colour_hierarchy(root), lambda h: h.update(Name="Alternate"))
    dimension = load_dimension(root / "dimensions" / "Colour.json")
    assert dimension.default_hierarchy.name == "Alternate"


def test_no_default_hierarchy_among_several_is_refused(tmp_path):
    root = write_model(tmp_path)
    hierarchy_dir = root / "dimensions" / "Colour.hierarchies"
    _edit_json(hierarchy_dir / "Colour.json", lambda h: h.update(Name="First"))
    second = json.loads((hierarchy_dir / "Colour.json").read_text(encoding="utf-8"))
    second["Name"] = "Second"
    (hierarchy_dir / "Second.json").write_text(json.dumps(second), encoding="utf-8")
    _edit_json(root / "dimensions" / "Colour.json",
               lambda d: d["Hierarchies@Code.links"].append("Colour.hierarchies/Second.json"))
    dimension = load_dimension(root / "dimensions" / "Colour.json")
    with pytest.raises(ModelError, match="no hierarchy named 'Colour' to use as the default"):
        dimension.default_hierarchy


def test_an_unknown_dimension_lookup_is_a_model_error(tmp_path):
    model = load_model(write_model(tmp_path))
    with pytest.raises(ModelError, match="no dimension named 'Region'"):
        model.hierarchy("Region")


# The package version and the data loader.


def test_a_source_tree_without_installation_reports_a_development_version(monkeypatch):
    def not_installed(name):
        raise importlib.metadata.PackageNotFoundError(name)

    monkeypatch.setattr(importlib.metadata, "version", not_installed)
    try:
        assert importlib.reload(pacioliscube).__version__ == "0.0.0.dev0"
    finally:
        monkeypatch.undo()
        importlib.reload(pacioliscube)


def test_data_loader_refusals(tmp_path):
    model = load_model(write_model(tmp_path / "model"))
    cube = model.cubes["Sales"]
    with pytest.raises(ModelError, match="file not found"):
        list(load_csv(tmp_path / "missing.csv", cube, model))
    empty = tmp_path / "empty.csv"
    empty.write_text("", encoding="utf-8")
    with pytest.raises(ModelError, match="the file is empty, expected a header row"):
        list(load_csv(empty, cube, model))
    with pytest.raises(ModelError, match="no cube named 'Budget'"):
        load_into_store(model, "Budget", empty, CellStore())
    with pytest.raises(CliError, match="the data directory could not be read") as refused:
        load_data(model, tmp_path / "no-such-directory")
    assert refused.value.code == EXIT_USAGE


# The engine.


def test_cell_store_refuses_floats_and_filters_by_cube():
    store = CellStore()
    with pytest.raises(TypeError, match="never float"):
        store.set("Sales", ("Red", "Units"), 1.5)
    store.set("Sales", ("Red", "Units"), Decimal("1"))
    store.set("Plan", ("Red", "Units"), Decimal("2"))
    assert [value for _, _, value in store.items("Plan")] == [Decimal("2")]


def _engine(tmp_path, rules=""):
    model = load_model(write_model(tmp_path, rules=rules))
    return model, engine._Engine(model, CellStore())


def test_engine_refuses_an_unknown_cube_and_unsupported_nodes(tmp_path):
    model, run = _engine(tmp_path)
    cube = model.cubes["Sales"]
    coordinate = ("Red", "Units")
    run.record("steps", operator="+")  # no trace: recording is a no-op
    with pytest.raises(ModelError, match="no cube named 'Budget'"):
        run.value("Budget", coordinate)
    with pytest.raises(EvaluationError, match="unsupported operator '%'"):
        run.evaluate_expression(BinaryOp("%", Number(Decimal("1")), Number(Decimal("2"))), cube, coordinate)
    with pytest.raises(EvaluationError, match="unsupported expression node Comparison"):
        run.evaluate_expression(Comparison("=", Number(Decimal("1")), Number(Decimal("1"))), cube, coordinate)
    with pytest.raises(EvaluationError, match="unsupported comparison '!='"):
        run.evaluate_condition(Comparison("!=", Number(Decimal("1")), Number(Decimal("1"))), cube, coordinate)


def test_consolidation_of_an_all_leaf_coordinate_reads_the_store(tmp_path):
    model, run = _engine(tmp_path)
    run.store.set("Sales", ("Red", "Units"), Decimal("4"))
    assert run.consolidated(model.cubes["Sales"], ("Red", "Units")) == Decimal("4")


def test_rule_arithmetic_overflow_is_a_calculation_error(tmp_path):
    model, run = _engine(tmp_path, rules="['Amount'] = ['Units'] * ['Price'];\n")
    run.store.set("Sales", ("Red", "Units"), Decimal("9E+999999"))
    run.store.set("Sales", ("Red", "Price"), Decimal("9E+999999"))
    with pytest.raises(EvaluationError, match="arithmetic failed"):
        run.value("Sales", ("Red", "Amount"))


def test_a_rule_area_too_large_to_materialise_is_refused(tmp_path, monkeypatch):
    monkeypatch.setattr(engine, "MAX_MATERIALISED_CELLS", 1)
    model = load_model(write_model(tmp_path, rules="['Amount'] = ['Units'] * ['Price'];\n"))
    with pytest.raises(EvaluationError, match="expands to more than 1 cells"):
        engine.evaluate(model, CellStore())


# The rules parser.


@pytest.mark.parametrize(
    "text,message",
    [
        ("['Amount'] = ;\n", "unexpected ';' in an expression"),
        ("['Amount'] = IF(['Units'], 1, 0);\n", "expected a comparison operator inside IF"),
        ("1;\n", "a statement must start with an area in square brackets"),
        ("FEEDERS;\n['Amount'] = 1;\n", "a statement after FEEDERS must be a feeder using =>"),
        ("['Amount'] = N + 1;\n", "N is outside the supported rule grammar"),
    ],
)
def test_rule_syntax_errors(text, message):
    with pytest.raises(RuleSyntaxError, match=message):
        parse_rules(text, Path("Sales.rules"))


def test_negating_a_cell_reference_subtracts_it_from_zero():
    rule = parse_rules("['Amount'] = -['Units'];\n", Path("Sales.rules")).rules[0]
    assert rule.expression == BinaryOp("-", Number(Decimal("0")), CellRef(None, ("Units",)))


def test_undefvals_is_accepted_and_ignored():
    rules = parse_rules("UNDEFVALS;\n['Amount'] = 1;\n", Path("Sales.rules"))
    assert not rules.skipcheck
    assert len(rules.rules) == 1


# Validation.


def _codes(model):
    return [(finding.code, finding.message) for finding in validate_model(model)]


def test_an_element_two_dimensions_hold_is_ambiguous(tmp_path):
    model = load_model(write_model(
        tmp_path,
        measures=MEASURES + ', {"Name": "Red", "Type": "Numeric"}',
        rules="['Amount'] = ['Red'];\n",
    ))
    assert any(code == "ARE001" and "'Red' is ambiguous" in message for code, message in _codes(model))


def test_a_db_argument_naming_a_missing_dimension_is_reported(tmp_path):
    model = load_model(write_model(tmp_path, rules="['Amount'] = DB('Sales', !Region, 'Units');\n"))
    assert ("ELE001", "!Region names a dimension that cube 'Sales' does not have") in _codes(model)


def test_a_feeder_target_naming_a_missing_dimension_is_reported(tmp_path):
    model = load_model(write_model(
        tmp_path, rules="SKIPCHECK;\nFEEDERS;\n['Units'] => DB('Sales', !Region, 'Amount');\n"
    ))
    assert ("ELE001", "!Region names a dimension that cube 'Sales' does not have") in _codes(model)


# The command line's backstops.


def test_engine_model_errors_take_the_invalid_model_code(tmp_path, monkeypatch):
    model = load_model(write_model(tmp_path))

    def model_fault(*args, **kwargs):
        raise ModelError("structural fault")

    monkeypatch.setattr(cli, "evaluate", model_fault)
    with pytest.raises(CliError) as refused:
        cli._evaluate(model, CellStore())
    assert refused.value.code == EXIT_INVALID_MODEL

    monkeypatch.setattr(cli, "consolidate_many", model_fault)
    with pytest.raises(CliError) as refused:
        list(cli._values(model, CellStore(), [("Sales", ("Red", "Units"))]))
    assert refused.value.code == EXIT_INVALID_MODEL


def test_a_cell_reference_with_an_empty_element_is_a_usage_error(tmp_path):
    model = load_model(write_model(tmp_path))
    with pytest.raises(CliError, match="has an empty element name") as refused:
        cli._parse_cell(model, "Sales:Red,")
    assert refused.value.code == EXIT_USAGE


@pytest.mark.parametrize(
    "error,code",
    [
        (EvaluationError("calculation fault"), EXIT_CALCULATION),
        (ModelError("model fault"), EXIT_INVALID_MODEL),
        (OSError("disk fault"), EXIT_USAGE),
    ],
)
def test_main_maps_an_unexpected_error_to_its_exit_code(monkeypatch, capsys, error, code):
    def handler(args):
        raise error

    class _Parser:
        def parse_args(self, argv):
            return argparse.Namespace(handler=handler)

    monkeypatch.setattr(cli, "build_parser", _Parser)
    assert cli.main(["anything"]) == code
    assert capsys.readouterr().err == f"pacioliscube: {error}\n"


def test_consolidation_overflow_is_a_calculation_error(tmp_path):
    model, run = _engine(tmp_path)
    run.store.set("Sales", ("Red", "Units"), Decimal("9E+999999"))
    run.store.set("Sales", ("Blue", "Units"), Decimal("9E+999999"))
    with pytest.raises(EvaluationError, match="arithmetic failed"):
        run.value("Sales", ("Total", "Units"))


def test_a_calculation_error_while_reading_cells_takes_the_calculation_code(tmp_path, monkeypatch):
    model = load_model(write_model(tmp_path))

    def calculation_fault(*args, **kwargs):
        raise EvaluationError("division by zero")

    monkeypatch.setattr(cli, "consolidate_many", calculation_fault)
    with pytest.raises(CliError) as refused:
        list(cli._values(model, CellStore(), [("Sales", ("Red", "Units"))]))
    assert refused.value.code == EXIT_CALCULATION
