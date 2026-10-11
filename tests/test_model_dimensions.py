"""Loading dimensions and weighted hierarchies out of git native model source."""

import json
from decimal import Decimal
from pathlib import Path

import pytest

from pacioliscube.model import ModelError, load_dimension

MINI = Path(__file__).parent / "fixtures" / "mini"
COLOUR = MINI / "dimensions" / "Colour.json"


def write_dimension(root: Path, name: str, elements: str, edges: str) -> Path:
    """Write a one hierarchy dimension in git native shape and return its file."""
    dimension_file = root / f"{name}.json"
    dimension_file.write_text(
        '{"Name": "%s", "Hierarchies@Code.links": ["%s.hierarchies/%s.json"]}' % (name, name, name),
        encoding="utf-8",
    )
    hierarchy_dir = root / f"{name}.hierarchies"
    hierarchy_dir.mkdir(exist_ok=True)
    (hierarchy_dir / f"{name}.json").write_text(
        '{"Name": "%s", "Elements": [%s], "Edges": [%s]}' % (name, elements, edges),
        encoding="utf-8",
    )
    return dimension_file


def test_loads_elements_and_edges():
    dimension = load_dimension(COLOUR)
    hierarchy = dimension.default_hierarchy
    assert dimension.name == "Colour"
    assert set(hierarchy.elements) == {"Total", "Red", "Blue", "Contra"}
    assert hierarchy.elements["Total"].element_type == "Consolidated"
    assert hierarchy.is_leaf("Red") is True
    assert hierarchy.is_leaf("Total") is False


@pytest.mark.parametrize("invalid", ["null", "12", '"name"', "[]"])
@pytest.mark.parametrize("collection", ["Elements", "Edges"])
def test_hierarchy_entries_must_be_objects(tmp_path, invalid, collection):
    path = write_dimension(tmp_path, "Test", invalid if collection == "Elements" else "",
                           invalid if collection == "Edges" else "")
    with pytest.raises(ModelError, match="expected an object") as caught:
        load_dimension(path)
    assert "Test.hierarchies" in str(caught.value)


@pytest.mark.parametrize("invalid", [None, 12, "entries", {}, True])
@pytest.mark.parametrize("collection", ["Elements", "Edges"])
def test_hierarchy_collections_must_be_lists(tmp_path, invalid, collection):
    path = write_dimension(tmp_path, "Test", "", "")
    source = tmp_path / "Test.hierarchies" / "Test.json"
    payload = json.loads(source.read_text(encoding="utf-8"))
    payload[collection] = invalid
    source.write_text(json.dumps(payload), encoding="utf-8")
    with pytest.raises(ModelError, match=rf"{collection} must be a list") as caught:
        load_dimension(path)
    assert str(source) in str(caught.value)  # nosec B101: pytest assertion


@pytest.mark.parametrize("omitted", [False, True])
def test_empty_or_omitted_hierarchy_collections_remain_valid(tmp_path, omitted):
    path = write_dimension(tmp_path, "Test", "", "")
    source = tmp_path / "Test.hierarchies" / "Test.json"
    if omitted:
        source.write_text('{"Name": "Test"}', encoding="utf-8")
    hierarchy = load_dimension(path).default_hierarchy
    assert hierarchy.elements == {}  # nosec B101: pytest assertion
    assert hierarchy.edges == ()  # nosec B101: pytest assertion


@pytest.mark.parametrize("invalid", [[], {}, None, 12, True])
def test_element_types_must_be_strings(tmp_path, invalid):
    path = write_dimension(tmp_path, "Test", '{"Name": "Leaf"}', "")
    source = tmp_path / "Test.hierarchies" / "Test.json"
    payload = json.loads(source.read_text(encoding="utf-8"))
    payload["Elements"][0]["Type"] = invalid
    source.write_text(json.dumps(payload), encoding="utf-8")
    with pytest.raises(ModelError, match="has type") as caught:
        load_dimension(path)
    assert str(source) in str(caught.value)  # nosec B101: pytest assertion


def test_edge_weights_are_decimal_and_signed():
    hierarchy = load_dimension(COLOUR).default_hierarchy
    weights = {edge.component: edge.weight for edge in hierarchy.children("Total")}
    assert weights == {"Red": Decimal("1"), "Blue": Decimal("1"), "Contra": Decimal("-1")}
    assert all(isinstance(weight, Decimal) for weight in weights.values())


def test_a_fractional_weight_keeps_its_written_precision(tmp_path):
    dimension_file = write_dimension(
        tmp_path,
        "Share",
        '{"Name": "Total", "Type": "Consolidated"}, {"Name": "Part", "Type": "Numeric"}',
        '{"ParentName": "Total", "ComponentName": "Part", "Weight": 0.1}',
    )
    hierarchy = load_dimension(dimension_file).default_hierarchy
    assert hierarchy.children("Total")[0].weight == Decimal("0.1")


