# v0.3.0

- Breaking: requires Python 3.14 or later; 0.2.0 remains the last release for Python 3.11 to 3.13 ([#113](https://github.com/ryanduguid/planning-analytics-model/pull/113)).
- Licence: Apache-2.0 replaces MIT, and the source distribution includes `NOTICE`. Anyone who redistributes the package or a derivative must keep that attribution. Versions already released stay under MIT ([#124](https://github.com/ryanduguid/planning-analytics-model/pull/124)).
- Model loading reports `Elements` or `Edges` collections that are not lists, and element types that are not a recognised string, as a `ModelError` naming the file, so `pacioliscube validate` exits 2 instead of ending in a `TypeError` ([#122](https://github.com/ryanduguid/planning-analytics-model/pull/122)). Leaf collection visits each shared subtree once: a fabricated 38-node hierarchy that took 524,287 child expansions for one leaf now takes 37.
- A rules file nested about 400 parentheses or IFs deep raises `RuleSyntaxError` instead of a `RecursionError` traceback ([#134](https://github.com/ryanduguid/planning-analytics-model/pull/134)).
- Rule evaluation indexes ordered rules by selector ([#114](https://github.com/ryanduguid/planning-analytics-model/pull/114)) and reuses successful same-cube reference metadata within each engine ([#117](https://github.com/ryanduguid/planning-analytics-model/pull/117)). The earliest matching rule still wins, and results are unchanged.

# v0.2.0

- Breaking: requires Python 3.11 or later. CPython 3.10 reached end of life on 1 October 2026; 0.1.3 remains the last release that installs on Python 3.10 ([#109](https://github.com/ryanduguid/planning-analytics-model/pull/109)).
- Breaking: `pacioliscube.evaluate.consolidate`, a single-cell entry point, is removed. Use `consolidate_many` ([#86](https://github.com/ryanduguid/planning-analytics-model/pull/86)).
- The shipped model spreads the New South Wales payroll tax threshold by the days in each month instead of by twelfths, through a `Days` driver, so monthly payroll tax figures, and quarters whose days differ from a quarter of the year, change; the full-year figure does not ([#92](https://github.com/ryanduguid/planning-analytics-model/pull/92)).
- Model loading reports more malformed input as a `ModelError` naming the file, and `pacioliscube validate` exits 2 instead of ending in a traceback or validating silently: duplicate or case-insensitively clashing object names ([#87](https://github.com/ryanduguid/planning-analytics-model/pull/87)), non-text names ([#93](https://github.com/ryanduguid/planning-analytics-model/pull/93)), manifest collections that are not lists and non-string links ([#96](https://github.com/ryanduguid/planning-analytics-model/pull/96)), malformed nested link properties ([#97](https://github.com/ryanduguid/planning-analytics-model/pull/97)), and JSON nested too deeply or holding an integer longer than 4,300 digits ([#100](https://github.com/ryanduguid/planning-analytics-model/pull/100)).
- The model files follow IBM's TM1 source specification (v0.2.2); the engine still reads the previous form, and CI checks the model with the independent tm1gitpy reader ([#94](https://github.com/ryanduguid/planning-analytics-model/pull/94)).

# v0.1.3

- Adds `pacioliscube explain`, which prints JSON evidence for how each requested cell is calculated: rule locations, inputs, arithmetic, IF branches and weighted contributions. The README documented it before any release carried it.
- Adds `pacioliscube compare`, which compares two data snapshots of the same model and keeps both explain results and the differences in the selected cells. It is an offline comparison; native TM1 agreement remains unverified.
- Feeder validation pairs each `DB()` coordinate with its own dimension of the target cube and compares whole feeder areas, so a swapped coordinate or a feeder aimed at the wrong slice is reported.
- A rule expression or weighted consolidation whose arithmetic overflows raises an `EvaluationError` naming its cube and coordinate, and a rules file outside the grammar exits with the invalid-model code 2 instead of a traceback.
- The `ClearVersion` and `ExportPnL` processes create their view and subsets as temporary objects, so concurrent runs no longer share them.
- The build backend is pinned to hatchling 1.32.0, and publication requires the component checks for the exact release commit.

# v0.1.2

- Publishes the attested wheel and source distribution to PyPI as `pacioliscube` through trusted publishing.
- No functional change since v0.1.1.

# v0.1.1

- The repository was renamed from PaciolisCube to planning-analytics-model, and the README now says the package is installed from source rather than from PyPI.
- The CSV loader refuses 2 rows that give the same cell different values and names both rows, and the loader and the model reader reject malformed consolidation coordinates and unusable temporary TM1 objects.
- A command that prints several cells shares one set of calculation caches across them instead of recomputing the model per cell.
- The command line reads its version from the installed package metadata rather than from a second copy of the number.
- Continuous integration gained ruff and mypy, Python 3.12 and 3.13, pinned action SHAs, job timeouts and concurrency groups, changed-line branch coverage over the validation logic, and a check that refuses a pull request carrying an AI authorship credit.
- The model's feeders now cover monthly depreciation, feeder arity is checked, and `docs/native-comparison.md` records native TM1 agreement as pending with the evidence a comparison would need.
