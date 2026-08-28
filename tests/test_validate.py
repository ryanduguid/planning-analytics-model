"""Structural validation findings, one test per check."""

from pathlib import Path

import pytest

from pacioliscube import validate as validation
from pacioliscube.model import load_model
from pacioliscube.validate import validate_model

REPO = Path(__file__).resolve().parents[1]
MODEL_ROOT = REPO / "model"


def build_model(
    root: Path,
    rules: str = "",
    cube_dimensions: str = '"../dimensions/Colour.json", "../dimensions/Measure.json"',
    extra_files: dict | None = None,
    manifest_cubes: str = '"cubes/Sales.json"',
    processes: str = "",
):
    """Write the smallest model that can carry the defect under test."""
    (root / "dimensions").mkdir(parents=True, exist_ok=True)
    (root / "cubes").mkdir(parents=True, exist_ok=True)
    for name, elements, edges in (
        (
            "Colour",
            '{"Name": "Total", "Type": "Consolidated"}, {"Name": "Red", "Type": "Numeric"},'
            ' {"Name": "Blue", "Type": "Numeric"}',
            '{"ParentName": "Total", "ComponentName": "Red", "Weight": 1},'
            ' {"ParentName": "Total", "ComponentName": "Blue", "Weight": 1}',
        ),
        (
            "Measure",
            '{"Name": "Units", "Type": "Numeric"}, {"Name": "Price", "Type": "Numeric"},'
            ' {"Name": "Amount", "Type": "Numeric"}',
            "",
        ),
    ):
        (root / "dimensions" / f"{name}.json").write_text(
            '{"Name": "%s", "Hierarchies@Code.links": ["%s.hierarchies/%s.json"]}' % (name, name, name),
            encoding="utf-8",
        )
        hierarchy_dir = root / "dimensions" / f"{name}.hierarchies"
        hierarchy_dir.mkdir(exist_ok=True)
        (hierarchy_dir / f"{name}.json").write_text(
            '{"Name": "%s", "Elements": [%s], "Edges": [%s]}' % (name, elements, edges),
            encoding="utf-8",
        )
    rules_link = ', "Rules@Code.link": "Sales.rules"' if rules else ""
    (root / "cubes" / "Sales.json").write_text(
        '{"Name": "Sales", "Dimensions@Code.links": [%s]%s}' % (cube_dimensions, rules_link),
        encoding="utf-8",
    )
    if rules:
        (root / "cubes" / "Sales.rules").write_text(rules, encoding="utf-8")
    process_entry = ""
    if processes:
        (root / "processes").mkdir(exist_ok=True)
        (root / "processes" / "Load.json").write_text(
            '{"Name": "Load", "Parameters": [], "DataSource": {}, "Code@Code.link": "Load.ti"}',
            encoding="utf-8",
        )
        (root / "processes" / "Load.ti").write_text(processes, encoding="utf-8")
        process_entry = ', "Processes": ["processes/Load.json"]'
    (root / "tm1project.json").write_text(
        '{"Version": 1.0, "Name": "built", "Objects": {"Dimensions":'
        ' ["dimensions/Colour.json", "dimensions/Measure.json"], "Cubes": [%s]%s}}'
        % (manifest_cubes, process_entry),
        encoding="utf-8",
    )
    for name, content in (extra_files or {}).items():
        target = root / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content, encoding="utf-8")
    return load_model(root)


def codes(model):
    return {finding.code for finding in validate_model(model)}


def test_a_clean_model_reports_nothing(tmp_path):
    model = build_model(
        tmp_path,
        rules="SKIPCHECK;\n['Amount'] = N: ['Units'] * ['Price'];\nFEEDERS;\n['Units'] => ['Amount'];\n",
    )
    assert validate_model(model) == ()


