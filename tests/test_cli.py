"""The command line: what each subcommand prints and which exit code it returns."""

import io
import json
import shutil
from contextlib import redirect_stderr, redirect_stdout
from decimal import Decimal
from functools import lru_cache
from pathlib import Path

import pytest

from pacioliscube import cli
from pacioliscube.cli import main

REPO = Path(__file__).resolve().parents[1]
MODEL = str(REPO / "model")
EXAMPLES = str(REPO / "examples")

CELL = "PnL:FY2026-27,Budget,Jul,CivilCo,Earthworks,Contract Revenue,Amount"
GROUP_EBITDA = "PnL:FY2026-27,Budget,FY,Group,All Cost Centres,EBITDA,Amount"
BUDGET_REPORT = ("report", MODEL, "--data", EXAMPLES, "--year", "FY2026-27", "--version", "Budget")

# The rows a profit and loss prints, for a test that has to build a model the
# report will accept. The order itself is pinned by its own test below.
STATEMENT_ROWS = (
    "Revenue",
    "Direct Costs",
    "Gross Margin",
    "Employment Costs",
    "Overheads",
    "EBITDA",
    "Depreciation",
    "EBIT",
)

# A rule the feeders do not cover, which is the one warning that carries no
# error alongside it.
UNFED_RULES = (
    "SKIPCHECK;\n"
    "['Amount'] = N: ['Units'] * ['Price'];\n"
    "['Price'] = N: 2;\n"
    "FEEDERS;\n"
    "['Units'] => ['Amount'];\n"
)
UNKNOWN_ELEMENT_RULES = "SKIPCHECK;\n['Ghost'] = N: 1;\nFEEDERS;\n['Units'] => ['Amount'];\n"
DIVIDE_BY_ZERO_RULES = (
    "SKIPCHECK;\n"
    "['Amount'] = N: ['Units'] / ['Price'];\n"
    "FEEDERS;\n"
    "['Units'] => ['Amount'];\n"
)
# Red and Blue are both elements of Colour, so this area names one dimension
# twice. Validation does not look for that, and the C qualifier keeps the area
# out of the evaluation pass, so the fault waits until a consolidated cell is
# asked for.
AMBIGUOUS_AREA_RULES = (
    "SKIPCHECK;\n"
    "['Red','Blue'] = C: 1;\n"
    "FEEDERS;\n"
    "['Units'] => ['Amount'];\n"
)


def run_once(*argv: str) -> tuple:
    """Run the command line, keeping its exit code and what it printed.

    A test that patches something the command line calls has to use this rather
    than the cached runner, because the patch changes the result the arguments
    would otherwise fix.
    """
    out, err = io.StringIO(), io.StringIO()
    with redirect_stdout(out), redirect_stderr(err):
        code = main(list(argv))
    return code, out.getvalue(), err.getvalue()


@lru_cache(maxsize=None)
def run(*argv: str) -> tuple:
    """Run the command line once, keeping its exit code and what it printed.

    Every subcommand only reads, so the same arguments give the same result.
    Holding the result lets several tests share one evaluation of the model.
    """
    return run_once(*argv)


