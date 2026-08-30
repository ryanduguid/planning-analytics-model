"""The install instructions have to match how the project is actually shipped."""

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def test_readme_declares_source_only_installation() -> None:
    # The project is not on PyPI, so a README that opens with pip install
    # pacioliscube sends every reader to a package that cannot be found.
    readme = " ".join((ROOT / "README.md").read_text(encoding="utf-8").split())

    assert "**Package lifecycle:** source-only." in readme
    assert "not published to PyPI" in readme
    assert "git clone https://github.com/ryanduguid/PaciolisCube.git" in readme
    assert "python -m pip install ." in readme
    assert "pip install pacioliscube" not in readme
