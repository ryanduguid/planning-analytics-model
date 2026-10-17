"""Bind the pending native comparison to real inputs and independent arithmetic."""

import csv
import hashlib
import json
import shutil
from decimal import Decimal
from pathlib import Path

import pytest

import test_calculations as arithmetic
from conftest import EXAMPLES, MODEL_ROOT, REPO
from pacioliscube.cli import main
from pacioliscube.data import load_data
from pacioliscube.evaluate import consolidate_many, evaluate


@pytest.fixture(scope="module")
def comparison():
    return json.loads((REPO / "docs/native-comparison-manifest.json").read_text(encoding="utf-8"))


def test_manifest_has_eight_baseline_cases_two_variants_and_no_native_claim(comparison):
    assert comparison["schema_version"] == 1
    assert [case["id"] for case in comparison["cases"]] == [
        f"TM1-{number:03d}" for number in range(1, 11)
    ]
    assert [case["variant"] for case in comparison["cases"]].count("baseline") == 8
    assert comparison["native_evidence"] == {
        "status": "pending", "server_version": None, "server_build": None,
        "operator": None, "run_date": None, "raw_exports": [],
    }
    assert comparison["tolerance"] == {"currency_unit": "0.01", "maximum_raw_difference": "0.005"}
    assert comparison["cases"][7]["native_zero_suppression"] is True
    assert comparison["cases"][9]["fresh_feeders_before_check_feeders"] is True


def test_manifest_hashes_cover_every_model_file_and_all_five_inputs(comparison):
    files = sorted(path for directory in (MODEL_ROOT, EXAMPLES) for path in directory.rglob("*")
                   if path.is_file())
    hashes = {
        path.relative_to(REPO).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in files
    }
    assert comparison["input_hashes"] == hashes
    assert len([name for name in hashes if name.startswith("examples/")]) == 5


@pytest.mark.parametrize("index", range(8))
def test_each_baseline_case_agrees_with_the_existing_independent_calculation(comparison, index):
    case = comparison["cases"][index]
    # Those existing tests derive each value from the synthetic input operands.
    getattr(arithmetic, case["independent_test"])()
    [actual] = consolidate_many(
        arithmetic.MODEL, arithmetic.CALCULATED, [(case["cube"], tuple(case["coordinates"]))]
    )
    assert abs(actual - Decimal(case["expected"])) <= Decimal("0.005")


def altered_inputs(tmp_path: Path, case: dict) -> Path:
    destination = tmp_path / "data"
    shutil.copytree(EXAMPLES, destination)
    removal = case["remove_rows"]
    path = destination / removal["file"]
    with path.open(encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle)
        fieldnames = reader.fieldnames
        rows = list(reader)
    retained = [row for row in rows
                if not all(row[field] == value for field, value in removal["where"].items())]
    assert len(retained) < len(rows), "variant must actually remove its intended synthetic rows"
    with path.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=fieldnames, lineterminator="\n")
        writer.writeheader()
        writer.writerows(retained)
    return destination


def test_missing_february_days_is_an_offline_error_not_a_native_observation(
    comparison, tmp_path, capsys
):
    case = comparison["cases"][8]
    assert case["expected"] == "N/A" and case["offline_exit"] == 3
    data = altered_inputs(tmp_path, case)
    cell = case["cube"] + ":" + ",".join(case["coordinates"])
    assert main(["calculate", str(MODEL_ROOT), "--data", str(data), "--cell", cell]) == 3
    result = capsys.readouterr()
    assert not result.out and "division by zero" in result.err


def test_removing_corporate_workforce_preserves_its_credit_in_the_group(comparison, tmp_path):
    case = comparison["cases"][9]
    data = altered_inputs(tmp_path, case)
    calculated = evaluate(arithmetic.MODEL, load_data(arithmetic.MODEL, data))
    cells = [(case["cube"], tuple(case["coordinates"])),
             (case["cube"], tuple(case["group_coordinates"]))]
    corporate, group = consolidate_many(arithmetic.MODEL, calculated, cells)
    expected_credit = -Decimal("1200000") * Decimal("0.0545")
    assert Decimal(case["expected"]) == expected_credit
    assert abs(corporate - expected_credit) <= Decimal("0.005")
    [original_group] = consolidate_many(arithmetic.MODEL, arithmetic.CALCULATED, [cells[1]])
    # Removing two 150,000 salaries removes their annual gross payroll-tax charge,
    # while the threshold credit remains in both Corporate and Group.
    removed_charge = Decimal("2") * Decimal("150000") * Decimal("1.12") * Decimal("0.0545")
    assert abs(group - (original_group - removed_charge)) <= Decimal("0.000000001")
