"""Reading long format CSV input into cube coordinates."""

from decimal import Decimal
from pathlib import Path

import pytest

from conftest import EXAMPLES, MODEL_ROOT
from pacioliscube.data import load_csv, load_into_store
from pacioliscube.evaluate import CellStore
from pacioliscube.model import ModelError, load_model

MODEL = load_model(MODEL_ROOT)
DRIVERS = MODEL.cubes["Drivers"]

HEADER = "Year,Version,Period,DriverMeasure,Value\n"


def write(tmp_path: Path, body: str) -> Path:
    path = tmp_path / "input.csv"
    path.write_text(HEADER + body, encoding="utf-8")
    return path


def test_a_long_format_csv_loads_into_coordinates(tmp_path):
    path = write(tmp_path, "FY2026-27,Budget,Full Year,SG Rate,0.12\n")
    rows = list(load_csv(path, DRIVERS, MODEL))
    assert rows == [(("FY2026-27", "Budget", "Full Year", "SG Rate"), Decimal("0.12"))]


def test_element_matching_is_case_insensitive_and_canonicalised(tmp_path):
    path = write(tmp_path, "fy2026-27,BUDGET,full year,sg rate,0.12\n")
    (coordinate, _value), = load_csv(path, DRIVERS, MODEL)
    assert coordinate == ("FY2026-27", "Budget", "Full Year", "SG Rate")


@pytest.mark.parametrize("second", [
    "FY2026-27,Budget,Full Year,SG Rate,0.13",
    " fy2026-27 ,BUDGET,full year,sg rate,0.13",
])
def test_conflicting_rows_name_the_file_and_both_row_numbers(tmp_path, second):
    path = write(tmp_path, "FY2026-27,Budget,Full Year,SG Rate,0.12\n\n" + second + "\n")
    store = CellStore()
    with pytest.raises(ModelError) as caught:
        load_into_store(MODEL, "Drivers", path, store)
    message = str(caught.value)
    assert str(path) in message
    assert "row 4" in message and "row 2" in message
    assert "conflicting" in message and "SG Rate" in message
    assert store.get("Drivers", ("FY2026-27", "Budget", "Full Year", "SG Rate")) == Decimal("0.12")


def test_repeated_rows_with_equal_decimal_values_remain_valid(tmp_path):
    path = write(tmp_path,
        "FY2026-27,Budget,Full Year,SG Rate,0.12\n"
        "fy2026-27,budget,full year,sg rate,0.120\n")
    rows = list(load_csv(path, DRIVERS, MODEL))
    assert len(rows) == 1
    assert load_into_store(MODEL, "Drivers", path, CellStore()) == 1


@pytest.mark.parametrize("period", ["FY", "q1"])
def test_consolidated_coordinates_are_rejected_with_row_context(tmp_path, period):
    path = write(tmp_path, f"FY2026-27,Budget,{period},SG Rate,0.12\n")
    with pytest.raises(ModelError) as caught:
        list(load_csv(path, DRIVERS, MODEL))
    assert str(path) in str(caught.value)
    assert "row 2" in str(caught.value)
    assert "consolidated" in str(caught.value)


def test_an_unknown_element_is_a_model_error_naming_the_row(tmp_path):
    path = write(tmp_path, "FY2026-27,Budget,Full Year,SG Rate,0.12\nFY2026-27,Budget,Full Year,Ghost,1\n")
    with pytest.raises(ModelError) as caught:
        list(load_csv(path, DRIVERS, MODEL))
    message = str(caught.value)
    assert "row 3" in message
    assert "Ghost" in message


def test_a_value_that_is_not_a_number_is_a_model_error_naming_the_row(tmp_path):
    path = write(tmp_path, "FY2026-27,Budget,Full Year,SG Rate,twelve\n")
    with pytest.raises(ModelError) as caught:
        list(load_csv(path, DRIVERS, MODEL))
    assert "row 2" in str(caught.value)
    assert "twelve" in str(caught.value)


def test_a_non_finite_value_is_refused(tmp_path):
    path = write(tmp_path, "FY2026-27,Budget,Full Year,SG Rate,NaN\n")
    with pytest.raises(ModelError) as caught:
        list(load_csv(path, DRIVERS, MODEL))
    assert "finite" in str(caught.value)


def test_a_row_with_the_wrong_column_count_is_a_model_error(tmp_path):
    path = write(tmp_path, "FY2026-27,Budget,Full Year,SG Rate\n")
    with pytest.raises(ModelError) as caught:
        list(load_csv(path, DRIVERS, MODEL))
    assert "row 2" in str(caught.value)


def test_a_header_with_the_wrong_column_count_is_a_model_error(tmp_path):
    path = tmp_path / "input.csv"
    path.write_text("Year,Version,Value\n", encoding="utf-8")
    with pytest.raises(ModelError) as caught:
        list(load_csv(path, DRIVERS, MODEL))
    assert "row 1" in str(caught.value)


def test_blank_rows_are_skipped(tmp_path):
    path = write(tmp_path, "\nFY2026-27,Budget,Full Year,SG Rate,0.12\n\n")
    assert len(list(load_csv(path, DRIVERS, MODEL))) == 1


def test_csv_values_parse_as_decimal_from_the_string(tmp_path):
    path = write(tmp_path, "FY2026-27,Budget,Full Year,SG Rate,0.1\n")
    (_coordinate, value), = load_csv(path, DRIVERS, MODEL)
    assert value == Decimal("0.1")
    assert str(value) == "0.1"


def test_a_byte_order_mark_is_tolerated(tmp_path):
    path = tmp_path / "input.csv"
    path.write_bytes(b"\xef\xbb\xbf" + (HEADER + "FY2026-27,Budget,Full Year,SG Rate,0.12\n").encode())
    assert len(list(load_csv(path, DRIVERS, MODEL))) == 1


def test_every_shipped_example_csv_loads_without_error():
    store = CellStore()
    counts = {
        "Drivers": load_into_store(MODEL, "Drivers", EXAMPLES / "drivers.csv", store),
        "Workforce": load_into_store(MODEL, "Workforce", EXAMPLES / "workforce.csv", store),
        "Revenue": load_into_store(MODEL, "Revenue", EXAMPLES / "revenue.csv", store),
        "Capex": load_into_store(MODEL, "Capex", EXAMPLES / "capex.csv", store),
        "PnL": load_into_store(MODEL, "PnL", EXAMPLES / "pnl-direct.csv", store),
    }
    assert all(count > 0 for count in counts.values()), counts


def test_the_examples_never_write_a_calculated_cell():
    # A loaded value under a rule calculated area would be silently shadowed by
    # the rule, so the shipped examples must never carry one.
    store = CellStore()
    load_into_store(MODEL, "PnL", EXAMPLES / "pnl-direct.csv", store)
    calculated_accounts = {
        "Contract Revenue",
        "Plant Hire Revenue",
        "Fuel",
        "Wages and Salaries",
        "Superannuation",
        "Payroll Tax",
        "Depreciation",
    }
    for _cube, coordinate, _value in store.items("PnL"):
        assert coordinate[5] not in calculated_accounts, coordinate
