"""The install instructions have to match how the project is actually shipped."""

from pathlib import Path


def test_readme_declares_the_published_package_and_the_clone(repo: Path) -> None:
    # pacioliscube 0.1.2 is on PyPI, so the README must offer that install and
    # must not tell a reader the project is unpublished.
    readme = " ".join((repo / "README.md").read_text(encoding="utf-8").split())

    assert "**Package lifecycle:** published." in readme
    assert "not published to PyPI" not in readme
    assert "source-only" not in readme
    assert "pip install pacioliscube" in readme
    assert "git clone https://github.com/ryanduguid/planning-analytics-model.git" in readme
    assert "python -m pip install ." in readme