def build_model(root: Path, rules: str = "", extra_files: dict | None = None) -> str:
    """Write the smallest model a command line test can point at."""
    (root / "dimensions" / "Colour.hierarchies").mkdir(parents=True, exist_ok=True)
    (root / "dimensions" / "Measure.hierarchies").mkdir(parents=True, exist_ok=True)
    (root / "cubes").mkdir(parents=True, exist_ok=True)
    (root / "dimensions" / "Colour.json").write_text(
        '{"Name": "Colour", "Hierarchies@Code.links": ["Colour.hierarchies/Colour.json"]}',
        encoding="utf-8",
    )
    (root / "dimensions" / "Colour.hierarchies" / "Colour.json").write_text(
        '{"Name": "Colour", "Elements": ['
        '{"Name": "Total", "Type": "Consolidated"}, {"Name": "Red", "Type": "Numeric"},'
        ' {"Name": "Blue", "Type": "Numeric"}], "Edges": ['
        '{"ParentName": "Total", "ComponentName": "Red", "Weight": 1},'
        ' {"ParentName": "Total", "ComponentName": "Blue", "Weight": 1}]}',
        encoding="utf-8",
    )
    (root / "dimensions" / "Measure.json").write_text(
        '{"Name": "Measure", "Hierarchies@Code.links": ["Measure.hierarchies/Measure.json"]}',
        encoding="utf-8",
    )
    (root / "dimensions" / "Measure.hierarchies" / "Measure.json").write_text(
        '{"Name": "Measure", "Elements": ['
        '{"Name": "Units", "Type": "Numeric"}, {"Name": "Price", "Type": "Numeric"},'
        ' {"Name": "Amount", "Type": "Numeric"}], "Edges": []}',
        encoding="utf-8",
    )
    rules_link = ', "Rules@Code.link": "Sales.rules"' if rules else ""
    (root / "cubes" / "Sales.json").write_text(
        '{"Name": "Sales", "Dimensions@Code.links": ["../dimensions/Colour.json",'
        ' "../dimensions/Measure.json"]%s}' % rules_link,
        encoding="utf-8",
    )
    if rules:
        (root / "cubes" / "Sales.rules").write_text(rules, encoding="utf-8")
    (root / "tm1project.json").write_text(
        '{"Version": 1.0, "Name": "built", "Objects": {"Dimensions":'
        ' ["dimensions/Colour.json", "dimensions/Measure.json"],'
        ' "Cubes": ["cubes/Sales.json"]}}',
        encoding="utf-8",
    )
    for name, content in (extra_files or {}).items():
        (root / name).write_text(content, encoding="utf-8")
    return str(root)


def build_data(root: Path, name: str = "sales.csv", body: str = "Red,Units,10\n") -> str:
    """Write a data directory beside a model, never inside it."""
    root.mkdir(parents=True, exist_ok=True)
    (root / name).write_text("Colour,Measure,Value\n" + body, encoding="utf-8")
    return str(root)


def write_report_data(folder, header: str):
    """A data directory holding one header only CSV for a shaped report model.

    The report needs a file to read, but the header alone is enough: every row
    it prints is a consolidation over cells that are simply absent.
    """
    folder.mkdir(parents=True, exist_ok=True)
    (folder / "PnL.csv").write_text(header + chr(10), encoding="utf-8")
    return folder


def build_report_model(root: Path, elements: dict) -> str:
    """Write a model holding one cube named PnL over the dimensions given.

    The report resolves every element of its slice before it reads a cell, so a
    model built here needs no rules and no data to exercise the report's guards.
    Each dimension holds the leaf elements named for it, in the order given, and
    the cube takes the dimensions in the same order.
    """
    (root / "cubes").mkdir(parents=True, exist_ok=True)
    for dimension, names in elements.items():
        folder = root / "dimensions" / f"{dimension}.hierarchies"
        folder.mkdir(parents=True, exist_ok=True)
        (root / "dimensions" / f"{dimension}.json").write_text(
            json.dumps(
                {
                    "Name": dimension,
                    "Hierarchies@Code.links": [f"{dimension}.hierarchies/{dimension}.json"],
                }
            ),
            encoding="utf-8",
        )
        (folder / f"{dimension}.json").write_text(
            json.dumps(
                {
                    "Name": dimension,
                    "Elements": [{"Name": name, "Type": "Numeric"} for name in names],
                    "Edges": [],
                }
            ),
            encoding="utf-8",
        )
    (root / "cubes" / "PnL.json").write_text(
        json.dumps(
            {
                "Name": "PnL",
                "Dimensions@Code.links": [f"../dimensions/{name}.json" for name in elements],
            }
        ),
        encoding="utf-8",
    )
    (root / "tm1project.json").write_text(
        json.dumps(
            {
                "Version": 1.0,
                "Name": "shaped",
                "Objects": {
                    "Dimensions": [f"dimensions/{name}.json" for name in elements],
                    "Cubes": ["cubes/PnL.json"],
                },
            }
        ),
        encoding="utf-8",
    )
    return str(root)


def dollars(printed: str) -> int:
    """The signed value of a printed amount, which brackets a negative."""
    if printed.startswith("(") and printed.endswith(")"):
        return -int(printed[1:-1].replace(",", ""))
    return int(printed.replace(",", ""))


