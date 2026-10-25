# pacioliscube changes

## 1.0.0

The engine is now a Rust command line. It replaces the Python package
`pacioliscube` 0.3.0 at the base revision; the import package and its
library API are removed, so this is a major version. Build it with the
toolchain pinned in `rust-toolchain.toml`:

```bash
cargo install --locked --path .
```

`validate`, `calculate`, `explain`, `compare` and `report` take the same
arguments and flags, return the same exit codes and print the same bytes on
stdout and stderr as 0.3.0, including `\r\n` line ends on Windows, except as
listed below. That includes argparse's usage lines, help text wrapped to the
terminal width (`COLUMNS`), colour (`PYTHON_COLORS`, `NO_COLOR`,
`FORCE_COLOR`, `TERM`), option abbreviations and error messages. The model
format, the CSV input and every figure are unchanged: arithmetic is exact
decimal with Python's default context (28 digits, half-even rounding).

The binary also carries what Python gained on main after 0dd9c53: `demo`
(PR 147), whose synthetic model and data are built into the binary rather than
shipped as package data; `report --format json` and `--format csv` (PR 152,
schema `profit-and-loss.v1`); and the native comparison manifest's checks
(PR 146), now in `tests/calculations.rs`. These print the same bytes as Python
at main 99a3704, and the argparse section of the golden file was regenerated
there.

`pacioliscube mcp MODEL_DIR --data DIR` is new in the Rust binary: a read-only
Model Context Protocol server on stdio, built on the official Rust SDK (rmcp),
with tools to list cubes and dimensions, read a cube slice, explain rules and
their feeders, explain cells and render the profit and loss. No tool takes a
path. It is handled before the argument parser, so the top-level help text
still matches 0.3.0.

Rule explanations retain source coordinates in local and cross-cube feeders,
keep repeated source coordinates together, resolve carried element names in
the target hierarchy and preserve quoted semicolons and hashes in statement
excerpts, including distinct statements sharing a line. Every tool rejects
unknown input fields, including `list_cubes`. Cell references enforce the
256-character limit on each cube and element name before reading the model.

A differential harness ran 0.3.0 at revision 0dd9c53 and this binary over
thousands of generated model trees, rules files, CSV inputs and command lines
and found no difference beyond those listed. `tests/data/python_golden.json`
records how 0.3.0's libraries behaved (decimal arithmetic and formatting, the
JSON reader, string case rules, `pathlib`, the chunked UTF-8 reader with the
`csv` module, `difflib`, `textwrap` and `argparse`), and the tests replay it on
every platform.

### Command line

The added subcommands are `demo` from main and the stdio `mcp` server above.
The Python library calls map
onto the command line:

| 0.3.0 Python API | Use instead |
| --- | --- |
| `pacioliscube.cli.main`, `build_parser` | the `pacioliscube` binary |
| `model.load_model`, `load_dimension`, `load_cube`, `load_process`, `ModelError` | `validate`, which loads the whole tree and refuses it with the same messages and exit 2 |
| `rules.parse_rules`, `RuleSyntaxError` | `validate`: rules files are parsed when the model loads |
| `validate.validate_model`, `Finding` | `validate` |
| `data.load_csv`, `load_into_store`, `load_data`, `cube_for_file`, `CUBE_BY_STEM` | `--data` on `calculate`, `explain`, `report`; `--previous-data` and `--current-data` on `compare` |
| `evaluate.evaluate`, `consolidate_many`, `CellStore` | `calculate` |
| `evaluate.explain` | `explain` |
| `report.money`, `rows`, `lines`, `fixed_slice` | `report` |
| `errors.CliError`, `EXIT_OK`, `EXIT_USAGE`, `EXIT_INVALID_MODEL`, `EXIT_CALCULATION` | exit codes 0 to 3, unchanged |
| `benchmarks/evaluate.py` | `cargo bench --bench evaluate`, which times this engine; compare two revisions' reports and fingerprints rather than importing a baseline file |
| `fuzz/rules_parser.py` (Atheris) | the cargo-fuzz target `rules_parser` |

### Documented divergences

- **Version.** `--version` prints `pacioliscube 1.0.0`.
- **Unhandled errors.** Where 0.3.0 let an exception escape, it printed a
  traceback and exited 1. The binary prints what 0.3.0 printed before the
  traceback, then one line, `pacioliscube: unexpected error: <the traceback's
  last line>`, and exits 1. The cases are: `report` rounding a figure of more
  than 28 digits (`decimal.InvalidOperation`); `compare` subtracting two
  figures whose difference overflows (`decimal.Overflow`); a CSV field longer
  than 131,072 characters (`_csv.Error`); a process `Parameters` value that
  is not a list, or a parameter that is not an object (`TypeError`,
  `AttributeError`); a figure too long to print in full (`MemoryError`); and,
  on Linux, a model link holding a NUL character (`ValueError`).