def test_cube_validation_phases_preserve_exact_finding_order(tmp_path):
    (tmp_path / "dimensions").mkdir(parents=True)
    (tmp_path / "dimensions" / "Ghost.json").write_text(
        '{"Name": "Ghost", "Hierarchies@Code.links": ["Ghost.hierarchies/Ghost.json"]}',
        encoding="utf-8",
    )
    (tmp_path / "dimensions" / "Ghost.hierarchies").mkdir()
    (tmp_path / "dimensions" / "Ghost.hierarchies" / "Ghost.json").write_text(
        '{"Name": "Ghost", "Elements": [{"Name": "One", "Type": "Numeric"}], "Edges": []}',
        encoding="utf-8",
    )
    model = build_model(
        tmp_path,
        cube_dimensions='"../dimensions/Colour.json", "../dimensions/Ghost.json"',
        rules=(
            "SKIPCHECK;\n"
            "['Ghost'] = N: DB('Missing', 'Red');\n"
            "FEEDERS;\n"
            "['Units'] => DB('Missing', !Colour, 'Amount');\n"
        ),
    )
    cube = model.cubes["Sales"]
    cube_location = str(cube.source)
    rule_location = f"{cube.rules_source} line 2"
    feeder_location = f"{cube.rules_source} line 4"

    dimensions = [
        validation.Finding(
            "error",
            "DIM001",
            "cube 'Sales' names dimension 'Ghost', which the model does not hold",
            cube_location,
        )
    ]
    rules = [
        validation.Finding(
            "error",
            "ELE001",
            "no dimension of cube 'Sales' holds an element named 'Ghost'",
            rule_location,
        ),
        validation.Finding(
            "error",
            "DIM001",
            "a rule reads cube 'Missing', which the model does not hold",
            rule_location,
        ),
    ]
    feeders = [
        validation.Finding(
            "error",
            "ELE001",
            "no dimension of cube 'Sales' holds an element named 'Units'",
            feeder_location,
        ),
        validation.Finding(
            "error",
            "DIM001",
            "a feeder points at cube 'Missing', which the model does not hold",
            feeder_location,
        ),
    ]

    assert validation._validate_cube_dimensions(model, cube) == dimensions
    assert validation._validate_cube_rules(model, cube) == rules
    assert validation._validate_cube_feeders(model, cube) == feeders
    phase_findings = dimensions + rules + feeders
    assert validation._validate_cube(model, cube) == phase_findings
    assert validate_model(model)[: len(phase_findings)] == tuple(phase_findings)


def test_dim001_a_cube_naming_an_absent_dimension(tmp_path):
    (tmp_path / "dimensions").mkdir(parents=True)
    (tmp_path / "dimensions" / "Ghost.json").write_text(
        '{"Name": "Ghost", "Hierarchies@Code.links": ["Ghost.hierarchies/Ghost.json"]}',
        encoding="utf-8",
    )
    (tmp_path / "dimensions" / "Ghost.hierarchies").mkdir()
    (tmp_path / "dimensions" / "Ghost.hierarchies" / "Ghost.json").write_text(
        '{"Name": "Ghost", "Elements": [{"Name": "One", "Type": "Numeric"}], "Edges": []}',
        encoding="utf-8",
    )
    model = build_model(
        tmp_path,
        cube_dimensions='"../dimensions/Colour.json", "../dimensions/Ghost.json"',
        extra_files={},
    )
    # Ghost is on disk and linked by the cube but the manifest never lists it,
    # so the model does not hold it and the cube's reference dangles.
    assert "DIM001" in codes(model)


def test_ele001_a_rule_naming_an_unknown_element(tmp_path):
    model = build_model(
        tmp_path,
        rules="SKIPCHECK;\n['Ghost'] = N: 1;\nFEEDERS;\n['Units'] => ['Ghost'];\n",
    )
    assert "ELE001" in codes(model)


def test_ele001_a_db_call_naming_an_unknown_element(tmp_path):
    model = build_model(
        tmp_path,
        rules="SKIPCHECK;\n['Amount'] = N: DB('Sales', 'Red', 'Ghost');\nFEEDERS;\n['Units'] => ['Amount'];\n",
    )
    assert "ELE001" in codes(model)


def test_are001_a_db_call_with_the_wrong_number_of_coordinates(tmp_path):
    model = build_model(
        tmp_path,
        rules="SKIPCHECK;\n['Amount'] = N: DB('Sales', 'Red');\nFEEDERS;\n['Units'] => ['Amount'];\n",
    )
    assert "ARE001" in codes(model)


def test_rul001_a_rule_targeting_a_consolidation_without_the_c_qualifier(tmp_path):
    model = build_model(
        tmp_path,
        rules="SKIPCHECK;\n['Total'] = N: 1;\nFEEDERS;\n['Units'] => ['Total'];\n",
    )
    assert "RUL001" in codes(model)


def test_a_c_qualified_rule_on_a_consolidation_is_accepted(tmp_path):
    model = build_model(
        tmp_path,
        rules="SKIPCHECK;\n['Total'] = C: 1;\nFEEDERS;\n['Units'] => ['Total'];\n",
    )
    assert "RUL001" not in codes(model)