def report_rows(*argv: str) -> dict:
    """The report's amount lines, keyed by their label."""
    code, out, _err = run(*argv)
    assert code == 0, out
    body = out.split("\n\n", 1)[1]
    rows = {}
    for line in body.splitlines():
        if not line.strip():
            continue
        label, amount = line.rsplit(maxsplit=1)
        rows[label.strip()] = amount
    return rows


def test_the_shipped_model_validates_with_a_zero_exit():
    code, out, _err = run("validate", MODEL)
    assert code == 0
    assert "0 errors, 0 warnings" in out


def test_the_model_directory_defaults_to_the_model_folder(monkeypatch, capsys):
    monkeypatch.chdir(REPO)
    assert main(["validate"]) == 0
    assert "0 errors, 0 warnings" in capsys.readouterr().out


def test_the_model_directory_may_be_named_as_an_option(tmp_path):
    # The packaging job installs the wheel and runs the command this way, so
    # the option has to reach the same place the positional does.
    root = build_model(tmp_path / "model", rules=UNKNOWN_ELEMENT_RULES)
    assert run("validate", "--model", root) == run("validate", root)


def test_naming_the_model_directory_twice_is_a_usage_error():
    code, _out, err = run("validate", MODEL, "--model", MODEL)
    assert code == 1
    assert "given twice" in err


def test_a_finding_prints_as_severity_code_location_then_message(tmp_path):
    root = build_model(tmp_path / "model", rules=UNKNOWN_ELEMENT_RULES)
    _code, out, _err = run("validate", root)
    line = out.splitlines()[0]
    assert line.startswith("error ELE001 ")
    assert line.endswith("no dimension of cube 'Sales' holds an element named 'Ghost'")
    assert "Sales.rules line 2:" in line


def test_errors_print_before_warnings_whatever_order_they_are_found_in(tmp_path):
    # The manifest check runs last, so this model finds its warning before its
    # error and the grouping has to put them back in severity order.
    root = build_model(
        tmp_path / "model", rules=UNFED_RULES, extra_files={"notes.txt": "left behind\n"}
    )
    _code, out, _err = run("validate", root)
    severities = [line.split(" ", 1)[0] for line in out.splitlines()[:-1]]
    assert severities == ["error", "warning"]


def test_validate_returns_two_when_the_model_has_an_error(tmp_path):
    root = build_model(tmp_path / "model", rules=UNKNOWN_ELEMENT_RULES)
    code, out, _err = run("validate", root)
    assert code == 2
    assert "1 error, 1 warning" in out


def test_validate_returns_zero_when_the_model_has_only_warnings(tmp_path):
    root = build_model(tmp_path / "model", rules=UNFED_RULES)
    code, out, _err = run("validate", root)
    assert code == 0
    assert "0 errors, 1 warning" in out


def test_a_model_directory_that_is_not_there_is_a_usage_error(tmp_path):
    code, _out, err = run("validate", str(tmp_path / "absent"))
    assert code == 1
    assert "no model directory" in err


def test_a_directory_holding_no_manifest_is_an_invalid_model(tmp_path):
    empty = tmp_path / "empty"
    empty.mkdir()
    code, _out, err = run("validate", str(empty))
    assert code == 2
    assert "tm1project.json" in err


def test_calculate_prints_the_value_at_a_leaf_cell():
    code, out, _err = run("calculate", MODEL, "--data", EXAMPLES, "--cell", CELL)
    assert code == 0
    assert out == f"{CELL} = 1534000\n"


def test_a_consolidated_coordinate_is_summed_through_the_hierarchy():
    # Every cost centre of both entities, over the whole year, less the costs
    # that hang under EBITDA. Nothing here is a stored cell.
    code, out, _err = run("calculate", MODEL, "--data", EXAMPLES, "--cell", GROUP_EBITDA)
    assert code == 0
    assert out == f"{GROUP_EBITDA} = 20066782.4118\n"


def test_cell_may_be_given_more_than_once():
    code, out, _err = run(
        "calculate", MODEL, "--data", EXAMPLES, "--cell", CELL, "--cell", GROUP_EBITDA
    )
    assert code == 0
    assert [line.split(" = ")[0] for line in out.splitlines()] == [CELL, GROUP_EBITDA]


