# v0.1.2

- Publishes the attested wheel and source distribution to PyPI as `pacioliscube` through trusted publishing.
- No functional change since v0.1.1.

# v0.1.1

- The repository was renamed from PaciolisCube to planning-analytics-model, and the README now says the package is installed from source rather than from PyPI.
- The CSV loader refuses two rows that give the same cell different values and names both rows, and the loader and the model reader reject malformed consolidation coordinates and unusable temporary TM1 objects.
- A command that prints several cells shares one set of calculation caches across them instead of recomputing the model per cell.
- The command line reads its version from the installed package metadata rather than from a second copy of the number.
- Continuous integration gained ruff and mypy, Python 3.12 and 3.13, pinned action SHAs, job timeouts and concurrency groups, changed-line branch coverage over the validation logic, and a check that refuses a pull request carrying an AI authorship credit.
- The model's feeders now cover monthly depreciation, feeder arity is checked, and `docs/native-comparison.md` records native TM1 agreement as pending with the evidence a comparison would need.