- **Recursion depth.** 0.3.0 ended in a `RecursionError` traceback on a
  hierarchy or rule chain deeper than its interpreter's recursion limit (from
  about 300 levels, depending on the path). The binary evaluates and validates
  such models; it has no recursion limit short of a 1 GiB stack. The refusals
  0.3.0 handled keep their limits: a rules file is refused as nested too
  deeply at the depth 0.3.0 refused it (counted through each platform's console
  script, including Windows' two additional launcher frames), and a JSON file
  at the stack depth CPython 3.15.0's scanner reached
  through the console script: inside a manifest's own object, 13,855 nested
  arrays or 15,587 nested objects on Windows, and about 65,208 of either on
  Linux, where Python's limit moved by a few levels between runs. A JSON
  document within a level (Windows) or about ten levels (Linux) of that limit
  may be judged differently.
- **Lone surrogates.** A JSON `\u` escape for a lone UTF-16 surrogate (U+D800
  to U+DFFF) is refused as invalid JSON when the model loads. 0.3.0 kept the
  character, and failed only if it later printed it as text, for example in
  an element name; an unused field passed.
- **Non-UTF-8 names.** A command line argument, or a file name the engine
  reads from the file system (a model or data directory entry, a symbolic
  link's target, the working directory), that is not Unicode text is refused
  with an unexpected-error line (`UnicodeError`) and exit 1. Reading it under
  replacement characters instead could name a different file and change a
  figure. 0.3.0 carried such bytes through with `surrogateescape`. A
  non-CSV file in a data directory is still ignored, whatever its name.
- **Interleaving.** Where stdout and stderr go to the same file, the order of
  their lines may differ from 0.3.0's, whose buffer sizes it does not copy.
  Each stream on its own is identical.
- **Python environment variables.** Output is always UTF-8, which is also
  Python 3.15's default; `PYTHONIOENCODING` is not read. Two variables are
  read as 0.3.0's interpreter read them: `PYTHONUNBUFFERED` makes stdout
  unbuffered, so a failed write ends the command with exit 1 rather than
  failing the final flush with exit 120, and `PYTHONINTMAXSTRDIGITS` sets
  the digit limit for JSON integers (0 for none; an invalid value stops the
  binary with CPython's start-up error and exit 1).
- **Demo storage.** `demo` writes its built-in model and data to a new
  private temporary directory and removes it afterwards, so it needs
  writable temporary storage; 0.3.0 read its package data in place.
- **Operating-system messages.** File errors are worded as Python worded them
  for the common cases (missing file, permission refused, sharing violation);
  unusual error codes may be worded differently. On Windows, a path that
  cannot be opened at all keeps the case it was written in when resolved,
  where Python looked up the name's case on disk.

### Checks

- CI runs `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D
  warnings` and the tests on Ubuntu and Windows. Line coverage is measured on
  Ubuntu with cargo-llvm-cov and gated at 90%. 0.3.0 was gated at 100% branch coverage of the package, with 100%
  changed-line branch coverage for `validate.py`.
- The dependency audit runs cargo-audit against the RustSec database for both
  lock files. The package job installs the release binary and runs it on the
  shipped model from outside the checkout.
- The tm1gitpy conformance job installs the hash-pinned tm1gitpy in
  `ci/requirements-tm1git.txt`, writes its reading of `model/` as JSON with
  `ci/tm1gitpy_model.py`, and a Rust test requires the engine to agree.
- Fuzzing moves from Atheris to cargo-fuzz on a pinned nightly toolchain.
- CodeQL analyses Rust in a read-only job. A separate job uploads its SARIF
  results with `security-events: write`; dependency build scripts never run
  with that write permission.
- Dependabot updates the Cargo locks, the tm1gitpy requirements and the
  actions. Snyk tests the Cargo dependencies through a CycloneDX SBOM.
- The pre-commit hooks run the same cargo commands as CI.
- Releases are source archives built by release-policy's `release-archive`
  workflow, which needs the `VERSION` file and runs
  `tests/test_source_archive.py`. No binary asset or PyPI upload is
  published; build the binary from the archive with Cargo.
- `.github/ci/check_gates.py` stays in Python: it is a gate helper shared
  byte for byte with other repositories.

### Tests

Every pytest module's behaviour moved to a Rust test file:

| 0.3.0 tests | Rust tests |
| --- | --- |
| `test_cli.py`, `test_compare.py` | `tests/cli.rs`, `tests/engine.rs` |
| `test_rules_parser.py`, `test_data_loading.py`, `test_evaluate.py`, `test_explain.py`, `test_rule_matching.py`, `test_reference_resolution.py`, `test_properties.py` | `tests/engine.rs`, and the reference-caching tests in `src/evaluate.rs` |
| `test_model_manifest.py`, `test_model_dimensions.py`, `test_model_content.py`, `test_tm1_source_form.py`, `test_model_rules.py` | `tests/model.rs` |
| `test_validate.py`, `test_error_paths.py` | `tests/validate.rs`, `tests/model.rs` |
| `test_calculations.py`, `test_conservation.py` | `tests/calculations.rs` |
| `test_fuzz_harness.py`, `test_release_checks.py`, `test_diff_coverage_policy.py`, `test_package_lifecycle.py` | `tests/repository.rs` |
| `test_benchmark.py` | the benchmark's own run-time checks in `benches/evaluate.rs` |

Hypothesis properties became seeded loops with the same seeds and example
counts. A few 0.3.0 tests have no counterpart because Rust's types rule out
the state they guarded: a float stored in a `CellStore`, an unsupported
operator, expression node or comparison reaching the evaluator, a source tree
without installed package metadata, and a changed `decimal` context (the
binary always uses Python's default context). Tests that injected an
operating-system error by patching now raise a real one: a locked file on
Windows, a file without read permission on Linux.
