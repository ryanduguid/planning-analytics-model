"""The shipped model's structure: the things a reviewer would check by eye."""

from pacioliscube.model import load_model

from conftest import MODEL_ROOT


EXPECTED_DIMENSIONS = {
    "Year",
    "Period",
    "Version",
    "Entity",
    "CostCentre",
    "Account",
    "Role",
    "Fleet",
    "DriverMeasure",
    "WorkforceMeasure",
    "RevenueMeasure",
    "CapexMeasure",
    "PnLMeasure",
}


def test_the_model_ships_every_dimension():
    assert set(load_model(MODEL_ROOT).dimensions) == EXPECTED_DIMENSIONS


def test_period_rolls_twelve_months_into_four_quarters_into_a_year():
    hierarchy = load_model(MODEL_ROOT).dimensions["Period"].default_hierarchy
    assert len(hierarchy.leaves_under("FY")) == 12
    assert len(hierarchy.children("FY")) == 4
    assert hierarchy.leaves_under("Q1") == ("Jul", "Aug", "Sep")


def test_the_financial_year_starts_in_july():
    hierarchy = load_model(MODEL_ROOT).dimensions["Period"].default_hierarchy
    assert hierarchy.leaves_under("FY")[0] == "Jul"
    assert hierarchy.leaves_under("FY")[-1] == "Jun"


def test_group_consolidates_both_operating_companies():
    hierarchy = load_model(MODEL_ROOT).dimensions["Entity"].default_hierarchy
    assert {edge.component for edge in hierarchy.children("Group")} == {"CivilCo", "HaulCo"}


def test_the_account_tree_reaches_ebit():
    hierarchy = load_model(MODEL_ROOT).dimensions["Account"].default_hierarchy
    assert "EBIT" in hierarchy.elements
    assert hierarchy.elements["EBIT"].element_type == "Consolidated"


def test_costs_subtract_through_negative_edge_weights():
    hierarchy = load_model(MODEL_ROOT).dimensions["Account"].default_hierarchy
    weights = {edge.component: edge.weight for edge in hierarchy.children("Gross Margin")}
    assert weights["Revenue"] == 1
    assert weights["Direct Costs"] == -1
    ebit_weights = {edge.component: edge.weight for edge in hierarchy.children("EBIT")}
    assert ebit_weights["Depreciation"] == -1


def test_every_account_leaf_reaches_ebit():
    hierarchy = load_model(MODEL_ROOT).dimensions["Account"].default_hierarchy
    reachable = set(hierarchy.leaves_under("EBIT"))
    leaves = {name for name in hierarchy.elements if hierarchy.is_leaf(name)}
    assert reachable == leaves
