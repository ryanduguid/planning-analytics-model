"""The source archive's own checks, run by release-policy's release-archive workflow.

That workflow runs `python -m unittest discover -s tests` on the tagged source
before it builds the archives. The engine's tests are Rust (`cargo test`);
these check only that the archive is complete and consistent.
"""

import json
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class SourceArchiveTests(unittest.TestCase):
    def test_version_file_matches_the_crate(self) -> None:
        version = (ROOT / "VERSION").read_text(encoding="utf-8").strip()
        manifest = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
        self.assertRegex(version, r"^\d+\.\d+\.\d+$")
        self.assertIn(f'\nversion = "{version}"\n', manifest)
        lock = (ROOT / "Cargo.lock").read_text(encoding="utf-8")
        self.assertIn(f'name = "pacioliscube"\nversion = "{version}"\n', lock)

    def test_licence_and_attribution_ship(self) -> None:
        for name in ("LICENSE", "NOTICE", "README.md", "CHANGELOG.md"):
            self.assertTrue((ROOT / name).is_file(), name)

    def test_the_model_manifest_lists_files_that_exist(self) -> None:
        manifest = json.loads((ROOT / "model" / "tm1project.json").read_text(encoding="utf-8"))
        for links in manifest["Objects"].values():
            for link in links:
                self.assertTrue((ROOT / "model" / link).is_file(), link)

    def test_the_toolchain_is_pinned(self) -> None:
        toolchain = (ROOT / "rust-toolchain.toml").read_text(encoding="utf-8")
        self.assertRegex(toolchain, re.compile(r'^channel = "\d+\.\d+"$', re.MULTILINE))


if __name__ == "__main__":
    unittest.main()