def test_a_cell_reference_matches_elements_without_regard_to_case():
    code, out, _err = run(
        "calculate",
        MODEL,
        "--data",
        EXAMPLES,
        "--cell",
        "pnl:fy2026-27,budget,jul,civilco,earthworks,contract revenue,amount",
    )
    assert code == 0
    assert out == f"{CELL} = 1534000\n"


def test_a_cell_reference_without_a_cube_is_a_usage_error():
    code, _out, err = run("calculate", MODEL, "--data", EXAMPLES, "--cell", "FY2026-27,Budget")
    assert code == 1
    assert "CUBE:element" in err


def test_a_cell_reference_naming_no_cube_of_the_model_is_a_usage_error():
    code, _out, err = run("calculate", MODEL, "--data", EXAMPLES, "--cell", "Ghost:Jul")
    assert code == 1
    assert "no cube named 'Ghost'" in err


def test_a_cell_reference_of_the_wrong_width_is_a_usage_error():
    code, _out, err = run("calculate", MODEL, "--data", EXAMPLES, "--cell", "PnL:FY2026-27,Budget")
    assert code == 1
    assert "gives 2 elements" in err
    assert "takes 7" in err


def test_a_cell_reference_naming_an_unknown_element_is_a_usage_error():
    code, _out, err = run(
        "calculate",
        MODEL,
        "--data",
        EXAMPLES,
        "--cell",
        "PnL:FY2026-27,Budget,Jul,CivilCo,Earthworks,Ghost,Amount",
    )
    assert code == 1
    assert "'Ghost'" in err


def test_a_data_directory_that_is_not_there_is_a_usage_error(tmp_path):
    code, _out, err = run("calculate", MODEL, "--data", str(tmp_path / "absent"), "--cell", CELL)
    assert code == 1
    assert "no data directory" in err


def test_a_data_directory_holding_no_csv_is_a_usage_error(tmp_path):
    empty = tmp_path / "data"
    empty.mkdir()
    code, _out, err = run("calculate", MODEL, "--data", str(empty), "--cell", CELL)
    assert code == 1
    assert "no CSV files" in err


def test_a_csv_row_naming_an_element_the_model_does_not_hold_is_a_usage_error(tmp_path):
    data = tmp_path / "data"
    data.mkdir()
    (data / "drivers.csv").write_text(
        "Year,Version,Period,DriverMeasure,Value\nFY2026-27,Budget,Full Year,Ghost,1\n",
        encoding="utf-8",
    )
    code, _out, err = run("calculate", MODEL, "--data", str(data), "--cell", CELL)
    assert code == 1
    assert "row 2" in err
    assert "Ghost" in err


def test_a_csv_that_is_not_utf_8_is_a_usage_error(tmp_path):
    # A spreadsheet exported as Windows-1252 is the ordinary way this arrives,
    # and a decode error has to carry an exit code rather than a traceback.
    data = tmp_path / "data"
    data.mkdir()
    (data / "drivers.csv").write_bytes(
        b"Year,Version,Period,DriverMeasure,Value\n"
        + "FY2026-27,Budget,Full Year,Crew \xa3 rate,1\n".encode("cp1252")
    )
    code, _out, err = run("calculate", MODEL, "--data", str(data), "--cell", CELL)
    assert code == 1
    assert "drivers.csv" in err
    assert "UTF-8" in err


def test_a_model_file_that_is_not_utf_8_is_an_invalid_model(tmp_path):
    root = Path(build_model(tmp_path / "model"))
    manifest = root / "tm1project.json"
    manifest.write_bytes(manifest.read_bytes().replace(b'"built"', b'"buil\xff"'))
    code, _out, err = run("validate", str(root))
    assert code == 2
    assert "UTF-8" in err


def test_an_operating_system_error_reading_a_csv_is_a_usage_error(tmp_path, monkeypatch):
    # A file that is locked or that vanishes mid run cannot be produced the same
    # way on every machine, so the refusal is injected at the point the command
    # line reads the file. What is being pinned is the exit code, not the cause.
    data = build_data(tmp_path / "data", name="drivers.csv", body="")

    def refuse(*_args, **_kwargs):
        raise PermissionError(13, "Permission denied")

    monkeypatch.setattr(cli, "load_into_store", refuse)
    code, _out, err = run_once("calculate", MODEL, "--data", data, "--cell", CELL)
    assert code == 1
    assert "Permission denied" in err
    # The exit code alone does not pin the handler: the backstop in main gives
    # the same code. Naming the file it failed on is what the handler adds, so
    # that is what has to be asserted.
    assert "drivers.csv" in err


