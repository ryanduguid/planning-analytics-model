"""Repository contract for the risk-based changed-line coverage pilot."""

import re
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]


def test_changed_line_coverage_is_scoped_and_fail_closed() -> None:
    pyproject = (ROOT / "pyproject.toml").read_text(encoding="utf-8")
    workflow_path = ROOT / ".github" / "workflows" / "ci.yml"
    if not workflow_path.is_file():
        if (ROOT / ".git").exists():
            pytest.fail("CI workflow is missing from the repository checkout")
        pytest.skip("CI workflow is not included in the source distribution")
    workflow = workflow_path.read_text(encoding="utf-8")

    assert re.search(r'"coverage==\d+\.\d+\.\d+"', pyproject)
    assert re.search(r'"diff-cover==\d+\.\d+\.\d+"', pyproject)
    assert "fetch-depth: 0" in workflow
    assert "--source=pacioliscube.validate" in workflow
    assert '--include="pacioliscube/validate.py"' in workflow
    assert "--compare-branch=origin/main" in workflow
    assert "--branch-coverage" in workflow
    assert "--fail-under=100" in workflow
