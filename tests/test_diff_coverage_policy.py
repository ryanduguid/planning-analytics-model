"""Repository contract for the risk-based changed-line coverage pilot."""

import re
from pathlib import Path

import pytest


def test_changed_line_coverage_is_scoped_and_fail_closed(repo: Path) -> None:
    ROOT = repo
    pyproject = (ROOT / "pyproject.toml").read_text(encoding="utf-8")
    workflow_path = ROOT / ".github" / "workflows" / "ci.yml"
    if not workflow_path.is_file():
        if (ROOT / ".git").exists():
            pytest.fail("CI workflow is missing from the repository checkout")
        pytest.skip("CI workflow is not included in the source distribution")
    workflow = workflow_path.read_text(encoding="utf-8")

    assert re.search(r'"coverage==\d+\.\d+\.\d+"', pyproject)
    assert re.search(r'"diff-cover==\d+\.\d+\.\d+"', pyproject)
    assert "fetch_depth: 0" in workflow
    assert "fetch-depth: ${{ matrix.fetch_depth }}" in workflow
    # Coverage is measured over the whole package for Codecov; the changed-line
    # gate stays scoped to validate.py through the --include below.
    assert "--source=pacioliscube -m pytest" in workflow  # nosec B101: pytest outcome assertion
    assert '--include="pacioliscube/validate.py"' in workflow
    assert "--compare-branch=origin/main" in workflow
    assert "--branch-coverage" in workflow
    assert "--fail-under=100" in workflow