def test_an_operating_system_error_reading_the_model_is_an_invalid_model(monkeypatch):
    def refuse(*_args, **_kwargs):
        raise PermissionError(13, "Permission denied")

    monkeypatch.setattr(cli, "load_model", refuse)
    code, _out, err = run_once("validate", MODEL)
    assert code == 2
    assert "Permission denied" in err


def test_a_csv_that_resolves_outside_the_data_directory_is_refused(tmp_path):
    # The model tree fences its own links, but a data directory is given
    # separately, so nothing fences its files except this. Without the fence a
    # link dropped in a data directory would read whatever it points at.
    outside = tmp_path / "outside.csv"
    outside.write_text("Year,Version,Period,DriverMeasure,Value\n", encoding="utf-8")
    data = tmp_path / "data"
    data.mkdir()
    link = data / "drivers.csv"
    try:
        link.symlink_to(outside)
    except (OSError, NotImplementedError) as error:
        pytest.skip(f"this machine will not create a symlink: {error}")
    if not link.is_symlink():
        pytest.skip("the link was created as a copy rather than a symlink")
    code, _out, err = run("calculate", MODEL, "--data", str(data), "--cell", CELL)
    assert code == 1
    assert "resolves outside the data directory" in err


def test_two_data_files_feeding_one_cube_are_a_usage_error(tmp_path):
    # The cell store takes the last write, so a stray file feeding a cube that a
    # shipped file already feeds overwrites the overlap in file name order. One
    # altered row turns Direct Costs from 14,884,800 into 1,014,624,800 and the
    # run still exits 0, which is why two files for one cube are refused.
    data = tmp_path / "data"
    data.mkdir()
    for path in Path(EXAMPLES).glob("*.csv"):
        shutil.copy(path, data / path.name)
    (data / "PnL.csv").write_text(
        "Year,Version,Period,Entity,CostCentre,Account,PnLMeasure,Value\n"
        "FY2026-27,Budget,Jul,CivilCo,Earthworks,Subcontractor Costs,Amount,1000000000\n",
        encoding="utf-8",
    )
    code, out, err = run(
        "report", MODEL, "--data", str(data), "--year", "FY2026-27", "--version", "Budget"
    )
    assert code == 1
    assert out == ""
    assert "PnL.csv" in err
    assert "pnl-direct.csv" in err


def test_conflicting_rows_are_a_usage_error_without_calculated_output(tmp_path):
    root = build_model(tmp_path / "model")
    data = build_data(tmp_path / "data", body="Red,Units,10\nred,units,20\n")
    code, out, err = run_once("calculate", root, "--data", data, "--cell", "Sales:Red,Units")
    assert code == 1
    assert out == ""
    assert "sales.csv" in err and "row 2" in err and "row 3" in err


def test_a_csv_matching_no_cube_is_a_usage_error(tmp_path):
    data = tmp_path / "data"
    data.mkdir()
    (data / "ledger.csv").write_text("Year,Value\n", encoding="utf-8")
    code, _out, err = run("calculate", MODEL, "--data", str(data), "--cell", CELL)
    assert code == 1
    assert "'ledger'" in err


def test_a_missing_required_option_is_a_usage_error():
    code, _out, _err = run("calculate", MODEL, "--cell", CELL)
    assert code == 1


def test_help_and_version_exit_zero():
    assert run("--help")[0] == 0
    assert run("--version")[0] == 0


def test_calculate_refuses_a_model_that_does_not_validate(tmp_path):
    root = build_model(tmp_path / "model", rules=UNKNOWN_ELEMENT_RULES)
    data = build_data(tmp_path / "data")
    code, out, err = run("calculate", root, "--data", data, "--cell", "Sales:Red,Amount")
    assert code == 2
    assert out == ""
    assert "ELE001" in err


def test_a_division_by_zero_while_evaluating_returns_three(tmp_path):
    # Price is never loaded, so it reads as zero and the slash operator, which
    # is the strict divide, refuses it.
    root = build_model(tmp_path / "model", rules=DIVIDE_BY_ZERO_RULES)
    data = build_data(tmp_path / "data")
    code, _out, err = run("calculate", root, "--data", data, "--cell", "Sales:Red,Amount")
    assert code == 3
    assert "division by zero" in err


