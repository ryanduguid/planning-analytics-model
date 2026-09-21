"""Scenario changes retain arithmetic and evidence of missing inputs."""
import json

from test_cli import build_data, build_model, run_once


def test_comparison_keeps_explanations_and_does_not_sum_overlapping_cells(tmp_path):
    model = build_model(tmp_path / "model", "['Amount'] = N: ['Units'] * ['Price'];")
    previous = build_data(tmp_path / "before", body="Red,Units,3\nRed,Price,10\nBlue,Units,2\nBlue,Price,10\n")
    current = build_data(tmp_path / "after", body="Red,Units,4\nRed,Price,12\nBlue,Units,2\nBlue,Price,10\n")
    code, out, err = run_once("compare", model, "--previous-data", previous, "--current-data", current,
                              "--cell", "Sales:Total,Amount", "--cell", "Sales:Red,Amount")
    assert code == 0, err
    result = json.loads(out)
    assert [r["previous"] for r in result["changes"]] == [str(3 * 10 + 2 * 10), str(3 * 10)]
    assert [r["current"] for r in result["changes"]] == [str(4 * 12 + 2 * 10), str(4 * 12)]
    assert [r["difference"] for r in result["changes"]] == ["18", "18"]
    assert "total" not in result
    assert result["current_explanation"]["cells"]["Sales['Red', 'Amount']"]["kind"] == "rule"


def test_removed_explicit_zero_is_an_evidence_change(tmp_path):
    model = build_model(tmp_path / "model")
    previous = build_data(tmp_path / "before", body="Red,Units,0\n")
    current = build_data(tmp_path / "after", body="Blue,Units,0\n")
    code, out, err = run_once("compare", model, "--previous-data", previous, "--current-data", current,
                              "--cell", "Sales:Red,Units")
    assert code == 0, err
    result = json.loads(out)
    assert result["changes"][0]["difference"] == "0"
    change = result["evidence_changes"][0]
    assert change["previous"]["kind"] == "input"
    assert change["current"]["kind"] == "default_zero"


def test_bad_current_snapshot_leaves_no_partial_comparison(tmp_path):
    model = build_model(tmp_path / "model")
    previous = build_data(tmp_path / "before")
    current = build_data(tmp_path / "after", body="Ghost,Units,3\n")
    code, out, err = run_once("compare", model, "--previous-data", previous, "--current-data", current,
                              "--cell", "Sales:Red,Units")
    assert code != 0 and out == "" and err