def test_element_lookup_is_case_insensitive():
    hierarchy = load_dimension(COLOUR).default_hierarchy
    assert hierarchy.is_leaf("red") is True
    assert hierarchy.resolve("rED") == "Red"


def test_resolving_an_unknown_element_raises():
    hierarchy = load_dimension(COLOUR).default_hierarchy
    with pytest.raises(ModelError) as caught:
        hierarchy.resolve("Green")
    assert "Green" in str(caught.value)


def test_leaves_under_returns_only_leaves_in_edge_order():
    hierarchy = load_dimension(COLOUR).default_hierarchy
    assert hierarchy.leaves_under("Total") == ("Red", "Blue", "Contra")


def test_leaves_under_a_leaf_is_the_leaf_itself():
    hierarchy = load_dimension(COLOUR).default_hierarchy
    assert hierarchy.leaves_under("Red") == ("Red",)


def test_a_component_shared_by_two_parents_is_reported_once(tmp_path):
    dimension_file = write_dimension(
        tmp_path,
        "Shared",
        '{"Name": "Top", "Type": "Consolidated"}, {"Name": "Left", "Type": "Consolidated"},'
        ' {"Name": "Right", "Type": "Consolidated"}, {"Name": "Leaf", "Type": "Numeric"}',
        '{"ParentName": "Top", "ComponentName": "Left", "Weight": 1},'
        ' {"ParentName": "Top", "ComponentName": "Right", "Weight": 1},'
        ' {"ParentName": "Left", "ComponentName": "Leaf", "Weight": 1},'
        ' {"ParentName": "Right", "ComponentName": "Leaf", "Weight": 1}',
    )
    hierarchy = load_dimension(dimension_file).default_hierarchy
    assert hierarchy.leaves_under("Top") == ("Leaf",)


def test_shared_subtrees_preserve_leaf_order_without_repeated_expansion(tmp_path, monkeypatch):
    layers = [(f"Left{level}", f"Right{level}") for level in range(12)]
    names = ["Top", *(name for layer in layers for name in layer), "First", "Second"]
    elements = [
        {"Name": name, "Type": "Numeric" if name in {"First", "Second"} else "Consolidated"}
        for name in names
    ]
    edges = [{"ParentName": "Top", "ComponentName": name, "Weight": 1} for name in layers[0]]
    for parents, children in zip(layers, layers[1:]):
        edges.extend(
            {"ParentName": parent, "ComponentName": child, "Weight": 1}
            for parent in parents for child in children
        )
    for parent, children in zip(layers[-1], (("First", "Second"), ("Second", "First"))):
        edges.extend(
            {"ParentName": parent, "ComponentName": child, "Weight": 1} for child in children
        )
    path = write_dimension(
        tmp_path, "Shared", ",".join(map(json.dumps, elements)), ",".join(map(json.dumps, edges))
    )
    hierarchy = load_dimension(path).default_hierarchy
    original_children = hierarchy.children
    expansions = 0

    def counted_children(name):
        nonlocal expansions
        expansions += 1
        return original_children(name)

    monkeypatch.setattr(hierarchy, "children", counted_children)
    assert hierarchy.leaves_under("top") == ("First", "Second")  # nosec B101: pytest assertion
    assert expansions <= 2 * len(edges)  # nosec B101: pytest assertion


def test_edge_naming_an_unknown_element_is_a_model_error(tmp_path):
    dimension_file = write_dimension(
        tmp_path,
        "Broken",
        '{"Name": "A", "Type": "Numeric"}',
        '{"ParentName": "A", "ComponentName": "Ghost", "Weight": 1}',
    )
    with pytest.raises(ModelError) as caught:
        load_dimension(dimension_file)
    message = str(caught.value)
    assert "Ghost" in message
    assert "Broken.json" in message.replace("\\", "/")


def test_a_cyclic_hierarchy_is_a_model_error(tmp_path):
    dimension_file = write_dimension(
        tmp_path,
        "Loop",
        '{"Name": "A", "Type": "Consolidated"}, {"Name": "B", "Type": "Consolidated"}',
        '{"ParentName": "A", "ComponentName": "B", "Weight": 1},'
        ' {"ParentName": "B", "ComponentName": "A", "Weight": 1}',
    )
    with pytest.raises(ModelError) as caught:
        load_dimension(dimension_file)
    message = str(caught.value)
    assert "A" in message and "B" in message


def test_a_missing_hierarchy_file_names_both_paths(tmp_path):
    dimension_file = tmp_path / "Absent.json"
    dimension_file.write_text(
        '{"Name": "Absent", "Hierarchies@Code.links": ["Absent.hierarchies/Absent.json"]}',
        encoding="utf-8",
    )
    with pytest.raises(ModelError) as caught:
        load_dimension(dimension_file)
    message = str(caught.value).replace("\\", "/")
    assert "Absent.hierarchies/Absent.json" in message
