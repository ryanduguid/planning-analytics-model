"""The repository paths and the model writer the test modules share.

pytest puts this directory on the import path, so a test module takes the paths
from here rather than counting directories up from its own ``__file__``, and
writes a model for a test by calling ``write_model`` rather than spelling the
same dimension, cube and manifest JSON a third time.

``write_model`` carries every option the three callers between them need: the
measures of the Measure dimension, the dimensions the cube links, the cubes the
manifest lists, a rules file, a TurboIntegrator process, and files written into
the tree that the manifest does not list. It returns the root it wrote, and
leaves loading to the caller, because a test that asserts a load failure needs
the tree without the load.
"""

import json
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[1]
MODEL_ROOT = REPO / "model"
EXAMPLES = REPO / "examples"

# Red and Blue roll up to Total, which is what a test needs to tell a leaf cell
# from a consolidated one.
COLOUR_ELEMENTS = (
    '{"Name": "Total", "Type": "Consolidated"}, {"Name": "Red", "Type": "Numeric"},'
    ' {"Name": "Blue", "Type": "Numeric"}'
)
COLOUR_EDGES = (
    '{"ParentName": "Total", "ComponentName": "Red", "Weight": 1},'
    ' {"ParentName": "Total", "ComponentName": "Blue", "Weight": 1}'
)
MEASURES = (
    '{"Name": "Units", "Type": "Numeric"}, {"Name": "Price", "Type": "Numeric"},'
    ' {"Name": "Amount", "Type": "Numeric"}'
)
CUBE_DIMENSIONS = '"../dimensions/Colour.json", "../dimensions/Measure.json"'


@pytest.fixture(scope="session")
def repo() -> Path:
    """The repository root, for a test that reads a tracked file by name."""
    return REPO


def write_model(
    root: Path,
    rules: str = "",
    measures: str = MEASURES,
    cube_dimensions: str = CUBE_DIMENSIONS,
    extra_files: dict | None = None,
    manifest_cubes: str = '"cubes/Sales.json"',
    processes: str = "",
    parameters: tuple[dict, ...] = (),
) -> Path:
    """Write the smallest model that can carry the defect under test."""
    (root / "dimensions").mkdir(parents=True, exist_ok=True)
    (root / "cubes").mkdir(parents=True, exist_ok=True)
    for name, elements, edges in (
        ("Colour", COLOUR_ELEMENTS, COLOUR_EDGES),
        ("Measure", measures, ""),
    ):
        (root / "dimensions" / f"{name}.json").write_text(
            '{"Name": "%s", "Hierarchies@Code.links": ["%s.hierarchies/%s.json"]}'
            % (name, name, name),
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
            json.dumps(
                {
                    "Name": "Load",
                    "Parameters": parameters,
                    "DataSource": {},
                    "Code@Code.link": "Load.ti",
                }
            ),
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
    return root
