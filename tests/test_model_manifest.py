"""Loading the manifest, cubes and processes of a whole model tree."""

from pathlib import Path

import pytest

from pacioliscube.model import ModelError, load_cube, load_model

MINI = Path(__file__).parent / "fixtures" / "mini"


def write_manifest(root: Path, objects: str) -> None:
    (root / "tm1project.json").write_text(
        '{"Version": 1.0, "Name": "x", "Objects": %s}' % objects, encoding="utf-8"
    )


def test_model_loads_every_object_the_manifest_lists():
    model = load_model(MINI)
    assert set(model.dimensions) == {"Colour", "Measure"}
    assert set(model.cubes) == {"Sales", "Cost"}
    assert set(model.processes) == {"Load"}


def test_a_cube_records_its_dimensions_in_order():
    cube = load_model(MINI).cubes["Sales"]
    assert cube.dimensions == ("Colour", "Measure")


def test_a_cube_loads_on_its_own_from_the_git_native_layout():
    # Every cube in this layout links its dimensions as ../dimensions/X.json, so
    # a loader fenced to the cube's own folder refuses the very file it exists
    # to read, and reports it as a path climbing out of the model root.
    cube = load_cube(MINI / "cubes" / "Sales.json")
    assert cube.dimensions == ("Colour", "Measure")


def test_a_cube_with_rules_carries_a_parsed_ruleset():
    cube = load_model(MINI).cubes["Sales"]
    assert cube.rules is not None
    assert cube.rules.skipcheck is True


def test_a_cube_without_a_rules_link_carries_no_ruleset():
    assert load_model(MINI).cubes["Cost"].rules is None


def test_a_cube_linking_a_missing_rules_file_names_both_paths(tmp_path):
    cube_file = tmp_path / "Ghost.json"
    cube_file.write_text(
        '{"Name": "Ghost", "Dimensions@Code.links": ["Colour.json"],'
        ' "Rules@Code.link": "Ghost.rules"}',
        encoding="utf-8",
    )
    (tmp_path / "Colour.json").write_text(
        '{"Name": "Colour", "Hierarchies@Code.links": ["Colour.hierarchies/Colour.json"]}',
        encoding="utf-8",
    )
    with pytest.raises(ModelError) as caught:
        load_cube(cube_file)
    message = str(caught.value).replace("\\", "/")
    assert "Ghost.json" in message
    assert "Ghost.rules" in message


def test_process_script_is_read_from_its_linked_file():
    process = load_model(MINI).processes["Load"]
    assert "sVersion = pVersion;" in process.script
    assert process.parameters[0]["Name"] == "pVersion"
    assert process.datasource["Type"] == "ASCII"


def test_the_file_list_covers_every_object_and_its_linked_text():
    model = load_model(MINI)
    names = {path.name for path in model.files}
    assert {"tm1project.json", "Sales.json", "Sales.rules", "Load.json", "Load.ti"} <= names


def test_manifest_naming_a_missing_file_is_a_model_error(tmp_path):
    write_manifest(tmp_path, '{"Cubes": ["cubes/Ghost.json"]}')
    with pytest.raises(ModelError) as caught:
        load_model(tmp_path)
    assert "cubes/Ghost.json" in str(caught.value).replace("\\", "/")


def test_a_path_escaping_the_model_root_is_refused(tmp_path):
    root = tmp_path / "model"
    root.mkdir()
    (tmp_path / "outside.json").write_text('{"Name": "Outside"}', encoding="utf-8")
    write_manifest(root, '{"Cubes": ["../outside.json"]}')
    with pytest.raises(ModelError) as caught:
        load_model(root)
    assert "outside the model root" in str(caught.value)


def test_a_missing_manifest_names_the_file(tmp_path):
    with pytest.raises(ModelError) as caught:
        load_model(tmp_path)
    assert "tm1project.json" in str(caught.value).replace("\\", "/")


def test_malformed_json_names_the_file(tmp_path):
    (tmp_path / "tm1project.json").write_text("{not json", encoding="utf-8")
    with pytest.raises(ModelError) as caught:
        load_model(tmp_path)
    assert "invalid JSON" in str(caught.value)