def test_a_model_fault_found_while_consolidating_returns_two(tmp_path):
    # The model loads and validates clean, so the fault is one the structural
    # checks do not reach rather than a bad argument. It takes the model's exit
    # code, the same one the evaluation pass gives the identical exception.
    root = build_model(tmp_path / "model", rules=AMBIGUOUS_AREA_RULES)
    data = build_data(tmp_path / "data")
    assert run("validate", root)[0] == 0
    code, _out, err = run("calculate", root, "--data", data, "--cell", "Sales:Total,Amount")
    assert code == 2
    assert "area names two elements of dimension 'Colour'" in err


def test_a_data_file_loads_into_the_cube_its_name_matches(tmp_path):
    root = build_model(
        tmp_path / "model",
        rules="SKIPCHECK;\n['Amount'] = N: ['Units'] * 2;\nFEEDERS;\n['Units'] => ['Amount'];\n",
    )
    data = build_data(tmp_path / "data")
    code, out, _err = run("calculate", root, "--data", data, "--cell", "Sales:Red,Amount")
    assert code == 0
    assert out == "Sales:Red,Amount = 20\n"


def test_the_report_prints_the_profit_and_loss_lines_in_statement_order():
    rows = report_rows(*BUDGET_REPORT)
    assert list(rows) == [
        "Revenue",
        "Direct Costs",
        "Gross Margin",
        "Employment Costs",
        "Overheads",
        "EBITDA",
        "Depreciation",
        "EBIT",
    ]


def test_the_report_totals_the_shipped_budget():
    rows = report_rows(*BUDGET_REPORT)
    assert rows == {
        "Revenue": "52,764,000",
        "Direct Costs": "(14,884,800)",
        "Gross Margin": "37,879,200",
        "Employment Costs": "(11,428,418)",
        "Overheads": "(6,384,000)",
        "EBITDA": "20,066,782",
        "Depreciation": "(1,382,143)",
        "EBIT": "18,684,640",
    }


def test_a_deduction_prints_in_brackets_and_a_result_prints_plain():
    # The model stores a cost as a positive number, so without a convention the
    # printed column reads as though every line adds.
    rows = report_rows(*BUDGET_REPORT)
    bracketed = {label for label, amount in rows.items() if amount.startswith("(")}
    assert bracketed == {"Direct Costs", "Employment Costs", "Overheads", "Depreciation"}


def test_the_report_help_names_the_sign_convention():
    _code, out, _err = run("report", "--help")
    assert "brackets" in out


def test_money_brackets_a_negative_and_prints_a_rounded_zero_plain():
    assert cli._money(Decimal("1234.5")) == "1,235"
    assert cli._money(Decimal("-1234.5")) == "(1,235)"
    assert cli._money(Decimal("-0.4")) == "0"


def test_the_printed_lines_articulate_to_within_the_rounding():
    # Each line is rounded on its own, so a subtotal can sit a dollar away from
    # the lines above it. Ask for that dollar of tolerance rather than force a
    # line to a figure it does not hold.
    rows = report_rows(*BUDGET_REPORT)
    money = {label: dollars(amount) for label, amount in rows.items()}
    assert money["Gross Margin"] == money["Revenue"] + money["Direct Costs"]
    assert abs(money["EBIT"] - (money["EBITDA"] + money["Depreciation"])) <= 1


def test_the_report_names_the_year_the_version_and_the_slice_it_read():
    _code, out, _err = run(
        "report", MODEL, "--data", EXAMPLES, "--year", "fy2026-27", "--version", "budget"
    )
    header = out.splitlines()[:2]
    assert header[0] == "Profit and loss for FY2026-27, Budget"
    assert header[1] == "PnL at FY, Group, All Cost Centres"


def test_the_report_returns_one_for_a_year_the_model_does_not_hold():
    code, _out, err = run(
        "report", MODEL, "--data", EXAMPLES, "--year", "FY2099-00", "--version", "Budget"
    )
    assert code == 1
    assert "FY2099-00" in err


def test_the_report_returns_one_for_a_version_the_model_does_not_hold():
    code, _out, err = run(
        "report", MODEL, "--data", EXAMPLES, "--year", "FY2026-27", "--version", "Forecast"
    )
    assert code == 1
    assert "Forecast" in err