def test_fed001_skipcheck_with_rules_and_no_feeders(tmp_path):
    model = build_model(tmp_path, rules="SKIPCHECK;\n['Amount'] = N: 1;\n")
    assert "FED001" in codes(model)


def test_fed002_a_calculated_area_no_feeder_points_at(tmp_path):
    model = build_model(
        tmp_path,
        rules="SKIPCHECK;\n['Amount'] = N: 1;\n['Price'] = N: 2;\nFEEDERS;\n['Units'] => ['Amount'];\n",
    )
    findings = {finding.code: finding for finding in validate_model(model)}
    assert "FED002" in findings
    assert findings["FED002"].severity == "warning"


def test_a_cross_cube_feeder_satisfies_fed002_in_the_target_cube(tmp_path):
    # Sales feeds nothing internally; a second cube feeds Sales' calculated area,
    # which is exactly how a reporting cube is fed in a real model.
    model = build_model(
        tmp_path,
        rules="SKIPCHECK;\n['Amount'] = N: 1;\nFEEDERS;\n['Units'] => ['Price'];\n",
        extra_files={
            "cubes/Feeder.json": '{"Name": "Feeder", "Dimensions@Code.links":'
            ' ["../dimensions/Colour.json", "../dimensions/Measure.json"],'
            ' "Rules@Code.link": "Feeder.rules"}',
            "cubes/Feeder.rules": "SKIPCHECK;\nFEEDERS;\n['Units'] => DB('Sales', !Colour, 'Amount');\n",
        },
        manifest_cubes='"cubes/Sales.json", "cubes/Feeder.json"',
    )
    assert "FED002" not in codes(model)


def test_a_cross_cube_feeder_naming_an_unknown_cube_is_an_error(tmp_path):
    model = build_model(
        tmp_path,
        rules="SKIPCHECK;\nFEEDERS;\n['Units'] => DB('Ghost', !Colour, 'Amount');\n",
    )
    assert "DIM001" in codes(model)


def test_a_cross_cube_feeder_naming_an_unknown_element_is_an_error(tmp_path):
    model = build_model(
        tmp_path,
        rules="SKIPCHECK;\nFEEDERS;\n['Units'] => DB('Sales', !Colour, 'Ghost');\n",
    )
    assert "ELE001" in codes(model)


def test_man001_a_file_the_manifest_does_not_list(tmp_path):
    model = build_model(
        tmp_path,
        rules="SKIPCHECK;\n['Amount'] = N: 1;\nFEEDERS;\n['Units'] => ['Amount'];\n",
        extra_files={"cubes/Forgotten.json": '{"Name": "Forgotten"}'},
    )
    findings = [f for f in validate_model(model) if f.code == "MAN001"]
    assert findings
    assert "Forgotten.json" in findings[0].location


def test_prc001_a_script_using_an_undeclared_parameter(tmp_path):
    model = build_model(tmp_path, processes="sVersion = pVersion;\n")
    assert "PRC001" in codes(model)


def test_a_declared_parameter_is_not_reported(tmp_path):
    model = build_model(tmp_path, processes="sVersion = 'Budget';\n")
    assert "PRC001" not in codes(model)


@pytest.mark.xfail(
    not (MODEL_ROOT / "tm1project.json").is_file(),
    reason="the shipped model tree lands with the cubes",
    strict=True,
)
def test_the_shipped_model_validates_clean():
    errors = [f for f in validate_model(load_model(MODEL_ROOT)) if f.severity == "error"]
    assert errors == []


def test_prose_in_a_comment_is_not_read_as_a_parameter(tmp_path):
    # "per" begins with p, and an earlier pattern flagged it as an undeclared
    # parameter, so the comment text of every shipped process failed the check.
    model = build_model(
        tmp_path,
        processes="#Region Prolog\n# One column per cube dimension, put in place.\n#EndRegion\n",
    )
    assert "PRC001" not in codes(model)


def test_a_hash_inside_a_quoted_string_does_not_start_a_comment(tmp_path):
    model = build_model(
        tmp_path,
        processes="#Region Prolog\nsTag = 'run #1';\nCellPutN(1, 'Sales', pGhost);\n#EndRegion\n",
    )
    assert "PRC001" in codes(model)


def test_an_undeclared_parameter_in_live_code_is_still_caught(tmp_path):
    model = build_model(
        tmp_path,
        processes="#Region Prolog\nnValue = CellGetN('Sales', pYear, 'Amount');\n#EndRegion\n",
    )
    assert "PRC001" in codes(model)