def test_the_report_refuses_a_cube_without_a_year_version_and_account(tmp_path):
    # Without these three every row would read the same cell, so the report
    # would print eight identical figures and head them as a statement.
    root = build_report_model(tmp_path / "model", {"Period": ("FY",), "Entity": ("Group",)})
    data = tmp_path / "data"
    data.mkdir()
    code, out, err = run(
        "report", root, "--data", str(data), "--year", "FY2026-27", "--version", "Budget"
    )
    assert code == 1
    assert out == ""
    assert "no Year, Version, Account dimension" in err


def test_the_report_refuses_a_model_that_rolls_up_to_something_else(tmp_path):
    # A model whose periods roll up to Full Year rather than FY loads and
    # validates clean. The report does not fit it, which is a different thing
    # from the model being wrong, so it does not take the invalid model code.
    root = build_report_model(
        tmp_path / "model",
        {
            "Year": ("FY2026-27",),
            "Version": ("Budget",),
            "Period": ("Full Year",),
            "Account": STATEMENT_ROWS,
        },
    )
    data = tmp_path / "data"
    data.mkdir()
    assert run("validate", root)[0] == 0
    code, out, err = run(
        "report", root, "--data", str(data), "--year", "FY2026-27", "--version", "Budget"
    )
    assert code == 1
    assert out == ""
    assert "Period 'FY'" in err
    assert "calculate" in err


def test_the_report_refuses_a_cube_holding_a_dimension_it_has_no_element_for(tmp_path):
    root = build_report_model(
        tmp_path / "model",
        {
            "Year": ("FY2026-27",),
            "Version": ("Budget",),
            "Account": STATEMENT_ROWS,
            "Colour": ("Red",),
        },
    )
    data = tmp_path / "data"
    data.mkdir()
    code, out, err = run(
        "report", root, "--data", str(data), "--year", "FY2026-27", "--version", "Budget"
    )
    assert code == 1
    assert out == ""
    assert "'Colour'" in err


def test_a_run_writes_nothing_into_the_model_or_the_data_directory(capsys):
    def snapshot():
        return {
            str(path): (path.stat().st_size, path.stat().st_mtime_ns)
            for base in (REPO / "model", REPO / "examples")
            for path in sorted(base.rglob("*"))
        }

    before = snapshot()
    # Called straight, not through the cached runner, so this really does run.
    assert main(list(BUDGET_REPORT)) == 0
    capsys.readouterr()
    assert snapshot() == before


def test_the_report_header_names_only_the_restrictions_it_applied(tmp_path):
    # A cube with no Period, Entity or CostCentre dimension was never narrowed
    # on any of them. An earlier header fell back to the report's own literals
    # and claimed "at FY, Group, All Cost Centres" over a whole cube total.
    root = build_report_model(
        tmp_path / "model",
        {"Year": ("FY2026-27",), "Version": ("Budget",), "Account": STATEMENT_ROWS},
    )
    data = write_report_data(tmp_path / "data", "Year,Version,Account,Value")
    code, out, _err = run(
        "report", root, "--data", str(data), "--year", "FY2026-27", "--version", "Budget"
    )
    assert code == 0
    assert "FY, Group, All Cost Centres" not in out
    assert "whole cube" in out


def test_the_report_header_names_a_partial_slice_without_inventing_the_rest(tmp_path):
    root = build_report_model(
        tmp_path / "model",
        {
            "Year": ("FY2026-27",),
            "Version": ("Budget",),
            "Period": ("FY",),
            "Account": STATEMENT_ROWS,
        },
    )
    data = write_report_data(tmp_path / "data", "Year,Version,Period,Account,Value")
    code, out, _err = run(
        "report", root, "--data", str(data), "--year", "FY2026-27", "--version", "Budget"
    )
    assert code == 0
    assert "PnL at FY" in out
    assert "Group" not in out
    assert "All Cost Centres" not in out


def test_the_shipped_report_still_names_its_full_slice():
    code, out, _err = run(
        "report", MODEL, "--data", str(EXAMPLES), "--year", "FY2026-27", "--version", "Budget"
    )
    assert code == 0
    assert "PnL at FY, Group, All Cost Centres" in out
